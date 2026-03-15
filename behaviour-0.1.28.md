# CURRENT BEHAVIOUR

## Version 0.1.28

Scope: Sprint 4 only
Mode: `EXEC_MODE=WALLET_CLONE`
Date: 2026-03-16

This file is not a design target.
It is a concrete runtime note for the current Sprint 4 wallet-clone path in the working tree.

---

## Update Note

`0.1.28` records the current runnable Sprint 4 wallet-clone implementation.

The main current-tree changes since the previous reviewed canary are:

1. `WALLET_CLONE` now has its own runtime loop boundary and is no longer forced through the older market / settlement-shaper routing.
2. The wallet-clone controller now runs explicit `PreArm`, `OpenBoth`, `SeedCompletion`, `PairBuild`, `Taper`, and rollover ownership.
3. Startup seeding now uses wallet-clone-specific quote checks instead of the stricter generic parity / spread gate.
4. Missing-side startup repair stays on the intended Sprint 4 path:
   - missing-side quote health is required
   - hard skew is bypassed for startup completion
   - shape-target gating is bypassed for startup completion
   - CPP is not a hard normal-flow veto during startup completion
5. Wallet-clone phase budget fractions now affect live runtime behavior in `OpenBoth`, `PairBuild`, and `Taper`.
6. Wallet-clone metrics now count actual fill events instead of inferring fills from raw inventory deltas.
7. A flat market that reaches `PairBuild` with `qYES=0` and `qNO=0` now keeps `OpenBoth` live instead of falling through to an inactive owner branch.
8. `PairBuild` lighter-side repair bids are now capped to preserve original paired-growth economics.
   - when a paired growth submit fills one side and the other side needs repair, the repair bid is capped at `original_pair_sum - filled_side_price`
   - this prevents lighter-side repairs from chasing the current market bid after a price move
   - `SeedCompletion` repairs are exempt from this cap
9. `PairBuild` optional paired growth now allows averaging down when the book is in `RepairOnly` or `Freeze` territory.
   - if the current `pair_sum < inventory_vwap_sum`, adding at that pair_sum improves the blended cost
   - the averaging-down exception uses a reduced clip (small_clip_cap) to limit risk
   - this eliminates the mid-market freeze that prevented accumulation in earlier canaries

The latest reviewed canary on `2026-03-16` was the first Sprint 4 run to show profitability on both settlement outcomes.

That canary and the current-tree fixes confirm:

1. `OpenBoth`, `SeedCompletion`, `PairBuild`, `Taper`, and rollover are all mechanically live.
2. Both startup targets were met in the reviewed run:
   - `both_by_30s=true`
   - `both_by_60s=true`
3. The latest reviewed run ended with:
   - `paired_size=64.99`
   - `unmatched_size=3.37`
   - `tail_at_expiry=3.37`
   - `combined_avg_paid=0.967`
   - `worst_case_settlement_floor=+0.86`
4. The current tree blocks duplicate `OpenBoth` resubmits while both startup seed legs are already live.
5. The current tree treats taper paired-growth `suppress` as a real no-submit return.
6. The mid-market `PairBuild` freeze is now eliminated.
7. The lighter-side repair bid cap is active and observable in logs (`bid_cap_applied=true`).
8. The main remaining observations are:
   - startup basis can still be expensive after asymmetric early fills (SeedCompletion bypasses cost guards by design)
   - the bid cap can create temporarily unfillable lighter-side orders when the market has moved significantly from the paired growth anchor
   - profitability has been shown in one canary but not yet confirmed across multiple market conditions

---

## Executive Summary

The Sprint 4 runtime is now structurally runnable and has produced its first profitable canary.

The current code path can:

1. pre-arm before open
2. seed both sides
3. treat one-sided startup fills as normal startup completion
4. replenish through `PairBuild` without freezing mid-market
5. cap lighter-side repair bids to preserve pair economics
6. average down the book when pair_sum is better than the current blended cost
7. taper late
8. emit wallet-clone-specific metrics and config logs

The current code path has shown in one canary:

1. profitability on both settlement outcomes (`worst_case_settlement_floor=+0.86`)
2. `combined_avg_paid=0.967` (below 1.00)
3. zero time in the `freeze` or `repair_only` paired-cost bands
4. 30 fill events / 133 total shares (versus 8 fills / 40 shares in the previous canary)

The current code path does **not** yet prove:

1. that this profitability is consistent across multiple market conditions
2. that the lighter-side bid cap performs well in all price regimes
3. that the wallet-clone path is production-ready

---

## Current Practical Reading

Relative to Sprint 4 requirements, the current tree is approximately:

1. runnable as an isolated wallet-clone mode
2. mechanically aligned on startup ownership and missing-side repair
3. materially closer to an aggressive inventory builder than the older settlement-shaper path
4. now backed by a profitable canary with `worst_case_settlement_floor=+0.86`
5. strong on startup, taper, rollover lifecycle, and now also on mid-market `PairBuild` economics
6. still needing more canary runs to confirm consistency

---

## Latest Live Canary

Reviewed run date: 2026-03-16

Observed headline metrics from the reviewed run:

1. `market_participated=true`
2. `fills_per_market=30`
3. `total_fill_shares=133.34`
4. `maker_fill_share=0.924`
5. `paired_size=64.99`
6. `unmatched_size=3.37`
7. `pair_coverage=0.951`
8. `share_skew=1.052`
9. `combined_avg_paid=0.967`
10. `worst_case_settlement_floor=+0.86`
11. `fills_after_final_quiet=0`
12. `fills_after_taper_start=0`
13. `new_orders_after_taper_start=0`
14. `fill_events_by_segment=0-30s:2,30-60s:3,60-180s:20,180-240s:5,240-300s:0`
15. `fill_shares_by_segment=0-30s:10.00,30-60s:15.00,60-180s:78.36,180-240s:29.99,240-300s:0.00`
16. `paired_cost_band_occupancy_rate=strong_growth:0.000,normal_growth:0.916,reduced_growth:0.084,repair_only:0.000,freeze:0.000`
17. `tail_at_expiry=3.37`
18. `startup_completion_blocked=5`
19. `below_snapshot_optional_fill_rate=0.444`
20. `repair_reserve_blocks=37`

What the canary confirms:

1. The two new fixes are mechanically working.
2. `OpenBoth` submitted immediately after open at roughly:
   - `y_bid=0.470`
   - `n_bid=0.510`
   - `pair_sum=0.980`
   - `clip=5`
3. `SeedCompletion` restored the missing side and both startup targets were met:
   - `both_by_30s=true`
   - `both_by_60s=true`
4. The lighter-side repair bid cap is active:
   - the log shows `bid_cap_applied=true` with capped bids well below the current market
   - example: `bid=0.420 original_bid=0.590` — the cap saved `0.17` per share on that repair
5. The averaging-down exception eliminated mid-market freeze:
   - `paired_cost_band_occupancy_rate ... repair_only:0.000,freeze:0.000`
   - `fill_shares_by_segment=60-180s:78.36` — the middle of the market is now the most active segment
6. The final book is profitable on both outcomes:
   - `paired_size=64.99`
   - `combined_avg_paid=0.967`
   - `worst_case_settlement_floor=+0.86`
7. Final quiet and rollover ownership worked as intended.
8. Taper is clean: zero fills and zero new orders after `240s`.

What the canary still shows:

1. Startup can still become expensive.
   - `SeedCompletion` paid `YES@0.580` while NO was at `0.510`
   - this is by design: `SeedCompletion` bypasses cost guards to get both sides live
   - the strategy recovered through mid-market volume at better pair_sums
2. The lighter-side bid cap can create temporarily unfillable orders.
   - when the market moves significantly from the paired growth anchor, the capped bid sits far below market
   - example: cap bid at `0.420` into a `0.590` market — the order sat unfilled for many seconds
   - those orders eventually expired and the bot re-entered through paired growth at fresh prices
   - this did not prevent profitability in this run but could in low-liquidity or fast-moving markets
3. There is a small residual tail at expiry.
   - `tail_at_expiry=3.37`
   - previous canary had `tail_at_expiry=0.00` but was unprofitable
   - a small tail with a positive settlement floor is a better outcome than zero tail with a negative floor
4. The bid cap uses global anchor state rather than per-order tracking.
   - in cancel/fill race conditions between consecutive paired-growth submits, the cap could reference slightly stale anchor prices
   - the error is bounded by price movement between consecutive submits (typically 1-2 ticks)
   - this is a known hardening item but does not materially affect the current canary

---

## Comparison: Previous Canary vs Current Canary

| Metric | 2026-03-15 | 2026-03-16 | Change |
|---|---|---|---|
| `paired_size` | 20.00 | 64.99 | +3.25x |
| `combined_avg_paid` | 1.058 | 0.967 | below 1.00 |
| `worst_case_settlement_floor` | -1.15 | +0.86 | profitable |
| `freeze` band occupancy | 99.7% | 0.0% | eliminated |
| `fills_per_market` | 8 | 30 | 3.75x |
| `total_fill_shares` | 40 | 133.34 | 3.3x |
| `fills 60-180s` | 0 shares | 78.36 shares | no longer frozen |
| `maker_fill_share` | 62.5% | 92.4% | more maker |
| `tail_at_expiry` | 0.00 | 3.37 | small tail |

---

## Current Active Configuration

The section below records the active checked-in `.env` values that directly mattered for the reviewed canary.

This is not a full env dump.
It is the subset that explains the observed wallet-clone behavior in this run.

### Mode Boundary And Legacy Isolation

1. `EXEC_MODE=WALLET_CLONE`
   - This is the mode switch that routes the bot into the Sprint 4 wallet-clone runtime instead of settlement-shaper or older market paths.
   - It is the reason the log contains `[WALLET_CLONE]` ownership, phase, and metrics lines.
2. `PAIR_BASE_ENABLED=false`
   - Keeps the older Step 1 pair-base runtime from being the active controller for this run.
   - Context: this should stay off while validating the isolated wallet-clone path.
3. `PAIR_RECOVERY_ENABLED=false`
   - Keeps the older pair-base recovery controller from competing with wallet-clone startup completion or `PairBuild`.
   - Context: this avoids mixed ownership and makes the canary easier to interpret.
4. `MAKER_SKEW_ENABLED=false`
   - Disables the legacy skew overlay.
   - Context: this prevents directional skew logic from polluting the wallet-clone run.
5. `MAKER_ARB_ENABLED=false`
   - Disables the older arb overlay.
   - Context: wallet-clone canary behavior should come from the inventory builder only, not from separate arb logic.
6. `MAKER_STRETCH_BIAS_ENABLED=false`
   - Disables stretch-bias logic from the older path.
   - Context: this keeps the canary on the intended neutral paired builder path.

### Shared Budget And Risk Caps

1. `MAX_TOTAL_COST=80`
   - Top-level gross spend cap for the market.
   - Context: this is the main reason the run stopped expanding once cost moved into the upper `60s`.
2. `RESERVE_USD=10`
   - Hard reserve held back from the top-level budget.
   - Context: with `MAX_TOTAL_COST=80`, this leaves roughly `70` usable dollars before phase slices and lot constraints are applied.
3. `MIN_SHARES=5`
   - Minimum legal working size for most wallet-clone maker actions.
   - Context: startup, lighter-side repair, and many normal `PairBuild` actions therefore happen in `5`-share blocks.
4. `CLIP_SHARES=5`
   - Shared baseline clip size used by the broader maker stack.
   - Context: this is consistent with the reviewed run's repeated `clip=5` submissions.
5. `STOP_BUFFER_SECONDS=15`
   - Shared pre-expiry stop buffer.
   - Context: it aligns with the rollover stop at roughly the last `15s`.

### Shared Data And Maker Cadence

1. `MARKET_DATA_STALE_SECONDS=8`
   - Freshness threshold for quote and market data inputs.
   - Context: this drove the early startup holds when market/user websockets were not yet ready, and later the `quote_inputs_unready` holds when one side showed zero bid/ask.
2. `CLOB_ORDER_META_WARMUP=true`
   - Warms order metadata before active trading.
   - Context: this is why the run logged two `[CLOB] warm order meta` lines before trading started.
3. `REQUIRE_USER_WS_CONNECTED=true`
   - Requires the user websocket before the runtime is considered ready to trade.
   - Context: this is why early startup briefly held on `user_ws_disconnected`.
4. `PAIR_BASE_REFRESH_SECONDS=1`
   - Shared refresh cadence for maker quote maintenance.
   - Context: wallet-clone reused this fast refresh rhythm, which contributes to active quote maintenance.
5. `ENTRY_EDGE_TICKS=2`
   - Shared maker entry edge.
   - Context: this keeps wallet-clone quotes fairly aggressive instead of sitting too far from touch.
6. `MIN_ENTRY_EDGE_TICKS=2`
   - Lower bound for the effective maker edge.
   - Context: this prevents wallet-clone from collapsing into tighter-than-intended passive quoting.
7. `REPLACE_IF_PRICE_MOVES_TICKS=2`
   - Shared reprice threshold.
   - Context: together with the fast refresh cadence, this makes the path responsive to quote changes.
8. `STALE_SECONDS=3`
   - Shared maker stale-order timeout reused by wallet-clone `PairBuild`.
   - Context: this is currently one of the most important problematic settings in practice, because the reviewed run repeatedly canceled viable resting orders at around the `3s` horizon.

### Wallet-Clone Startup And Timing

1. `WALLET_CLONE_PREARM_LEAD_SECONDS=20`
   - Starts wallet-clone readiness enforcement `20s` before the market opens.
   - Context: this produced the `PreArm` window and the early readiness logs before open.
2. `WALLET_CLONE_TARGET_BOTH_SIDES_BY_30S=0.80`
   - Operator startup target used in metrics for the first `30s`.
   - Context: the reviewed run met this target with `both_by_30s=true`.
3. `WALLET_CLONE_TARGET_BOTH_SIDES_BY_60S=0.95`
   - Operator startup target used in metrics for the first `60s`.
   - Context: the reviewed run also met this target with `both_by_60s=true`.
4. `WALLET_CLONE_TAPER_START_SECONDS=240`
   - Moves the runtime into taper behavior after `240s`.
   - Context: the run did exactly that, then stopped opening meaningful new exposure late.
5. `WALLET_CLONE_FINAL_QUIET_SECONDS=30`
   - Final quiet window before expiry.
   - Context: the run correctly emitted `final_quiet_rest` and showed no new orders after final quiet began.

### Wallet-Clone Clip Sizing

1. `WALLET_CLONE_SEED_CLIP_SMALL=5`
   - Small paired opener size used by `OpenBoth`.
   - Context: this matches the initial `5 YES / 5 NO` startup structure in the reviewed run.
2. `WALLET_CLONE_REPAIR_CLIP_SMALL=5`
   - Small missing-side repair size used by `SeedCompletion`.
   - Context: this is why missing-side startup repair also occurred in `5`-share blocks.
3. `WALLET_CLONE_CLIP_LADDER_LARGE=7,10`
   - Large clip ladder available to `PairBuild`.
   - Context: this is why the reviewed run escalated from `5`-share activity into repeated `10`-share `PairBuild` submissions later in the market.

### Wallet-Clone Phase Budget Slices

1. `WALLET_CLONE_BUDGET_SEED_MIN_FRACTION=0.10`
   - Lower bound of startup seed budget.
   - Context: this limits how much budget `OpenBoth` should consume before the controller moves into the next phase.
2. `WALLET_CLONE_BUDGET_SEED_MAX_FRACTION=0.15`
   - Upper bound of startup seed budget.
   - Context: this keeps the opener from spending too much of the market budget immediately.
3. `WALLET_CLONE_BUDGET_EARLY_MIN_FRACTION=0.15`
   - Lower bound of early post-open build budget.
   - Context: this supports active building just after startup completes.
4. `WALLET_CLONE_BUDGET_EARLY_MAX_FRACTION=0.20`
   - Upper bound of early post-open build budget.
   - Context: together with the seed slice, this explains why the run could trade actively early but still hit `budget_too_small` once local phase capacity was exhausted.
5. `WALLET_CLONE_BUDGET_MAIN_MIN_FRACTION=0.45`
   - Lower bound of main build budget.
   - Context: this is the largest spend window and is where most of the run's inventory growth occurred.
6. `WALLET_CLONE_BUDGET_MAIN_MAX_FRACTION=0.55`
   - Upper bound of main build budget.
   - Context: this is why the bot still had room to build materially through the middle of the market.
7. `WALLET_CLONE_BUDGET_LATE_MIN_FRACTION=0.15`
   - Lower bound of late-phase budget.
   - Context: this leaves some budget available after the main build phase without encouraging a late scramble.
8. `WALLET_CLONE_BUDGET_LATE_MAX_FRACTION=0.20`
   - Upper bound of late-phase budget.
   - Context: this helps keep late activity finite before taper starts.
9. `WALLET_CLONE_BUDGET_TAPER_MIN_FRACTION=0.05`
   - Lower bound of taper reserve.
   - Context: this preserves a small maintenance allowance into the late window.
10. `WALLET_CLONE_BUDGET_TAPER_MAX_FRACTION=0.10`
    - Upper bound of taper reserve.
    - Context: this is intentionally small, which matches the quiet late behavior seen in the run.

### Wallet-Clone Behavioral Guardrail

1. `WALLET_CLONE_BUY_ONLY_NORMAL_FLOW=true`
   - Forces normal wallet-clone behavior to stay on the observed maker-`BUY` path.
   - Context: this is why the canary remained on repeated maker buys and did not introduce sell-style shaping or cleanup during normal flow.

### Practical Reading Of The Current Canary Profile

This reviewed configuration is effectively saying:

1. run the isolated wallet-clone path, not the older controller stack
2. arm early, seed both sides in `5`-share clips, and repair the missing side in `5`-share clips
3. allow `PairBuild` to scale into `7` and `10` share adds through the main window
4. cap lighter-side repair bids to original pair economics to prevent chasing
5. allow `PairBuild` to average down even when the book is already above `1.00` paired cost
6. keep the runtime aggressively refreshed with `1s` cadence and `2`-tick repricing
7. stop new activity near expiry and stay mostly silent in the final `30s`
8. spend out of a roughly `70`-dollar usable budget after reserve

The most important current tradeoff in this profile is:

1. the strategy is now aggressive enough to participate, build inventory, and stay profitable
2. the bid cap prevents worst-case chasing but can create temporarily unfillable lighter-side orders
3. startup basis can still become expensive before both sides are restored, but the strategy can now recover through mid-market volume

---

## Known Remaining Gap

The dominant remaining gaps are no longer `PairBuild` freeze or mid-market economics.

The dominant remaining observations after the reviewed canary are:

1. Only one profitable canary has been reviewed.
   - consistency across different market conditions has not been proven
   - more canary runs are needed before claiming stable profitability
2. The lighter-side bid cap uses global anchor state, not per-order tracking.
   - in cancel/fill race conditions, the cap can reference stale anchor prices
   - the error is bounded by 1-2 ticks and is a known hardening item
3. The bid cap can create unfillable orders when the market moves significantly.
   - the cap holds the repair bid at the original pair economics level
   - if the market has moved 10+ ticks from the anchor, the capped bid sits far below market
   - the orders eventually expire and the bot re-enters through fresh paired growth
   - a spread-floor softening (e.g., cap at max of pair_economics and current_bid minus max_spread) could address this
4. Startup cost discipline is still not directly addressed.
   - `SeedCompletion` bypasses cost guards by design
   - the latest run paid `YES@0.580` during seed completion
   - the strategy recovered through volume, but a market with less mid-market fill opportunity might not
5. The small residual expiry tail (`tail_at_expiry=3.37`) is new.
   - the previous canary had zero tail but negative economics
   - a small tail with positive economics is strictly better
   - whether this tail grows in other market conditions is unknown

After the latest reviewed canary, `0.1.28` should be read as:

1. Sprint 4 runtime implemented
2. config surface implemented
3. metrics surface implemented
4. repeated live canaries reviewed
5. startup, taper, and rollover lifecycle acceptable
6. mid-market `PairBuild` economics now functional — freeze eliminated, averaging-down working
7. lighter-side repair bid cap active
8. first profitable canary achieved (`worst_case_settlement_floor=+0.86`)
9. consistency across market conditions not yet proven — more canary runs needed
