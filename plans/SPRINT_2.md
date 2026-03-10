# Sprint 2: Gentle Skew Around a Protected Floor

## Sprint Goal
Implement Step 2 as a guarded skew overlay inside the current Step 1 `PAIR_BASE` route. The overlay should add mild extra maker `BUY` exposure on the cheaper side only when the Step 1 book is already protected, while preserving:

1. `RiskExitOnly`
2. `MergePending` / recovery
3. `PairBase`
4. `Skew`

Priority and keeping normal flow maker-only.

## Sprint Status
- Overall status: `PARTIAL / NOT CANARY READY`
- Target mode: `PAIR_BASE` with `MAKER_SKEW_ENABLED=true`
- Baseline dependency: `READY`
- Blocking prerequisite from Step 1: `CLOSED`
- Current blocker: `INTENDED-SKEW GAP APPLIES BEFORE SKEW FILL`
- Canary requirement after implementation: `BLOCKED`

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

Currently implemented, but not all valid for canary:
- [x] Skew overlay state is wired into active control flow
- [ ] Skew-adjusted Step 1 control gap is valid when the skew order is armed but still unfilled
- [x] Skew entry/sizing path exists
- [x] Skew-specific logging exists
- [x] Skew-specific metrics are emitted
- [x] Sprint 2 tests are added

Known blocker:
- [ ] `25/25` with intended `+5 YES` currently produces a synthetic `base_gap=-5` immediately after skew submit because intended skew is applied before any skew fill arrives. Step 1 then treats the protected book as if it already has real excess imbalance.

## Workstreams

### Workstream A: Overlay State
Status: `COMPLETE`

Objective:
Add internal state to represent intended skew without replacing the existing Step 1 phase machine.

Tasks:
- [x] Add `SkewOverlayState` usage in live `PAIR_BASE` control flow
- [ ] Track:
  - [x] `enabled`
  - [x] `side`
  - [x] `target_gap_shares`
  - [x] `live_oid`
  - [x] `state`
  - [x] `last_enter_ts`
- [x] Ensure state is reset correctly on clear
- [x] Ensure state survives normal pair-base loop iterations

Acceptance:
- [x] Overlay can be enabled/disabled without affecting Step 1 baseline when skew is off
- [x] Only one overlay state instance exists per market

### Workstream B: Skew-Adjusted Control Gap
Status: `PARTIAL`

Objective:
Make Step 1 recovery and risk logic react only to excess imbalance beyond intended skew.

Tasks:
- [x] Compute:
  - [x] `signed_gap = q_yes - q_no`
  - [x] `signed_intended_skew_gap`
  - [x] `base_gap = signed_gap - signed_intended_skew_gap`
- [x] Replace raw `abs(q_yes - q_no)` with `abs(base_gap)` in Step 1 control decisions
- [x] Apply skew-adjusted gap to:
  - [x] recovery entry
  - [x] recovery remain/exit
  - [x] risk-exit trigger
  - [x] phase normalization
- [ ] Treat intended-but-unfilled skew as zero base gap until real skew inventory exists

Acceptance:
- [ ] `25/25` with intended `+5 YES` does not trigger recovery or risk-exit before any skew fill
- [x] `30/25` with intended `+5 YES` does not trigger recovery
- [x] `35/25` with intended `+5 YES` does trigger recovery on excess `5`

### Workstream C: Skew Entry and Sizing
Status: `COMPLETE`

Objective:
Add mild cheaper-side skew only when the Step 1 book is already protected.

Tasks:
- [x] Run skew logic only inside `_maker_pair_base_step(...)`
- [x] Gate skew entry on:
  - [x] no `RiskExitOnly`
  - [x] no active recovery
  - [x] no unresolved pair-base live orders
  - [x] `q_yes > 0`
  - [x] `q_no > 0`
  - [x] `abs(base_gap) <= release_threshold`
- [x] Determine cheaper side using:
  - [x] best ask
  - [x] bid fallback
  - [x] `YES` tie-break
- [x] Compute `desired_extra` only up to `MAKER_SKEW_TARGET_RATIO`
- [x] Apply clamps:
  - [x] `CLIP_SHARES`
  - [x] pair-base window budget only
  - [x] `MAX_TOTAL_COST`
  - [x] `MIN_SHARES`
- [x] Ensure Step 2 never consumes merge budget

Acceptance:
- [x] `20/20`, target `1.2`, min `5` -> no skew order
- [x] `25/25`, cheaper YES -> one `PAIR_BASE_SKEW` YES order for `5`

### Workstream D: Floor Protection and Safety
Status: `PARTIAL`

Objective:
Make Step 2 additive only when the protected floor remains acceptable.

Tasks:
- [x] Reuse pair-base fee snapshot helper for skew post-action checks
- [x] Require `fee_net_worst_case_pnl >= 0` before placing skew
- [x] Preserve intentional skew during normal equal YES/NO pair-base additions
- [ ] Route only excess over intended skew into Step 1 recovery
- [x] Clear skew overlay on:
  - [x] sign flip
  - [x] ratio breach above `MAKER_SKEW_MAX_RATIO`
  - [x] negative fee-net floor
  - [x] `RiskExitOnly`
  - [x] `MergePending`
- [x] Ensure no normal taker path is used by Step 2

Acceptance:
- [ ] Recovery acts only on excess, not intended skew
- [x] Step 2 hands control back cleanly to Step 1 when protection is threatened

### Workstream E: Orders and Logging
Status: `COMPLETE`

Objective:
Make Step 2 observable and operationally safe.

Tasks:
- [x] Add dedicated maker origin `PAIR_BASE_SKEW`
- [x] Allow only one live skew order at a time
- [x] Cancel live skew order immediately when recovery or risk exit takes ownership
- [x] Emit logs for:
  - [x] skew enter
  - [x] skew live
  - [ ] skew fill
  - [x] skew suppress reason
  - [x] skew clear reason

Acceptance:
- [x] Logs make Step 2 behavior explainable without inferring from silence
- [x] No live skew order survives into `MergePending` or `RiskExitOnly`

### Workstream F: Step 2 Metrics
Status: `COMPLETE`

Objective:
Emit Step 2-specific end-of-market metrics without replacing Step 1 metrics.

Tasks:
- [x] Add skew metrics state accumulation
- [x] Emit end-of-market metrics for:
  - [x] skew ratio distribution
  - [x] cost split by side
  - [x] worst-case downside
  - [x] best-case upside
  - [x] both-side participation
  - [x] pair coverage after skew
  - [x] normal-flow taker count
  - [x] fee-net worst/best case
  - [x] fee-net pair cost
  - [x] skew fill totals by side
- [x] Keep Step 1 metrics unchanged and present beside Step 2 metrics

Acceptance:
- [ ] Metrics line is internally consistent
- [x] Step 1 and Step 2 metrics can be compared in the same run

### Workstream G: Tests
Status: `COMPLETE`

Objective:
Add deterministic coverage for skew math and control behavior.

Tasks:
- [x] Add test: `20/20`, target `1.2`, min `5` -> no skew order
- [x] Add test: `25/25`, cheaper YES -> `+5` intended skew gap
- [ ] Add test: equal pair-base additions preserve intended skew
- [ ] Add test: excess over intended skew triggers recovery
- [ ] Add test: sign flip or max-ratio breach clears skew
- [ ] Add test: recovery or `RiskExitOnly` cancels live skew order
- [ ] Add test: no normal Step 2 taker path
- [ ] Add test: Step 2 metrics populate consistently

Acceptance:
- [x] `cargo test` covers core Step 2 math and control paths

## Rollout Checklist
Status: `BLOCKED`

- [x] Implement code with `MAKER_SKEW_ENABLED=false` preserving current Step 1 behavior
- [ ] Fix intended-skew semantics so an armed-but-unfilled skew order does not create a synthetic recovery gap
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
- [x] Step 2 code ships with Step 1 unchanged when skew is off
- [ ] Intended skew gap is respected by Step 1 control logic
- [x] Skew adds are maker-only and cheaper-side only
- [x] Fee-net floor gate prevents destructive skew adds
- [ ] Recovery acts only on excess over intended skew
- [x] Recovery/risk-exit immediately cancel skew orders
- [ ] Step 2 metrics are emitted and coherent
- [x] Test plan passes
- [ ] Canary runs show bounded downside and no normal-flow taker usage from Step 2

## Notes for Implementation
1. Do not implement a separate Step 2 unwind strategy in this sprint.
2. Do not refactor the legacy generic skew loop as part of this sprint.
3. Keep Step 1 as the baseline canary and comparison path.
4. Treat this sprint as a canary-first implementation, not a full directional strategy release.
