# Sprint 4 Checklist: Wallet-Clone Pair-First Accumulator

Use this file as the concrete execution tracker for Sprint 4.
The design target is [SPRINT_4.md](C:/Works/aldipranata.com/polybot-sprint-4/plans/SPRINT_4.md).

## Current Status
- Overall status: `IN PROGRESS`
- Recommended boundary: `NEW MODE`
- Current blocker: `CONSULTATION_RULE_INTEGRATION_AND_SECOND_CANARY`

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

## Phase 4A: Post-Canary PairBuild Hardening
- [x] Add wallet-clone-specific stale timeout policy for lighter-side live orders
- [x] Add wallet-clone-specific stale timeout policy for paired-growth live orders
- [x] Add wallet-clone-specific stale timeout policy for asymmetric submit-resolution cleanup
- [x] Stop using the shared generic `STALE_SECONDS` horizon as the only stale-cancel rule for wallet-clone `PairBuild`
- [x] Re-check minimum maker notional after final exchange quantization in the wallet-clone submit path
- [x] Block exchange-invalid sub-minimum maker orders before venue submission
- [x] Replace time-only stale cancel behavior with quality-aware persistence for `PairBuild`
- [x] Preserve good opposite-side live orders during asymmetric refresh instead of canceling for symmetry alone
- [x] Add per-side repost hysteresis and dedup
- [x] Add projected post-add paired-cost guard for optional `PairedGrowth` only
- [x] Keep startup completion exempt from optional paired-growth cost suppression
- [x] Keep required lighter-side recovery exempt from optional paired-growth cost suppression
- [x] Make lighter-side recovery use smaller clips when cost quality weakens
- [x] Make lighter-side-first dominate while the book remains materially skewed
- [x] Fast-cancel broken paired-growth asymmetry when one live leg is orphaned by a recent counterpart submit reject
- [x] Add projected repaired-book pay-up discipline so lighter-side recovery clips down or holds before rebuilding an expensive paired book
- [ ] If needed, split `PairBuild` internally into `PairedGrowth` and `LighterRepair`
- [ ] Add logs for:
  - wallet-clone-specific stale timeout decisions
  - persistence / cancel-validity decisions
  - asymmetric refresh preservation decisions
  - repost hysteresis suppressions
  - projected paired-cost suppression
  - blocked sub-minimum maker orders

## Phase 4B: Consultation Rule Integration
Execution note:
- [ ] Execute this phase top-to-bottom. Do not start later items until the earlier helper / decision primitives exist in code and tests.

### 4B.1 Decision Primitives
- [ ] Add shared Sprint 4 helpers in `src/bot.rs` for:
  - `paired_size`
  - `tail_size`
  - `share_skew_ratio`
  - `worst_case_settlement_floor`
  - `projected_paired_cost`
- [x] Add one helper that classifies the projected paired-cost band:
  - `< 0.94`
  - `0.94 - 0.98`
  - `0.98 - 1.00`
  - `1.00 - 1.02`
  - `> 1.02`
- [x] Add one helper that computes the minimum required repair reserve for the likely lighter-side repair
- [x] Add unit tests for all new helpers before wiring them into `PairBuild`

### 4B.2 Optional Add Price / Edge Gates
- [x] Add below-snapshot gating for optional buys only
- [x] Make the below-snapshot rule explicit in code and logs:
  - optional `PairedGrowth` must be strictly better than same-side snapshot
  - `OpenBoth`, `SeedCompletion`, and required lighter-side repair remain exempt unless separately restricted
- [ ] Add non-negative `edge_model_minus_price` gating for optional buys when the live signal exists
- [x] Add the first size-up band at about `edge_model_minus_price >= 0.05`
- [x] If the live model signal does not exist in Sprint 4 runtime, document and implement the exact fallback rule instead of leaving the gate implicit
- [ ] Add explicit hold / suppress log reasons for:
  - `optional_buy_not_below_snapshot`
  - `optional_buy_negative_edge`
  - `optional_buy_weak_edge_reduced_size`

### 4B.3 Paired-Cost Regime Enforcement
- [x] Route optional `PairedGrowth` through the projected paired-cost band helper
- [x] Enforce these normal-flow rules:
  - `< 0.94` strong paired growth
  - `0.94 - 0.98` normal paired growth
  - `0.98 - 1.00` maintenance / reduced growth
  - `1.00 - 1.02` repair-only
  - `> 1.02` default freeze / skip
- [x] Stop optional growth above `projected_paired_cost > 1.00`
- [x] Default-freeze or skip above `projected_paired_cost > 1.02`, unless an explicit approved repair exception is added later
- [x] Keep `OpenBoth` and `SeedCompletion` outside these optional-growth gates
- [ ] Add explicit hold / suppress log reasons for each paired-cost band transition

### 4B.4 Repair Reserve And Tail Protection
- [x] Add repair-budget reserve before heavy-side growth
- [x] Block heavy-side optional growth whenever remaining budget cannot fund the likely lighter-side repair plus the configured reserve buffer
- [ ] Make `PairBuild` score `worst_case_settlement_floor` and `tail_size` ahead of exact equality cosmetics
- [x] Add explicit hold reason when growth is blocked by repair reserve instead of by price quality
- [x] Add metrics for growth blocked by repair reserve
- [x] Add metrics for growth blocked by projected floor / tail deterioration

### 4B.5 Exact-Gap Repair Behavior
- [x] Replace fixed lighter-side repair clips with exact-gap or smallest-valid-repair sizing
- [x] Round the exact-gap repair up only as much as needed to satisfy exchange minimum notional / precision rules
- [x] Prevent repeated invalid tiny repair attempts once the minimum valid repair size is known
- [x] Clarify and implement when opposite-side live orders may be preserved during lighter-side repair versus canceled for clean repair ownership:
  - preserve only if remaining size and price are compatible with the repair target
  - cancel / handoff otherwise
- [x] Add explicit logs for:
  - exact-gap repair sizing
  - rounded-up minimum-valid repair sizing
  - preserve-vs-cancel ownership decisions during lighter-side repair

### 4B.6 Late-Window Floor / Tail Policy
- [x] Tighten the time-band policy to:
  - `0-210s` normal growth
  - `210-240s` reduced growth
  - `240-270s` repair-first
  - `270-300s` no optional adds
- [x] After `240s`, make floor and tail improvement beat average-cost cosmetics
- [x] After `270s`, allow only the smallest repair that improves floor or reduces tail
- [x] Add explicit late hold reasons that distinguish:
  - repair-first suppression
  - no-optional-adds suppression
  - floor / tail priority decisions

### 4B.7 Metrics And Canary Review Surface
- [x] Add canary metrics for:
  - below-snapshot optional fill rate
  - paired-cost band occupancy
  - tail at expiry
  - worst-case settlement floor at expiry
- [x] Surface the new metrics in the final `[WALLET_CLONE][METRICS]` summary
- [ ] Make the second canary review explicitly compare:
  - paired core cost quality
  - final tail size
  - worst-case settlement floor
  - whether the remaining tail still wipes out the paired edge

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
- [x] Document Sprint 4 behavior note once first canary is run
- [x] Update `OBJECTIVES_STATUS.md` only after Sprint 4 is truly runnable
- [x] Create Sprint 4 high-level behavior note after first canary review

## Tests
- [x] Unit test Sprint 4 mode routing
- [x] Unit test Sprint 4 phase timing boundaries
- [x] Unit test `PreArm -> OpenBoth`
- [x] Unit test owner routing from one-sided `OpenBoth` into `SeedCompletion`
- [x] Unit test owner routing back to `PairBuild` when both sides are live
- [x] Unit test `PairBuild` decision routing for paired growth, lighter-side recovery, and CPP throttling
- [x] Unit test wallet-clone-specific stale timeout policy
- [x] Unit test post-quantization minimum-maker-notional guard
- [x] Unit test quality-aware persistence for `PairBuild`
- [x] Unit test asymmetric refresh preserves a good opposite-side live order
- [x] Unit test per-side repost hysteresis / dedup
- [x] Unit test projected paired-cost suppression for optional `PairedGrowth`
- [x] Unit test lighter-side recovery clips down when cost quality weakens
- [x] Unit test lighter-side-first ownership dominance while skew remains materially stretched
- [x] Unit test broken paired-growth asymmetry cancels on the short wallet-clone horizon
- [x] Unit test projected repaired-book cap blocks extreme lighter-side pay-up
- [x] Unit test below-snapshot gating for optional buys
- [ ] Unit test optional-growth block above `projected_paired_cost > 1.00`
- [ ] Unit test repair-only behavior in the `1.00 - 1.02` band
- [x] Unit test repair-budget reserve blocks heavy-side growth when likely repair cannot be funded
- [ ] Unit test exact-gap / smallest-valid-repair sizing
- [ ] Unit test late `240s+` floor/tail-first behavior
- [ ] Unit test non-negative-edge gate or documented fallback behavior
- [ ] Unit test startup missing-side bypass over normal shape gating
- [ ] Unit test opening without favorite/underdog dependency
- [x] Unit test taper suppression after `240s`
- [x] Unit test final-quiet suppression after `270s`
- [x] Unit test clone metrics helpers

## Validation Run Requirements
- [x] Run `cargo check -q`
- [x] Run targeted Sprint 4 tests
- [x] Run `cargo test -q` for behavior-changing implementation
- [x] Run first Sprint 4 canary
- [x] Review first canary for:
  - participation
  - seed timing
  - second-side timing
  - maker share
  - fill cadence
  - late taper
  - final-minute quieting
  - whether CPP stayed informational and did not suppress aggressive inventory building
- [ ] Run second Sprint 4 canary after PairBuild hardening
- [ ] Review second canary for:
  - stale-cancel / repost churn
  - exchange-invalid maker-order rejects
  - unmatched size
  - share skew
  - combined average paid
  - below-snapshot optional fill rate
  - worst-case settlement floor
  - whether the remaining tail still wipes out the paired edge
  - whether startup and taper stayed intact

## Done Criteria
- [x] Sprint 4 has a separate runnable mode
- [x] opening is pre-armed and immediate
- [x] one-sided startup fills are treated as normal startup completion
- [x] missing-side startup restoration is no longer blocked by normal shape rules
- [x] normal flow replenishes continuously
- [x] normal flow stays maker-first and `BUY`-only
- [x] late taper behavior is visible and measurable
- [x] Sprint 4 metrics are emitted and coherent
- [x] first canary is reviewable against the wallet-clone fingerprint
- [ ] CPP logic does not collapse aggressive high-frequency participation
- [ ] wallet-clone `PairBuild` no longer churns viable resting maker orders on a generic stale horizon
- [ ] wallet-clone normal flow no longer leaks sub-minimum maker orders to the exchange
- [x] optional growth respects the cheap-pair rule, below-snapshot rule, and non-negative-edge rule or explicit fallback
- [ ] repair reserve prevents heavy-side growth from stranding the likely repair tail
- [ ] exact-gap lighter-side repair no longer leaves avoidable residual tails
- [ ] final paired inventory quality is at or near break-even and the remaining tail does not wipe out the paired edge on reviewed canaries
