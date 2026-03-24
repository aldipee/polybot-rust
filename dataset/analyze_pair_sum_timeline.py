#!/usr/bin/env python3
"""
Trade-level timeline analysis of Vidardx pair sum evolution.
Shows how the running VWAP sum (Up + Down) evolves trade-by-trade,
whether it crosses above 1.00, and how/if it comes back below.
"""
import pandas as pd
import sys

# Load data
trades = pd.read_parquet("/home/user/polybot-rust/dataset/vidarx_trade_all.parquet")
positions = pd.read_csv("/home/user/polybot-rust/dataset/vidarx_trade_all_closed_position.csv")

# Only BUY trades (the analysis said zero sells)
trades = trades[trades["side"] == "BUY"].copy()
trades = trades.sort_values(["conditionId", "timestamp"])

# Get market-level PnL from closed positions
# Group by conditionId to get total PnL per market
market_pnl = positions.groupby("conditionId").agg(
    total_pnl=("realizedPnl", "sum"),
    outcomes=("outcome", list),
).reset_index()

# For each market, compute trade-by-trade running VWAP per side and pair sum
def analyze_market(condition_id, market_trades):
    up_trades = market_trades[market_trades["outcome"] == "Up"].copy()
    down_trades = market_trades[market_trades["outcome"] == "Down"].copy()

    if len(up_trades) == 0 or len(down_trades) == 0:
        return None

    # Build timeline: every trade changes one side's running VWAP
    # We need to replay chronologically
    all_trades = market_trades[["timestamp", "outcome", "price", "size", "t_into_s"]].copy()
    all_trades = all_trades.sort_values("timestamp")

    up_cum_cost = 0.0
    up_cum_qty = 0.0
    down_cum_cost = 0.0
    down_cum_qty = 0.0

    timeline = []

    for _, t in all_trades.iterrows():
        if t["outcome"] == "Up":
            up_cum_cost += t["price"] * t["size"]
            up_cum_qty += t["size"]
        else:
            down_cum_cost += t["price"] * t["size"]
            down_cum_qty += t["size"]

        up_vwap = up_cum_cost / up_cum_qty if up_cum_qty > 0 else None
        down_vwap = down_cum_cost / down_cum_qty if down_cum_qty > 0 else None

        pair_sum = None
        if up_vwap is not None and down_vwap is not None:
            pair_sum = up_vwap + down_vwap

        timeline.append({
            "timestamp": t["timestamp"],
            "t_into_s": t["t_into_s"],
            "side": t["outcome"],
            "price": t["price"],
            "size": t["size"],
            "up_vwap": up_vwap,
            "down_vwap": down_vwap,
            "up_qty": up_cum_qty,
            "down_qty": down_cum_qty,
            "pair_sum": pair_sum,
        })

    return pd.DataFrame(timeline)


# Get profitable and unprofitable markets
market_pnl_sorted = market_pnl.sort_values("total_pnl", ascending=False)
profitable = market_pnl_sorted[market_pnl_sorted["total_pnl"] > 50].head(3)
unprofitable = market_pnl_sorted[market_pnl_sorted["total_pnl"] < -50].tail(3)

print("=" * 120)
print("TRADE-LEVEL PAIR SUM TIMELINE ANALYSIS")
print("=" * 120)

for label, markets in [("PROFITABLE", profitable), ("UNPROFITABLE", unprofitable)]:
    print(f"\n{'#' * 120}")
    print(f"### {label} MARKETS ###")
    print(f"{'#' * 120}")

    for _, mkt in markets.iterrows():
        cid = mkt["conditionId"]
        pnl = mkt["total_pnl"]

        mkt_trades = trades[trades["conditionId"] == cid]
        title = mkt_trades["title"].iloc[0] if len(mkt_trades) > 0 else "?"
        final = mkt_trades["final_outcome"].iloc[0] if "final_outcome" in mkt_trades.columns else "?"

        tl = analyze_market(cid, mkt_trades)
        if tl is None:
            print(f"\n  [{label}] {title} — SKIPPED (single-sided)")
            continue

        # Only rows where both sides have trades (pair_sum exists)
        tl_paired = tl[tl["pair_sum"].notna()]

        if len(tl_paired) == 0:
            continue

        # Key metrics
        max_pair_sum = tl_paired["pair_sum"].max()
        min_pair_sum = tl_paired["pair_sum"].min()
        final_pair_sum = tl_paired.iloc[-1]["pair_sum"]

        # How many times did it cross above 1.00?
        above_1 = tl_paired[tl_paired["pair_sum"] >= 1.0]
        below_1 = tl_paired[tl_paired["pair_sum"] < 1.0]

        # Find crossings
        pair_sums = tl_paired["pair_sum"].values
        crossings_up = 0  # crossed from <1.0 to >=1.0
        crossings_down = 0  # crossed from >=1.0 to <1.0
        for i in range(1, len(pair_sums)):
            if pair_sums[i-1] < 1.0 and pair_sums[i] >= 1.0:
                crossings_up += 1
            elif pair_sums[i-1] >= 1.0 and pair_sums[i] < 1.0:
                crossings_down += 1

        # Time spent above 1.0
        pct_above = len(above_1) / len(tl_paired) * 100

        print(f"\n{'=' * 120}")
        print(f"  MARKET: {title}")
        print(f"  PnL: ${pnl:+.2f} | Final Outcome: {final} | Total Trades: {len(mkt_trades)}")
        print(f"  Final Pair Sum: {final_pair_sum:.4f} | Min: {min_pair_sum:.4f} | Max: {max_pair_sum:.4f}")
        print(f"  Crossed ABOVE 1.00: {crossings_up}x | Crossed BACK below: {crossings_down}x")
        print(f"  Trades above 1.00: {len(above_1)}/{len(tl_paired)} ({pct_above:.1f}%)")
        print(f"  Up qty: {tl_paired.iloc[-1]['up_qty']:.1f} | Down qty: {tl_paired.iloc[-1]['down_qty']:.1f}")
        print(f"  Final Up VWAP: {tl_paired.iloc[-1]['up_vwap']:.4f} | Final Down VWAP: {tl_paired.iloc[-1]['down_vwap']:.4f}")
        print(f"{'=' * 120}")

        # Print key moments in the timeline
        print(f"\n  {'Trade#':>7} | {'t_into_s':>8} | {'Side':>5} | {'Price':>6} | {'Size':>8} | {'Up VWAP':>8} | {'Dn VWAP':>8} | {'PairSum':>8} | Note")
        print(f"  {'-'*7}-+-{'-'*8}-+-{'-'*5}-+-{'-'*6}-+-{'-'*8}-+-{'-'*8}-+-{'-'*8}-+-{'-'*8}-+------")

        # Show: first paired trade, every crossing, moments of max/min, last 3 trades
        # Plus sample every ~50 trades to show progression
        key_indices = set()

        # First paired trade
        key_indices.add(0)

        # Crossings
        for i in range(1, len(pair_sums)):
            if (pair_sums[i-1] < 1.0 and pair_sums[i] >= 1.0) or \
               (pair_sums[i-1] >= 1.0 and pair_sums[i] < 1.0):
                key_indices.add(i-1)
                key_indices.add(i)

        # Max and min pair sum
        key_indices.add(tl_paired["pair_sum"].idxmax() - tl_paired.index[0])
        key_indices.add(tl_paired["pair_sum"].idxmin() - tl_paired.index[0])

        # Sample every N trades
        step = max(1, len(tl_paired) // 15)
        for i in range(0, len(tl_paired), step):
            key_indices.add(i)

        # Last 3
        for i in range(max(0, len(tl_paired)-3), len(tl_paired)):
            key_indices.add(i)

        key_indices = sorted([i for i in key_indices if 0 <= i < len(tl_paired)])

        prev_above = None
        for idx in key_indices:
            row = tl_paired.iloc[idx]
            note = ""
            if row["pair_sum"] >= 1.0 and (prev_above is False or prev_above is None):
                note = ">>> CROSSED ABOVE 1.00"
            elif row["pair_sum"] < 1.0 and prev_above is True:
                note = "<<< CAME BACK BELOW 1.00"

            if idx == tl_paired["pair_sum"].idxmax() - tl_paired.index[0]:
                note += " [MAX]"
            if idx == tl_paired["pair_sum"].idxmin() - tl_paired.index[0]:
                note += " [MIN]"
            if idx == len(tl_paired) - 1:
                note += " [FINAL]"

            prev_above = row["pair_sum"] >= 1.0

            t_into = row["t_into_s"] if pd.notna(row["t_into_s"]) else -1
            print(f"  {idx+1:>7} | {t_into:>8.1f} | {row['side']:>5} | {row['price']:>.4f} | {row['size']:>8.1f} | {row['up_vwap']:>.4f} | {row['down_vwap']:>.4f} | {row['pair_sum']:>.4f} | {note}")

# Now aggregate stats across ALL markets
print(f"\n\n{'#' * 120}")
print("### AGGREGATE: ALL MARKETS PAIR SUM CROSSING ANALYSIS ###")
print(f"{'#' * 120}")

all_market_stats = []
for cid in market_pnl["conditionId"].unique():
    mkt_trades = trades[trades["conditionId"] == cid]
    pnl_row = market_pnl[market_pnl["conditionId"] == cid].iloc[0]

    tl = analyze_market(cid, mkt_trades)
    if tl is None:
        continue
    tl_p = tl[tl["pair_sum"].notna()]
    if len(tl_p) == 0:
        continue

    ps = tl_p["pair_sum"].values
    crossings_up = sum(1 for i in range(1, len(ps)) if ps[i-1] < 1.0 and ps[i] >= 1.0)
    crossings_down = sum(1 for i in range(1, len(ps)) if ps[i-1] >= 1.0 and ps[i] < 1.0)

    ever_above = any(p >= 1.0 for p in ps)
    final_above = ps[-1] >= 1.0
    started_above = ps[0] >= 1.0 if len(ps) > 0 else False

    all_market_stats.append({
        "conditionId": cid,
        "pnl": pnl_row["total_pnl"],
        "profitable": pnl_row["total_pnl"] > 0,
        "final_pair_sum": ps[-1],
        "max_pair_sum": max(ps),
        "min_pair_sum": min(ps),
        "ever_above_1": ever_above,
        "final_above_1": final_above,
        "started_above_1": started_above,
        "crossings_up": crossings_up,
        "crossings_down": crossings_down,
        "came_back": ever_above and not final_above,
        "pct_above": sum(1 for p in ps if p >= 1.0) / len(ps) * 100,
        "total_paired_trades": len(tl_p),
    })

stats = pd.DataFrame(all_market_stats)

print(f"\nTotal markets analyzed: {len(stats)}")
print(f"Markets that EVER crossed above 1.00: {stats['ever_above_1'].sum()} ({stats['ever_above_1'].mean()*100:.1f}%)")
print(f"Markets that FINISHED above 1.00: {stats['final_above_1'].sum()} ({stats['final_above_1'].mean()*100:.1f}%)")
print(f"Markets that crossed above then CAME BACK below: {stats['came_back'].sum()} ({stats['came_back'].mean()*100:.1f}%)")
print(f"Markets that NEVER crossed above 1.00: {(~stats['ever_above_1']).sum()} ({(~stats['ever_above_1']).mean()*100:.1f}%)")

print(f"\n--- PROFITABLE markets (PnL > 0) ---")
prof = stats[stats["profitable"]]
print(f"  Count: {len(prof)}")
print(f"  Ever above 1.00: {prof['ever_above_1'].sum()} ({prof['ever_above_1'].mean()*100:.1f}%)")
print(f"  Came back below: {prof['came_back'].sum()} ({prof['came_back'].mean()*100:.1f}%)")
print(f"  Final above 1.00: {prof['final_above_1'].sum()} ({prof['final_above_1'].mean()*100:.1f}%)")
print(f"  Avg crossings above: {prof['crossings_up'].mean():.1f}")
print(f"  Avg % time above 1.00: {prof['pct_above'].mean():.1f}%")
print(f"  Avg final pair sum: {prof['final_pair_sum'].mean():.4f}")
print(f"  Avg max pair sum: {prof['max_pair_sum'].mean():.4f}")

print(f"\n--- UNPROFITABLE markets (PnL < 0) ---")
unprof = stats[~stats["profitable"]]
print(f"  Count: {len(unprof)}")
print(f"  Ever above 1.00: {unprof['ever_above_1'].sum()} ({unprof['ever_above_1'].mean()*100:.1f}%)")
print(f"  Came back below: {unprof['came_back'].sum()} ({unprof['came_back'].mean()*100:.1f}%)")
print(f"  Final above 1.00: {unprof['final_above_1'].sum()} ({unprof['final_above_1'].mean()*100:.1f}%)")
print(f"  Avg crossings above: {unprof['crossings_up'].mean():.1f}")
print(f"  Avg % time above 1.00: {unprof['pct_above'].mean():.1f}%")
print(f"  Avg final pair sum: {unprof['final_pair_sum'].mean():.4f}")
print(f"  Avg max pair sum: {unprof['max_pair_sum'].mean():.4f}")

# Show the "came back" markets in detail
came_back = stats[stats["came_back"]].sort_values("crossings_up", ascending=False)
print(f"\n--- Markets that CROSSED ABOVE 1.00 then CAME BACK (detail) ---")
print(f"  {'PnL':>10} | {'Final PS':>8} | {'Max PS':>8} | {'Cross Up':>8} | {'Cross Dn':>8} | {'%Above':>6}")
print(f"  {'-'*10}-+-{'-'*8}-+-{'-'*8}-+-{'-'*8}-+-{'-'*8}-+-{'-'*6}")
for _, r in came_back.iterrows():
    print(f"  ${r['pnl']:>+9.2f} | {r['final_pair_sum']:>8.4f} | {r['max_pair_sum']:>8.4f} | {r['crossings_up']:>8} | {r['crossings_down']:>8} | {r['pct_above']:>5.1f}%")
