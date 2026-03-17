# SETTLEMENT_SHAPER Decision Spec

## Purpose
This file is the implementation-level decision spec for:

```env
EXEC_MODE=SETTLEMENT_SHAPER
```

It translates the strategy and sprint documents into concrete runtime decision rules for a coding model.

Primary references:
1. `plans/SPRINT_3.md`
2. `OBJECTIVES.md`
3. `OBJECTIVES_STATUS.md`

This file is intentionally narrower than `SPRINT_3.md`.
`SPRINT_3.md` describes the strategy and rollout.
This file defines:
1. runtime state ownership
2. phase-local allowed actions
3. action scoring / precedence
4. maker vs aggressive execution rules
5. repair behavior when targets conflict

## Core Strategy Identity
`SETTLEMENT_SHAPER` is a settlement payoff-shaping engine.

Normal path:
1. always buy both sides
2. spend more dollars on the favorite
3. end with more shares on the underdog
4. keep pair coverage high
5. keep skew mild
6. hold to settlement
7. redeem winners after resolution

This mode is not optimized for:
1. flat inventory
2. protected floor at all times
3. one-sided prediction
4. generic skew accumulation

## State Ownership Priority
Runtime ownership order is strict:

1. `RiskExitOnly`
2. `EntryRepair`
3. `ShapeRepair`
4. `PairResting`
5. `Skew`
6. `SettlementRedeem`

Interpretation:
1. if a higher-priority owner is active, lower-priority owners must not submit new orders
2. `Skew` is a normal-flow overlay only
3. `SettlementRedeem` begins only after the market is resolved

## Runtime State Model
The new mode should use these state concepts.

### Top-level state
1. `DiscoveryArm`
2. `SeedBothSides`
3. `EarlyBuild`
4. `MainAccumulation`
5. `FinishShape`
6. `FreezeRepairOnly`
7. `SettlementRedeem`

### Repair ownership
1. `EntryRepair`
   - one side missing
   - one leg rejected
   - one-leg startup fill before both sides are live
   - immediate order arming asymmetry

2. `ShapeRepair`
   - pair coverage too weak
   - skew too high
   - favorite/underdog fractions too far from target
   - `vwap_sum` regime degraded enough that the current book shape is invalid

### Overlay ownership
1. `Skew`
   - bounded underdog-share shaping
   - only when `PairResting` is already healthy

## Required Derived Metrics
At every decision tick, compute:

### Inventory state
1. `q_up`
2. `q_down`
3. `cost_up`
4. `cost_down`
5. `cost_total = cost_up + cost_down`

### Coverage / skew
1. `pair_coverage = min(q_up, q_down) / max(q_up, q_down)`
2. `share_skew_ratio = max(q_up, q_down) / min(q_up, q_down)`

### Cost / share fractions
1. `favorite_cost_fraction = cost_favorite / cost_total`
2. `favorite_share_fraction = q_favorite / (q_up + q_down)`
3. `underdog_share_fraction = q_underdog / (q_up + q_down)`

### Cost quality
1. `inventory_vwap_sum = (cost_up / q_up) + (cost_down / q_down)`
2. `market_snapshot_vwap_sum`
   - computed from current live executable prices
   - use current best ask on both sides if present
   - fallback policy defined in `Favorite / Underdog Detection`

### Target-state deltas
1. `target_cost_favorite = targetFavoriteCostFraction * cost_total`
2. `target_shares_underdog = targetUnderdogShareFraction * (q_up + q_down)`
3. `coverage_gap = target_pair_coverage - pair_coverage`
4. `skew_gap = share_skew_ratio - target_share_skew_ratio`
5. `favorite_cost_gap = target_cost_favorite - cost_favorite`
6. `underdog_share_gap = target_shares_underdog - q_underdog`

### Settlement views
1. `pnl_if_up_wins = q_up - cost_total`
2. `pnl_if_down_wins = q_down - cost_total`

## Favorite / Underdog Detection
This must be stable and hysteretic.

### Default ranking source
Use this precedence:
1. midpoint if both sides have fresh best bid and best ask
2. otherwise best ask
3. otherwise best bid
4. otherwise keep prior assignment

### Assignment
1. higher price side = favorite
2. lower price side = underdog

### Hysteresis
Do not flip favorite/underdog on a tiny or transient difference.

Required config:
1. `FAV_UNDERDOG_SWITCH_MIN_DIFF=0.01`
2. `FAV_UNDERDOG_SWITCH_CONFIRM_UPDATES=3`

Decision rule:
1. if `abs(price_up - price_down) < FAV_UNDERDOG_SWITCH_MIN_DIFF`, keep previous assignment
2. if difference exceeds threshold, require `FAV_UNDERDOG_SWITCH_CONFIRM_UPDATES` consecutive qualifying updates before switching
3. if no prior assignment exists, use current ranking immediately

## Target Bands
Default target bands:

1. `pair_coverage_soft_min = 0.80`
2. `pair_coverage_good = 0.90`
3. `share_skew_target_low = 1.05`
4. `share_skew_target_high = 1.20`
5. `share_skew_soft_cap = 1.30`
6. `share_skew_hard_cap = 1.40`
7. `favorite_cost_frac_low = 0.60`
8. `favorite_cost_frac_high = 0.67`
9. `underdog_share_frac_low = 0.51`
10. `underdog_share_frac_high = 0.60`
11. `vwap_sum_great = 0.94`
12. `vwap_sum_good = 0.97`
13. `vwap_sum_stop_overlay = 1.00`

Derived implementation targets:
1. `target_pair_coverage = 0.90` in healthy normal flow
2. `target_share_skew_ratio = 1.10` as a neutral center inside the good band
3. `target_favorite_cost_fraction = 0.635` as neutral center
4. `target_underdog_share_fraction = 0.555` as neutral center

## Phase Budgets
The mode must keep separate phase budgets, not just a total window budget.

Default budget slices:
1. `SeedBothSides = 10-15%`
2. `EarlyBuild = 15-20%`
3. `MainAccumulation = 45-55%`
4. `FinishShape = 15-20%`
5. `FreezeRepairOnly reserve = 5-10%`

Decision rule:
1. each phase can use only its own budget slice plus any explicitly allowed carry-forward
2. `FreezeRepairOnly` reserve cannot be spent during earlier phases
3. aggressive size-up must come from `MainAccumulation` budget only

## Allowed Actions by Phase

### DiscoveryArm
Allowed:
1. subscribe to market/user streams
2. warm market metadata
3. resolve token IDs
4. initialize state

Not allowed:
1. any new trading action before the market is armed and data is fresh

### SeedBothSides (0-30s)
Goal:
1. establish both sides immediately
2. avoid one-sided starts

Allowed actions:
1. `seed_up_small`
2. `seed_down_small`
3. `seed_both_small_batch`
4. `EntryRepair`

Preferred clip sizes:
1. `5`
2. `10`
3. `20`

Constraints:
1. no underdog overlay
2. no aggressive `40/80`
3. do not bias hard yet

### EarlyBuild (30-60s)
Goal:
1. maintain both sides
2. start favorite-dollar bias
3. start underdog-share bias

Allowed actions:
1. `buy_favorite_small`
2. `buy_underdog_small`
3. `buy_lighter_share_side_small`
4. `EntryRepair`
5. `ShapeRepair`

Constraints:
1. no large underdog overlay
2. no 80-share clips
3. focus on reaching `pair_coverage >= 0.80`

### MainAccumulation (60-180s)
Goal:
1. build most inventory here
2. push toward the target settlement shape

Allowed actions:
1. `favorite_sizeup_small_or_medium`
2. `underdog_overlay_small`
3. `buy_lighter_share_side`
4. `EntryRepair`
5. `ShapeRepair`
6. selective `favorite_sizeup_large`

Constraints:
1. `40/80` only if:
   - `pair_coverage >= 0.85`
   - `market_snapshot_vwap_sum < 0.97`
   - budget slice available
2. underdog overlay only if:
   - `pair_coverage >= 0.90`
   - `share_skew_ratio < 1.15`
   - `market_snapshot_vwap_sum < 0.97`
   - stretch condition supports it

### FinishShape (180-240s)
Goal:
1. stop sloppy growth
2. move toward target final shape

Allowed actions:
1. `favorite_repair_small`
2. `buy_lighter_share_side_small`
3. `micro_underdog_overlay`
4. `ShapeRepair`

Constraints:
1. no new 80-share clips
2. no new overlay if `share_skew_ratio >= 1.20`
3. if `market_snapshot_vwap_sum > 1.00`, stop optionality and do repair only

### FreezeRepairOnly (240-300s)
Goal:
1. preserve final shape
2. avoid making the final book worse

Allowed actions:
1. `micro_repair`
2. `hold`
3. `RiskExitOnly` only if hard invalid state occurs

Constraints:
1. no new overlay
2. no new 40/80 size-up
3. only minimal repair if target violations are still material

### SettlementRedeem
Goal:
1. settle and redeem winning inventory

Allowed actions:
1. wait for resolution
2. identify winner
3. redeem winning tokens
4. compute realized settlement PnL

## Clip Ladder by Action Type

### Small maker clips
Default sizes:
1. `5`
2. `10`
3. `20`
4. `25`

Use for:
1. seed
2. normal both-side building
3. underdog overlay
4. late repair

### Medium clips
1. `40`

Use for:
1. favorite-side size-up
2. stronger floor reinforcement
3. coverage repair when shape is already healthy enough

### Large clips
1. `80`

Use only for:
1. favorite-side size-up
2. `MainAccumulation`
3. `pair_coverage >= 0.85`
4. `market_snapshot_vwap_sum < 0.97`
5. remaining phase budget exists

Do not use `80` for routine underdog overlay.

## Action Families

### SeedBothSides actions
1. `seed_up_small`
2. `seed_down_small`
3. `seed_both_small_batch`

Primary objective:
1. reach both-side participation

### Favorite-side size-up
Purpose:
1. improve floor
2. increase favorite cost fraction

Typical clips:
1. `20`
2. `25`
3. `40`
4. `80`

### Underdog overlay
Purpose:
1. increase convexity
2. increase underdog share fraction

Typical clips:
1. `5`
2. `10`
3. `20`
4. sometimes `25`

### EntryRepair
Purpose:
1. restore both-side participation after arming/fill asymmetry

It does not try to flatten the book.

### ShapeRepair
Purpose:
1. restore target final shape
2. improve coverage
3. correct excessive skew
4. repair bad cost/share fractions

It does not optimize toward equal shares unless the target shape itself demands it.

## Action Scoring
Action choice must be driven by explicit target-state improvement, not loosely by heuristics.

For every candidate action, compute:
1. projected `pair_coverage`
2. projected `share_skew_ratio`
3. projected `favorite_cost_fraction`
4. projected `underdog_share_fraction`
5. projected `inventory_vwap_sum`
6. projected `market_snapshot_vwap_sum` cost of the action
7. budget usage
8. execution type penalty

### Score components
For action `a`, define:

1. `coverage_improvement(a)`
2. `favorite_cost_improvement(a)`
3. `underdog_share_improvement(a)`
4. `skew_penalty(a)`
5. `vwap_penalty(a)`
6. `budget_penalty(a)`
7. `aggression_penalty(a)`

### Required score shape
Suggested implementation form:

```text
score(a) =
  w_coverage * coverage_improvement(a)
  + w_favorite_cost * favorite_cost_improvement(a)
  + w_underdog_share * underdog_share_improvement(a)
  - w_skew * skew_penalty(a)
  - w_vwap * vwap_penalty(a)
  - w_budget * budget_penalty(a)
  - w_aggression * aggression_penalty(a)
```

Required priority rule:
1. no action may be selected if it breaches `share_skew_hard_cap`
2. no action may be selected if it consumes forbidden phase budget
3. no overlay action may be selected if `market_snapshot_vwap_sum > vwap_sum_stop_overlay`

### Execution-type penalties
Use this priority bias:
1. maker small clip
2. maker medium clip
3. maker large favorite size-up
4. aggressive favorite size-up
5. `RiskExitOnly`

The new mode remains maker-first in normal flow.

## Conflict Resolution
When targets conflict, use this precedence:

1. `pair_coverage_soft_min` breach
2. `share_skew_hard_cap` breach
3. `pair_coverage_good` target
4. `share_skew_target` band
5. `favorite_cost_fraction` target
6. `underdog_share_fraction` target
7. optional overlay improvement

Interpretation:
1. if coverage is weak, repair coverage first
2. if skew is too large, reduce skew pressure before adding overlay
3. cost/share fraction shaping comes after the book is structurally safe enough

## Maker vs Aggressive Execution Rules

### Default normal flow
Use:
1. maker
2. post-only
3. GTC / GTD

### Aggressive execution allowed only for favorite-side size-up
Allowed only if:
1. phase is `MainAccumulation`
2. `pair_coverage >= 0.85`
3. `market_snapshot_vwap_sum < 0.97`
4. size is `40` or `80`
5. budget remains in `MainAccumulation`

Aggressive execution is not allowed for:
1. routine underdog overlay
2. ordinary `ShapeRepair`
3. ordinary `EntryRepair`

### Emergency / pathological execution
`RiskExitOnly` remains available for:
1. feed failure
2. hard budget breach
3. hard skew breach
4. pathological market conditions

## EntryRepair Decision Rules
Use `EntryRepair` when:
1. one side has not armed
2. one side rejected or no-oid
3. one side filled before the other was live
4. startup asymmetry exists

Objective:
1. restore both-side participation
2. not equal shares

Preferred action order:
1. maker on missing side
2. maker refresh on missing side
3. if startup has become pathological, route to `RiskExitOnly`

`EntryRepair` must not:
1. optimize for flat inventory
2. optimize for favorite cost fraction first

## ShapeRepair Decision Rules
Use `ShapeRepair` when:
1. `pair_coverage < pair_coverage_soft_min`
2. `share_skew_ratio > share_skew_soft_cap`
3. `favorite_cost_fraction` far outside target
4. `underdog_share_fraction` far outside target
5. `market_snapshot_vwap_sum` is too poor for additional overlay but the current shape is not acceptable

Objective:
1. repair toward target final shape

Preferred action order depends on the failing target:

### If coverage is weak
1. buy lighter-share side
2. prefer maker
3. ignore optional overlay

### If favorite cost fraction too low
1. favorite-side size-up
2. choose clip size by phase and budget

### If underdog share fraction too low and skew still safe
1. small underdog overlay

### If skew too high
1. stop overlay
2. buy lighter-share side only
3. route to `RiskExitOnly` if hard cap is exceeded and cannot be repaired safely

## Stretch / Mean-Reversion Overlay
Enable underdog overlay only when all are true:
1. `pair_coverage >= 0.90`
2. `share_skew_ratio < 1.15`
3. `market_snapshot_vwap_sum < 0.97`
4. budget remains
5. stretch condition is satisfied

Suggested stretch triggers:
1. if `binance_delta_from_start > 0` and `RSI >= 52` and underdog is `Down` -> allow small `Down` overlay
2. if `binance_delta_from_start < 0` and `RSI <= 48` and underdog is `Up` -> allow small `Up` overlay

This is a mild overlay only.

## `vwap_sum` Regime Policy
Use `market_snapshot_vwap_sum` for the next action decision.
Use `inventory_vwap_sum` for evaluating existing book quality.

### Regimes
1. `great`: `< 0.94`
2. `good`: `0.94 - 0.97`
3. `caution`: `0.97 - 1.00`
4. `stop_overlay`: `> 1.00`

Policy:
1. `great/good`
   - normal shaping allowed
2. `caution`
   - no new aggressive overlay
   - repair and favorite-size-up only if target deltas justify it
3. `stop_overlay`
   - no new optionality
   - repair only

## SettlementRedeem Rules
After market resolution:
1. identify winner
2. winning shares redeem to `$1.00`
3. losing shares are worth `$0`
4. compute realized settlement PnL:

```text
realized_pnl = winning_shares - total_cost - fees
```

This mode is expected to hold inventory to settlement in the normal path.

## Metrics Required
The new mode must emit, per market:
1. `pair_coverage` distribution
2. `share_skew_ratio` distribution
3. `favorite_cost_fraction`
4. `underdog_share_fraction`
5. `inventory_vwap_sum`
6. `market_snapshot_vwap_sum`
7. clip-size mix by phase
8. maker/taker cost mix
9. settlement PnL
10. windows with `share_skew_ratio > 1.30`
11. windows with `pair_coverage < 0.80`
12. windows with `market_snapshot_vwap_sum > 1.00`

## Canary Rollback Conditions
Rollback Sprint 3 canary if any of these are materially worse than baseline:

1. median `pair_coverage < 0.80`
2. too many windows with `share_skew_ratio > 1.30`
3. too many windows with `market_snapshot_vwap_sum > 1.00`
4. settlement ROI worse than `MAKER_SKEW_ARB` baseline
5. favorite cost fraction misses target range too often
6. underdog share fraction misses target range too often

## Implementation Order
Coding order should be:

1. metrics first
2. favorite/underdog hysteresis
3. time-phase controller
4. `SettlementRedeem`
5. target-state model
6. `EntryRepair`
7. `ShapeRepair`
8. favorite-side size-up
9. underdog overlay
10. stretch logic last

## Done Means Done
Another model should treat this mode as implementation-ready only when:

1. `EXEC_MODE=SETTLEMENT_SHAPER` dispatch exists
2. per-phase state and budgets are live
3. favorite/underdog hysteresis is live
4. `inventory_vwap_sum` and `market_snapshot_vwap_sum` are separate and used correctly
5. `EntryRepair` and `ShapeRepair` are distinct in code and logs
6. favorite-side size-up and underdog overlay are distinct in code and metrics
7. normal path reaches `SettlementRedeem`
8. canary metrics emit all required fingerprint fields
9. rollback conditions can be evaluated from emitted metrics
