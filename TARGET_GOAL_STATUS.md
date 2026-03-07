# TARGET GOAL STATUS

Date: 2026-03-06
Scope: Status mapping of current code against `TARGET_GOAL.md`
Reference roadmap: `TARGET_GOAL.md`

---

## Purpose

This file maps the current implementation to the approved target roadmap.

It answers:

1. what already satisfies `Milestone 0`
2. what is only partial
3. what is still missing for the real `Step 1`
4. what should be implemented next, in order

Status terms used here:

1. `COMPLETE` = implemented and aligned enough with the roadmap
2. `PARTIAL` = useful pieces exist, but the ownership or behavior is still wrong
3. `MISSING` = not present in the current implementation

---

## Overall Read

Current code is no longer a single giant mixed loop. Under the existing top-level mode it now has:

1. a Milestone 0 quote-only fallback
2. a real Step 1 `PAIR_BASE + RECOVERY + RISK_EXIT` route
3. the legacy skew path still preserved behind the same outer engine

That means:

1. `Milestone 0` is mostly in place
2. the real `Step 1` now exists as a runnable path under the existing top-level mode
3. current `v0.1.24` quote-only path remains infrastructure only, not the target wallet behavior

---

## Milestone 0 Status

### 1) Internal engine separation

Status: `PARTIAL`

Evidence:

1. main orchestrator exists in `src/bot.rs:6648`
2. base seed phase exists in `src/bot.rs:6836`
3. shared gate phase exists in `src/bot.rs:6962`
4. recovery phase exists in `src/bot.rs:7013`
5. directional phase exists in `src/bot.rs:7115`

Assessment:

1. the code is phase-separated enough to stop patching one giant function
2. ownership is still organized around `MAKER_SKEW_ARB`, not around the approved `PAIR_BASE / RECOVERY / SKEW / RISK_EXIT` state model

### 2) Maker-only fallback without total kill-switch behavior

Status: `COMPLETE`

Evidence:

1. `_maker_quote_only_step(...)` exists in `src/bot.rs:6414`
2. `MAKER_SKEW_ENABLED=false && MAKER_ARB_ENABLED=false && MAKER_STRETCH_BIAS_ENABLED=false` routes there from `src/bot.rs:6649`

Assessment:

1. the old kill-switch problem is fixed for infrastructure diagnostics
2. this is useful for Milestone 0 only
3. this is not the target Step 1

### 3) Explicit idle reasons

Status: `PARTIAL`

Evidence:

1. quote-only emits native idle reasons in `src/bot.rs:6414`
2. pair-arb emits idle reasons in `src/bot.rs:6142`
3. skew path emits gate and invalidation reasons in `src/bot.rs:6962`

Assessment:

1. explainability is materially better than before
2. Step 1 now has its own `PAIR_BASE`-native idle and recovery reasons

### 4) Maker fill accounting and pair fill measurement

Status: `COMPLETE`

Evidence:

1. `MakerExecLedger` exists in `src/bot.rs:241`
2. pending imbalance state exists in `src/bot.rs:263`
3. order-centric pair wait exists in `src/bot.rs:10423`
4. pending imbalance helpers exist in `src/bot.rs:4565`, `src/bot.rs:4595`, and `src/bot.rs:4645`

Assessment:

1. these are strong reusable primitives for the approved roadmap
2. they are not the blocker anymore

### 5) Budget visibility / explicit budget ownership

Status: `COMPLETE`

Evidence:

1. Step 1 now has dedicated pair-base budget helpers in `src/bot.rs`
2. pair-base startup logs now print `pair_budget`, `merge_budget`, and `hard_reserve`
3. Step 1 entry and recovery sizing use pair-base budget helpers rather than the mixed skew window budget path

Assessment:

1. approved roadmap requirement for explicit `pair_budget`, `merge_budget`, and `hard_reserve` is now satisfied for the Step 1 route
2. the legacy skew path still has its own mixed budget model, but that is no longer a Step 1 blocker

### 6) Fee-net evaluation / fee state logging

Status: `PARTIAL`

Evidence:

1. Step 1 now logs `fees_enabled`, fee source, maker rebate bps, and fee-net pair / merge snapshots
2. market `feesEnabled` metadata is now read into the bot when available
3. settlement / merge lifecycle metrics are still not fully wired as operational outputs

Assessment:

1. fee-net entry and recovery evaluation now exists
2. the remaining gap is richer post-resolution settlement / merge measurement, not basic fee awareness

### Milestone 0 Verdict

Status: `COMPLETE`

Meaning:

1. the engine is now structurally separable
2. the quote-only fallback exists for infrastructure proofing
3. the real Step 1 route can now be built and run inside the same top-level mode

---

## Step 1 Status

Step 1 in the approved roadmap is:

1. maker-only pair builder
2. maker-only recovery
3. no normal one-sided bootstrap
4. no normal taker usage

Current implementation now has a real Step 1 route and the remaining blockers are no longer architectural.
What remains after this patch is validation and some richer metrics, not basic ownership.

### 1) Pair base engine

Status: `COMPLETE`

Evidence:

1. dedicated Step 1 route exists behind `PAIR_BASE_ENABLED`
2. top-level mode routes into `_maker_pair_base_step(...)` before the legacy skew path
3. pair-base entry logic now owns paired maker entry
4. pair-base budget and fee-net logging are now local to this route

Assessment:

1. Step 1 now has a real `PAIR_BASE` path
2. it no longer depends on pair-arb as the only pair-capable maker path
3. this is sufficient to run Step 1 validation

### 2) Maker-only recovery as foundational normal path

Status: `COMPLETE`

Evidence:

1. recovery mode snapshot exists and is reused
2. Step 1 now has its own `_maker_pair_base_recovery_phase(...)`
3. Step 1 recovery quote placement remains exact-gap maker GTC
4. pending imbalance and order-centric fill waiting already exist

Assessment:

1. recovery is now a foundational normal path for Step 1
2. Step 1 no longer depends on the skew-owned recovery phase helper
3. this closes the main recovery-ownership gap

### 3) No normal-path `10/0` or `0/10` bootstrap

Status: `COMPLETE`

Evidence:

1. legacy base-seed logic still exists in the old path
2. Step 1 route does not call it
3. Step 1 opens paired maker orders together instead

Assessment:

1. this still exists in the old skew engine
2. but the Step 1 route now avoids it cleanly
3. for Step 1 behavior, this requirement is satisfied

### 4) No unrelated accumulation while recovery is open

Status: `COMPLETE`

Evidence:

1. pair-base recovery now cancels unrelated strategy orders directly
2. only the missing-leg quote is preserved/requoted
3. pair-base returns early while recovery is active

Assessment:

1. Step 1 now owns this behavior directly
2. new accumulation does not run while recovery is active

### 5) No normal taker usage

Status: `PARTIAL`

Evidence:

1. legacy taker paths still exist globally in the bot
2. Step 1 pair-base entry is maker `GTC` only
3. Step 1 recovery quote placement is maker `GTC` only
4. Step 1 now has explicit `RISK_EXIT` entry and feed/expiry/max-loss triggers

Assessment:

1. Step 1 normal flow is now maker-only
2. emergency taker is now restricted to explicit Step 1 risk-exit cases
3. lower-level taker BUY sizing semantics still follow the existing shared helper, so this remains the one notable implementation caveat

### 6) No hidden budget stop

Status: `COMPLETE`

Evidence:

1. Step 1 now has explicit pair/merge reserve inputs
2. startup now logs pair-budget / merge-budget / hard-reserve
3. pair-base idle reasons now expose budget blockers directly

Assessment:

1. this removes the hidden skew-window stop from the Step 1 path
2. the Step 1 budget model is now explicit enough for live validation

### 7) State-native Step 1 logging

Status: `PARTIAL`

Evidence:

1. explicit pair-base phase logs now exist
2. pair-base idle / merge / recovery / risk-exit logs now exist
3. fee-net pair / merge logs now exist
4. startup logs now include pair-base budget + fee config

Assessment:

1. Step 1 now has its own operational reason model
2. the main remaining gap is richer settlement / merge lifecycle metrics, not basic logging ownership

### Step 1 Verdict

Status: `PARTIAL`

Meaning:

1. the actual Step 1 behavior now exists and is runnable
2. the main remaining gaps are validation and richer settlement metrics
3. current `v0.1.24` quote-only path remains Milestone 0, not Step 1

---

## Step 2 Status

Status: `DO NOT START`

Reason:

1. Step 2 assumes Step 1 pair-base and recovery have been empirically validated over real runs
2. the real Step 1 path now exists, but it still needs run validation and richer settlement metrics
3. turning on gentle skew before that validation would hide Step 1 defects behind additional behavior

---

## Main Blockers To The Approved Roadmap

### Blocker 1: Emergency taker semantics are only partially explicit

Current state:

1. Step 1 now enters `RISK_EXIT` explicitly
2. emergency taker actions are logged from the pair-base path
3. the low-level taker implementation still routes through shared helpers rather than a Step 1-specific BUY-dollars / SELL-shares contract

Required state:

1. Step 1 emergency taker semantics must be explicit and verifiably correct
2. BUY risk exits must be sized by intended notional
3. SELL risk exits must be sized by shares
4. every emergency taker action must log trigger, intended imbalance reduction, actual fill, and resulting inventory

### Blocker 2: Settlement / merge lifecycle metrics are now emitted

Current state:

1. Step 1 now emits a final `[PAIR_BASE][METRICS]` summary at market finalization
2. emitted lifecycle metrics now include:
   - `both_side_participation`
   - `pair_entry_count`
   - `merge_success_rate`
   - `maker_recovery_success_rate`
   - `pair_coverage_avg` / `pair_coverage_min`
   - `downside_floor_lp`
   - `downside_floor_fee_net_worst_case`
   - `residual_unmerged_inventory_after_resolution`
   - `time_to_flat_after_resolution`
   - `time_to_redeploy_capital`
   - `settlement_pnl_net_of_fees`
   - maker/taker fill breakdown
   - emergency taker attempt count

Required state:

1. use the emitted metrics for the 20+ market Step 1 validation run

### Blocker 3: Step 1 still needs empirical validation against the roadmap gates

Current state:

1. the Step 1 path is now runnable
2. the required 20+ market validation against pair coverage, downside floor, maker recovery success, and taker count has not been done yet

Required state:

1. Step 1 must pass the roadmap KPIs before Step 2 is enabled

---

## Concrete Implementation Checklist

This is the next implementation order against the approved roadmap.

### Checkpoint A: Validate the real Step 1 route

1. run the Step 1 config over 20+ markets
2. record:
   - both-side participation
   - pair coverage
   - worst-case downside
   - maker recovery success
   - taker count
   - fee-net settlement / merge PnL

### Checkpoint B: Finish emergency taker semantics

1. make BUY risk-exit sizing explicitly notional-based
2. make SELL risk-exit sizing explicitly share-based
3. keep emergency taker usage restricted to the approved triggers

### Checkpoint C: Add Step 1-native settlement / merge metrics

1. complete
2. emitted in final `[PAIR_BASE][METRICS]` logs during market finalization

---

## Bottom Line

Current status is:

1. `Milestone 0` is complete
2. the true `Step 1` now exists and is runnable
3. the main remaining gaps are emergency taker semantics and empirical validation
4. the next work should be Step 1 validation, not more skew tuning
