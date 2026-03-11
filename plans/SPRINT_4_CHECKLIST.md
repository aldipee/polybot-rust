# Sprint 4 Checklist: Wallet-Clone Pair-First Accumulator

Use this file as the concrete execution tracker for Sprint 4.
The design target is [SPRINT_4.md](c:/Works/aldipranata.com/polybot-convert-rust/plans/SPRINT_4.md).

## Current Status
- Overall status: `NOT STARTED`
- Recommended boundary: `NEW MODE`
- Current blocker: `SPRINT_4 NOT IMPLEMENTED`

## Phase 0: Boundary And Objective Isolation
- [ ] Add Sprint 4 runtime mode dispatch
- [ ] Keep `MAKER_SKEW_ARB` unchanged
- [ ] Keep `SETTLEMENT_SHAPER` unchanged
- [ ] Add Sprint 4 startup config log
- [ ] Add Sprint 4 phase/owner log prefixes
- [ ] Confirm Sprint 4 path does not call Sprint 3 target-shape planner in normal flow

## Phase 1: Pre-Arm
- [ ] Add pre-arm runtime state
- [ ] Add next-market discovery before open
- [ ] Add asset-id readiness before open
- [ ] Add pre-arm feed freshness checks
- [ ] Add explicit pre-arm hold reasons
- [ ] Log when pre-arm becomes ready
- [ ] Prove market-open does not begin with first-time discovery

## Phase 2: Open Both
- [ ] Add `OpenBoth` owner/state
- [ ] Submit both maker `BUY` seed orders immediately at market start
- [ ] Use small neutral seed clip
- [ ] Remove favorite/underdog gating from opening
- [ ] Add `OPEN_BOTH` submit logs
- [ ] Add opening metrics:
  - first seed submit time
  - first fill time
  - paired-open attempt count

## Phase 3: Seed Completion
- [ ] Add `SeedCompletion` owner/state
- [ ] Route one-sided startup inventory into `SeedCompletion`
- [ ] Keep `SeedCompletion` separate from Sprint 3 `ShapeRepair`
- [ ] Allow missing-side repair to bypass normal hard-skew veto
- [ ] Allow missing-side repair to bypass normal shape-target veto
- [ ] Allow missing-side repair to bypass hard CPP profitability veto
- [ ] Allow `SeedCompletion` to borrow later phase budget
- [ ] Add logs:
  - missing side
  - startup completion reason
  - time since first side
  - completion success / failure
- [ ] Add metrics:
  - both sides by `30s`
  - both sides by `60s`
  - time from first fill to second-side fill

## Phase 4: Pair Build
- [ ] Add `PairBuild` owner/state
- [ ] Repost two-sided maker quotes after fills
- [ ] Keep paired growth active through the main window
- [ ] Add lighter-side-first logic when imbalance stretches
- [ ] Keep normal flow `BUY`-only
- [ ] Prevent one-shot seed then idle behavior
- [ ] Make CPP a light sizing hint, not a hard normal-flow stop
- [ ] Prefer smaller or lighter-side clips before going idle on mediocre CPP
- [ ] Add paired-growth submit / rest / suppress logs

## Phase 5: Taper
- [ ] Add late `Taper` owner/state or equivalent late-phase controller
- [ ] Reduce new expansion after `240s`
- [ ] Suppress almost all new activity in final `30s`
- [ ] Keep only tiny maintenance / repair late
- [ ] Preserve rollover stop-buffer behavior
- [ ] Add taper metrics:
  - fills after `240s`
  - fills after `270s`
  - new-order count after `240s`

## Phase 6: Clone Metrics
- [ ] Add market participation metric
- [ ] Add maker fill share metric
- [ ] Add fills-per-market metric
- [ ] Add fill distribution by:
  - `0-30s`
  - `30-60s`
  - `60-180s`
  - `180-240s`
  - `240-300s`
- [ ] Add paired-size metric
- [ ] Add unmatched-size metric
- [ ] Add final-minute activity metric
- [ ] Add average realized combined paid price metric
- [ ] Add skipped-optional-add count
- [ ] Add startup-completion blocked count
- [ ] Emit dedicated final `[WALLET_CLONE][METRICS]` summary

## Phase 7: Config And Docs
- [ ] Add Sprint 4 env keys to `src/env_contract.rs`
- [ ] Document Sprint 4 env keys in `ENVIRONMENT.md`
- [ ] Document Sprint 4 behavior note once first canary is run
- [ ] Update `TARGET_GOAL_STATUS.md` only after Sprint 4 is truly runnable

## Tests
- [ ] Unit test Sprint 4 mode routing
- [ ] Unit test Sprint 4 phase timing boundaries
- [ ] Unit test `PreArm -> OpenBoth`
- [ ] Unit test `OpenBoth -> SeedCompletion`
- [ ] Unit test `SeedCompletion -> PairBuild`
- [ ] Unit test startup missing-side bypass over normal shape gating
- [ ] Unit test opening without favorite/underdog dependency
- [ ] Unit test taper suppression after `240s`
- [ ] Unit test final-quiet suppression after `270s`
- [ ] Unit test clone metrics helpers

## Validation Run Requirements
- [ ] Run `cargo check -q`
- [ ] Run targeted Sprint 4 tests
- [ ] Run `cargo test -q` for behavior-changing implementation
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
- [ ] Sprint 4 has a separate runnable mode
- [ ] opening is pre-armed and immediate
- [ ] one-sided startup fills are treated as normal startup completion
- [ ] missing-side startup restoration is no longer blocked by normal shape rules
- [ ] normal flow replenishes continuously
- [ ] normal flow stays maker-first and `BUY`-only
- [ ] late taper behavior is visible and measurable
- [ ] Sprint 4 metrics are emitted and coherent
- [ ] first canary is reviewable against the wallet-clone fingerprint
- [ ] CPP logic does not collapse aggressive high-frequency participation
