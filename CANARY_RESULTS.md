# Canary Results Journal

## Sprint 4 Wallet-Clone Mode

Scope: `EXEC_MODE=WALLET_CLONE` on BTC 5-minute up/down markets
Tracking: sequential canary runs with code changes between runs

---

## Canary 1

**Date**: 2026-03-16 00:19
**Market**: btc-updown-5m-1773595200
**Code version**: bid cap (strict pair-economics only) + averaging-down exception

| Metric | Value |
|---|---|
| `fills_per_market` | 30 |
| `total_fill_shares` | 133.34 |
| `maker_fill_share` | 0.924 |
| `paired_size` | 64.99 |
| `unmatched_size` | 3.37 |
| `pair_coverage` | 0.951 |
| `share_skew` | 1.052 |
| `combined_avg_paid` | 0.967 |
| `worst_case_settlement_floor` | **+0.86** |
| `tail_at_expiry` | 3.37 |
| `freeze` band occupancy | 0.0% |
| `normal_growth` band occupancy | 91.6% |
| `fill_events_by_segment` | 0-30s:2, 30-60s:3, 60-180s:20, 180-240s:5, 240-300s:0 |
| `fill_shares_by_segment` | 0-30s:10, 30-60s:15, 60-180s:78.36, 180-240s:29.99, 240-300s:0 |
| `both_by_30s` | true |
| `both_by_60s` | true |
| `repair_reserve_blocks` | 37 |
| `skipped_optional_adds` | 110 |
| `settlement_pnl_net_of_fees` | N/A (log truncated) |

**Result**: PROFITABLE (both outcomes)

**What happened**:
- PreArm ready 12.9s before open
- SeedCompletion restored missing side at t_into=22.5s (slow — 21s one-sided)
- Bid cap activated (`bid_cap_applied=true bid=0.420 original_bid=0.590`) — saved 0.17/share on lighter repairs
- Some capped orders sat unfilled for several seconds but the bot recovered through fresh paired growth cycles
- Averaging-down kept PairBuild active through the middle of the market (zero freeze time)
- Mid-market segment (60-180s) produced 78 shares — the most active window
- Small tail at expiry (3.37 shares) but positive floor

**Key observation**: First ever profitable canary. Averaging-down fix eliminated the freeze that killed all previous runs.

---

## Canary 2

**Date**: 2026-03-16 00:34
**Market**: btc-updown-5m-1773596100
**Code version**: same as canary 1

| Metric | Value |
|---|---|
| `fills_per_market` | 34 |
| `total_fill_shares` | 129.98 |
| `maker_fill_share` | 0.885 |
| `paired_size` | 64.98 |
| `unmatched_size` | 0.01 |
| `pair_coverage` | 1.000 |
| `share_skew` | 1.000 |
| `combined_avg_paid` | 0.982 |
| `worst_case_settlement_floor` | **+1.19** |
| `tail_at_expiry` | 0.01 |
| `freeze` band occupancy | 0.0% |
| `normal_growth` band occupancy | 6.5% |
| `reduced_growth` band occupancy | 93.5% |
| `fill_events_by_segment` | 0-30s:3, 30-60s:7, 60-180s:22, 180-240s:2, 240-300s:0 |
| `fill_shares_by_segment` | 0-30s:10, 30-60s:29.99, 60-180s:79.99, 180-240s:10, 240-300s:0 |
| `both_by_30s` | true |
| `both_by_60s` | true |
| `repair_reserve_blocks` | 98 |
| `skipped_optional_adds` | 150 |
| `settlement_pnl_net_of_fees` | +1.08 |

**Resolution**: NO won (BTC settled at 71524.54 vs price_to_beat 71537.69, diff=-0.018%)
**Final position**: qYES=64.99, qNO=64.98, cost=63.79, lp=+1.19

**Result**: PROFITABLE (both outcomes, confirmed PnL +1.08 after fees)

**What happened**:
- SeedCompletion was fast (1.3s) — cheapest startup of all canaries
- Nearly perfect balance at expiry (tail=0.01)
- Zero freeze time, active mid-market accumulation
- 80 shares accumulated in the 60-180s window
- Clean taper: zero fills and orders after 240s
- Best canary so far on all dimensions

**Key observation**: Fast SeedCompletion leads to clean startup economics. The rest of the market builds on a good foundation.

---

## Canary 3

**Date**: 2026-03-16 00:39
**Market**: btc-updown-5m-1773596400
**Code version**: same as canary 1

| Metric | Value |
|---|---|
| `fills_per_market` | 6 |
| `total_fill_shares` | 16.00 |
| `maker_fill_share` | 1.000 |
| `paired_size` | 6.00 |
| `unmatched_size` | 4.00 |
| `pair_coverage` | 0.600 |
| `share_skew` | 1.667 |
| `combined_avg_paid` | 1.007 |
| `worst_case_settlement_floor` | **-1.90** |
| `tail_at_expiry` | 4.00 |
| `freeze` band occupancy | 0.0% |
| `reduced_growth` band occupancy | 100% |
| `fill_events_by_segment` | 0-30s:5, 30-60s:0, 60-180s:0, 180-240s:0, 240-300s:1 |
| `fill_shares_by_segment` | 0-30s:15, 30-60s:0, 60-180s:0, 180-240s:0, 240-300s:1 |
| `both_by_30s` | true |
| `both_by_60s` | true |
| `repair_reserve_blocks` | 1 |
| `skipped_optional_adds` | 90 |
| `settlement_pnl_net_of_fees` | +2.10 |

**Resolution**: NO won (BTC settled at 71535.61 vs price_to_beat 71537.69, diff=-0.003%)
**Final position**: qYES=6.00, qNO=10.00, cost=7.90, lp=+2.10

**Result**: LOSS on worst-case basis (got lucky — NO won so actual PnL was +2.10, but floor was -1.90)

**What happened**:
- Bid cap made lighter-side repairs unfillable when market moved significantly
- `bid_cap_applied=true bid=0.420 original_bid=0.590` — capped 17 ticks below market, order never filled
- The capped repair orders sat for 6+ seconds, got stale-canceled, repeated — never filled
- Bot stuck at 6 YES / 10 NO for the entire market (5 fills in 0-30s, then dead for 210s)
- `repair_reserve_block` also contributed to lockout during early skew

**Root cause**: Bid cap was too strict — no spread floor. The pair-economics cap created orders far below the current market that had zero chance of filling.

**Fix applied after this canary**: Added spread floor `max(pair_economics_cap, active_bid - 0.03)` so capped orders stay within 3 ticks of market.

---

## Canary 4

**Date**: 2026-03-16 00:54
**Market**: btc-updown-5m-1773597300
**Code version**: bid cap with spread floor (0.03) added

| Metric | Value |
|---|---|
| `fills_per_market` | 4 |
| `total_fill_shares` | 15.00 |
| `maker_fill_share` | 1.000 |
| `paired_size` | 5.00 |
| `unmatched_size` | 5.00 |
| `pair_coverage` | 0.500 |
| `share_skew` | 2.000 |
| `combined_avg_paid` | 0.940 |
| `worst_case_settlement_floor` | **-1.50** |
| `tail_at_expiry` | 5.00 |
| `freeze` band occupancy | 0.0% |
| `repair_only` band occupancy | 89.4% |
| `fill_events_by_segment` | 0-30s:3, 30-60s:1, 60-180s:0, 180-240s:0, 240-300s:0 |
| `fill_shares_by_segment` | 0-30s:10, 30-60s:5, 60-180s:0, 180-240s:0, 240-300s:0 |
| `both_by_30s` | true |
| `both_by_60s` | true |
| `repair_reserve_blocks` | 46 |
| `skipped_optional_adds` | 175 |
| `bid_cap_applied` | never triggered |

**Resolution**: NO won (bot lost because it was YES-heavy)
**Final position**: qYES=10.00, qNO=5.00, cost=6.50

**Result**: LOSS (-1.50 worst case, actual loss since NO won)

**What happened**:
- The bid cap spread floor was never the issue — `bid_cap_applied=true` never appeared in this run
- The blocker was `lighter_side_projected_cost_cap` (fired for 193 seconds straight)
- After paired growth filled YES at 0.27 (very cheap) but NO never filled, the bot was stuck at 10 YES / 5 NO
- The NO side needed repair at ~0.73, but `lighter_side_projected_cost_cap` computed projected cost >1.03 and blocked
- The irony: buying 5 NO at 0.73 would have improved worst_case_settlement_floor from -1.50 to ~-0.15
- The cost cap prevented a repair that would have massively reduced risk
- 89.4% of PairBuild time spent in `repair_only` band, unable to act

**Root cause**: `lighter_side_projected_cost_cap` and `lighter_price_discipline_block` blocked repairs that would have improved the settlement floor. These functions optimized for blended cost quality when they should have prioritized rebalancing.

**Fix applied after this canary**:
1. Both `lighter_extreme_projected_cost_block` and `lighter_price_discipline_block` now bypass when the repair would improve `worst_case_settlement_floor`
2. In a binary market (prices < 1.0), lighter-side repair always improves the floor, so these caps will no longer permanently lock the bot out of rebalancing

---

## Canary 5

**Date**: 2026-03-16 01:10
**Market**: btc-updown-5m-1773598200
**Code version**: spread floor (0.03) + floor-improvement bypass on lighter-side cost caps

| Metric | Value |
|---|---|
| `fills_per_market` | 36 |
| `total_fill_shares` | 140.54 |
| `maker_fill_share` | 0.929 |
| `paired_size` | 65.00 |
| `unmatched_size` | 10.55 |
| `pair_coverage` | 0.860 |
| `share_skew` | 1.162 |
| `combined_avg_paid` | 0.960 |
| `worst_case_settlement_floor` | **-0.92** |
| `tail_at_expiry` | 10.55 |
| `repair_only` band occupancy | 81.3% |
| `freeze` band occupancy | 6.4% |
| `fill_events_by_segment` | 0-30s:4, 30-60s:10, 60-180s:19, 180-240s:3, 240-300s:0 |
| `fill_shares_by_segment` | 0-30s:15, 30-60s:24.99, 60-180s:70.56, 180-240s:30, 240-300s:0 |
| `both_by_30s` | true |
| `both_by_60s` | true |
| `repair_reserve_blocks` | 75 |
| `skipped_optional_adds` | 84 |
| `averaging_down=true` submits | 9 |
| `lighter_side_projected_cost_cap` fires | **0** |
| `bid_cap_applied` | 1 (bid=0.510 original=0.540) |

**Outcome**: DOWN (NO won). Bot was YES-heavy.
**Final position**: qYES=75.55, qNO=65.00, cost=65.92
**Actual PnL**: 65.00 - 65.92 = **-$0.92 loss**

**Result**: LOSS (-$0.92)

**What happened**:
- The canary 4 killer (`lighter_side_projected_cost_cap`) fired **zero times** — the floor-improvement bypass works
- 9 averaging-down submits went through, keeping mid-market active (70 shares in 60-180s)
- The bot accumulated actively — 36 fills, 140 total shares, the most of any canary
- BTC moved strongly during this market, making YES cheap (down to 0.05) and NO expensive (up to 0.94)
- The bot accumulated more YES than NO because YES was the cheap side with taker flow
- A 20-share YES repair at 0.05 late in the market added to the YES-heavy tail
- Final position: 75.55 YES / 65.00 NO — a 10.55-share YES tail
- Market settled DOWN → NO won → the YES tail was the losing side
- `worst_case_settlement_floor = -0.92` — small loss but still negative

**Why it lost**:
- In a strongly directional market, the bot becomes heavy on the side that's getting cheaper (YES in this case)
- Maker accumulation means the bot buys what takers sell into it — in a down move, takers sell YES
- The 10.55 unmatched YES tail at $0.92 cost was worth $0 when NO won
- The paired core (65/65) was fine at `combined_avg_paid=0.960`, but the tail wiped out the edge

**Positive signals despite the loss**:
- The lockout bug from canary 4 is fully fixed
- Mid-market accumulation is active and healthy
- The loss is small (-$0.92) vs canary 3/4 losses (-$1.50 to -$1.90)
- The paired core itself was profitable — the problem is the tail, not the core economics

---

## Canary 6

**Date**: 2026-03-16 01:15
**Market**: btc-updown-5m-1773598500
**Code version**: same as canary 5

| Metric | Value |
|---|---|
| `fills_per_market` | 26 |
| `total_fill_shares` | 119.99 |
| `maker_fill_share` | 0.917 |
| `paired_size` | 59.99 |
| `unmatched_size` | 0.01 |
| `pair_coverage` | 1.000 |
| `share_skew` | 1.000 |
| `combined_avg_paid` | **1.026** |
| `worst_case_settlement_floor` | **-1.55** |
| `tail_at_expiry` | 0.01 |
| `reduced_growth` band occupancy | 15.6% |
| `repair_only` band occupancy | 84.4% |
| `freeze` band occupancy | 0.0% |
| `fill_events_by_segment` | 0-30s:4, 30-60s:3, 60-180s:18, 180-240s:0, 240-300s:1 |
| `fill_shares_by_segment` | 0-30s:14.99, 30-60s:15, 60-180s:85, 180-240s:0, 240-300s:5 |
| `both_by_30s` | true |
| `both_by_60s` | true |
| `repair_reserve_blocks` | 0 |
| `skipped_optional_adds` | 99 |
| `averaging_down=true` submits | 5 |
| `lighter_side_projected_cost_cap` fires | 0 |
| `bid_cap_applied` | 13 (all unfilled YES repair attempts from 128-234s) |

**Outcome**: DOWN (NO won)
**Final position**: qYES=59.99, qNO=60.00, cost=61.55
**Actual PnL**: 60.00 - 61.55 = **-$1.55 loss** (guaranteed loss on either outcome)

**Result**: LOSS (-$1.55)

**What happened**:
- The bot finished **perfectly balanced** (tail=0.01, pair_coverage=1.000) and still lost
- This is a pure **expensive paired core** failure: `combined_avg_paid=1.026`
- The market had persistent overround — YES+NO prices consistently summed above 1.00
- The bot spent 84.4% of time in `repair_only` band, never reaching `normal_growth`
- Every pair bought was above $1.00 total, so more volume meant more loss
- 5 averaging-down submits fired but couldn't bring the average below 1.00
- The most expensive fill: YES@0.96 during taper (fill #24) to close a 5-share tail — cost $4.80 for 5 shares
- 13 bid-capped lighter-side YES repairs all sat unfilled (YES was expensive side throughout)
- Loss formula: `(1.026 - 1.00) × 59.99 = $1.56` — matches the actual loss

**Why it lost**:
- The market never offered sub-1.00 paired prices for long enough
- Seed was borderline: YES@0.50 + NO@0.48 = 0.98 (good), but subsequent pairs crept above 1.00
- YES prices rose through the run (0.48 → 0.58 → 0.66 → 0.96) while NO fell (0.48 → 0.41 → 0.33)
- The bot kept averaging down in repair_only band, each pair at ~1.01-1.03, digging deeper
- Even with zero tail and perfect balance, `combined_avg_paid > 1.00` means guaranteed loss

**New failure mode**: This is different from canaries 3-5. Those lost due to lockout or directional tails. Canary 6 lost because the market itself was persistently overpriced — no amount of balancing helps when every pair costs more than $1.00.

**Key question**: Should the bot stop accumulating when `combined_avg_paid` is above 1.00 and not improving? The averaging-down logic currently allows adds when `pair_sum < inventory_vwap_sum`, but in this market, that kept the bot buying expensive pairs.

---

## Canary 7

**Date**: 2026-03-16 01:55
**Market**: btc-updown-5m-1773600900
**Code version**: same as canary 5-6 (new env features NOT enabled)

| Metric | Value |
|---|---|
| `fills_per_market` | 27 |
| `total_fill_shares` | 124.99 |
| `maker_fill_share` | 0.880 |
| `paired_size` | 60.00 |
| `unmatched_size` | 4.99 |
| `combined_avg_paid` | **1.003** |
| `worst_case_settlement_floor` | **-3.24** |
| `tail_at_expiry` | 4.99 |
| `repair_only` band | 89.3% |
| `freeze` band | 8.4% |
| `averaging_down=true` | 7 |
| `taker_fallback` | 0 (not enabled) |
| `max_unmatched_suppress` | 0 (not enabled) |

**Outcome**: DOWN (NO won). Bot was NO-heavy (60 YES / 65 NO).
**Actual PnL**: +$1.75 (got lucky — NO won)
**Would have lost**: -$3.24 if YES won

**Result**: WIN (lucky) — floor was -$3.24

---

## Canary 8

**Date**: 2026-03-16 02:00
**Market**: btc-updown-5m-1773601200
**Code version**: same as canary 7 (new env features NOT enabled)

| Metric | Value |
|---|---|
| `fills_per_market` | 31 |
| `total_fill_shares` | 129.97 |
| `maker_fill_share` | 0.962 |
| `paired_size` | 64.98 |
| `unmatched_size` | 0.01 |
| `combined_avg_paid` | **0.973** |
| `worst_case_settlement_floor` | **+1.74** |
| `tail_at_expiry` | 0.01 |
| `normal_growth` band | 90.2% |
| `strong_growth` band | 9.6% |
| `averaging_down=true` | 0 |
| `taker_fallback` | 0 (not enabled) |

**Outcome**: DOWN (NO won). Bot perfectly balanced.
**Actual PnL**: +$1.74 net of fees
**Final position**: qYES=64.99, qNO=64.98, cost=63.24

**Result**: PROFIT (both outcomes) — best run since canary 2

---

## Canary 9

**Date**: 2026-03-16 02:05
**Market**: btc-updown-5m-1773601500
**Code version**: same as canary 7-8 (new env features NOT enabled)

| Metric | Value |
|---|---|
| `fills_per_market` | 28 |
| `total_fill_shares` | 114.99 |
| `maker_fill_share` | 0.957 |
| `paired_size` | 54.99 |
| `unmatched_size` | 5.01 |
| `combined_avg_paid` | **1.120** |
| `worst_case_settlement_floor` | **-9.85** |
| `tail_at_expiry` | 5.01 |
| `repair_only` band | 51.7% |
| `freeze` band | 18.8% |
| `averaging_down=true` | 6 |
| `taker_fallback` | 0 (not enabled) |

**Outcome**: UP (+0.008%). Bot was NO-heavy (55 YES / 60 NO).
**Actual PnL**: **-$9.85**
**Final position**: qYES=54.99, qNO=60.00, cost=64.84

**Result**: CATASTROPHIC LOSS — `avg_paid=1.120`, market overround + 18.8% freeze

**What happened**: The market had persistent overround. The bot kept averaging down (6 submits) despite pair_sum staying above 1.00. It accumulated 55 paired shares at $1.12 average — a guaranteed $6.60 loss on the core alone, plus a 5-share tail on the wrong side.

**This is exactly what `WALLET_CLONE_AVERAGING_DOWN_MAX_PAIR_SUM=0.995` would have prevented.** The bot would have stopped accumulating early and limited the loss to ~$1-2 instead of $9.85.

---

## Canary 10

**Date**: 2026-03-16 02:15
**Market**: btc-updown-5m-1773602100
**Code version**: taker fallback + pair_sum cap + tail cap ALL ENABLED

| Metric | Value |
|---|---|
| `fills_per_market` | 31 |
| `total_fill_shares` | 129.98 |
| `maker_fill_share` | **0.731** (down from 88-96%) |
| `paired_size` | 64.98 |
| `unmatched_size` | 0.02 |
| `combined_avg_paid` | **1.015** |
| `worst_case_settlement_floor` | **-0.96** |
| `tail_at_expiry` | 0.02 |
| `repair_only` band | 93.9% |
| `taker_fallback` fired | **8** |
| `averaging_down=true` | 7 |
| `max_unmatched_suppress` | 0 |

**Outcome**: YES won (+0.12%)
**Final position**: qYES=64.98, qNO=65.00, cost=65.94
**Actual PnL**: **-$0.96**

**Result**: LOSS — taker repairs made the bot more expensive (73% maker vs 88-96% before). Near-balanced but `avg_paid=1.015` still above break-even.

---

## Canary 11

**Date**: 2026-03-16 02:20
**Market**: btc-updown-5m-1773602400
**Code version**: same as canary 10 (all features enabled)

| Metric | Value |
|---|---|
| `fills_per_market` | 19 |
| `total_fill_shares` | 134.99 |
| `maker_fill_share` | **0.296** (70% taker!) |
| `paired_size` | 55.00 |
| `unmatched_size` | **24.99** |
| `combined_avg_paid` | **1.017** |
| `worst_case_settlement_floor` | **-12.35** |
| `tail_at_expiry` | **24.99** |
| `reduced_growth` band | 55.9% |
| `repair_only` band | 43.3% |
| `taker_fallback` fired | **14** |
| `averaging_down=true` | 5 |
| `max_unmatched_suppress` | **0** (tail cap didn't catch taker-created tails) |

**Outcome**: pending resolution
**Final position**: qYES=55.00(?), qNO=80.00(?), cost=67.35(?)
**Worst case PnL**: **-$12.35**

**Result**: CATASTROPHIC — taker fallback created a 25-share tail with 70% taker fills. The tail cap didn't fire because taker fills bypass the paired-growth suppression path.

---

## Lessons Learned: The Taker Experiment Failed

Canaries 10-11 proved that taker fallback for lighter-side repair **goes against the clone objective**:

1. **maker_fill_share collapsed**: 73% (canary 10) and 29.6% (canary 11) vs 88-96% in maker-only canaries
2. **Taker is more expensive**: paying ask + tick + 2% fee per fill erodes the edge
3. **Taker creates uncontrolled tails**: the 14 taker attempts in canary 11 with clips of 10-20 created a 25-share tail
4. **Tail cap doesn't catch taker fills**: `max_unmatched_shares_suppress` only gates paired growth submits, not taker fills
5. **The clone wallet is a patient maker** — it doesn't chase with taker orders

**Decision**: Disable taker fallback permanently. Return to maker-only with safety caps.

```env
WALLET_CLONE_LIGHTER_REPAIR_TAKER_ENABLED=false
WALLET_CLONE_AVERAGING_DOWN_MAX_PAIR_SUM=0.995
WALLET_CLONE_MAX_UNMATCHED_SHARES=8
```

---

## Canary 12

**Date**: 2026-03-16 07:49
**Market**: btc-updown-5m-1773622200
**Code version**: maker-only + `MAX_PAIR_SUM=0.995` + `MAX_UNMATCHED=8`, taker disabled

| Metric | Value |
|---|---|
| `fills_per_market` | 33 |
| `total_fill_shares` | 120.94 |
| `maker_fill_share` | 87.6% |
| `paired_size` | 60.00 |
| `unmatched_size` | 0.94 |
| `combined_avg_paid` | **1.065** |
| `worst_case_settlement_floor` | **-4.39** |
| `tail_at_expiry` | 0.94 |
| `freeze` band | **68.1%** |
| `repair_only` band | 15.8% |
| `reduced_growth` band | 12.7% |
| `normal_growth` band | 3.3% |
| `averaging_down=true` | 7 |
| `max_unmatched_suppress` | 0 |
| `taker_fallback` | 0 (disabled) |
| `bid_cap_applied` | 10 |
| Segments | 0-30s:6, 30-60s:20, 60-180s:65, 180-240s:30, 240-300s:0 |

**Outcome**: pending resolution
**Final position**: qYES≈60, qNO≈61, cost≈64.39
**Worst case PnL**: **-$4.39**

**Result**: LOSS — `combined_avg_paid=1.065`, 68% freeze band

**What happened**:
- Taker disabled — back to maker-only (87.6% maker). Good.
- `MAX_PAIR_SUM=0.995` is enabled but **didn't prevent the expensive core**
- The market spent 68% in freeze (pair_sum > 1.02) and 15.8% in repair_only
- But the brief 3.3% normal_growth + 12.7% reduced_growth windows were enough for the bot to accumulate 60 paired shares
- 7 averaging-down submits fired when pair_sum dipped below 0.995 momentarily
- The blended cost ended at 1.065 — most fills happened during the brief cheap windows, but the expensive fills from repair_only/freeze phases dragged the average up

**Why `MAX_PAIR_SUM=0.995` didn't help enough**:
- The gate only blocks the **averaging-down exception** (RepairOnly/Freeze bands)
- Normal paired growth in `reduced_growth` band (0.98-1.00) is NOT gated by `MAX_PAIR_SUM`
- The bot accumulated ~15 pairs in `reduced_growth` windows at pair_sum 0.98-1.00
- Those are individually borderline but collectively push avg_paid above 1.00
- The remaining problem: **normal paired growth in reduced_growth band still runs without a combined_avg_paid circuit breaker**

---

## Canary 13

**Date**: 2026-03-16 15:25
**Market**: btc-updown-5m-1773649500
**Code version**: hard `PairBuild` guard set enabled (cheap-core gate, truthful `repair_only`/`freeze`, relative tail caps, bad-regime shutdown)

| Metric | Value |
|---|---|
| `fills_per_market` | 10 |
| `total_fill_shares` | 40.00 |
| `maker_fill_share` | 87.5% |
| `paired_size` | 20.00 |
| `unmatched_size` | 0.00 |
| `combined_avg_paid` | **1.012** |
| `worst_case_settlement_floor` | **-0.25** |
| `tail_at_expiry` | 0.00 |
| `normal_growth` band | 1.7% |
| `reduced_growth` band | 1.7% |
| `repair_only` band | **96.6%** |
| `freeze` band | 0.0% |
| `paired_size_delta_by_state` | normal_growth: 15.00, all other bands: 0.00 |
| `bad_regime_expensive_ratio` | **0.901** |
| `bad_regime_shutdown` | true |
| `repair_reserve_blocks` | 0 |
| `skipped_optional_adds` | 64 |

**Final position**: qYES=20.00, qNO=20.00, cost=20.25
**Actual PnL**: **-$0.25 guaranteed loss** (balanced book, both outcomes lose)

**Result**: LOSS - expensive core, zero tail

**What happened**:
- Early paired-growth adds were accepted at projected paired cost `0.970` and `0.977`.
- One-sided fills left the bot YES-heavy, and lighter-side NO repairs then completed at progressively worse prices (`0.580`, `0.640`, `0.700`, `0.730`).
- The bot rebalanced cleanly back to `20 / 20` with no expiry tail.
- After that, the new guard set behaved correctly: optional growth stayed blocked in `repair_only`, `bad_regime_shutdown=true`, and there was no paired-size leakage in `repair_only` or `freeze`.

**Key observation**: The old optional-growth leak looks closed here. The remaining loss came from **realized paired core drift after one-sided paired growth had to be repaired later at worse prices**.

---

## Canary 14

**Date**: 2026-03-16 15:29
**Market**: btc-updown-5m-1773649800
**Code version**: same as canary 13

| Metric | Value |
|---|---|
| `fills_per_market` | 5 |
| `total_fill_shares` | 19.99 |
| `maker_fill_share` | 100.0% |
| `paired_size` | 9.99 |
| `unmatched_size` | 0.01 |
| `combined_avg_paid` | **1.045** |
| `worst_case_settlement_floor` | **-0.45** |
| `tail_at_expiry` | 0.01 |
| `normal_growth` band | 0.2% |
| `reduced_growth` band | 15.4% |
| `repair_only` band | 6.7% |
| `freeze` band | **77.7%** |
| `paired_size_delta_by_state` | normal_growth: 5.00, all other bands: 0.00 |
| `bad_regime_expensive_ratio` | **0.629** |
| `bad_regime_shutdown` | true |
| `repair_reserve_blocks` | 1 |
| `skipped_optional_adds` | 276 |

**Final position**: qYES=10.00, qNO=9.99, cost=10.45
**Actual PnL**: about **-$0.45 guaranteed loss**

**Result**: LOSS - expensive core, essentially no tail

**What happened**:
- The guard set was strict from the start: the bot spent most of the run holding in `reduced_growth`, then `freeze`, with only one normal-growth paired expansion.
- A reserve-aware paired-growth submit went through at projected paired cost `0.975`, with a real repair reserve held aside.
- That pair did not complete symmetrically; the later lighter-side NO repair filled around `0.690`, which pushed the realized paired core above parity.
- After the book returned to roughly `10 / 10`, the bot stopped adding: `bad_regime_shutdown=true`, one `repair_reserve_block`, and no paired-size leakage outside `normal_growth`.

**Key observation**: This is the same failure shape as canary 13, just smaller and cleaner. The active leak is now **expensive repair completion after a one-sided paired-growth fill**, not tail growth and not optional-growth leakage.

---

## Summary Table

| Canary | Date | Floor | Avg Paid | Paired | Tail | Fills | Maker% | Cause | Result |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 03-16 00:19 | +0.86 | 0.967 | 64.99 | 3.37 | 30 | 92% | — | PROFIT |
| 2 | 03-16 00:34 | +1.19 | 0.982 | 64.98 | 0.01 | 34 | 96% | — | PROFIT |
| 3 | 03-16 00:39 | -1.90 | 1.007 | 6.00 | 4.00 | 6 | 100% | Lockout (bid cap) | LOSS |
| 4 | 03-16 00:54 | -1.50 | 0.940 | 5.00 | 5.00 | 4 | 100% | Lockout (cost cap) | LOSS |
| 5 | 03-16 01:10 | -0.92 | 0.960 | 65.00 | 10.55 | 36 | 92% | Directional tail | LOSS |
| 6 | 03-16 01:15 | -1.55 | 1.026 | 59.99 | 0.01 | 26 | 92% | Expensive core | LOSS |
| 7 | 03-16 01:55 | -3.24 | 1.003 | 60.00 | 4.99 | 27 | 88% | Tail + borderline | WIN (lucky) |
| 8 | 03-16 02:00 | **+1.74** | **0.973** | 64.98 | 0.01 | 31 | 96% | — | **PROFIT** |
| 9 | 03-16 02:05 | -9.85 | 1.120 | 54.99 | 5.01 | 28 | 96% | Expensive core | LOSS (-9.85) |
| 10 | 03-16 02:15 | -0.96 | 1.015 | 64.98 | 0.02 | 31 | 73% | Taker too expensive | LOSS |
| 11 | 03-16 02:20 | -12.35 | 1.017 | 55.00 | 24.99 | 19 | 30% | Taker tail explosion | LOSS |
| 12 | 03-16 07:49 | -4.39 | 1.065 | 60.00 | 0.94 | 33 | 88% | Expensive core (cap leaked) | LOSS |
| 13 | 03-16 15:25 | -0.25 | 1.012 | 20.00 | 0.00 | 10 | 88% | Expensive core after repair completion | LOSS |
| 14 | 03-16 15:29 | -0.45 | 1.045 | 9.99 | 0.01 | 5 | 100% | Expensive core after repair completion | LOSS |

**Running totals** (14 canaries):
- Safe profits (floor > 0): canaries 1, 2, 8 = **+3.79**
- All other canaries had negative floor
- The new hard `PairBuild` guard set improved the shape of losses in canaries 13-14: no tail damage, no `repair_only`/`freeze` growth leakage, but still negative floor from expensive repair completion
- The pair_sum cap helped vs canary 9 (-$9.85 uncapped -> canary 12 -$4.39 with cap) but did not solve expensive core on its own

## Loss Mode Classification

| Mode | Canaries | Description | Fix |
|---|---|---|---|
| **Lockout** | 3, 4 | Bot cannot trade | Fixed (spread floor + floor bypass) |
| **Directional tail** | 5, 7 | Accumulates losing side | Historical problem; replaced by relative tail caps |
| **Expensive core (uncapped)** | 9 | No pair_sum gate, catastrophic | Historical problem; pair_sum cap reduced worst case |
| **Expensive core (cap leaked)** | 6, 12 | Normal growth in reduced_growth band still overpaid | Closed by the hard optional-growth gate |
| **Expensive core (repair completion)** | 13, 14 | Pair starts cheap, then one-sided completion repair pushes realized core above parity | Needs lighter-side repair economics gate |
| **Taker damage** | 10, 11 | Taker too expensive + tail | Disabled |
| **Profitable** | 1, 2, 8 | Cheap pairs, normal_growth dominant | Maker-only, pair_sum < 1.00 |

## Features Status

| Feature | Env Key | Status | Canaries active |
|---|---|---|---|
| Bid cap spread floor | (hardcoded 0.03) | Always on | 1-14 |
| Floor-improvement bypass | (hardcoded) | Always on | 4-14 |
| Averaging-down exception | (hardcoded) | Historical only | 1-12 |
| Pair sum cap on avg-down | `WALLET_CLONE_AVERAGING_DOWN_MAX_PAIR_SUM` | Historical only | 10-12 |
| Tail cap | `WALLET_CLONE_MAX_UNMATCHED_SHARES` | Historical only | 10-12 |
| Taker repair fallback | `WALLET_CLONE_LIGHTER_REPAIR_TAKER_ENABLED` | **DISABLED** | 10-11 only |
| Hard optional-growth gate | (hardcoded paired-cost bands) | Always on | 13-14 |
| Truthful `repair_only` / `freeze` | (hardcoded) | Always on | 13-14 |
| Relative tail caps | `WALLET_CLONE_TAIL_CAP_*` | Enabled | 13-14 |
| Bad-regime shutdown | `WALLET_CLONE_BAD_REGIME_*` | Enabled | 13-14 |

## Code Changes Between Canaries

| After Canary | Fix Applied |
|---|---|
| pre-1 | Bid cap (strict pair-economics) + averaging-down exception |
| 3 | Bid cap spread floor: `max(pair_economics_cap, active_bid - 0.03)` |
| 4 | Floor-improvement bypass on `lighter_extreme_projected_cost_block` and `lighter_price_discipline_block` |
| 6 | Added configurable `AVERAGING_DOWN_MAX_PAIR_SUM`, `MAX_UNMATCHED_SHARES`, `LIGHTER_REPAIR_TAKER_ENABLED` (all disabled by default) |
| 9 | Enabled all three features for canaries 10-11 |
| 11 | Disabled taker fallback. Kept `MAX_PAIR_SUM=0.995` and `MAX_UNMATCHED=8` |
| 12 | Enabled the hard `PairBuild` guard set: universal cheap-core gating, truthful `repair_only`/`freeze`, relative tail caps, bad-regime shutdown, and richer canary metrics |
| 14 | Patched reserve sizing against executable maker repair clip and recomputation of cheap-core gating after reserve clip (not yet canary-validated) |
