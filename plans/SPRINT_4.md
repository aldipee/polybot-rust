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
8. stay aggressive in normal flow rather than waiting for ideal projected economics

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
- Overall status: `NOT STARTED`
- Target outcome: `WALLET-CLONE CANARY`
- Dependency on Sprint 3: `REUSE PARTS ONLY`
- Recommended runtime boundary: `NEW MODE`
- Current dominant reason: `SPRINT_3 OBJECTIVE MISMATCH`

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
Sprint 4 must behave like an aggressive inventory builder, not a conservative filter.

Required normal-path policy:

1. participation is preferred over selectivity
2. both-side completion is preferred over shape neatness
3. paired replenishment is preferred over waiting for ideal projected CPP or perfect projected shape
4. the controller should keep building unless a hard budget, hard venue constraint, or explicit emergency-risk rule blocks the action

Not allowed as an always-on normal-path veto:

1. hard CPP profitability gate
2. hard projected shape-perfectness gate
3. favorite/underdog target-pressure gate
4. mild temporary imbalance by itself

Allowed hard blockers:

1. no legal order size
2. no usable quote
3. stale or invalid market data
4. explicit hard budget breach
5. explicit terminal emergency policy

### 9. CPP / Cost-Quality Requirement
CPP, combined paid price, and similar two-sided cost-quality measures are informational quality signals, not the core controller.

Required policy:

1. `OpenBoth` must not require CPP profitability to seed both sides
2. `SeedCompletion` must not require CPP profitability to restore the missing side
3. `PairBuild` may use CPP only as a light clip-sizing hint, not as a standing stop rule
4. poor CPP may reduce optional add size, but must not stop required two-sided completion
5. Sprint 4 should remain willing to build inventory aggressively through most of the market even when the next clip is not individually attractive in isolation
6. profitability should be judged at the market-population level and repeated-fill level, not as a per-clip hard veto

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
3. sharply reduce new activity after `240s`

## Clip And Sizing Requirements
Observed behavior implies:

1. small seed and repair clips
2. repeated replenishment
3. larger passive clips as a secondary pattern

Sprint 4 should support:

1. small clips around `10-15`
2. repeated passive replenishment
3. optional larger passive clip family around `40-80`

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
Status: `NOT STARTED`

Objective:
Create a clean Sprint 4 wallet-clone path without hidden Sprint 3 shaping assumptions.

Tasks:
- [ ] Add dedicated top-level runtime dispatch for the Sprint 4 mode
- [ ] Ensure Sprint 4 does not route through Sprint 3 target-shape scoring
- [ ] Add Sprint 4 startup logs showing:
  - mode
  - phase controller
  - pre-arm status
  - clip family
  - clone timing targets
- [ ] Keep existing `MAKER_SKEW_ARB` and `SETTLEMENT_SHAPER` behavior unchanged

Acceptance:
- [ ] Sprint 4 can run without invoking favorite-dollar / underdog-share controller goals
- [ ] Logs make the objective difference from Sprint 3 explicit

### Workstream B: Pre-Arm Lifecycle
Status: `NOT STARTED`

Objective:
Make the engine ready before the market opens.

Tasks:
- [ ] Add pre-open market discovery and asset mapping ownership
- [ ] Add readiness checks for:
  - next market selected
  - asset IDs available
  - market/user feed freshness
  - initial quote inputs available
- [ ] Add explicit pre-arm hold reasons
- [ ] Ensure market-open does not start with fresh discovery by default

Acceptance:
- [ ] Sprint 4 can enter the market already armed
- [ ] startup logs show pre-arm completed before opening actions

### Workstream C: Open-Both Seed Engine
Status: `NOT STARTED`

Objective:
Submit both sides immediately and neutrally at market start.

Tasks:
- [ ] Add `OpenBoth` owner/state
- [ ] Submit paired maker `BUY` orders on both sides immediately at window start
- [ ] Use small seed clips
- [ ] Avoid favorite/underdog gating during opening
- [ ] Add explicit paired-open logs and metrics

Acceptance:
- [ ] both sides are attempted immediately at market start
- [ ] opening behavior remains maker-first and neutral

### Workstream D: Seed Completion Ownership
Status: `NOT STARTED`

Objective:
Treat one-sided startup fills as normal startup completion.

Tasks:
- [ ] Add `SeedCompletion` owner/state distinct from Sprint 3 `ShapeRepair`
- [ ] Route true one-sided startup books into `SeedCompletion`
- [ ] Ignore normal shape targets while in `SeedCompletion`
- [ ] Ignore hard skew vetoes for true missing-side restoration unless the action worsens missing-side completion
- [ ] Ignore hard CPP profitability vetoes for true missing-side restoration
- [ ] Allow `SeedCompletion` to borrow from later phase budgets
- [ ] Add explicit timing counters:
  - time to first side
  - time to second side
  - both sides by `30s`
  - both sides by `60s`

Acceptance:
- [ ] missing-side startup recovery is not blocked by normal shape gates
- [ ] logs clearly distinguish startup completion from two-sided shape repair

### Workstream E: Continuous Pair Build And Replenishment
Status: `NOT STARTED`

Objective:
Make normal flow look like a continuous paired accumulator.

Tasks:
- [ ] Add `PairBuild` owner/state
- [ ] Repost two-sided maker quotes after fills
- [ ] keep paired growth active through the main window
- [ ] add both sides while reasonably balanced
- [ ] add the lighter side first when imbalance stretches
- [ ] prevent one-shot seed then long idle behavior
- [ ] ensure CPP is only a light clip-sizing hint, not a hard normal-flow veto
- [ ] prefer smaller or lighter-side clips before choosing idle when CPP is mediocre

Acceptance:
- [ ] normal flow shows repeated paired or near-paired replenishment
- [ ] build logic remains maker-first

### Workstream F: Late Taper And Final-Minute Suppression
Status: `NOT STARTED`

Objective:
Match the observed late quieting profile.

Tasks:
- [ ] Add `Taper` owner/state or equivalent late-phase policy
- [ ] sharply reduce new accumulation after `240s`
- [ ] allow only small maintenance/repair after `240s`
- [ ] suppress almost all new activity in the final `30s`
- [ ] preserve current foreground rollover stop-buffer behavior

Acceptance:
- [ ] late logs show taper ownership and reduced activity
- [ ] final-minute activity is intentionally minimal

### Workstream G: Clone Metrics And Reviewability
Status: `NOT STARTED`

Objective:
Measure Sprint 4 against the clone fingerprint, not Sprint 3 shape goals.

Tasks:
- [ ] Add metrics for:
  - market participation rate
  - first-fill latency
  - first-opposite-side latency
  - percent both-sides-positive by `30s`
  - percent both-sides-positive by `60s`
  - fills per market
  - maker fill share
  - fill distribution by window segment
  - late-minute fill share
  - paired size
  - unmatched size
  - average two-sided cost quality
  - average realized combined paid price by market
- [ ] Add simple suppression metrics for:
  - skipped optional adds
  - startup-completion blocked count
- [ ] Emit dedicated final Sprint 4 metrics summary
- [ ] keep Sprint 3 metrics separate

Acceptance:
- [ ] the canary can be judged directly against the wallet-clone review
- [ ] metrics do not depend on favorite-dollar / underdog-share success

### Workstream H: Config Surface
Status: `NOT STARTED`

Objective:
Expose the Sprint 4 knobs that are tuning knobs, not hidden logic switches.

Tasks:
- [ ] Add new Sprint 4 env keys to `src/env_contract.rs`
- [ ] Document Sprint 4 env keys in `ENVIRONMENT.md`
- [ ] Expose controls for:
  - pre-arm lead time
  - small clip family
  - large clip family
  - phase budget slices
  - taper timing
  - startup completion time targets
  - clone maker cadence

Acceptance:
- [ ] operator-facing knobs exist for timing and sizing
- [ ] core ownership logic does not depend on undocumented env flags

### Workstream I: Tests
Status: `NOT STARTED`

Objective:
Add deterministic coverage for Sprint 4 behavior.

Tasks:
- [ ] Add phase-boundary tests for Sprint 4 timing
- [ ] Add state-routing tests:
  - `PreArm -> OpenBoth`
  - `OpenBoth -> SeedCompletion`
  - `SeedCompletion -> PairBuild`
  - `PairBuild -> Taper`
- [ ] Add tests proving one-sided startup repair bypasses normal shape gating
- [ ] Add tests proving favorite/underdog does not gate opening or seed completion
- [ ] Add tests proving late taper suppresses normal expansion
- [ ] Add tests for clone metrics helper outputs

Acceptance:
- [ ] core Sprint 4 state and gating behavior is covered by Rust tests

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

## Assumptions
1. Existing maker order lifecycle, book freshness, and fill-accounting infrastructure can be reused.
2. Existing rollover behavior is good enough to preserve.
3. Sprint 3 code can be partially reused, but Sprint 4 must not inherit Sprint 3 objectives implicitly.
4. Clone mode should prioritize observable mechanics over inferred hidden quote-management details.
5. The implementation should stay helper-first inside `src/bot.rs`, with env wiring in `src/env_contract.rs` and operator docs in `ENVIRONMENT.md`.
