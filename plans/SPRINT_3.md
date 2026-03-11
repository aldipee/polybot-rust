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

## Current Canary Requirements
The long-term Sprint 3 target still includes settlement accounting and redemption support.
However, the current live canary requirements are now:

1. foreground near-expiry rollover must match `MAKER_SKEW_ARB`
   - stop trading inside the normal stop-buffer window
   - let `main` advance to the next market immediately
   - do not keep the foreground market loop pinned waiting through expiry
2. one-sided paired-growth books must not get trapped in indefinite `ShapeRepair`
   - if the remaining repair is below one maker lot, the controller needs a lot-aware fallback
   - that fallback should route back to `PairResting` or an explicit healthy-rest state, not repeat `sub_min_best_action=hold` forever

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

### Dual `vwap_sum`
Split `vwap_sum` into two different quantities:

1. `inventory_vwap_sum`
   - derived from actual acquired inventory
   - used to judge final book quality
2. `market_snapshot_vwap_sum`
   - derived from current live market prices / top-of-book
   - used to decide whether the next clip is attractive enough to place

Do not mix them. One is about what the bot already owns; the other is about whether the next action should be allowed.

## Primary Control Variables
This strategy should stop using `fee_net_worst_case_pnl >= 0` as the always-on normal-path rule.

The main objective variables are:

1. `pair_coverage`
2. `share_skew_ratio`
3. `favorite_cost_fraction`
4. `underdog_share_fraction`
5. `inventory_vwap_sum`
6. `market_snapshot_vwap_sum`

The normal path should optimize for:

1. high pair coverage
2. mild skew
3. favorite-dollar bias
4. underdog-share bias
5. low enough two-sided cost that settlement still has edge over many windows

### Target-State Math
This mode should not run on loose target bands alone. It needs explicit target deltas per tick:

```text
target_cost_favorite = targetFavoriteCostFraction * costTotal
target_shares_underdog = targetUnderdogShareFraction * (q_up + q_down)
coverage_gap = targetPairCoverage - pairCoverage
skew_gap = shareSkewRatio - targetShareSkewRatio
favorite_cost_gap = target_cost_favorite - actual_cost_favorite
underdog_share_gap = target_shares_underdog - actual_shares_underdog
```

Every normal-flow action should be evaluated by:

1. how much it improves `coverage_gap`
2. how much it improves `favorite_cost_gap`
3. how much it improves `underdog_share_gap`
4. how much it worsens or improves `market_snapshot_vwap_sum`
5. whether it violates skew caps or phase budgets

The implementation should be a target-state controller, not a heuristic soup.

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

## Pre-Implementation Corrections
Before coding too far into Sprint 3, keep these eight corrections explicit:

1. Rename the old recovery meaning in the new mode.
   - `MergePending` is a dangerous name for `SETTLEMENT_SHAPER`.
   - In this mode the distinct concepts are:
     - `EntryRepair`
     - `ShapeRepair`
   - The new mode should not inherit a hidden flat-first objective from old naming.

2. Keep entry asymmetry repair and shape repair separate.
   - `EntryRepair` fixes missing-side / rejected-side / startup asymmetry.
   - `ShapeRepair` repairs the final settlement shape.
   - They are not the same control problem.

3. Drive behavior from explicit target-state math.
   - Every action should be scored against:
     - `coverage_gap`
     - `skew_gap`
     - `favorite_cost_gap`
     - `underdog_share_gap`
   - Do not let the mode devolve into heuristic drift.

4. Use a stable favorite / underdog primitive with hysteresis.
   - Do not allow favorite / underdog to flap on noisy top-of-book updates.
   - Keep hysteresis as a first-class part of the design.

5. Treat `inventory_vwap_sum` and `market_snapshot_vwap_sum` as different variables.
   - `inventory_vwap_sum` judges the quality of what the bot already owns.
   - `market_snapshot_vwap_sum` judges whether the next trade should be allowed.

6. Enforce phase budgets explicitly.
   - The bot must not spend the whole window too early.
   - Seed / early / main / finish / freeze reserve should each have their own spend ceiling.

7. Separate favorite-side size-up from underdog overlay.
   - favorite-side size-up:
     - floor support
     - larger clips
   - underdog overlay:
     - convexity shaping
     - smaller clips
   - These should be distinct actions, metrics, and controls.

8. Define hard canary rollback conditions up front.
   - The canary should be judged by settlement-book fingerprint, not by "did it trade".
   - Rollback conditions must be explicit before rollout begins.

## New Mode Target
Sprint 3 should land as:

1. `EXEC_MODE=SETTLEMENT_SHAPER`

with this runtime ownership order:

1. `RiskExitOnly`
2. `EntryRepair` as short-lived startup / entry asymmetry repair
3. `ShapeRepair` as target-shape repair
4. `PairResting` as the main shaping engine
5. `Skew` as a bounded normal-flow underdog overlay
6. `SettlementRedeem` after resolution

`EXEC_MODE=MAKER_SKEW_ARB` should remain as the existing fallback/canary baseline until the new mode is proven.

## Concrete Module Mapping
### `PairResting`
This becomes the main engine. Its job is:

1. always maintain both sides
2. target favorite-side dollars
3. target underdog-side shares
4. keep pair coverage high
5. keep skew mild

### `EntryRepair`
This becomes entry / inventory asymmetry recovery, not the main optimization layer.

Use it only for:

1. one side missing
2. one side rejected
3. accidental one-leg fill
4. bad startup asymmetry

It should be short-lived and its objective is:

1. restore both-side participation
2. not restore equal shares by default

### `ShapeRepair`
This is separate from startup repair.

Use it when:

1. pair coverage is too low
2. skew is too high
3. `inventory_vwap_sum` or `market_snapshot_vwap_sum` is too bad
4. favorite / underdog fractions drift too far from target

Its objective is:

1. repair toward target final shape
2. not repair toward flat unless the shape has become invalid

### `Skew`
This becomes a bounded normal-flow shaping layer, not just a late optional overlay.

Use it only when:

1. pair coverage is already healthy
2. skew is still mild
3. `market_snapshot_vwap_sum` is still attractive
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

Current canary requirement:

1. keep `SettlementRedeem` and settlement accounting implemented
2. but let the foreground market loop roll over near expiry like `MAKER_SKEW_ARB`
3. do not block the next market by waiting for resolution in the foreground trading loop

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
   - `inventory_vwap_sum`
   - `market_snapshot_vwap_sum`

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
     - `market_snapshot_vwap_sum < 0.97`
     - stretch state agrees
     - budget remains available
4. allow `40` or `80` share size-up only when:
   - `60 <= t_into <= 180`
   - `pair_coverage >= 0.85`
   - `market_snapshot_vwap_sum < 0.97`
   - skew is not already too large
   - and mostly on the favorite side

### Phase E: 180-240s Finish Shape
Goal:

1. stop sloppy growth
2. leave the book in the right settlement shape

Rules:

1. if `share_skew_ratio >= 1.20`, stop underdog overlay
2. if `pair_coverage < 0.85`, repair coverage
3. if `market_snapshot_vwap_sum > 1.00`, stop opening new optionality
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

## Phase Budgets
Do not let the mode spend the whole window budget too early. Add phase-local budget ceilings:

1. `SeedBothSides`: `10-15%`
2. `EarlyBuild`: `15-20%`
3. `MainAccumulation`: `45-55%`
4. `FinishShape`: `15-20%`
5. `FreezeRepairOnly` reserve: `5-10%`

These should be enforced as explicit phase-local spend ceilings on top of the overall window budget.

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
`inventory_vwap_sum` and `market_snapshot_vwap_sum` must become first-class gates with explicit green / yellow / red behavior.

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
6. good `market_snapshot_vwap_sum`

### Gap I: Settlement Path Is Missing
The wallet style is BUY-only in normal flow and relies on settlement / redemption as part of the business logic.

Current engine is still too pre-expiry / flatten oriented.

### Gap J: Metrics Do Not Yet Measure The Right Success Criteria
The new strategy needs explicit tracking of:

1. favorite cost fraction
2. underdog share fraction
3. `inventory_vwap_sum` regime occupancy
4. `market_snapshot_vwap_sum` regime occupancy
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
5. `target_inventory_vwap_sum_band`
6. `target_market_snapshot_vwap_sum_band`

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
2. `market_snapshot_vwap_sum`
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
   - `inventory_vwap_sum` or `market_snapshot_vwap_sum` is bad
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
Use both:

1. `inventory_vwap_sum` to judge the quality of the current held book
2. `market_snapshot_vwap_sum` to judge whether the next trade should be allowed

with explicit behavior bands:

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
6. `market_snapshot_vwap_sum` threshold

### 10. Add Favorite / Underdog Detection As A First-Class Primitive
Detect favorite / underdog every tick from live side prices:

1. midpoint when stable
2. otherwise best ask / best bid proxy
3. otherwise fair-price fallback

This needs hysteresis so the role does not flap on noisy top-of-book updates.

Suggested controls:

1. `FAV_UNDERDOG_SWITCH_MIN_DIFF=0.01`
2. `FAV_UNDERDOG_SWITCH_CONFIRM_UPDATES=3`

Rule:

1. if `abs(price_up - price_down) < switch_min_diff`, keep the previous assignment
2. only switch favorite / underdog after the stronger difference persists for the required confirm count

## Suggested Sprint 3 Workstreams
### Workstream A: Target State Model
Add explicit target-state fields for:

1. favorite / underdog side
2. target cost split
3. target share split
4. target coverage band
5. target skew band
6. `inventory_vwap_sum` band
7. `market_snapshot_vwap_sum` band
8. explicit target deltas:
   - `coverage_gap`
   - `skew_gap`
   - `favorite_cost_gap`
   - `underdog_share_gap`

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

### Workstream D: Entry / Shape Repair
Split repair into two controllers:

1. `EntryRepair`
   - one side missing
   - one side rejected
   - one-leg startup fill
   - restore both-side participation, not equal shares
2. `ShapeRepair`
   - coverage too low
   - skew too high
   - target fractions too far off
   - repair toward target final shape

### Workstream E: Size Ladder And Aggressive Size-Up
Add explicit clip ladder support:

1. `5, 10, 20, 25`
2. `40`
3. `80`

with strict phase and quality gates.

Also split:

1. favorite-side size-up
   - `40 / 80`
   - floor and favorite-dollar support
2. underdog overlay
   - `5 / 10 / 20 / 25`
   - convexity shaping only when coverage is already healthy

### Workstream F: `vwap_sum` Regime Gating
Add direct strategy control from:

1. `inventory_vwap_sum` bands
2. `market_snapshot_vwap_sum` bands

### Workstream G: Favorite / Underdog Stability And Stretch Overlay
Add:

1. favorite / underdog hysteresis
2. delta/RSI-based mild underdog convexity overlay

### Workstream H: Metrics
Add final-state and phase-state metrics that measure whether the bot is copying the wallet fingerprint.

Required metrics:

1. `pair_coverage` distribution
2. `share_skew_ratio` distribution
3. `favorite_cost_fraction`
4. `underdog_share_fraction`
5. `inventory_vwap_sum` regime occupancy
6. `market_snapshot_vwap_sum` regime occupancy
6. realized settlement PnL
7. maker/taker cost mix
8. percent of windows with skew `> 1.30`
9. percent of windows with pair coverage `< 0.80`
10. clip-size mix by phase

## Recommended Implementation Order
Do not build this all at once.

1. add target-state metrics first
2. add favorite / underdog detection with hysteresis
3. add time-phase controller
4. add settlement redeem module
5. add target-state model and target deltas
6. add `EntryRepair` and `ShapeRepair`
7. add explicit size ladder and favorite-side size-up
8. add underdog overlay
9. add stretch overlay last

## Implementation Checklist
Current status:

1. Sprint 3 implementation started
2. `EXEC_MODE=SETTLEMENT_SHAPER` now exists as a live canary route with its own loop, runtime state, startup/phase logs, and bounded maker actions
3. Mode Boundary And Routing is complete for the first canary boundary PR
4. Workstream A derived metrics foundation is complete in the live canary
5. Workstream G stable favorite / underdog detection with hysteresis and midpoint -> ask/bid proxy -> fair-price fallback pricing is live in the canary
6. Workstream C phase-local budget ceilings and phase-specific handler routing are live in the canary
7. Workstream B `SettlementRedeem` state ownership and resolved-market settlement accounting are live, and foreground near-expiry rollover now matches `MAKER_SKEW_ARB`
8. Workstream D owner split is live, `EntryRepair` / `ShapeRepair` have bounded maker execution slices, normal-path repair no longer requires `fee_net_worst_case_pnl >= 0`, sub-min shape repair now compares multiple action families, Workstream E clip ladder support plus distinct favorite-side size-up and underdog overlay are live, and Workstream F explicit `vwap_sum` regime gating now controls normal optionality
9. Workstream H metrics and canary instrumentation is complete, including final-state metrics, distribution counters, per-phase action summaries, and internal consistency checks surfaced in runtime/final logs
10. Real live canaries have now exercised startup seed, entry repair, balanced base build, one additional paired-growth step, and next-market rollover
11. Repair sizing no longer rounds `EntryRepair` / `ShapeRepair` clips up to the next ladder rung; repair intents now round down to the nearest maker lot inside the allowed bucket cap
12. Owner routing now returns no-legal-repair books to `PairResting` with explicit rest reasons, instead of leaving them in indefinite `ShapeRepair` loops
13. Paired growth now has a narrow blocked-rebuild `inventory_vwap_sum` allowance, so `PairResting` is no longer hard-stopped at `vwap_sum_good` when recovering from `repair_blocked_sub_lot_rest`, `repair_blocked_inventory_quality_rest`, or `repair_blocked_hard_skew_rest`
14. Paired growth now waits on existing family live orders, ignores already-filled family slots in asymmetry detection, and only records a fresh paired-growth action when both legs are actually live
15. Config And Docs env allowlist, operator documentation, and `behaviour-0.1.27.md` are complete for the live settlement-shaper canary surface
16. True missing-side startup recovery now bypasses the normal hard-skew reject path, settlement-shaper builder orders now bypass the generic maker recovery gate, and directional-step now uses a settlement-shaper core-build allowance instead of repair-style target-pressure rejection
17. Directional-step now also has a near-target blocked-rest `inventory_vwap_sum` allowance, so late-phase one-lot underdog steps can stay live when they reach good coverage and target skew even if projected held `vwap_sum` is slightly above the generic rebuild ceiling
18. Near-target partial-fill books now treat an almost-exact one-lot underdog surplus as already achieved, so a live state like `49.99 / 45.00` should rest instead of asking for one more full underdog lot
19. Healthy late books with poor held `inventory_vwap_sum` now stay in `PairResting` instead of falling into `ShapeRepair -> inventory_quality_poor`, so the late builder keeps ownership for books like `45.00 / 44.99`
20. Current next task: run a fresh live canary to validate that a late healthy book no longer flips into `ShapeRepair -> inventory_quality_poor` and instead stays in `PairResting` for directional build or clean rest; Workstream G stretch overlay gating remains intentionally deferred
21. Checklist below remains the concrete build order for the new mode

### Mode Boundary And Routing
- [x] Add `EXEC_MODE=SETTLEMENT_SHAPER` to the runtime mode dispatch
- [x] Keep `EXEC_MODE=MAKER_SKEW_ARB` unchanged as the fallback baseline
- [x] Ensure Sprint 3 logic lives in a new mode path, not as another patch inside the old generic skew loop
- [x] Add startup logs that clearly show the mode, budgets, target bands, and active phase controller

### Workstream A: Target State Model
Completed in this pass:
1. added pure derived-metric helpers and tests
2. wired the read-only canary to compute and log the settlement-shaper snapshot
3. stable favorite / underdog role assignment now feeds the canary snapshot math

- [ ] Add per-market target state for:
  - favorite / underdog side
  - target pair coverage band
  - target share skew band
  - target favorite cost fraction band
  - target underdog share fraction band
  - target `inventory_vwap_sum` band
  - target `market_snapshot_vwap_sum` band
- [x] Add helpers to compute:
  - `pair_coverage`
  - `share_skew_ratio`
  - `favorite_cost_fraction`
  - `underdog_share_fraction`
  - `inventory_vwap_sum`
  - `market_snapshot_vwap_sum`
- [x] Add explicit target-gap helpers:
  - `coverage_gap`
  - `skew_gap`
  - `favorite_cost_gap`
  - `underdog_share_gap`
- [x] Add tests for those helpers against representative inventory states

### Workstream B: Discovery / Settlement Ownership
Completed in this pass:
1. `SettlementRedeem` phase ownership activates only after a valid resolved snapshot is observed
2. resolved-market settlement accounting now latches winner, payout, and loser-zeroed settled shares into runtime state
3. final metrics and trade-row reporting now surface settlement claim status, winner, payout, and realized settlement PnL
4. foreground near-expiry rollover now stops like `MAKER_SKEW_ARB`, so the next market can start without waiting through expiry in the trading loop

- [x] Add a dedicated `SETTLEMENT_SHAPER` runtime state object
- [x] Add Discovery / Arm lifecycle for the new mode
- [x] Add `SettlementRedeem` state ownership after resolution
- [x] Implement resolved-market redemption / realized settlement accounting flow
- [x] Surface settlement result in final metrics and trade-row logging

### Workstream C: Time-Phase Engine
Completed in this pass:
1. phase-local budget ceilings now derive from the settlement-shaper budget slices
2. the read-only canary routes through explicit phase-specific handlers and logs budget availability

- [x] Add explicit phase enum:
  - `DiscoveryArm`
  - `SeedBothSides`
  - `EarlyBuild`
  - `MainAccumulation`
  - `FinishShape`
  - `FreezeRepairOnly`
- [x] Add time-phase transition helper from `t_into_s`
- [x] Add phase-local budget ceilings:
  - seed `10-15%`
  - early `15-20%`
  - main `45-55%`
  - finish `15-20%`
  - freeze / repair reserve `5-10%`
- [x] Route normal decisions through phase-specific handlers
- [x] Add phase-specific logs so the run shows which phase is active and why

### Workstream D: Entry / Shape Repair
Completed in this pass:
1. the canary now selects an explicit owner from `EntryRepair`, `ShapeRepair`, `PairResting`, and `SettlementRedeem`
2. owner transition logs now show controller reason and preserve `EntryRepair > ShapeRepair` priority
3. `EntryRepair` now submits a bounded maker quote on the missing side and cancels stale entry-repair orders when ownership changes
4. `ShapeRepair` now submits a bounded maker quote on the side that moves the book toward the target coverage / cost-share shape and cancels stale shape-repair orders when ownership changes
5. normal-path repair now uses target-shape pressure instead of a hard non-negative `fee_net_worst_case_pnl` gate, while still blocking hard-skew worsening actions
6. sub-min `ShapeRepair` now compares `hold`, `continue shaping`, `exact heavy-side sell`, and `taker buy light side`, then executes the best-scoring target-shape action
7. focused tests now cover sub-min raw-gap planning, exact heavy-side sell shape improvement, and candidate choice away from flatten-first behavior
8. repair intents now keep lot-quantized exact sizing instead of rounding up to the next ladder rung
9. sub-lot and no-legal-repair books now route back to `PairResting` with explicit rest reasons instead of staying in indefinite `ShapeRepair`
10. paired growth now allows a mild `inventory_vwap_sum` overrun when rebuilding from blocked repair-rest states, including `repair_blocked_sub_lot_rest`, instead of stopping strictly at `vwap_sum_good`
11. paired growth now waits on existing family live orders, ignores filled family slots for asymmetry detection, and no longer counts a missing leg as a fresh paired-growth submit
12. true missing-side startup recovery now bypasses the normal hard-skew rejection path, so EntryRepair can restore both-side participation after a one-leg seed fill
13. settlement-shaper builder orders now bypass the generic maker recovery gate, and directional-step now uses a settlement-shaper core-build allowance instead of repair-style target-pressure rejection

- [x] Add `EntryRepair` as a separate controller for:
  - one side missing
  - one side rejected
  - one-leg startup fill
  - startup asymmetry
- [x] Add `ShapeRepair` as a separate controller for:
  - weak coverage
  - excessive skew
  - favorite / underdog target drift
  - bad `inventory_vwap_sum` / `market_snapshot_vwap_sum`
- [x] Make `EntryRepair` restore both-side participation, not equal shares
- [x] Make `ShapeRepair` repair toward target shape, not flat
- [x] Remove the assumption that normal-path recovery must keep `fee_net_worst_case_pnl >= 0`
- [x] Make sub-min handling compare:
  - hold
  - continue shaping
  - exact heavy-side sell
  - taker buy light side
- [x] Add tests proving recovery repairs toward target shape, not merely equal shares
- [x] Add a lot-aware fallback so one-sided paired-growth books do not remain trapped in `weak_coverage -> ShapeRepair -> sub_min_best_action=hold`
- [x] Allow true missing-side startup recovery to bypass the normal hard-skew reject path so EntryRepair can restore the missing leg after a one-leg seed fill
- [x] Stop settlement-shaper paired growth / directional-step from inheriting the generic maker recovery gate, and let directional-step use a blocked-rebuild core-build allowance when it improves the current held book

### Workstream E: Size Ladder And Aggressive Size-Up
Completed in this pass:
1. settlement-shaper clip buckets now exist as explicit `small` / `medium` / `large` ladder choices
2. builder / overlay intents still use the explicit ladder, while repair intents now round down to the nearest maker lot inside the allowed bucket cap
3. ladder gating keeps `EntryRepair` on small clips, allows medium coverage repair only in healthy main-build conditions, and reserves `80` for future favorite-side size-up
4. runtime config / submit logs now surface the active clip ladder and chosen clip bucket
5. `PairResting` now owns a distinct favorite-side size-up maker path with origin/logging separate from `ShapeRepair`
6. favorite-side size-up now only opens `40 / 80` candidates in allowed phase/quality windows, with `80` remaining restricted to `MainAccumulation`
7. `PairResting` now owns a distinct underdog-overlay maker path with its own small-clip ladder, origin, and submit logs
8. underdog overlay now stays bounded to healthy coverage / skew / market windows and only opens micro clips in `FinishShape`

- [x] Add explicit clip ladder support:
  - small: `5, 10, 20, 25`
  - medium: `40`
  - large: `80`
- [x] Add favorite-side size-up as a distinct action for `40 / 80` clips
- [x] Add underdog overlay as a distinct action for `5 / 10 / 20 / 25` clips
- [x] Restrict large size-up to:
  - main build window
  - good coverage
  - acceptable `market_snapshot_vwap_sum`
  - acceptable skew
- [x] Add logs that state which clip bucket was chosen and why

### Workstream F: `vwap_sum` Regime Gating
- [x] Add explicit regime helper:
  - green `< 0.94`
  - good `0.94 - 0.97`
  - caution `0.97 - 1.00`
  - stop overlay `> 1.00`
- [x] Make normal shaping behavior depend on both:
  - `inventory_vwap_sum`
  - `market_snapshot_vwap_sum`
- [x] Stop opening new optionality when the regime is above the overlay cutoff
- [x] Add tests that verify each regime produces the expected action restrictions

### Workstream G: Favorite / Underdog Detection And Stretch Overlay
- [x] Add favorite / underdog detection from live side pricing
- [x] Add hysteresis controls:
  - `FAV_UNDERDOG_SWITCH_MIN_DIFF`
  - `FAV_UNDERDOG_SWITCH_CONFIRM_UPDATES`
- [x] Add fallback order:
  - midpoint if stable
  - otherwise best ask / best bid proxy
  - otherwise fair-price fallback
- [ ] Add stretch overlay gating from:
  - `binance_delta_from_start`
  - RSI
  - current underdog
  - coverage threshold
  - skew threshold
  - `market_snapshot_vwap_sum` threshold
- [ ] Keep overlay bounded and disabled outside the approved regime

### Workstream H: Metrics And Canary Instrumentation
- [x] Add final-state metrics for:
  - `pair_coverage`
  - `share_skew_ratio`
  - `favorite_cost_fraction`
  - `underdog_share_fraction`
  - `inventory_vwap_sum`
  - `market_snapshot_vwap_sum`
  - realized settlement PnL
  - maker/taker cost mix
  - clip-size mix by phase
- [x] Add distribution counters for:
  - windows with skew `> 1.30`
  - windows with pair coverage `< 0.80`
- [x] Add per-phase action counts and size totals
- [x] Emit a dedicated `[SETTLEMENT_SHAPER][METRICS]` summary line in `src/main.rs`
- [x] Verify metrics are internally consistent before trusting canary results

### Config And Docs
- [x] Add new env keys to `src/env_contract.rs`
- [x] Document all Sprint 3 env keys in `ENVIRONMENT.md`
- [ ] Update `TARGET_GOAL_STATUS.md` when Sprint 3 has a real runnable canary
- [x] Add a `behaviour-<version>.md` note once the first `SETTLEMENT_SHAPER` canary is run

### Canary Readiness Criteria
- [x] New mode compiles and tests pass
- [x] Final logs expose the active phase, target state, and settlement result
- [x] No hidden fallback into `MAKER_SKEW_ARB` behavior when `EXEC_MODE=SETTLEMENT_SHAPER`
- [x] Metrics are emitted and internally coherent
- [x] First canary can be run with `EXEC_MODE=SETTLEMENT_SHAPER`
- [x] Baseline comparison remains possible with `EXEC_MODE=MAKER_SKEW_ARB`

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
   - `inventory_vwap_sum`
   - `market_snapshot_vwap_sum`
   - realized settlement PnL
   - maker/taker cost mix
   - clip-size mix by phase

Hard rollback conditions:

1. median `pair_coverage < 0.80`
2. too many windows with `share_skew_ratio > 1.30`
3. too many windows with `market_snapshot_vwap_sum > 1.00`
4. settlement ROI worse than the `MAKER_SKEW_ARB` baseline
5. favorite cost fraction fails to reach target range
6. underdog share fraction fails to reach target range

## Bottom Line
The gap is not just one more overlay or different skew tuning.

The gap is:

1. current bot is still a pair-repair / protected-floor engine
2. target wallet is a settlement payoff-shaping engine

That means Sprint 3 is a strategy-objective rewrite and should be built as:

1. `EXEC_MODE=SETTLEMENT_SHAPER`

not as a small patch on Step 2.

The implementation should be a target-state controller, not a collection of heuristics.


