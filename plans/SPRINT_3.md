# Sprint 3: Settlement Payoff-Shaping Engine

## Sprint Goal
Introduce a new execution mode:

1. `EXEC_MODE=SETTLEMENT_SHAPER`

and use it to replace the current pair-repair / protected-floor objective with the wallet-matched objective:

1. always build both sides in the same BTC 5-minute market
2. spend more dollars on the favorite side
3. end with more shares on the underdog side
4. keep pair coverage high and skew mild
5. hold to settlement in the normal path
6. redeem after resolution

This sprint is not a tuning pass. It is a strategy-objective rewrite and a new mode boundary.

## Different Strategy From The Current Protected-Floor Bot
The current bot was shaped around:

1. pair first
2. hard recovery ownership
3. protected downside floor
4. mild skew only after floor safety

That was a reasonable fit for the `0x8e9c...` style wallet.

It is not a correct fit for `0x2d8b... / vidarx`.

The new target is:

1. always buy both sides
2. hold to settlement in the normal path
3. spend more dollars on the favorite
4. end with more shares on the underdog
5. allow a mildly negative floor when the final shaped book is still attractive enough
6. win across many windows, not necessarily on every single window

## Strategy Baseline From The New Spec
The target wallet is best described as:

1. two-sided settlement payoff-shaping
2. favorite side = floor leg
3. underdog side = convexity leg
4. high pair coverage, mild skew, low two-sided total cost

Target operating ranges:

1. `pair_coverage`
   - soft minimum: `0.80`
   - good: `0.90+`
2. `share_skew_ratio`
   - ideal: `1.05 - 1.20`
   - soft cap: `1.30`
   - hard cap: `1.40`
3. `favorite_cost_fraction`
   - target: `0.60 - 0.67`
4. `underdog_share_fraction`
   - target: `0.51 - 0.60`
5. `vwap_sum`
   - great: `< 0.94`
   - good: `0.94 - 0.97`
   - caution: `0.97 - 1.00`
   - stop new optionality: `> 1.00`

## Primary Control Variables
This strategy should stop using `fee_net_worst_case_pnl >= 0` as the always-on normal-path rule.

The main objective variables are:

1. `pair_coverage`
2. `share_skew_ratio`
3. `favorite_cost_fraction`
4. `underdog_share_fraction`
5. `vwap_sum`

The normal path should optimize for:

1. high pair coverage
2. mild skew
3. favorite-dollar bias
4. underdog-share bias
5. low enough two-sided cost that settlement still has edge over many windows

## Current Engine Summary
Current runtime architecture inside `EXEC_MODE=MAKER_SKEW_ARB` is:

1. `RiskExitOnly`
2. `MergePending` / recovery
3. `PairBase`
4. `Skew`

What current code is good at:

1. owning imbalance correctly
2. keeping Step 1 maker-first in normal flow
3. resolving many cycles back to flat
4. using terminal taker logic to bound loss

What current code is still optimizing for:

1. pair completion
2. damage control
3. flattening mismatch

That is not the same as the new target.

## New Mode Target
Sprint 3 should land as:

1. `EXEC_MODE=SETTLEMENT_SHAPER`

with this runtime ownership order:

1. `RiskExitOnly`
2. `MergePending` as short-lived entry / inventory asymmetry repair
3. `PairResting` as the main shaping engine
4. `Skew` as a bounded normal-flow underdog overlay
5. `SettlementRedeem` after resolution

`EXEC_MODE=MAKER_SKEW_ARB` should remain as the existing fallback/canary baseline until the new mode is proven.

## Concrete Module Mapping
### `PairResting`
This becomes the main engine. Its job is:

1. always maintain both sides
2. target favorite-side dollars
3. target underdog-side shares
4. keep pair coverage high
5. keep skew mild

### `MergePending`
This becomes entry / inventory asymmetry recovery, not the main optimization layer.

Use it only for:

1. one side missing
2. one side rejected
3. accidental one-leg fill
4. bad startup asymmetry

It should be short-lived.

### `Skew`
This becomes a bounded normal-flow shaping layer, not just a late optional overlay.

Use it only when:

1. pair coverage is already healthy
2. skew is still mild
3. `vwap_sum` is still attractive
4. the book will not blow out

### `RiskExitOnly`
Keep it as the emergency brake:

1. feed failure exits
2. hard budget / hard skew exits
3. pathological market handling
4. late bad asymmetry exits

### `SettlementRedeem`
Add a new module for the normal end-of-window path:

1. wait for `market_resolved`
2. identify the winner
3. redeem winning tokens
4. mark losing side to zero
5. compute fee-adjusted realized settlement PnL

## Window Lifecycle
### Phase A: Discovery / Arm
At market open:

1. discover the Up / Down token IDs
2. subscribe market and user channels
3. initialize:
   - `q_up`, `q_down`
   - `cost_up`, `cost_down`
   - `pair_coverage`
   - `share_skew_ratio`
   - `favorite`, `underdog`
   - `favorite_cost_fraction`, `underdog_share_fraction`
   - `vwap_sum`

### Phase B: 0-30s Seed Both Sides
Goal:

1. establish both sides immediately
2. avoid one-sided starts
3. build coverage, not skew

Action:

1. place small maker orders on both sides
2. default clip ladder: `5, 10, 20`
3. no hard bias yet
4. batch the initial pair if the client path supports it cleanly

### Phase C: 30-60s Early Build
Goal:

1. keep both sides active
2. start moving dollars toward the favorite
3. start moving clip count toward the underdog

Action:

1. if `pair_coverage < 0.80`, buy the lighter-share side first
2. otherwise:
   - larger dollar notional to the favorite
   - more clip count to the underdog

### Phase D: 60-180s Main Build
This is the engine room.

Decision tree:

1. if `pair_coverage < 0.80`
   - buy the lighter-share side
   - ignore overlay
2. if `0.80 <= pair_coverage < 0.90`
   - keep building both
   - favorite-dollar bias
   - underdog clip-count bias
3. if `pair_coverage >= 0.90`
   - allow small underdog overlay only when:
     - `share_skew_ratio < 1.20`
     - `vwap_sum < 0.97`
     - stretch state agrees
     - budget remains available
4. allow `40` or `80` share size-up only when:
   - `60 <= t_into <= 180`
   - `pair_coverage >= 0.85`
   - `vwap_sum < 0.97`
   - skew is not already too large
   - and mostly on the favorite side

### Phase E: 180-240s Finish Shape
Goal:

1. stop sloppy growth
2. leave the book in the right settlement shape

Rules:

1. if `share_skew_ratio >= 1.20`, stop underdog overlay
2. if `pair_coverage < 0.85`, repair coverage
3. if `vwap_sum > 1.00`, stop opening new optionality
4. keep favorite cost fraction and underdog share fraction in range
5. use mostly `5, 10, 20, 25`, occasional `40`

### Phase F: 240-300s Freeze / Micro-Repair
Goal:

1. protect the final book
2. avoid making the settlement shape worse late

Rules:

1. no new overlay
2. no new large clips
3. only tiny repair buys if coverage is still weak
4. otherwise stop and wait for settlement

## Concrete Gaps
### Gap A: Wrong Core Objective
Current behavior asks:

1. how do I get back to flat or near-flat safely?

Target behavior asks:

1. how do I keep a high-coverage, mildly skewed, low-cost settlement book?

Implication:

1. flatten back to equal shares is the wrong default objective
2. maintain a covered asymmetric settlement book must become the new default objective

### Gap B: No Explicit Favorite-Dollar / Underdog-Share Targeting
Current code tracks coverage, skew, and fee-net, but does not explicitly target:

1. `favorite_cost_fraction = 0.60 - 0.67`
2. `underdog_share_fraction = 0.51 - 0.60`

### Gap C: No Time-Phased Shaping Model
Current engine does not directly implement the five-phase shaping schedule:

1. seed both sides
2. early build
3. main accumulation
4. finish shape
5. freeze / micro-repair

### Gap D: Sub-Min Policy Is Wrong For This Strategy
Current bot still has flatten-first sub-min logic.

For this wallet style, sub-min handling must compare:

1. hold
2. continue shaping
3. exact heavy-side sell
4. taker buy light side

against the target final book, not only against immediate risk reduction.

### Gap E: No Explicit `vwap_sum` Regime Control
`vwap_sum` must become a first-class gate with explicit green / yellow / red behavior.

### Gap F: No Clip Ladder / Size-Up Logic Matching The Wallet
Target clip ladder is:

1. small maker: `5, 10, 20, 25`
2. medium: `40`
3. large: `80`

with large size-up mostly for favorite-side shaping during the main build phase.

### Gap G: Current Step 2 Overlay Is The Wrong Abstraction
Current Step 2 is a gentle cheaper-side skew overlay.

Target behavior is:

1. favorite-dollar / underdog-share settlement shaping inside a high-coverage two-sided book

### Gap H: Stretch Overlay Is Not Implemented Correctly Yet
The target strategy wants a mild mean-reversion overlay using:

1. `binance_delta_from_start`
2. `RSI`
3. underdog alignment
4. good coverage
5. low skew
6. good `vwap_sum`

### Gap I: Settlement Path Is Missing
The wallet style is BUY-only in normal flow and relies on settlement / redemption as part of the business logic.

Current engine is still too pre-expiry / flatten oriented.

### Gap J: Metrics Do Not Yet Measure The Right Success Criteria
The new strategy needs explicit tracking of:

1. favorite cost fraction
2. underdog share fraction
3. `vwap_sum` regime occupancy
4. final pair coverage distribution
5. final skew distribution
6. clip-size mix by phase
7. maker vs aggressive size-up mix
8. realized settlement PnL

## What Can Be Reused
The current engine still has good components worth keeping:

1. ownership order and state-machine discipline
2. pair-base accounting and fill ownership
3. fee-net helper infrastructure
4. recovery and risk-exit safety rails
5. top-of-book gating and maker order lifecycle
6. websocket / market ownership framework
7. canary metrics framework

Sprint 3 should reuse the control core, but behind a new mode boundary:

1. `EXEC_MODE=SETTLEMENT_SHAPER`

not by continuing to overload `EXEC_MODE=MAKER_SKEW_ARB`.

## What Must Change
### 1. Replace Repair-To-Flat With Repair-To-Target-Shape
Recovery should stop asking:

1. how do I get `q_up ~= q_down`?

It should ask:

1. how do I get back to the intended favorite-cost / underdog-share shape while keeping coverage high?

### 2. Add Target State Variables
The bot needs explicit per-market targets:

1. `target_pair_coverage`
2. `target_share_skew_ratio`
3. `target_favorite_cost_fraction`
4. `target_underdog_share_fraction`
5. `target_vwap_sum_band`

### 3. Introduce A Time-Phase Controller
Add explicit phase logic:

1. `DiscoveryArm`
2. `SeedBothSides`
3. `EarlyBuild`
4. `MainAccumulation`
5. `FinishShape`
6. `FreezeRepairOnly`

### 4. Promote Size-Up To A First-Class Action
Add explicit shaped clip selection:

1. small maker ladder
2. medium size-up
3. selective aggressive size-up

Tie this to:

1. pair coverage
2. `vwap_sum`
3. time phase
4. current cost/share split
5. favorite / underdog side

### 5. Change Sub-Min Policy
Remove the hard flatten-first bias for sub-min residuals.

Sub-min handling must become:

1. keep shaping if the final book is still good
2. flatten only when:
   - coverage is weak
   - skew is too high
   - `vwap_sum` is bad
   - or time-left is too short

### 6. Add Settlement Redemption
Add post-resolution redemption and realized settlement accounting.

### 7. Make Execution Policy Match The Wallet
Normal flow should be:

1. maker-first
2. post-only by default
3. GTC / GTD by default
4. selective aggressive size-up only for larger shaping clips

### 8. Make `vwap_sum` A First-Class Gate
Use explicit behavior bands:

1. green
2. yellow
3. red

### 9. Add Stretch Overlay As A Separate Decision
Do not fold this into generic skew.

Gate it by:

1. delta from start
2. RSI
3. current underdog
4. coverage threshold
5. skew threshold
6. `vwap_sum` threshold

### 10. Add Favorite / Underdog Detection As A First-Class Primitive
Detect favorite / underdog every tick from live side prices:

1. midpoint when stable
2. otherwise best ask / best bid proxy
3. otherwise fair-price fallback

## Suggested Sprint 3 Workstreams
### Workstream A: Target State Model
Add explicit target-state fields for:

1. favorite / underdog side
2. target cost split
3. target share split
4. target coverage band
5. target skew band
6. target `vwap_sum` band

### Workstream B: Discovery / Settlement Ownership
Add:

1. discovery / arm lifecycle for the new mode
2. `SettlementRedeem`
3. fee-adjusted realized settlement accounting

### Workstream C: Time-Phase Engine
Add explicit phase controller:

1. discovery / arm
2. `0-30s`
3. `30-60s`
4. `60-180s`
5. `180-240s`
6. `240-300s`

### Workstream D: Shape-Oriented Recovery
Replace pure flat-recovery logic with target-shape recovery logic.

### Workstream E: Size Ladder And Aggressive Size-Up
Add explicit clip ladder support:

1. `5, 10, 20, 25`
2. `40`
3. `80`

with strict phase and quality gates, and mostly favorite-side size-up.

### Workstream F: `vwap_sum` Regime Gating
Add direct strategy control from `vwap_sum` bands.

### Workstream G: Stretch Overlay
Add delta/RSI-based mild underdog convexity overlay.

### Workstream H: Metrics
Add final-state and phase-state metrics that measure whether the bot is copying the wallet fingerprint.

Required metrics:

1. `pair_coverage` distribution
2. `share_skew_ratio` distribution
3. `favorite_cost_fraction`
4. `underdog_share_fraction`
5. `vwap_sum` regime occupancy
6. realized settlement PnL
7. maker/taker cost mix
8. percent of windows with skew `> 1.30`
9. percent of windows with pair coverage `< 0.80`
10. clip-size mix by phase

## Recommended Implementation Order
Do not build this all at once.

1. add target-state metrics first
2. add favorite / underdog detection
3. add time-phase controller
4. add settlement redeem module
5. change recovery objective from flat to target shape
6. add `vwap_sum` regime gates
7. add explicit size ladder and medium / large size-up
8. add stretch overlay last

## Implementation Checklist
Current status:

1. Sprint 3 implementation not started
2. `EXEC_MODE=SETTLEMENT_SHAPER` does not exist yet
3. Checklist below is the concrete build order for the new mode

### Mode Boundary And Routing
- [ ] Add `EXEC_MODE=SETTLEMENT_SHAPER` to the runtime mode dispatch
- [ ] Keep `EXEC_MODE=MAKER_SKEW_ARB` unchanged as the fallback baseline
- [ ] Ensure Sprint 3 logic lives in a new mode path, not as another patch inside the old generic skew loop
- [ ] Add startup logs that clearly show the mode, budgets, target bands, and active phase controller

### Workstream A: Target State Model
- [ ] Add per-market target state for:
  - favorite / underdog side
  - target pair coverage band
  - target share skew band
  - target favorite cost fraction band
  - target underdog share fraction band
  - target `vwap_sum` band
- [ ] Add helpers to compute:
  - `pair_coverage`
  - `share_skew_ratio`
  - `favorite_cost_fraction`
  - `underdog_share_fraction`
  - `vwap_sum`
- [ ] Add tests for those helpers against representative inventory states

### Workstream B: Discovery / Settlement Ownership
- [ ] Add a dedicated `SETTLEMENT_SHAPER` runtime state object
- [ ] Add Discovery / Arm lifecycle for the new mode
- [ ] Add `SettlementRedeem` state ownership after resolution
- [ ] Implement resolved-market redemption / realized settlement accounting flow
- [ ] Surface settlement result in final metrics and trade-row logging

### Workstream C: Time-Phase Engine
- [ ] Add explicit phase enum:
  - `DiscoveryArm`
  - `SeedBothSides`
  - `EarlyBuild`
  - `MainAccumulation`
  - `FinishShape`
  - `FreezeRepairOnly`
- [ ] Add time-phase transition helper from `t_into_s`
- [ ] Route normal decisions through phase-specific handlers
- [ ] Add phase-specific logs so the run shows which phase is active and why

### Workstream D: Shape-Oriented Recovery
- [ ] Replace pure flat-recovery objective with target-shape recovery
- [ ] Make `MergePending` short-lived and only for:
  - one side missing
  - one side rejected
  - one-leg fill
  - startup asymmetry
- [ ] Remove the assumption that normal-path recovery must keep `fee_net_worst_case_pnl >= 0`
- [ ] Make sub-min handling compare:
  - hold
  - continue shaping
  - exact heavy-side sell
  - taker buy light side
- [ ] Add tests proving recovery repairs toward target shape, not merely equal shares

### Workstream E: Size Ladder And Aggressive Size-Up
- [ ] Add explicit clip ladder support:
  - small: `5, 10, 20, 25`
  - medium: `40`
  - large: `80`
- [ ] Add favorite-side size-up gating for `40 / 80` clips
- [ ] Restrict large size-up to:
  - main build window
  - good coverage
  - acceptable `vwap_sum`
  - acceptable skew
- [ ] Add logs that state which clip bucket was chosen and why

### Workstream F: `vwap_sum` Regime Gating
- [ ] Add explicit regime helper:
  - green `< 0.94`
  - good `0.94 - 0.97`
  - caution `0.97 - 1.00`
  - stop overlay `> 1.00`
- [ ] Make normal shaping behavior depend on the current regime
- [ ] Stop opening new optionality when the regime is above the overlay cutoff
- [ ] Add tests that verify each regime produces the expected action restrictions

### Workstream G: Favorite / Underdog Detection And Stretch Overlay
- [ ] Add favorite / underdog detection from live side pricing
- [ ] Add fallback order:
  - midpoint if stable
  - otherwise best ask / best bid proxy
  - otherwise fair-price fallback
- [ ] Add stretch overlay gating from:
  - `binance_delta_from_start`
  - RSI
  - current underdog
  - coverage threshold
  - skew threshold
  - `vwap_sum` threshold
- [ ] Keep overlay bounded and disabled outside the approved regime

### Workstream H: Metrics And Canary Instrumentation
- [ ] Add final-state metrics for:
  - `pair_coverage`
  - `share_skew_ratio`
  - `favorite_cost_fraction`
  - `underdog_share_fraction`
  - `vwap_sum`
  - realized settlement PnL
  - maker/taker cost mix
  - clip-size mix by phase
- [ ] Add distribution counters for:
  - windows with skew `> 1.30`
  - windows with pair coverage `< 0.80`
- [ ] Add per-phase action counts and size totals
- [ ] Emit a dedicated `[SETTLEMENT_SHAPER][METRICS]` summary line in `src/main.rs`
- [ ] Verify metrics are internally consistent before trusting canary results

### Config And Docs
- [ ] Add new env keys to `src/env_contract.rs`
- [ ] Document all Sprint 3 env keys in `ENVIRONMENT.md`
- [ ] Update `TARGET_GOAL_STATUS.md` when Sprint 3 has a real runnable canary
- [ ] Add a `behaviour-<version>.md` note once the first `SETTLEMENT_SHAPER` canary is run

### Canary Readiness Criteria
- [ ] New mode compiles and tests pass
- [ ] Final logs expose the active phase, target state, and settlement result
- [ ] No hidden fallback into `MAKER_SKEW_ARB` behavior when `EXEC_MODE=SETTLEMENT_SHAPER`
- [ ] Metrics are emitted and internally coherent
- [ ] First canary can be run with `EXEC_MODE=SETTLEMENT_SHAPER`
- [ ] Baseline comparison remains possible with `EXEC_MODE=MAKER_SKEW_ARB`

## Initial Config Shape
Sprint 3 should aim for a first config surface like:

1. `STRATEGY_MODE=VIDARX_SETTLEMENT_SHAPE`
2. window budgets:
   - median / min / typical max / hard max
3. shape targets:
   - pair coverage bands
   - skew bands
   - favorite cost fraction band
   - underdog share fraction band
4. timing phases:
   - seed / early build / main build / finish shape / freeze
5. clip ladder:
   - small / medium / large
6. overlay / stretch controls:
   - coverage floor
   - skew cap
   - `vwap_sum` cap
   - RSI thresholds
7. execution controls:
   - maker default
   - post-only default
   - default order type
   - aggressive size-up enable
8. settlement controls:
   - `ENABLE_SETTLEMENT_REDEEM=true`

## Canary Recommendation
Sprint 3 should start with:

1. `EXEC_MODE=SETTLEMENT_SHAPER` as the new canary
2. `EXEC_MODE=MAKER_SKEW_ARB` kept intact as the fallback baseline

Canary rule:

1. new mode off by default in production rollout
2. enable only by explicitly selecting:
   - `EXEC_MODE=SETTLEMENT_SHAPER`
3. compare against current baseline on:
   - final pair coverage
   - final share skew
   - favorite cost fraction
   - underdog share fraction
   - `vwap_sum`
   - realized settlement PnL
   - maker/taker cost mix
   - clip-size mix by phase

## Bottom Line
The gap is not just one more overlay or different skew tuning.

The gap is:

1. current bot is still a pair-repair / protected-floor engine
2. target wallet is a settlement payoff-shaping engine

That means Sprint 3 is a strategy-objective rewrite and should be built as:

1. `EXEC_MODE=SETTLEMENT_SHAPER`

not as a small patch on Step 2.
