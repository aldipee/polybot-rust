#!/usr/bin/env python3
"""
What metric does Vidardx implicitly optimize while allowing pair sum > 1.00?

For each market, trace trade-by-trade:
1. Inventory imbalance (|q_up - q_down| / (q_up + q_down))
2. Marginal cost of lagging side (price of each trade on the lighter side)
3. Time-awareness (do they trade differently early vs late?)
4. Pair sum convergence trajectory (how does pair sum move over time?)

Focus on the ABOVE-1.00 periods: what is the bot doing during those trades?
"""
import pandas as pd
import numpy as np

trades = pd.read_parquet("/home/user/polybot-rust/dataset/vidarx_trade_all.parquet")
positions = pd.read_csv("/home/user/polybot-rust/dataset/vidarx_trade_all_closed_position.csv")

trades = trades[trades["side"] == "BUY"].copy()
trades = trades.sort_values(["conditionId", "timestamp"])

market_pnl = positions.groupby("conditionId").agg(
    total_pnl=("realizedPnl", "sum"),
).reset_index()

def analyze_market_detailed(condition_id, market_trades):
    up_trades = market_trades[market_trades["outcome"] == "Up"]
    down_trades = market_trades[market_trades["outcome"] == "Down"]
    if len(up_trades) == 0 or len(down_trades) == 0:
        return None

    all_trades = market_trades[["timestamp", "outcome", "price", "size", "t_into_s"]].copy()
    all_trades = all_trades.sort_values("timestamp")

    up_cum_cost = 0.0
    up_cum_qty = 0.0
    down_cum_cost = 0.0
    down_cum_qty = 0.0

    records = []
    for _, t in all_trades.iterrows():
        if t["outcome"] == "Up":
            up_cum_cost += t["price"] * t["size"]
            up_cum_qty += t["size"]
        else:
            down_cum_cost += t["price"] * t["size"]
            down_cum_qty += t["size"]

        if up_cum_qty > 0 and down_cum_qty > 0:
            up_vwap = up_cum_cost / up_cum_qty
            down_vwap = down_cum_cost / down_cum_qty
            pair_sum = up_vwap + down_vwap
            total_qty = up_cum_qty + down_cum_qty
            imbalance = abs(up_cum_qty - down_cum_qty) / total_qty
            lighter_side = "Up" if up_cum_qty < down_cum_qty else "Down"
            heavier_side = "Down" if lighter_side == "Up" else "Up"
            is_lighter_side_trade = t["outcome"] == lighter_side
            is_heavier_side_trade = t["outcome"] == heavier_side

            records.append({
                "t_into_s": t["t_into_s"],
                "side": t["outcome"],
                "price": t["price"],
                "size": t["size"],
                "up_vwap": up_vwap,
                "down_vwap": down_vwap,
                "pair_sum": pair_sum,
                "up_qty": up_cum_qty,
                "down_qty": down_cum_qty,
                "imbalance": imbalance,
                "lighter_side": lighter_side,
                "is_lighter_side_trade": is_lighter_side_trade,
                "is_heavier_side_trade": is_heavier_side_trade,
                "above_1": pair_sum >= 1.0,
            })

    return pd.DataFrame(records) if records else None


# Collect stats across all markets
all_above_1_trades = []
all_below_1_trades = []
all_crossing_moments = []
market_behavior = []

for _, mkt in market_pnl.iterrows():
    cid = mkt["conditionId"]
    mkt_trades = trades[trades["conditionId"] == cid]
    tl = analyze_market_detailed(cid, mkt_trades)
    if tl is None or len(tl) == 0:
        continue

    above = tl[tl["above_1"]]
    below = tl[~tl["above_1"]]

    if len(above) > 0:
        all_above_1_trades.append(above)
    if len(below) > 0:
        all_below_1_trades.append(below)

    # Track crossing moments
    ps = tl["pair_sum"].values
    for i in range(1, len(ps)):
        if ps[i-1] < 1.0 and ps[i] >= 1.0:
            all_crossing_moments.append({
                "direction": "cross_above",
                "imbalance_before": tl.iloc[i-1]["imbalance"],
                "imbalance_at": tl.iloc[i]["imbalance"],
                "pair_sum": ps[i],
                "t_into_s": tl.iloc[i]["t_into_s"],
                "trade_side": tl.iloc[i]["side"],
                "is_lighter": tl.iloc[i]["is_lighter_side_trade"],
                "pnl": mkt["total_pnl"],
            })
        elif ps[i-1] >= 1.0 and ps[i] < 1.0:
            all_crossing_moments.append({
                "direction": "cross_below",
                "imbalance_before": tl.iloc[i-1]["imbalance"],
                "imbalance_at": tl.iloc[i]["imbalance"],
                "pair_sum": ps[i],
                "t_into_s": tl.iloc[i]["t_into_s"],
                "trade_side": tl.iloc[i]["side"],
                "is_lighter": tl.iloc[i]["is_lighter_side_trade"],
                "pnl": mkt["total_pnl"],
            })

    # Per-market summary
    market_behavior.append({
        "pnl": mkt["total_pnl"],
        "ever_above": len(above) > 0,
        "pct_above": len(above) / len(tl) * 100 if len(tl) > 0 else 0,
        "final_pair_sum": tl.iloc[-1]["pair_sum"],
        "final_imbalance": tl.iloc[-1]["imbalance"],
        "above_lighter_frac": above["is_lighter_side_trade"].mean() if len(above) > 0 else None,
        "below_lighter_frac": below["is_lighter_side_trade"].mean() if len(below) > 0 else None,
        "above_avg_price": above["price"].mean() if len(above) > 0 else None,
        "below_avg_price": below["price"].mean() if len(below) > 0 else None,
        "above_avg_size": above["size"].mean() if len(above) > 0 else None,
        "below_avg_size": below["size"].mean() if len(below) > 0 else None,
    })

above_df = pd.concat(all_above_1_trades) if all_above_1_trades else pd.DataFrame()
below_df = pd.concat(all_below_1_trades) if all_below_1_trades else pd.DataFrame()
crossings_df = pd.DataFrame(all_crossing_moments) if all_crossing_moments else pd.DataFrame()
mkt_df = pd.DataFrame(market_behavior)

print("=" * 100)
print("WHAT DOES VIDARDX OPTIMIZE DURING PAIR SUM > 1.00 PERIODS?")
print("=" * 100)

# 1. LIGHTER SIDE TARGETING
print("\n### 1. LIGHTER SIDE TARGETING ###")
print(f"When pair_sum >= 1.00:")
print(f"  Trades on LIGHTER (lagging) side: {above_df['is_lighter_side_trade'].mean()*100:.1f}%")
print(f"  Trades on HEAVIER side:           {above_df['is_heavier_side_trade'].mean()*100:.1f}%")
print(f"When pair_sum < 1.00:")
print(f"  Trades on LIGHTER (lagging) side: {below_df['is_lighter_side_trade'].mean()*100:.1f}%")
print(f"  Trades on HEAVIER side:           {below_df['is_heavier_side_trade'].mean()*100:.1f}%")

# 2. MARGINAL COST: what price do they pay during above-1.00 periods?
print("\n### 2. MARGINAL COST OF TRADES ###")
above_lighter = above_df[above_df["is_lighter_side_trade"]]
above_heavier = above_df[above_df["is_heavier_side_trade"]]
below_lighter = below_df[below_df["is_lighter_side_trade"]]
below_heavier = below_df[below_df["is_heavier_side_trade"]]

print(f"When pair_sum >= 1.00:")
print(f"  Lighter side trades — avg price: {above_lighter['price'].mean():.4f}, median: {above_lighter['price'].median():.4f}, avg size: {above_lighter['size'].mean():.1f}")
print(f"  Heavier side trades — avg price: {above_heavier['price'].mean():.4f}, median: {above_heavier['price'].median():.4f}, avg size: {above_heavier['size'].mean():.1f}")
print(f"When pair_sum < 1.00:")
print(f"  Lighter side trades — avg price: {below_lighter['price'].mean():.4f}, median: {below_lighter['price'].median():.4f}, avg size: {below_lighter['size'].mean():.1f}")
print(f"  Heavier side trades — avg price: {below_heavier['price'].mean():.4f}, median: {below_heavier['price'].median():.4f}, avg size: {below_heavier['size'].mean():.1f}")

# 3. IMBALANCE DURING ABOVE-1.00 PERIODS
print("\n### 3. INVENTORY IMBALANCE ###")
print(f"Average imbalance when pair_sum >= 1.00: {above_df['imbalance'].mean():.4f}")
print(f"Average imbalance when pair_sum < 1.00:  {below_df['imbalance'].mean():.4f}")
print(f"Median imbalance when pair_sum >= 1.00:  {above_df['imbalance'].median():.4f}")
print(f"Median imbalance when pair_sum < 1.00:   {below_df['imbalance'].median():.4f}")

# 4. CROSSING ANALYSIS: what causes crosses above and back below?
print("\n### 4. CROSSING ANALYSIS ###")
if len(crossings_df) > 0:
    cross_above = crossings_df[crossings_df["direction"] == "cross_above"]
    cross_below = crossings_df[crossings_df["direction"] == "cross_below"]
    print(f"Crosses ABOVE 1.00 ({len(cross_above)} events):")
    print(f"  Caused by lighter side trade: {cross_above['is_lighter'].mean()*100:.1f}%")
    print(f"  Caused by heavier side trade: {(~cross_above['is_lighter']).mean()*100:.1f}%")
    print(f"  Avg imbalance at crossing:    {cross_above['imbalance_at'].mean():.4f}")
    print(f"  Avg t_into_s at crossing:     {cross_above['t_into_s'].mean():.1f}s")
    print(f"Crosses BACK BELOW 1.00 ({len(cross_below)} events):")
    print(f"  Caused by lighter side trade: {cross_below['is_lighter'].mean()*100:.1f}%")
    print(f"  Caused by heavier side trade: {(~cross_below['is_lighter']).mean()*100:.1f}%")
    print(f"  Avg imbalance at crossing:    {cross_below['imbalance_at'].mean():.4f}")
    print(f"  Avg t_into_s at crossing:     {cross_below['t_into_s'].mean():.1f}s")

# 5. TIME AWARENESS: does behavior change over the window?
print("\n### 5. TIME SEGMENTATION ###")
for t_start, t_end, label in [(0, 60, "0-60s"), (60, 120, "60-120s"), (120, 180, "120-180s"), (180, 240, "180-240s"), (240, 300, "240-300s")]:
    above_seg = above_df[(above_df["t_into_s"] >= t_start) & (above_df["t_into_s"] < t_end)]
    below_seg = below_df[(below_df["t_into_s"] >= t_start) & (below_df["t_into_s"] < t_end)]
    total_seg_above = len(above_seg)
    total_seg_below = len(below_seg)
    total = total_seg_above + total_seg_below
    if total == 0:
        continue
    lighter_above = above_seg["is_lighter_side_trade"].mean() * 100 if total_seg_above > 0 else 0
    avg_price_above = above_seg["price"].mean() if total_seg_above > 0 else 0
    avg_imb_above = above_seg["imbalance"].mean() if total_seg_above > 0 else 0
    print(f"  {label}: above_1={total_seg_above:>5} trades, lighter_frac={lighter_above:.1f}%, avg_price={avg_price_above:.3f}, avg_imbalance={avg_imb_above:.3f}")

# 6. PAIR SUM DELTA PER TRADE: what moves pair sum down?
print("\n### 6. PAIR SUM MOVEMENT PER TRADE ###")
# For above-1.00 periods, what is the average pair_sum change per trade?
for df_label, df in [("Above 1.00", above_df), ("Below 1.00", below_df)]:
    if len(df) < 2:
        continue
    lighter_trades = df[df["is_lighter_side_trade"]]
    heavier_trades = df[df["is_heavier_side_trade"]]
    # Can't compute delta easily across markets, but we can look at price vs vwap
    # When you buy lighter side at price P, the lighter VWAP changes.
    # A lighter side trade at price < current_lighter_vwap pulls pair sum DOWN.
    # A heavier side trade at price > current_heavier_vwap pushes pair sum UP.
    if len(lighter_trades) > 0:
        # How does lighter side trade price compare to the lighter side's VWAP at that moment?
        lighter_below_vwap = 0
        lighter_above_vwap = 0
        for _, row in lighter_trades.iterrows():
            if row["lighter_side"] == "Up":
                vwap = row["up_vwap"]
            else:
                vwap = row["down_vwap"]
            if row["price"] < vwap:
                lighter_below_vwap += 1
            else:
                lighter_above_vwap += 1
        total_l = lighter_below_vwap + lighter_above_vwap
        print(f"  {df_label} — Lighter side trades below own VWAP: {lighter_below_vwap}/{total_l} ({lighter_below_vwap/total_l*100:.1f}%)")

# 7. SIZE COMPARISON: are above-1.00 trades larger or smaller?
print("\n### 7. TRADE SIZE COMPARISON ###")
print(f"Average trade size when pair_sum >= 1.00: {above_df['size'].mean():.1f} shares")
print(f"Average trade size when pair_sum <  1.00: {below_df['size'].mean():.1f} shares")
print(f"Median trade size when pair_sum >= 1.00:  {above_df['size'].median():.1f} shares")
print(f"Median trade size when pair_sum <  1.00:  {below_df['size'].median():.1f} shares")
print(f"Lighter side avg size above 1.00: {above_lighter['size'].mean():.1f}")
print(f"Lighter side avg size below 1.00: {below_lighter['size'].mean():.1f}")

# 8. KEY QUESTION: What brings it back? Price or quantity?
print("\n### 8. CONVERGENCE MECHANISM: PRICE vs QUANTITY ###")
# When pair_sum is above 1.00, does Vidardx:
# (a) Buy the lighter side CHEAPLY (below its VWAP) — price-driven VWAP dilution
# (b) Buy the lighter side in LARGE SIZE — quantity-driven VWAP dilution
# (c) STOP buying the heavier side — preventing further deterioration
print("During above-1.00 periods:")
heavier_count = len(above_heavier)
lighter_count = len(above_lighter)
total_above = len(above_df)
print(f"  Total trades:         {total_above}")
print(f"  Lighter side trades:  {lighter_count} ({lighter_count/total_above*100:.1f}%)")
print(f"  Heavier side trades:  {heavier_count} ({heavier_count/total_above*100:.1f}%)")
if lighter_count > 0:
    print(f"  Lighter side avg price:   {above_lighter['price'].mean():.4f}")
    print(f"  Lighter side avg size:    {above_lighter['size'].mean():.1f}")
if heavier_count > 0:
    print(f"  Heavier side avg price:   {above_heavier['price'].mean():.4f}")
    print(f"  Heavier side avg size:    {above_heavier['size'].mean():.1f}")

# What fraction of lighter trades during above-1.00 are below the CURRENT lighter vwap?
lighter_below_count = 0
lighter_total = 0
for _, row in above_lighter.iterrows():
    vwap = row["up_vwap"] if row["lighter_side"] == "Up" else row["down_vwap"]
    lighter_total += 1
    if row["price"] < vwap:
        lighter_below_count += 1
if lighter_total > 0:
    print(f"  Lighter side trades BELOW own VWAP: {lighter_below_count}/{lighter_total} ({lighter_below_count/lighter_total*100:.1f}%)")
    print(f"  → These trades PULL pair sum DOWN (VWAP dilution)")

# Does the heavier side STOP during above-1.00?
print("\nDuring below-1.00 periods:")
heavier_below = len(below_heavier)
lighter_below = len(below_lighter)
total_below = len(below_df)
print(f"  Lighter side trades: {lighter_below} ({lighter_below/total_below*100:.1f}%)")
print(f"  Heavier side trades: {heavier_below} ({heavier_below/total_below*100:.1f}%)")

print("\n### CONCLUSION ###")
print("The comparison of lighter-side trade fraction above vs below 1.00 reveals")
print("whether Vidardx SHIFTS its focus to the lagging side during overshoots,")
print("or maintains the same balanced approach regardless of pair sum.")
