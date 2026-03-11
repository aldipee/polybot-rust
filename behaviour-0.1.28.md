# CURRENT BEHAVIOUR

## Version 0.1.28

Scope: Sprint 4 only  
Mode: `EXEC_MODE=WALLET_CLONE`  
Date: 2026-03-11

This file is not a design target.
It is a concrete runtime note for the current Sprint 4 wallet-clone path in the working tree.

---

## Update Note

`0.1.28` records the current runnable Sprint 4 wallet-clone implementation.

The main current-tree changes are:

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

The latest live canary has now been reviewed.

That canary confirms:

1. `OpenBoth` and `SeedCompletion` now work mechanically in live flow.
2. Both startup targets were met in the reviewed run:
   - `both_by_30s=true`
   - `both_by_60s=true`
3. `Taper` and final quiet behavior are also working as intended.
4. The main remaining problems are no longer startup ownership bugs.
5. The main remaining problems are `PairBuild` churn, minimum-order enforcement leakage, and still-weak paired economics.

---

## Executive Summary

The Sprint 4 runtime is now structurally runnable and has been through a real wallet-clone canary review.

The current code path can:

1. pre-arm before open
2. seed both sides
3. treat one-sided startup fills as normal startup completion
4. replenish through `PairBuild`
5. taper late
6. emit wallet-clone-specific metrics and config logs

The current code path does **not** yet prove:

1. that `PairBuild` is economically close enough to the target wallet
2. that stale-order handling in live maker flow is tuned correctly
3. that the wallet-clone path is production-ready

---

## Current Practical Reading

Relative to Sprint 4 requirements, the current tree is approximately:

1. runnable as an isolated wallet-clone mode
2. mechanically aligned on startup ownership and missing-side repair
3. materially closer to an aggressive inventory builder than the older settlement-shaper path
4. now backed by one real canary, but still not behaviorally close enough to declare wallet match

---

## Latest Live Canary

Reviewed run date: 2026-03-11

Observed headline metrics from the reviewed run:

1. `market_participated=true`
2. `fills_per_market=23`
3. `total_fill_shares=135.00`
4. `maker_fill_share=0.778`
5. `paired_size=65.00`
6. `unmatched_size=5.00`
7. `pair_coverage=0.929`
8. `share_skew=1.077`
9. `combined_avg_paid=1.019`
10. `fills_after_taper_start=0`
11. `fills_after_final_quiet=0`

What the canary confirms:

1. Startup is materially improved.
2. `OpenBoth` produced the first fill at roughly `3.4s`.
3. `SeedCompletion` restored the missing side by roughly `6.6s`.
4. The bot traded actively through the main window.
5. Taper and final quiet suppressed late activity correctly.

What the canary still shows:

1. `PairBuild` is still canceling viable resting maker orders too aggressively.
   - repeated `lighter_side_live_order_stale_cancel`
   - repeated `asymmetric_submit_stale_cancel`
   - many of those cancels occur around the current `STALE_SECONDS` horizon
2. The bot still leaks exchange-invalid small orders in some paths.
   - reviewed run included `invalid amount for a marketable BUY order ($0.3), min size: $1`
3. The bot is still not economically clean enough in normal flow.
   - paired inventory ended slightly above break-even cost
   - residual unmatched inventory remained at market end
4. The remaining gap is no longer "can it trade at all?"
5. The remaining gap is "can `PairBuild` accumulate paired inventory without churn and overpaying?"

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
4. keep the runtime aggressively refreshed with `1s` cadence and `2`-tick repricing
5. stop new activity near expiry and stay mostly silent in the final `30s`
6. spend out of a roughly `70`-dollar usable budget after reserve

The most important current tradeoff in this profile is:

1. it is aggressive enough to participate and build inventory
2. but `STALE_SECONDS=3` is currently too short for the live maker behavior seen in this canary
3. that short stale timeout is likely one of the direct causes of cancel/repost churn in `PairBuild`

---

## Known Remaining Gap

The dominant remaining gap is no longer hidden config wiring, missing controller ownership, or startup completion.

The dominant remaining gaps after the reviewed canary are:

1. `PairBuild` stale timeout policy is too aggressive for live maker resting orders.
2. Wallet-clone normal flow can still leak sub-minimum notional orders to the exchange.
3. Paired-growth economics are still too weak:
   - `combined_avg_paid` finished above `1.00`
   - residual unmatched size remained
4. The next validation loop should focus on:
   - reducing stale cancel / repost churn
   - hardening minimum-notional enforcement
   - improving post-startup paired-growth cost quality

After the first reviewed canary, `0.1.28` should be read as:

1. Sprint 4 runtime implemented
2. config surface implemented
3. metrics surface implemented
4. first live canary reviewed
5. startup and taper behavior acceptable
6. `PairBuild` still needs more work before claiming wallet-clone match
