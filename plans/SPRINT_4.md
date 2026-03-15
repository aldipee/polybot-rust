# Sprint 4: Wallet-Clone Pair-First Accumulator

## Sprint Goal
Introduce a new wallet-clone execution track that copies the observed wallet mechanics more closely than Sprint 3.

The Sprint 4 objective is:

1. always participate in nearly every 5-minute BTC market
2. seed both sides immediately with maker `BUY` orders
3. treat one-sided startup fills as normal
4. restore the missing side quickly
5. keep accumulating through most of the window with continuous two-sided replenishment
6. taper late and mostly stop in the last minute
7. stay maker-heavy and `BUY`-only in the normal path
8. stay aggressive in normal flow only while paired cost, tail, and repair reserve remain inside the allowed bands

This sprint is not an optimization pass.
It is a behavior-clone sprint.

## Clone Target
The target is an executed-behavior clone of the observed wallet, not a hidden quote-management clone.

Sprint 4 should match these observable traits as closely as practical:

1. near-total market participation
2. maker-first normal flow
3. both sides started fast
4. one-sided startup fill treated as normal startup completion, not exceptional failure
5. continuous replenishment with many fills per market
6. activity concentrated before the last minute
7. no normal intrawindow sell exits
8. aggressive inventory building rather than selective low-frequency entry
9. profitable through repeated cheap maker accumulation, not by trading rarely

## Why Sprint 4 Exists
Sprint 3 is currently a settlement-shaper objective.
It optimizes for:

1. more dollars on the favorite
2. more shares on the underdog
3. target-shape control
4. settlement-shape repair

That is not the same objective as the wallet-clone review.

Sprint 4 therefore must not be framed as:

1. another Sprint 3 tuning pass
2. a slight target-band adjustment
3. a favorite/underdog refinement

Sprint 4 is a distinct behavioral track:

1. pair-first
2. accumulation-first
3. startup-completion-first
4. shape-light
5. late-tapered
6. aggressive rather than conservative

## Status
- Overall status: `IN PROGRESS`
- Target outcome: `SECOND_WALLET-CLONE CANARY`
- Dependency on Sprint 3: `REUSE PARTS ONLY`
- Recommended runtime boundary: `NEW MODE`
- Current dominant reason: `CONSULTATION_RULE_INTEGRATION_AND_SECOND_CANARY`

## Working Metric Definitions
Use these working definitions consistently across Sprint 4 planning, code, and canary review.

1. `paired_size = min(qYES, qNO)`
2. `tail_size = abs(qYES - qNO)`
3. `share_skew_ratio = max(qYES, qNO) / min(qYES, qNO)` when both sides are positive
4. `worst_case_settlement_floor = min(qYES, qNO) - total_cost`
5. `projected_paired_cost` means the projected paired average paid after the next action is applied
6. `below_snapshot` means the optional buy price is strictly better than the same-side snapshot price at decision time

## Required Runtime Boundary
Sprint 4 should land behind a separate top-level path.

Recommended mode:

1. `EXEC_MODE=WALLET_CLONE`

Reason:

1. Sprint 3 remains a settlement-shaper and should stay explainable on its own terms
2. Sprint 4 should not inherit favorite-dollar / underdog-share goals as hidden defaults
3. clone validation needs clean logs, metrics, and rollback criteria independent from Sprint 3

Alternative only if required:

1. `SETTLEMENT_SHAPER_CLONE_MODE=true`

This is second-best.
Prefer a dedicated mode.

## Core Strategy Requirements

### 1. Pre-Arm Requirement
The engine must be ready before the market opens.

Required behavior:

1. market discovery completed before window start
2. YES/NO asset mapping completed before window start
3. market/user channels already warm or warming before window start
4. initial paired maker quotes prepared before window start

Not acceptable:

1. spending the first opening seconds discovering the market
2. delaying first paired seed while waiting for basic metadata

### 2. Opening Requirement
At window start:

1. place maker `BUY` orders on both sides immediately
2. use small seed clips
3. do not require favorite / underdog confirmation to start
4. do not prefer one side during initial seed

### 3. Startup Asymmetry Requirement
One-sided startup fill is normal.

Required behavior:

1. if one side fills and the other does not, enter a dedicated startup-completion state
2. missing-side restoration overrides normal shape logic
3. missing-side restoration overrides hard skew vetoes
4. missing-side restoration may borrow budget from later phases
5. the controller should keep posting the missing side until both sides exist or the market has clearly missed the target timing profile

Timing expectations:

1. both sides positive by `30s` in most markets
2. both sides positive by `60s` in almost all markets

### 4. Continuous Replenishment Requirement
Once both sides exist:

1. keep two-sided maker quotes working through most of the market
2. replenish after fills
3. add both sides while reasonably balanced
4. add the lighter side first when imbalance stretches
5. do not collapse normal flow into one-shot seed then idle
6. prefer continued building over conservative waiting when budgets and hard risk limits still allow it

### 5. Late Taper Requirement
Late behavior should resemble the observed wallet timing profile.

Required behavior:

1. most activity occurs before `240s`
2. after about `240s`, stop new expansion
3. allow only small maintenance / repair late
4. almost nothing should happen in the final `30s`

### 6. Normal-Flow Execution Requirement
Normal flow must be:

1. maker-first
2. `BUY`-only
3. post-only resting orders by default
4. no normal intrawindow sell exits

Taker is allowed only for explicit emergency policy, not ordinary accumulation.

### 7. Control Variable Requirement
Sprint 4 should optimize for:

1. both-side existence
2. paired size
3. unmatched size
4. combined average paid / two-sided cost quality
5. time-to-second-side
6. participation and fill cadence

Sprint 4 should not optimize for as a first-class normal-path target:

1. favorite spend target
2. underdog share target
3. preferred final settlement pattern

### 8. Aggression Requirement
Sprint 4 must behave like an aggressive inventory builder inside cheap-pair regimes, not like a conservative filter and not like a participation-at-any-price bot.

Required normal-path policy:

1. both-side completion is preferred over shape neatness
2. paired replenishment is preferred over exact-equality cosmetics
3. participation is preferred only while projected paired cost remains in the allowed bands and repair reserve remains intact
4. the controller should keep building while the next action preserves acceptable paired cost, tail, and worst-case settlement floor
5. heavy-side growth must stop once remaining budget is no longer enough to fund likely lighter-side repair

Not allowed as an always-on normal-path veto:

1. exact equality as a primary controller objective
2. favorite/underdog target-pressure gate
3. mild temporary imbalance by itself

Allowed hard blockers:

1. no legal order size
2. no usable quote
3. stale or invalid market data
4. explicit hard budget breach
5. explicit terminal emergency policy
6. projected paired cost outside the allowed regime for the requested action
7. optional heavy-side growth that would strand a repair tail

### 9. Paired-Cost / Floor Requirement
Projected paired cost and worst-case settlement floor are first-class Sprint 4 control variables.

Required policy:

1. `OpenBoth` must not require paired-cost profitability to seed both sides
2. `SeedCompletion` must not require paired-cost profitability to restore the missing side
3. `PairBuild` must gate optional growth on `projected_paired_cost`, not only on current held-book cost
4. `PairBuild` must evaluate `worst_case_settlement_floor` and `tail_size`, not just exact equality
5. once `projected_paired_cost > 1.00`, optional growth stops
6. `1.00 - 1.02` is repair-only territory
7. above `1.02`, default behavior is freeze / skip unless a later consultation-approved emergency repair exception is added explicitly
8. after `240s`, only actions that improve floor or reduce tail should remain available

Recommended operating bands:

1. `< 0.94`: strong paired growth
2. `0.94 - 0.98`: normal paired growth
3. `0.98 - 1.00`: maintenance / reduced growth
4. `1.00 - 1.02`: repair-only
5. `> 1.02`: freeze / skip by default

### 10. Consultation-Derived Economic Rule Requirement
Sprint 4 should absorb the strongest rule signals from the trader analysis and consultant review.

Required policy:

1. trade both sides or skip the market
2. optional buys must be below same-side snapshot price
3. optional buys must require non-negative `edge_model_minus_price` or an explicit documented fallback if that signal is not available live
4. meaningful size-up should begin only in a clearly positive edge band, with `0.05` as the recommended starting threshold
5. default skew should remain mild and should favor the higher-priced side when any extra size is justified
6. if directional overlay survives after `60s`, it should not fight the sign of `binance_delta_from_start`
7. lighter-side repair should be exact-gap or smallest-valid-repair based, not repeated blind `5/10` retries
8. the engine should not claim clone-complete status until the cheap-pair rule, tail rule, and below-snapshot rule are all explicit in code and metrics

### 11. Post-Canary PairBuild Hardening Requirement
After the first reviewed Sprint 4 canary, the remaining gap is no longer startup ownership.

The remaining gap is `PairBuild` quality.

Required hardening:

1. wallet-clone live maker orders must not be canceled on the shared generic `STALE_SECONDS` horizon alone
2. `PairBuild` must use a wallet-clone-specific stale / live-order timeout policy for:
   - lighter-side-first live orders
   - paired-growth live orders
   - asymmetric submit-resolution cleanup
3. the runtime must not send exchange-invalid sub-minimum maker orders after final quantization
4. minimum maker notional must be re-checked after the final exchange-precision size is computed, not only before that path
5. optional normal-flow `PairedGrowth` must use projected matched-book cost quality, not only current inventory cost quality
6. if projected post-add paired inventory would become too expensive, the bot should:
   - reduce clip first
   - then suppress optional paired growth
   - but still allow startup completion and required lighter-side recovery
7. when the live book is materially skewed, lighter-side-first ownership should dominate until the skew returns inside a tighter normal band
8. exact-gap repair and repair-budget reserve should prevent cheap paired cores from being stranded by a losing tail
9. opposite-side live order preservation during lighter-side repair must be conditional:
   - preserve only if the remaining size and price are still compatible with the repair target
   - otherwise hand it off and repair cleanly

Not acceptable after this hardening pass:

1. repeated stale-cancel / repost churn on still-viable resting maker orders
2. exchange rejects for sub-minimum maker notional
3. normal paired-growth continuing to add optional inventory while projected paired cost is already above a small acceptable band over payout

Expected next-canary targets after this hardening pass:

1. `both_by_30s=true`
2. `both_by_60s=true`
3. `unmatched_size <= 2`
4. `share_skew <= 1.03`
5. `combined_avg_paid <= 1.005`
6. materially fewer `*_stale_cancel` events than the first reviewed canary

## Required State Model
Sprint 4 should use a simpler mechanical state flow than Sprint 3.

### `PreArm`
Use before market open.

Responsibilities:

1. warm discovery
2. arm market metadata
3. prepare first two-sided quotes

### `OpenBoth`
Use at market start.

Responsibilities:

1. submit both sides immediately
2. seed with small maker clips
3. no directional preference

### `SeedCompletion`
Use when exactly one side has live inventory during startup.

Responsibilities:

1. restore the missing side
2. ignore normal shape goals
3. ignore hard skew veto unless the action worsens the missing-side problem
4. keep ownership until both sides exist or the timing profile has already failed
5. ignore hard CPP profitability vetoes during true missing-side restoration

### `PairBuild`
Main normal-flow state.

Responsibilities:

1. keep both sides working
2. replenish after fills
3. build paired size through most of the window
4. favor the lighter side when imbalance grows
5. use CPP only as a light sizing hint, not as a standing hard stop

### `Taper`
Late state.

Responsibilities:

1. stop new optional expansion
2. allow only small maintenance / completion
3. protect the late book

### `HoldSettleRollover`
Terminal normal-flow state.

Responsibilities:

1. stop foreground trading near expiry
2. preserve rollover behavior
3. keep normal flow `BUY`-only

## Phase Schedule Requirements
Sprint 4 should explicitly align with the observed timing distribution.

Recommended phase windows:

1. `0-30s`: immediate two-sided seed
2. `30-60s`: startup completion and early paired build
3. `60-180s`: main accumulation
4. `180-240s`: late paired build / maintenance
5. `240-300s`: taper / repair-only / mostly rest

Required policy:

1. keep early activation strong
2. keep main volume concentrated before `240s`
3. use `0-210s` as the normal paired-growth band
4. use `210-240s` as reduced-growth / maintenance band
5. use `240-270s` as repair-first / taper band
6. use `270-300s` as no-optional-adds band

## Clip And Sizing Requirements
Observed behavior implies:

1. small seed and repair clips
2. repeated replenishment
3. larger passive clips as a secondary pattern

Sprint 4 should support:

1. opener clips around `12-24`
2. exact-gap lighter-side repair rounded up to the smallest valid repair size
3. repeated passive replenishment
4. optional larger passive clip family around `40-80` only in the strongest paired-cost regimes

Large clips must not replace the many-fill engine.

## Scope
In scope:

1. new wallet-clone mode boundary
2. pre-arm lifecycle
3. startup-completion owner
4. continuous paired replenishment behavior
5. clone-specific timing and late taper rules
6. clone metrics and canary logging
7. tests for state transitions, timing, and gating

Out of scope:

1. hidden quote-management imitation beyond observable behavior
2. Sprint 3 stretch overlay
3. favorite-dollar / underdog-share settlement shaping as a core target
4. optimization for profitability beyond observed mechanics
5. release/version bump

## Workstreams

### Workstream A: Mode Boundary And Objective Isolation
Status: `COMPLETE`

Objective:
Create a clean Sprint 4 wallet-clone path without hidden Sprint 3 shaping assumptions.

Tasks:
- [x] Add dedicated top-level runtime dispatch for the Sprint 4 mode
- [x] Ensure Sprint 4 does not route through Sprint 3 target-shape scoring
- [x] Add Sprint 4 startup logs showing:
  - mode
  - phase controller
  - pre-arm status
  - clip family
  - clone timing targets
- [x] Keep existing `MAKER_SKEW_ARB` and `SETTLEMENT_SHAPER` behavior unchanged

Acceptance:
- [x] Sprint 4 can run without invoking favorite-dollar / underdog-share controller goals
- [x] Logs make the objective difference from Sprint 3 explicit

### Workstream B: Pre-Arm Lifecycle
Status: `COMPLETE`

Objective:
Make the engine ready before the market opens.

Tasks:
- [x] Require pre-open selected-market and asset-mapping readiness
- [x] Add readiness checks for:
  - next market selected
  - asset IDs available
  - market/user feed freshness
  - initial quote inputs available
- [x] Add explicit pre-arm hold reasons
- [x] Ensure market-open does not start with fresh discovery by default

Acceptance:
- [x] Sprint 4 can enter the market already armed
- [x] startup logs show pre-arm completed before opening actions

### Workstream C: Open-Both Seed Engine
Status: `COMPLETE`

Objective:
Submit both sides immediately and neutrally at market start.

Tasks:
- [x] Add `OpenBoth` owner/state
- [x] Submit paired maker `BUY` orders on both sides immediately at window start
- [x] Use small seed clips
- [x] Avoid favorite/underdog gating during opening
- [x] Add explicit paired-open logs and metrics

Acceptance:
- [x] both sides are attempted immediately at market start
- [x] opening behavior remains maker-first and neutral

### Workstream D: Seed Completion Ownership
Status: `COMPLETE`

Objective:
Treat one-sided startup fills as normal startup completion.

Tasks:
- [x] Add `SeedCompletion` owner/state distinct from Sprint 3 `ShapeRepair`
- [x] Route true one-sided startup books into `SeedCompletion`
- [x] Ignore normal shape targets while in `SeedCompletion`
- [x] Ignore hard skew vetoes for true missing-side restoration unless the action worsens missing-side completion
- [x] Ignore hard CPP profitability vetoes for true missing-side restoration
- [x] Require only missing-side quote health during `SeedCompletion`, not paired parity/spread readiness
- [x] Allow `SeedCompletion` to borrow from later phase budgets
- [x] Add explicit timing counters:
  - time to first side
  - time to second side
  - both sides by `30s`
  - both sides by `60s`

Acceptance:
- [x] missing-side startup recovery is not blocked by normal shape gates
- [x] logs clearly distinguish startup completion from two-sided shape repair

### Workstream E: Continuous Pair Build And Replenishment
Status: `COMPLETE`

Objective:
Make normal flow look like a continuous paired accumulator.

Tasks:
- [x] Add `PairBuild` owner/state
- [x] Repost two-sided maker quotes after fills
- [x] keep paired growth active through the main window
- [x] add both sides while reasonably balanced
- [x] add the lighter side first when imbalance stretches
- [x] prevent one-shot seed then long idle behavior
- [x] ensure CPP is only a light clip-sizing hint, not a hard normal-flow veto
- [x] prefer smaller or lighter-side clips before choosing idle when CPP is mediocre

Acceptance:
- [x] normal flow shows repeated paired or near-paired replenishment
- [x] build logic remains maker-first

### Workstream F: Late Taper And Final-Minute Suppression
Status: `COMPLETE`

Objective:
Match the observed late quieting profile.

Tasks:
- [x] Add `Taper` owner/state or equivalent late-phase policy
- [x] sharply reduce new accumulation after `240s`
- [x] allow only small maintenance/repair after `240s`
- [x] suppress almost all new activity in the final `30s`
- [x] preserve current foreground rollover stop-buffer behavior

Acceptance:
- [x] late logs show taper ownership and reduced activity
- [x] final-minute activity is intentionally minimal

### Workstream G: Clone Metrics And Reviewability
Status: `COMPLETE`

Objective:
Measure Sprint 4 against the clone fingerprint, not Sprint 3 shape goals.

Tasks:
- [x] Add startup timing metrics for:
  - first-fill latency
  - first-opposite-side latency
  - percent both-sides-positive by `30s`
  - percent both-sides-positive by `60s`
- [x] Add broader clone metrics for:
  - market participation rate
  - fills per market
  - maker fill share
  - fill distribution by window segment
  - late-minute fill share
  - paired size
  - unmatched size
  - average two-sided cost quality
  - average realized combined paid price by market
- [x] Add simple suppression metrics for:
  - skipped optional adds
  - startup-completion blocked count
- [x] Emit dedicated final Sprint 4 metrics summary
- [x] keep Sprint 3 metrics separate

Acceptance:
- [x] the canary can be judged directly against the wallet-clone review
- [x] metrics do not depend on favorite-dollar / underdog-share success

### Workstream H: Config Surface
Status: `COMPLETE`

Objective:
Expose the Sprint 4 knobs that are tuning knobs, not hidden logic switches.

Tasks:
- [x] Add new Sprint 4 env keys to `src/env_contract.rs`
- [x] Document Sprint 4 env keys in `ENVIRONMENT.md`
- [x] Expose controls for:
  - pre-arm lead time
  - small clip family
  - large clip family
  - phase budget slices
  - taper timing
  - startup completion time targets
  - clone maker cadence via the shared maker refresh / replace knobs

Acceptance:
- [x] operator-facing knobs exist for timing and sizing
- [x] core ownership logic does not depend on undocumented env flags

### Workstream I: Tests
Status: `IN PROGRESS`

Objective:
Add deterministic coverage for Sprint 4 behavior.

Tasks:
- [x] Add phase-boundary tests for Sprint 4 timing
- [x] Add `PreArm -> OpenBoth` timing/routing coverage
- [x] Add owner-routing coverage for one-sided `OpenBoth -> SeedCompletion`
- [x] Add owner-routing coverage for returning to `PairBuild` when both sides are live
- [x] Add `PairBuild` decision coverage for paired growth, lighter-side recovery, and CPP throttling
- [ ] Add `PairBuild -> Taper` routing coverage
- [ ] Add tests proving one-sided startup repair bypasses normal shape gating
- [ ] Add tests proving favorite/underdog does not gate opening or seed completion
- [x] Add tests proving late taper suppresses normal expansion
- [x] Add tests for clone metrics helper outputs

Acceptance:
- [ ] core Sprint 4 state and gating behavior is covered by Rust tests

### Workstream J: Post-Canary PairBuild Hardening
Status: `IN_PROGRESS`

Objective:
Move Sprint 4 from "mechanically live" to "closer wallet clone" by fixing the remaining `PairBuild` churn and economics gaps shown by the first reviewed canary.

Protected scope:
- Keep `PreArm`, `OpenBoth`, `SeedCompletion`, `Taper`, and `HoldSettleRollover` unchanged unless a later canary disproves them.
- Focus this workstream on mid-market `PairBuild` persistence, repair behavior, and paired economics.

Tasks:
- [x] Add wallet-clone-specific stale timeout policy for:
  - lighter-side-first live orders
  - paired-growth live orders
  - asymmetric submit-resolution cleanup
- [x] Stop using the shared generic `STALE_SECONDS` horizon as the only stale-cancel rule for wallet-clone `PairBuild`
- [x] Re-check minimum maker notional after final exchange quantization in the wallet-clone submit path
- [x] Block exchange-invalid sub-minimum maker orders before venue submission
- [x] Replace time-only stale cancel behavior with quality-aware persistence for `PairBuild` orders:
  - keep "aged but still acceptable" orders live
  - cancel only when economically invalid, quote inputs are unusable, or taper / rollover requires withdrawal
- [x] Preserve good opposite-side live orders during asymmetric updates instead of canceling for symmetry alone
- [x] Add per-side repost hysteresis and dedup:
  - no repeated same-side replace without a fill, meaningful quote move, or cooldown expiry
  - do not repost at the same price immediately after canceling it
- [x] Add projected post-add paired-cost guard for optional `PairedGrowth` only
- [x] Keep startup completion exempt from that optional paired-growth cost guard
- [x] Keep required lighter-side recovery exempt from that optional paired-growth cost guard
- [x] Make lighter-side recovery use smaller clips and stronger price discipline when cost quality weakens
- [x] Make lighter-side-first dominate while the book remains materially skewed and suppress competing paired-growth during repair
- [x] Fast-cancel broken paired-growth asymmetry when one live leg is orphaned by a recent counterpart submit reject
- [x] Add projected repaired-book pay-up discipline so lighter-side recovery clips down or holds before recreating an expensive paired book
- [ ] If still needed after the above, split `PairBuild` internally into `PairedGrowth` and `LighterRepair` behaviors without redesigning the outer Sprint 4 lifecycle
- [ ] Add explicit logs for:
  - wallet-clone-specific stale timeout decisions
  - persistence / cancel-validity decisions
  - asymmetric refresh preservation decisions
  - repost hysteresis suppressions
  - projected paired-cost suppression
  - blocked sub-minimum maker orders

Acceptance:
- [ ] first reviewed canary defects are addressed in code
- [ ] next canary is expected to reduce churn and improve paired economics without weakening startup or taper
- [ ] next canary should show materially fewer stale-cancel / repost loops, no invalid sub-minimum maker orders, and a lower final paired `combined_avg_paid` than the current reviewed baseline

### Workstream K: Consultation Rule Integration
Status: `IN_PROGRESS`

Objective:
Translate the consultation-derived rule set into explicit Sprint 4 code and canary criteria.

Tasks:
- [ ] Make `projected_paired_cost`, `tail_size`, and `worst_case_settlement_floor` first-class `PairBuild` decision inputs
- [x] Gate optional adds on below-snapshot price quality
- [x] Gate optional adds on non-negative `edge_model_minus_price` when that live signal exists, or document and implement an explicit fallback
  - Wallet-clone fallback while no live edge model exists: use the minimum same-side snapshot gap across YES/NO as the optional-buy edge proxy.
  - If either paired-growth bid is not strictly below its same-side snapshot, skip the optional add.
  - If the minimum snapshot gap is positive but below `0.05`, cap the optional add to the small clip bucket instead of allowing a larger size-up.
- [x] Add the recommended paired-cost regime map:
  - `< 0.94`
  - `0.94 - 0.98`
  - `0.98 - 1.00`
  - `1.00 - 1.02`
  - `> 1.02`
- [x] Add repair-budget reserve so heavy-side growth cannot strand the likely lighter-side repair
- [x] Replace fixed lighter-side repair behavior with exact-gap or smallest-valid-repair sizing
- [x] Tighten time-band policy to:
  - `0-210s` normal growth
  - `210-240s` reduced growth
  - `240-270s` repair-first
  - `270-300s` no optional adds
- [x] Make late wallet-clone taper decisions evaluate settlement floor and tail ahead of average-cost cosmetics
- [x] Clarify and implement when an opposite-side live order may be preserved versus canceled during lighter-side repair
- [x] Add canary reporting for:
  - below-snapshot fill rate
  - tail at expiry
  - worst-case settlement floor
  - paired-cost band occupancy

Acceptance:
- [ ] Sprint 4 decisions are explainable in terms of paired cost, floor, and tail rather than exact equality or fill count
- [ ] optional growth no longer occurs above `projected_paired_cost > 1.00`
- [ ] late canaries show controlled tail and acceptable worst-case settlement floor even when paired core cost is good
- [ ] the consultation-derived rules are visible in code, metrics, and canary review notes

## Public / Config Additions
Add a Sprint 4 env surface and document it.

Suggested initial keys:

```env
EXEC_MODE=WALLET_CLONE

WALLET_CLONE_PREARM_LEAD_SECONDS=20
WALLET_CLONE_SEED_CLIP_SMALL=15
WALLET_CLONE_REPAIR_CLIP_SMALL=15
WALLET_CLONE_CLIP_LADDER_LARGE=40,80

WALLET_CLONE_BUDGET_SEED_MIN_FRACTION=0.10
WALLET_CLONE_BUDGET_SEED_MAX_FRACTION=0.15
WALLET_CLONE_BUDGET_EARLY_MIN_FRACTION=0.15
WALLET_CLONE_BUDGET_EARLY_MAX_FRACTION=0.20
WALLET_CLONE_BUDGET_MAIN_MIN_FRACTION=0.45
WALLET_CLONE_BUDGET_MAIN_MAX_FRACTION=0.55
WALLET_CLONE_BUDGET_LATE_MIN_FRACTION=0.15
WALLET_CLONE_BUDGET_LATE_MAX_FRACTION=0.20
WALLET_CLONE_BUDGET_TAPER_MIN_FRACTION=0.05
WALLET_CLONE_BUDGET_TAPER_MAX_FRACTION=0.10

WALLET_CLONE_TARGET_BOTH_SIDES_BY_30S=0.80
WALLET_CLONE_TARGET_BOTH_SIDES_BY_60S=0.95
WALLET_CLONE_TAPER_START_SECONDS=240
WALLET_CLONE_FINAL_QUIET_SECONDS=30
WALLET_CLONE_BUY_ONLY_NORMAL_FLOW=true
```

## Canary Requirements
Sprint 4 canary should be judged by behavior, not by profit-first tuning.

Required canary review areas:

1. market participation
2. startup timing
3. missing-side restoration timing
4. maker fill share
5. fill cadence through the window
6. final-minute suppression
7. paired vs unmatched inventory
8. whether CPP stayed informational instead of suppressing normal inventory building
9. stale-cancel / repost churn in `PairBuild`
10. exchange-invalid maker-order rejects
11. projected paired-cost quality versus final combined average paid
12. below-snapshot optional fill rate
13. tail size and worst-case settlement floor at expiry

## Acceptance Criteria
Sprint 4 is complete only when all are true:

1. Sprint 4 runs behind its own clear mode boundary
2. market-open behavior is pre-armed and immediate
3. one-sided startup fills route to startup completion, not normal shape repair
4. missing-side restoration is not vetoed by normal shape logic
5. normal flow replenishes continuously through most of the market
6. late activity tapers sharply after `240s`
7. normal flow remains maker-first and `BUY`-only
8. clone metrics are emitted and coherent
9. canary behavior can be compared directly against the review fingerprint
10. CPP / cost-quality logic never collapses aggressive high-frequency participation
11. wallet-clone `PairBuild` no longer churns viable resting maker orders on a generic stale horizon
12. wallet-clone normal flow no longer leaks sub-minimum maker orders to the exchange
13. optional adds respect the cheap-pair rule, below-snapshot rule, and non-negative-edge rule or an explicit documented fallback
14. heavy-side growth no longer strands a repair tail because repair reserve is enforced
15. final paired inventory quality is at or near break-even and the remaining tail does not wipe out the paired edge on reviewed canaries

## Assumptions
1. Existing maker order lifecycle, book freshness, and fill-accounting infrastructure can be reused.
2. Existing rollover behavior is good enough to preserve.
3. Sprint 3 code can be partially reused, but Sprint 4 must not inherit Sprint 3 objectives implicitly.
4. Clone mode should prioritize observable mechanics over inferred hidden quote-management details.
5. The implementation should stay helper-first inside `src/bot.rs`, with env wiring in `src/env_contract.rs` and operator docs in `ENVIRONMENT.md`.
