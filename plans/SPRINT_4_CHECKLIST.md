# Sprint 4 Checklist: Wallet-Clone Pair-First Accumulator

Use this file as the concrete execution tracker for Sprint 4.
The design target is [SPRINT_4.md](C:/Works/aldipranata.com/polybot-sprint-4/plans/SPRINT_4.md).

## Current Status
- Overall status: `IN PROGRESS`
- Recommended boundary: `NEW MODE`
- Current blocker: `FIRST_SPRINT_4_CANARY_NOT_RUN`

## Phase 0: Boundary And Objective Isolation
- [x] Add Sprint 4 runtime mode dispatch
- [x] Keep `MAKER_SKEW_ARB` unchanged
- [x] Keep `SETTLEMENT_SHAPER` unchanged
- [x] Add Sprint 4 startup config log
- [x] Add Sprint 4 phase/owner log prefixes
- [x] Confirm Sprint 4 path does not call Sprint 3 target-shape planner in normal flow

## Phase 1: Pre-Arm
- [x] Add pre-arm runtime state
- [x] Require selected market before open
- [x] Add asset-id readiness before open
- [x] Add pre-arm feed freshness checks
- [x] Add explicit pre-arm hold reasons
- [x] Log when pre-arm becomes ready
- [x] Prove market-open does not begin with first-time discovery

## Phase 2: Open Both
- [x] Add `OpenBoth` owner/state
- [x] Submit both maker `BUY` seed orders immediately at market start
- [x] Use small neutral seed clip
- [x] Remove favorite/underdog gating from opening
- [x] Add `OPEN_BOTH` submit logs
- [x] Add opening metrics:
  - first seed submit time
  - first fill time
  - paired-open attempt count

## Phase 3: Seed Completion
- [x] Add `SeedCompletion` owner/state
- [x] Route one-sided startup inventory into `SeedCompletion`
- [x] Keep `SeedCompletion` separate from Sprint 3 `ShapeRepair`
- [x] Allow missing-side repair to bypass normal hard-skew veto
- [x] Allow missing-side repair to bypass normal shape-target veto
- [x] Allow missing-side repair to bypass hard CPP profitability veto
- [x] Require only missing-side quote health during `SeedCompletion`
- [x] Allow `SeedCompletion` to borrow later phase budget
- [x] Add logs:
  - missing side
  - startup completion reason
  - time since first side
  - completion success / failure
- [x] Add metrics:
  - both sides by `30s`
  - both sides by `60s`
  - time from first fill to second-side fill

## Phase 4: Pair Build
- [x] Add `PairBuild` owner/state
- [x] Repost two-sided maker quotes after fills
- [x] Keep paired growth active through the main window
- [x] Add lighter-side-first logic when imbalance stretches
- [x] Keep normal flow `BUY`-only
- [x] Prevent one-shot seed then idle behavior
- [x] Make CPP a light sizing hint, not a hard normal-flow stop
- [x] Prefer smaller or lighter-side clips before going idle on mediocre CPP
- [x] Add paired-growth submit / rest / suppress logs

## Phase 5: Taper
- [x] Add late `Taper` owner/state or equivalent late-phase controller
- [x] Reduce new expansion after `240s`
- [x] Suppress almost all new activity in final `30s`
- [x] Keep only tiny maintenance / repair late
- [x] Preserve rollover stop-buffer behavior
- [x] Add taper metrics:
  - fills after `240s`
  - fills after `270s`
  - new-order count after `240s`

## Phase 6: Clone Metrics
- [x] Add market participation metric
- [x] Add maker fill share metric
- [x] Add fills-per-market metric
- [x] Add fill distribution by:
  - `0-30s`
  - `30-60s`
  - `60-180s`
  - `180-240s`
  - `240-300s`
- [x] Add paired-size metric
- [x] Add unmatched-size metric
- [x] Add final-minute activity metric
- [x] Add average realized combined paid price metric
- [x] Add skipped-optional-add count
- [x] Add startup-completion blocked count
- [x] Emit dedicated final `[WALLET_CLONE][METRICS]` summary

## Phase 7: Config And Docs
- [x] Add Sprint 4 env keys to `src/env_contract.rs`
- [x] Document Sprint 4 env keys in `ENVIRONMENT.md`
- [ ] Document Sprint 4 behavior note once first canary is run
- [x] Update `TARGET_GOAL_STATUS.md` only after Sprint 4 is truly runnable

## Tests
- [x] Unit test Sprint 4 mode routing
- [x] Unit test Sprint 4 phase timing boundaries
- [x] Unit test `PreArm -> OpenBoth`
- [x] Unit test owner routing from one-sided `OpenBoth` into `SeedCompletion`
- [x] Unit test owner routing back to `PairBuild` when both sides are live
- [x] Unit test `PairBuild` decision routing for paired growth, lighter-side recovery, and CPP throttling
- [ ] Unit test startup missing-side bypass over normal shape gating
- [ ] Unit test opening without favorite/underdog dependency
- [x] Unit test taper suppression after `240s`
- [x] Unit test final-quiet suppression after `270s`
- [x] Unit test clone metrics helpers

## Validation Run Requirements
- [x] Run `cargo check -q`
- [x] Run targeted Sprint 4 tests
- [x] Run `cargo test -q` for behavior-changing implementation
- [ ] Run first Sprint 4 canary
- [ ] Review first canary for:
  - participation
  - seed timing
  - second-side timing
  - maker share
  - fill cadence
  - late taper
  - final-minute quieting
  - whether CPP stayed informational and did not suppress aggressive inventory building

## Done Criteria
- [x] Sprint 4 has a separate runnable mode
- [x] opening is pre-armed and immediate
- [x] one-sided startup fills are treated as normal startup completion
- [x] missing-side startup restoration is no longer blocked by normal shape rules
- [x] normal flow replenishes continuously
- [x] normal flow stays maker-first and `BUY`-only
- [x] late taper behavior is visible and measurable
- [x] Sprint 4 metrics are emitted and coherent
- [ ] first canary is reviewable against the wallet-clone fingerprint
- [ ] CPP logic does not collapse aggressive high-frequency participation
