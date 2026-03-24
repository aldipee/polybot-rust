#!/usr/bin/env python3
"""
Verify the gap_side hypothesis:
  gap_side = vwap_side_prev - trade_price_side

Claims to verify:
1. gap>0 and delta_sum<0 should dominate (trade below VWAP pulls pair sum down)
2. First recovery trade should always have gap>0 and delta_sum<0
3. First recovery on opposite side of crossing trade
4. First recovery on lagging side

Also check the proposed impact formula:
  impact(side,size) = size * (vwap_side_prev - price_side) / (qty_side_prev + size)
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

# Counters
total_crossed_above = 0
total_recovered = 0
total_unrecovered = 0

gap_pos_delta_neg = 0  # gap>0 and delta_sum<0 (repair)
gap_neg_delta_pos = 0  # gap<0 and delta_sum>0 (deterioration)
sign_violations = 0    # gap and delta have same unexpected sign
total_steps_above = 0

first_recovery_gap_pos = 0
first_recovery_gap_neg = 0
first_recovery_on_opposite_side = 0
first_recovery_on_same_side = 0
first_recovery_on_lagging_side = 0
first_recovery_total = 0

late_cross_recovered = 0
late_cross_unrecovered = 0

for cid in market_pnl["conditionId"].unique():
    mkt_trades = trades[trades["conditionId"] == cid]
    up_t = mkt_trades[mkt_trades["outcome"] == "Up"]
    dn_t = mkt_trades[mkt_trades["outcome"] == "Down"]
    if len(up_t) == 0 or len(dn_t) == 0:
        continue

    all_t = mkt_trades[["timestamp", "outcome", "price", "size", "t_into_s"]].copy()
    all_t = all_t.sort_values("timestamp").reset_index(drop=True)

    up_cum_cost = 0.0
    up_cum_qty = 0.0
    dn_cum_cost = 0.0
    dn_cum_qty = 0.0

    timeline = []
    for _, t in all_t.iterrows():
        # Store prev state
        prev_up_vwap = up_cum_cost / up_cum_qty if up_cum_qty > 0 else None
        prev_dn_vwap = dn_cum_cost / dn_cum_qty if dn_cum_qty > 0 else None
        prev_up_qty = up_cum_qty
        prev_dn_qty = dn_cum_qty

        if t["outcome"] == "Up":
            up_cum_cost += t["price"] * t["size"]
            up_cum_qty += t["size"]
        else:
            dn_cum_cost += t["price"] * t["size"]
            dn_cum_qty += t["size"]

        up_vwap = up_cum_cost / up_cum_qty if up_cum_qty > 0 else None
        dn_vwap = dn_cum_cost / dn_cum_qty if dn_cum_qty > 0 else None

        pair_sum = (up_vwap + dn_vwap) if (up_vwap is not None and dn_vwap is not None) else None
        prev_pair_sum = (prev_up_vwap + prev_dn_vwap) if (prev_up_vwap is not None and prev_dn_vwap is not None) else None

        # Compute gap_side
        if t["outcome"] == "Up" and prev_up_vwap is not None:
            gap_side = prev_up_vwap - t["price"]
            prev_side_qty = prev_up_qty
        elif t["outcome"] == "Down" and prev_dn_vwap is not None:
            gap_side = prev_dn_vwap - t["price"]
            prev_side_qty = prev_dn_qty
        else:
            gap_side = None
            prev_side_qty = 0

        # Compute delta_sum
        delta_sum = (pair_sum - prev_pair_sum) if (pair_sum is not None and prev_pair_sum is not None) else None

        # Compute impact
        impact = None
        if gap_side is not None and prev_side_qty > 0:
            impact = t["size"] * gap_side / (prev_side_qty + t["size"])

        # Lighter side at this moment
        lighter = None
        if up_cum_qty > 0 and dn_cum_qty > 0:
            lighter = "Up" if up_cum_qty < dn_cum_qty else "Down"

        timeline.append({
            "t_into_s": t["t_into_s"],
            "side": t["outcome"],
            "price": t["price"],
            "size": t["size"],
            "pair_sum": pair_sum,
            "prev_pair_sum": prev_pair_sum,
            "delta_sum": delta_sum,
            "gap_side": gap_side,
            "impact": impact,
            "lighter_side": lighter,
            "up_qty": up_cum_qty,
            "dn_qty": dn_cum_qty,
        })

    tl = pd.DataFrame(timeline)
    tl_paired = tl[tl["pair_sum"].notna()].reset_index(drop=True)
    if len(tl_paired) == 0:
        continue

    ps = tl_paired["pair_sum"].values

    # Find all crossing-above events and recovery events
    i = 0
    while i < len(ps):
        # Find crossing above 1.0
        if i > 0 and ps[i-1] < 1.0 and ps[i] >= 1.0:
            total_crossed_above += 1
            crossing_trade_side = tl_paired.iloc[i]["side"]
            crossing_t = tl_paired.iloc[i]["t_into_s"]
            is_late = crossing_t is not None and crossing_t >= 240

            # Track all steps while above 1.0
            j = i
            recovered = False
            first_recovery_idx = None
            while j < len(ps):
                if ps[j] < 1.0:
                    recovered = True
                    first_recovery_idx = j
                    break

                # Count gap/delta signs for steps above 1.0
                row = tl_paired.iloc[j]
                if row["gap_side"] is not None and row["delta_sum"] is not None:
                    total_steps_above += 1
                    g = row["gap_side"]
                    d = row["delta_sum"]
                    if g > 1e-9 and d < -1e-9:
                        gap_pos_delta_neg += 1
                    elif g < -1e-9 and d > 1e-9:
                        gap_neg_delta_pos += 1
                    elif abs(g) <= 1e-9 or abs(d) <= 1e-9:
                        sign_violations += 1  # rounding/noise
                    else:
                        sign_violations += 1  # same sign (gap>0,delta>0 or gap<0,delta<0)
                j += 1

            if recovered:
                total_recovered += 1
                if is_late:
                    late_cross_recovered += 1
                # Check first recovery trade
                rec_row = tl_paired.iloc[first_recovery_idx]
                first_recovery_total += 1
                if rec_row["gap_side"] is not None and rec_row["gap_side"] > 1e-9:
                    first_recovery_gap_pos += 1
                else:
                    first_recovery_gap_neg += 1
                # Was recovery on opposite side of crossing trade?
                if rec_row["side"] != crossing_trade_side:
                    first_recovery_on_opposite_side += 1
                else:
                    first_recovery_on_same_side += 1
                # Was recovery on lagging side?
                if rec_row["lighter_side"] is not None and rec_row["side"] == rec_row["lighter_side"]:
                    first_recovery_on_lagging_side += 1
            else:
                total_unrecovered += 1
                if is_late:
                    late_cross_unrecovered += 1

            i = j if j < len(ps) else len(ps)
        else:
            i += 1

print("=" * 100)
print("VERIFICATION OF gap_side HYPOTHESIS")
print("=" * 100)

print(f"\n### MARKET COUNTS ###")
print(f"Crossed above 1.00: {total_crossed_above}")
print(f"Recovered below 1.00: {total_recovered}")
print(f"Unrecovered: {total_unrecovered}")

print(f"\n### TRADE-STEP SIGN RULE (while above 1.00) ###")
print(f"Total steps above 1.00: {total_steps_above}")
print(f"gap>0 AND delta_sum<0 (repair):       {gap_pos_delta_neg} ({gap_pos_delta_neg/total_steps_above*100:.1f}%)")
print(f"gap<0 AND delta_sum>0 (deterioration): {gap_neg_delta_pos} ({gap_neg_delta_pos/total_steps_above*100:.1f}%)")
print(f"Sign violations (rounding/noise):      {sign_violations} ({sign_violations/total_steps_above*100:.1f}%)")
print(f"Sum check: {gap_pos_delta_neg + gap_neg_delta_pos + sign_violations} = {total_steps_above}")

print(f"\n### FIRST RECOVERY TRADE ###")
print(f"Total first-recovery events: {first_recovery_total}")
print(f"gap>0 (bought below VWAP): {first_recovery_gap_pos}/{first_recovery_total} ({first_recovery_gap_pos/first_recovery_total*100:.1f}%)")
print(f"gap<=0 (bought at/above VWAP): {first_recovery_gap_neg}/{first_recovery_total} ({first_recovery_gap_neg/first_recovery_total*100:.1f}%)")
print(f"On OPPOSITE side of crossing trade: {first_recovery_on_opposite_side}/{first_recovery_total} ({first_recovery_on_opposite_side/first_recovery_total*100:.1f}%)")
print(f"On SAME side as crossing trade: {first_recovery_on_same_side}/{first_recovery_total} ({first_recovery_on_same_side/first_recovery_total*100:.1f}%)")
print(f"On LAGGING (lighter) side: {first_recovery_on_lagging_side}/{first_recovery_total} ({first_recovery_on_lagging_side/first_recovery_total*100:.1f}%)")

print(f"\n### LATE CROSSINGS (240-300s) ###")
print(f"Recovered: {late_cross_recovered}")
print(f"Unrecovered: {late_cross_unrecovered}")

print(f"\n### MATHEMATICAL TAUTOLOGY CHECK ###")
print("""
Is gap>0 → delta_sum<0 a tautology?

When you buy side S at price P with size Q:
  new_vwap_S = (old_vwap_S * old_qty_S + P * Q) / (old_qty_S + Q)
  delta_vwap_S = new_vwap_S - old_vwap_S = Q * (P - old_vwap_S) / (old_qty_S + Q)

If gap_side = old_vwap_S - P > 0 (bought below VWAP):
  → P < old_vwap_S
  → delta_vwap_S < 0 (VWAP decreases)
  → delta_pair_sum = delta_vwap_S (other side unchanged) < 0

So YES: gap>0 → delta_sum<0 is a MATHEMATICAL TAUTOLOGY.
It's not a discovery about Vidardx's strategy — it's algebra.
Every trade below the current side VWAP will decrease the pair sum.
Every trade above the current side VWAP will increase the pair sum.

The gap_side metric IS the impact formula, just without the size weighting.
""")

# Now check the REAL question: does Vidardx's trade distribution differ
# above vs below 1.00 in terms of gap_side?
print("### REAL QUESTION: Does Vidardx trade differently above 1.00? ###")

all_above_gaps = []
all_below_gaps = []

for cid in market_pnl["conditionId"].unique():
    mkt_trades = trades[trades["conditionId"] == cid]
    up_t = mkt_trades[mkt_trades["outcome"] == "Up"]
    dn_t = mkt_trades[mkt_trades["outcome"] == "Down"]
    if len(up_t) == 0 or len(dn_t) == 0:
        continue

    all_t = mkt_trades[["timestamp", "outcome", "price", "size", "t_into_s"]].copy()
    all_t = all_t.sort_values("timestamp").reset_index(drop=True)

    up_cum_cost = 0.0
    up_cum_qty = 0.0
    dn_cum_cost = 0.0
    dn_cum_qty = 0.0

    for _, t in all_t.iterrows():
        prev_up_vwap = up_cum_cost / up_cum_qty if up_cum_qty > 0 else None
        prev_dn_vwap = dn_cum_cost / dn_cum_qty if dn_cum_qty > 0 else None

        if t["outcome"] == "Up":
            up_cum_cost += t["price"] * t["size"]
            up_cum_qty += t["size"]
        else:
            dn_cum_cost += t["price"] * t["size"]
            dn_cum_qty += t["size"]

        up_vwap = up_cum_cost / up_cum_qty if up_cum_qty > 0 else None
        dn_vwap = dn_cum_cost / dn_cum_qty if dn_cum_qty > 0 else None

        if up_vwap is None or dn_vwap is None:
            continue

        pair_sum = up_vwap + dn_vwap

        if t["outcome"] == "Up" and prev_up_vwap is not None:
            gap = prev_up_vwap - t["price"]
        elif t["outcome"] == "Down" and prev_dn_vwap is not None:
            gap = prev_dn_vwap - t["price"]
        else:
            continue

        record = {"gap": gap, "size": t["size"], "price": t["price"]}
        if pair_sum >= 1.0:
            all_above_gaps.append(record)
        else:
            all_below_gaps.append(record)

above_gaps = pd.DataFrame(all_above_gaps)
below_gaps = pd.DataFrame(all_below_gaps)

print(f"\nAbove 1.00 ({len(above_gaps)} trades):")
print(f"  Avg gap_side:    {above_gaps['gap'].mean():.6f}")
print(f"  Median gap_side: {above_gaps['gap'].median():.6f}")
print(f"  % with gap > 0:  {(above_gaps['gap'] > 1e-9).mean()*100:.1f}%")
print(f"  Avg |gap| when gap>0: {above_gaps[above_gaps['gap'] > 1e-9]['gap'].mean():.4f}")

print(f"\nBelow 1.00 ({len(below_gaps)} trades):")
print(f"  Avg gap_side:    {below_gaps['gap'].mean():.6f}")
print(f"  Median gap_side: {below_gaps['gap'].median():.6f}")
print(f"  % with gap > 0:  {(below_gaps['gap'] > 1e-9).mean()*100:.1f}%")
print(f"  Avg |gap| when gap>0: {below_gaps[below_gaps['gap'] > 1e-9]['gap'].mean():.4f}")

print(f"\n### VERDICT ###")
print(f"If Vidardx actively targets gap>0 trades when above 1.00,")
print(f"we'd see a HIGHER gap>0 fraction above 1.00 vs below.")
print(f"Actual: above={( above_gaps['gap'] > 1e-9).mean()*100:.1f}% vs below={(below_gaps['gap'] > 1e-9).mean()*100:.1f}%")
