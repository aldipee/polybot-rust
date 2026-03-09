# Sprint 2: Gentle Skew Around a Protected Floor

## Sprint Goal
Implement Step 2 as a guarded skew overlay inside the current Step 1 `PAIR_BASE` route. The overlay should add mild extra maker `BUY` exposure on the cheaper side only when the Step 1 book is already protected, while preserving:

1. `RiskExitOnly`
2. `MergePending` / recovery
3. `PairBase`
4. `Skew`

Priority and keeping normal flow maker-only.

## Sprint Status
- Overall status: `NOT STARTED`
- Target mode: `PAIR_BASE` with `MAKER_SKEW_ENABLED=true`
- Baseline dependency: `READY`
- Blocking prerequisite from Step 1: `CLOSED`
- Canary requirement after implementation: `REQUIRED`

## Scope
In scope:
1. Add skew overlay state and control logic in `src/bot.rs`
2. Make Step 1 control gap skew-aware
3. Add guarded skew entry/sizing
4. Add skew safety clears on recovery/risk exit
5. Add skew-specific logging and end-of-market metrics
6. Add tests for skew math and state transitions

Out of scope:
1. New public API surface
2. Dedicated Step 2 unwind strategy
3. Aggressive directional skew beyond target ratio
4. Taker usage in normal Step 2 flow

## Baseline Assumptions
1. Step 1 remains the control owner of recovery and risk exit.
2. Step 2 remains invalid unless evaluated fee-net, not gross LP.
3. Cheaper-side selection uses best ask first, then best bid fallback.
4. Existing config is reused:
   - `MAKER_SKEW_ENABLED`
   - `MAKER_SKEW_TARGET_RATIO`
   - `MAKER_SKEW_MAX_RATIO`
5. Intended plan source was `STEP2_GENTLE_SKEW_PLAN.md`; this file is the executable sprint tracker.

## Current Readiness
Completed before Sprint 2:
- [x] Step 1 pair-base engine exists
- [x] Step 1 recovery ownership is stable enough to build on
- [x] `forced_negative_economics` early exit works
- [x] stale late-fill reopen bug is fixed
- [x] current Step 1 can run as baseline canary

Not started for Sprint 2:
- [ ] Skew overlay state is wired into active control flow
- [ ] Skew-adjusted Step 1 control gap is active
- [ ] Skew entry/sizing path exists
- [ ] Skew-specific logging exists
- [ ] Skew-specific metrics are emitted
- [ ] Sprint 2 tests are added

## Workstreams

### Workstream A: Overlay State
Status: `NOT STARTED`

Objective:
Add internal state to represent intended skew without replacing the existing Step 1 phase machine.

Tasks:
- [ ] Add `SkewOverlayState` usage in live `PAIR_BASE` control flow
- [ ] Track:
  - [ ] `enabled`
  - [ ] `side`
  - [ ] `target_gap_shares`
  - [ ] `live_oid`
  - [ ] `state`
  - [ ] `last_enter_ts`
- [ ] Ensure state is reset correctly on clear
- [ ] Ensure state survives normal pair-base loop iterations

Acceptance:
- [ ] Overlay can be enabled/disabled without affecting Step 1 baseline when skew is off
- [ ] Only one overlay state instance exists per market

### Workstream B: Skew-Adjusted Control Gap
Status: `NOT STARTED`

Objective:
Make Step 1 recovery and risk logic react only to excess imbalance beyond intended skew.

Tasks:
- [ ] Compute:
  - [ ] `signed_gap = q_yes - q_no`
  - [ ] `signed_intended_skew_gap`
  - [ ] `base_gap = signed_gap - signed_intended_skew_gap`
- [ ] Replace raw `abs(q_yes - q_no)` with `abs(base_gap)` in Step 1 control decisions
- [ ] Apply skew-adjusted gap to:
  - [ ] recovery entry
  - [ ] recovery remain/exit
  - [ ] risk-exit trigger
  - [ ] phase normalization

Acceptance:
- [ ] `30/25` with intended `+5 YES` does not trigger recovery
- [ ] `35/25` with intended `+5 YES` does trigger recovery on excess `5`

### Workstream C: Skew Entry and Sizing
Status: `NOT STARTED`

Objective:
Add mild cheaper-side skew only when the Step 1 book is already protected.

Tasks:
- [ ] Run skew logic only inside `_maker_pair_base_step(...)`
- [ ] Gate skew entry on:
  - [ ] no `RiskExitOnly`
  - [ ] no active recovery
  - [ ] no unresolved pair-base live orders
  - [ ] `q_yes > 0`
  - [ ] `q_no > 0`
  - [ ] `abs(base_gap) <= release_threshold`
- [ ] Determine cheaper side using:
  - [ ] best ask
  - [ ] bid fallback
  - [ ] `YES` tie-break
- [ ] Compute `desired_extra` only up to `MAKER_SKEW_TARGET_RATIO`
- [ ] Apply clamps:
  - [ ] `CLIP_SHARES`
  - [ ] pair-base window budget only
  - [ ] `MAX_TOTAL_COST`
  - [ ] `MIN_SHARES`
- [ ] Ensure Step 2 never consumes merge budget

Acceptance:
- [ ] `20/20`, target `1.2`, min `5` -> no skew order
- [ ] `25/25`, cheaper YES -> one `PAIR_BASE_SKEW` YES order for `5`

### Workstream D: Floor Protection and Safety
Status: `NOT STARTED`

Objective:
Make Step 2 additive only when the protected floor remains acceptable.

Tasks:
- [ ] Reuse pair-base fee snapshot helper for skew post-action checks
- [ ] Require `fee_net_worst_case_pnl >= 0` before placing skew
- [ ] Preserve intentional skew during normal equal YES/NO pair-base additions
- [ ] Route only excess over intended skew into Step 1 recovery
- [ ] Clear skew overlay on:
  - [ ] sign flip
  - [ ] ratio breach above `MAKER_SKEW_MAX_RATIO`
  - [ ] negative fee-net floor
  - [ ] `RiskExitOnly`
  - [ ] `MergePending`
- [ ] Ensure no normal taker path is used by Step 2

Acceptance:
- [ ] Recovery acts only on excess, not intended skew
- [ ] Step 2 hands control back cleanly to Step 1 when protection is threatened

### Workstream E: Orders and Logging
Status: `NOT STARTED`

Objective:
Make Step 2 observable and operationally safe.

Tasks:
- [ ] Add dedicated maker origin `PAIR_BASE_SKEW`
- [ ] Allow only one live skew order at a time
- [ ] Cancel live skew order immediately when recovery or risk exit takes ownership
- [ ] Emit logs for:
  - [ ] skew enter
  - [ ] skew live
  - [ ] skew fill
  - [ ] skew suppress reason
  - [ ] skew clear reason

Acceptance:
- [ ] Logs make Step 2 behavior explainable without inferring from silence
- [ ] No live skew order survives into `MergePending` or `RiskExitOnly`

### Workstream F: Step 2 Metrics
Status: `NOT STARTED`

Objective:
Emit Step 2-specific end-of-market metrics without replacing Step 1 metrics.

Tasks:
- [ ] Add skew metrics state accumulation
- [ ] Emit end-of-market metrics for:
  - [ ] skew ratio distribution
  - [ ] cost split by side
  - [ ] worst-case downside
  - [ ] best-case upside
  - [ ] both-side participation
  - [ ] pair coverage after skew
  - [ ] normal-flow taker count
  - [ ] fee-net worst/best case
  - [ ] fee-net pair cost
  - [ ] skew fill totals by side
- [ ] Keep Step 1 metrics unchanged and present beside Step 2 metrics

Acceptance:
- [ ] Metrics line is internally consistent
- [ ] Step 1 and Step 2 metrics can be compared in the same run

### Workstream G: Tests
Status: `NOT STARTED`

Objective:
Add deterministic coverage for skew math and control behavior.

Tasks:
- [ ] Add test: `20/20`, target `1.2`, min `5` -> no skew order
- [ ] Add test: `25/25`, cheaper YES -> `+5` intended skew gap
- [ ] Add test: equal pair-base additions preserve intended skew
- [ ] Add test: excess over intended skew triggers recovery
- [ ] Add test: sign flip or max-ratio breach clears skew
- [ ] Add test: recovery or `RiskExitOnly` cancels live skew order
- [ ] Add test: no normal Step 2 taker path
- [ ] Add test: Step 2 metrics populate consistently

Acceptance:
- [ ] `cargo test` covers core Step 2 math and control paths

## Rollout Checklist
Status: `NOT STARTED`

- [ ] Implement code with `MAKER_SKEW_ENABLED=false` preserving current Step 1 behavior
- [ ] Use canary config:
  - [ ] `MAKER_SKEW_ENABLED=true`
  - [ ] `MAKER_SKEW_TARGET_RATIO=1.2`
  - [ ] `MAKER_SKEW_MAX_RATIO=2.2`
- [ ] Run Step 1 baseline comparison against Step 2 canary
- [ ] Review:
  - [ ] skew ratio distribution
  - [ ] downside floor
  - [ ] fee-net pair cost
  - [ ] normal-flow taker count
  - [ ] pair coverage after skew

## Done Criteria
Sprint 2 is done only when all are true:
- [ ] Step 2 code ships with Step 1 unchanged when skew is off
- [ ] Intended skew gap is respected by Step 1 control logic
- [ ] Skew adds are maker-only and cheaper-side only
- [ ] Fee-net floor gate prevents destructive skew adds
- [ ] Recovery acts only on excess over intended skew
- [ ] Recovery/risk-exit immediately cancel skew orders
- [ ] Step 2 metrics are emitted and coherent
- [ ] Test plan passes
- [ ] Canary runs show bounded downside and no normal-flow taker usage from Step 2

## Notes for Implementation
1. Do not implement a separate Step 2 unwind strategy in this sprint.
2. Do not refactor the legacy generic skew loop as part of this sprint.
3. Keep Step 1 as the baseline canary and comparison path.
4. Treat this sprint as a canary-first implementation, not a full directional strategy release.
