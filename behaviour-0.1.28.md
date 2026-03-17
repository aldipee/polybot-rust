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
9. `PairBuild` optional paired growth is now economically strict instead of exception-driven.
   - `repair_only` and `freeze` no longer allow optional paired growth
   - `reduced_growth` is clipped to maintenance size
   - optional paired growth now requires a cheap enough projected paired core before snapshot-quality checks are even considered
   - if repair reserve clips the paired-growth size, the cheap-core gate is evaluated again on that final reserve-limited clip instead of the stale pre-reserve size
10. Lighter-side repair now refuses any minimum-valid order that would overshoot the exact live gap and create a fresh opposite tail.
   - repair reserve budgets against the smallest executable maker repair clip, not just the raw exact live gap
   - if the exact live gap is not currently maker-repairable because venue minimum notional would force an overshoot, `PairBuild` stays in paired growth instead of parking in a permanent lighter-side hold
   - repair reserve does not reserve budget for those currently unexecutable lighter-side repairs
11. PairBuild tail control is now time-banded and relative to paired size.
   - `0-210s`: tail cap `10%`
   - `210-240s`: tail cap `5%`
   - `240s+`: tail cap `2%`
   - if the tail is outside band, `PairBuild` prioritizes lighter-side repair instead of optional paired growth
   - exception: if the current exact-gap lighter repair is not maker-executable yet, paired growth is allowed to continue rather than deadlocking on tail-cap priority alone
12. A bad-regime shutdown now disables optional paired growth when early projected paired-cost observations stay too expensive for too much of the market.
13. Final wallet-clone metrics now include paired-size delta by state, bad-regime shutdown state, and a canary-success summary.
14. `PairBuild` lighter-side repair now has a post-cap completion-core gate.
   - if the final capped repair bid would complete the book into `repair_only` / `freeze` economics without improving the current paired core, the repair is held
   - the check uses the final tick-rounded capped repair bid, not the raw market bid

The completed canary set reviewed on `2026-03-16` now shows mixed outcomes, not a stable profitable state.

1. Canary 1 and Canary 2 proved the path can finish with `combined_avg_paid < 1.00` and a positive worst-case floor.
2. Canary 5 lost with active healthy participation because a late lighter-side YES repair overshot the live gap and left a `10.55` share YES tail at expiry.
3. Canary 6 lost while essentially flat because the paired core itself was accumulated above parity (`combined_avg_paid=1.026`).
4. Canaries 13 and 14 stayed balanced with tiny expiry tails, but still lost because one-sided paired growth had to be completed later at expensive lighter-side repair prices.
5. The next market `btc-updown-5m-1773598800` had only reached `PreArm` in captured logs at review time, so it is not treated as a completed canary.

The reviewed canary set and the current-tree fixes confirm:

1. `OpenBoth`, `SeedCompletion`, `PairBuild`, `Taper`, and rollover are all mechanically live.
2. Both startup targets were met repeatedly in the profitable and losing completed runs:
   - `both_by_30s=true`
   - `both_by_60s=true`
3. The current tree blocks duplicate `OpenBoth` resubmits while both startup seed legs are already live.
4. The current tree treats taper paired-growth `suppress` as a real no-submit return.
5. Mid-market `PairBuild` now stays active only while the projected paired core remains in an allowed cost regime.
6. The lighter-side repair bid cap is active and observable in logs (`bid_cap_applied=true`).
7. Profitable paired-core behaviour is possible:
   - Canary 1 ended with `combined_avg_paid=0.967` and `worst_case_settlement_floor=+0.86`
   - Canary 2 ended with `combined_avg_paid=0.982` and `worst_case_settlement_floor=+1.19`
8. The main remaining live issues are:
   - startup basis can still be expensive after asymmetric early fills (`SeedCompletion` bypasses cost guards by design)
   - the new completion-core repair gate still needs fresh live canaries to prove it closes the post-fill expensive-core loss from canaries 13 and 14
   - the bid cap can still create temporarily unfillable lighter-side orders when the market has moved significantly from the paired-growth anchor

---

## Executive Summary

The Sprint 4 runtime is structurally runnable and can be profitable, but the latest completed canaries still show live loss modes.

The current code path can:

1. pre-arm before open
2. seed both sides
3. treat one-sided startup fills as normal startup completion
4. replenish through `PairBuild` only while the projected paired core stays inside the allowed cost band
5. cap lighter-side repair bids to preserve pair economics
6. refuse lighter-side repairs that would overshoot the exact live gap
7. refuse lighter-side repairs that would complete the book into a worse `repair_only` / `freeze` core after cap
8. prioritize lighter-side repair when tail is too large relative to paired size
9. shut optional growth down in early bad markets
10. taper late
11. emit richer wallet-clone-specific metrics and config logs

The current code path has shown across the latest completed canaries:

1. profitability is possible (`worst_case_settlement_floor=+0.86` and `+1.19` in canaries 1 and 2)
2. the earlier directional-tail loss mode was real (`tail_at_expiry=10.55`, floor `-0.92` in canary 5)
3. the post-hardening expensive-core loss mode was also real even with tiny tails (`combined_avg_paid=1.012` / `1.045` in canaries 13 and 14)
4. lockout failures from canaries 3 and 4 are materially improved by the spread floor and floor-improvement bypass fixes

The current code path does **not** yet prove:

1. that profitability is consistent across multiple market conditions
2. that the new exact-gap repair and relative tail caps keep expiry tails controlled in live markets
3. that the new hard optional-growth gate stops expensive paired-core canaries in live markets
4. that the wallet-clone path is production-ready

---

## Current Practical Reading

Relative to Sprint 4 requirements, the current tree is approximately:

1. runnable as an isolated wallet-clone mode
2. mechanically aligned on startup ownership and missing-side repair
3. materially closer to an aggressive inventory builder than the older settlement-shaper path
4. proven capable of profitable books in some runs, but not yet stable across regimes
5. strong on startup, taper, rollover lifecycle, and now also on mid-market participation
6. still exposed to two live economic failure modes: directional tail and expensive paired core
7. still needing more canary runs and tighter guards before claiming consistent profitability

---

## Latest Completed Canaries

Reviewed completed runs date: 2026-03-16

The latest completed canaries before the incomplete `btc-updown-5m-1773598800` start were:

| Market | Result | `combined_avg_paid` | `tail_at_expiry` | `worst_case_settlement_floor` | Main read |
|---|---|---|---|---|---|
| `btc-updown-5m-1773598200` | LOSS | `0.960` | `10.55` | `-0.92` | profitable core, losing tail |
| `btc-updown-5m-1773598500` | LOSS | `1.026` | `0.01` | `-1.55` | balanced book, expensive core |

What these completed runs confirm:

1. The canary 3 and 4 lockout failures are materially improved.
   - the bot stayed active through the middle of the market
   - `fills_per_market` remained high (`36` and `26`)
2. A directional tail is still a live loss mode.
   - canary 5 ended `qYES=75.55`, `qNO=65.00`
   - a late lighter-side YES repair filled `20` shares even though the exact live gap was about `10`
   - taper then started with `budget_too_small`, so the tail was never repaired
3. An expensive paired core is still a live loss mode.
   - canary 6 ended essentially flat (`tail_at_expiry=0.01`)
   - but `combined_avg_paid=1.026`, so the book was guaranteed to lose on either outcome
4. The averaging-down exception is now a mixed result.
   - it successfully removed the earlier mid-market freeze
   - it also allowed new paired adds while projected paired cost remained above `1.00`
5. Final quiet and rollover ownership still work mechanically.
   - both losing runs rolled over cleanly
   - the losses were economic, not lifecycle failures

The current-tree interpretation should therefore be:

1. profitable books are possible
2. the bot now participates actively enough to expose real edge and real risk
3. the remaining blockers are no longer "bot does nothing"
4. the remaining blockers are "bot still buys the wrong shape or too-expensive shape"

---

## Representative Reviewed Runs

| Metric | Canary 2 (`1773596100`) | Canary 5 (`1773598200`) | Canary 6 (`1773598500`) |
|---|---|---|---|
| Result | profit | loss | loss |
| `paired_size` | 64.98 | 65.00 | 59.99 |
| `combined_avg_paid` | 0.982 | 0.960 | 1.026 |
| `tail_at_expiry` | 0.01 | 10.55 | 0.01 |
| `worst_case_settlement_floor` | +1.19 | -0.92 | -1.55 |
| Main failure / success read | balanced profitable core | directional tail | expensive paired core |

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

1. the strategy is now aggressive enough to participate and build inventory in real size
2. the bid cap prevents worst-case chasing but can create temporarily unfillable lighter-side orders
3. averaging-down keeps the bot active, but it can still build an expensive paired core if the market stays overround
4. startup basis can still become expensive before both sides are restored, and late repair sizing can still overshoot into a fresh tail

---

## Known Remaining Gap

The dominant remaining gaps are no longer `PairBuild` freeze or "bot does not trade."

The dominant remaining observations after the reviewed completed canaries are:

1. Profitability is not yet stable across market conditions.
   - the same code produced both positive floors and negative floors on `2026-03-16`
   - more canary runs are still needed before claiming stable profitability
2. Lighter-side repair sizing can overshoot the actual live gap.
   - canary 5 rounded a YES repair up to `20` shares while the exact gap was about `10`
   - that single fill turned a nearly balanced book into the losing tail
3. Taper can arrive with too little remaining budget to repair a live tail.
   - canary 5 entered taper with `budget_too_small`
   - lifecycle ownership worked, but no late cleanup happened
4. The bid cap still uses global anchor state, not per-order tracking.
   - in cancel/fill race conditions, the cap can reference stale anchor prices
   - the error is bounded by 1-2 ticks and is a known hardening item
5. Averaging-down still uses relative improvement, not an absolute parity stop.
   - canary 6 kept adding while projected paired cost stayed in `RepairOnly`
   - this improved the blended book, but the final paired core still finished above `1.00`
6. Startup cost discipline is still not directly addressed.
   - `SeedCompletion` bypasses cost guards by design
   - profitable recovery remains possible, but it still depends on later market opportunity

After the latest reviewed completed canaries, `0.1.28` should be read as:

1. Sprint 4 runtime implemented
2. config surface implemented
3. metrics surface implemented
4. repeated live canaries reviewed
5. startup, taper, and rollover lifecycle acceptable
6. mid-market `PairBuild` participation now functional - freeze eliminated, averaging-down working
7. lighter-side repair bid cap active
8. profitability demonstrated in some runs, but not yet stable
9. two live loss modes remain: directional tail and expensive paired core
10. more canary runs and tighter guards are still needed before claiming consistent profitability
