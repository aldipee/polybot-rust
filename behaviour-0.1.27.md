# CURRENT BEHAVIOUR

## Version 0.1.27

Scope: Sprint 3 only  
Mode: `EXEC_MODE=SETTLEMENT_SHAPER`  
Date: 2026-03-10

This file is not a design target.
It is a concrete runtime behaviour note for the current `SETTLEMENT_SHAPER` canary.

---

## Executive Summary

The current Sprint 3 implementation is not yet behaving like the intended settlement-shaping wallet.

What works now:

1. the bot routes correctly into `SETTLEMENT_SHAPER`
2. startup auto-slug rollover is fixed
3. flat startup can seed both sides
4. one-leg startup fills can trigger `EntryRepair`
5. post-expiry it waits for resolution instead of using the old pre-expiry flatten path

What is still wrong:

1. the bot often trades only at the opening
2. after the opening seed, it frequently falls into repeated `ShapeRepair` hold loops
3. it is not yet acting like a high-frequency inventory builder
4. it is not yet reliably building toward the Sprint 3 final shape:
   - more dollars on the favorite
   - more shares on the underdog
   - high pair coverage
   - mild skew
   - hold to settlement

The current canary behaves more like:

1. seed both sides
2. repair startup asymmetry if one leg fills first
3. evaluate target drift against fixed target centers
4. block most follow-on shaping because the next trade is either:
   - below the 5-share maker minimum
   - blocked by hard skew
   - or blocked by `stop_overlay` / `repair_only`

That is a valid canary for routing and ownership.
It is not yet a valid Sprint 3 behavioural match.

---

## Current Observed Pattern

Across the latest live Sprint 3 runs, the dominant pattern is:

1. `DiscoveryArm`
2. seed both sides with a single small maker pair
3. if one leg fills first, run `EntryRepair` on the missing side
4. once both sides exist, switch to `ShapeRepair`
5. then hold for the rest of the market because the next shaping action is blocked

In practical terms:

1. the bot is proving it can enter the market
2. it is not proving it can keep accumulating into the intended final wallet shape

---

## Concrete Runtime Evidence

### Case A: balanced-ish market, seed then sub-min hold

Observed market shape:

1. YES around `0.48`
2. NO around `0.51`

Observed behaviour:

1. seed both sides fired
2. one side filled first, then `EntryRepair` completed the missing side
3. final live inventory became `qYES=5`, `qNO=5`
4. after that, `ShapeRepair` repeatedly logged:
   - `reason=sub_min_best_action action=hold`
   - raw requested shaping clips around `3.9 -> 1.6`

Interpretation:

1. with maker minimum `5` shares, the controller could not place the next shaping trade
2. the remaining shape drift was real, but it was below the executable maker lot size
3. the bot therefore held for the rest of the market

This means:

1. the bot entered correctly
2. the bot did not continue building inventory after the initial `5/5`

### Case B: asymmetric market, seed then hard-skew block

Observed market shape:

1. YES around `0.85`
2. NO around `0.14`

Observed behaviour:

1. seed both sides fired with `5/5`
2. one side filled first, then `EntryRepair` completed the missing side
3. final live inventory again became `qYES=5`, `qNO=5`
4. cost split became heavily favorite-weighted:
   - `favorite_cost_fraction=0.859`
   - `underdog_share_fraction=0.500`
5. after that, `ShapeRepair` repeatedly logged:
   - `reason=hard_skew_breach side=NO trigger=favorite_cost_drift`

Interpretation:

1. the controller wanted to buy more underdog shares to reduce favorite-cost drift
2. but the next underdog buy would push the projected book through the hard skew guard
3. so the controller blocked itself and held

This means:

1. the bot entered correctly
2. it then got stuck trying to repair toward a target it could not safely reach

---

## Why Increasing `MAX_TOTAL_COST` To 120 Did Not Fix It

Increasing total budget from `60` to `120` did not materially change the behaviour because the live blocker was not only the total budget.

### 1. Seed size did not scale with the higher budget

The runtime config still showed:

1. `min_shares=5`
2. `clip_shares=5`

So the opening seed stayed:

1. `clip=5`
2. one maker order per side

That means higher total budget did not automatically produce a larger opening inventory.

### 2. Fixed target centers are not feasibility-aware

The controller still evaluates drift using fixed target centers:

1. `target_favorite_cost_fraction = 0.635`
2. `target_underdog_share_fraction = 0.555`

Those targets are treated as active drifts even when:

1. current side prices are extremely asymmetric
2. maker lot size is fixed at `5`
3. hard skew cap is fixed at `1.40`
4. the next valid 5-share action cannot move the book toward those centers without breaking another hard rule

So the controller keeps seeing `favorite_cost_drift`, even when the next legal trade is not actually viable.

### 3. `stop_overlay` disables the normal optional builder paths

In the same runs, the logs repeatedly showed:

1. `market_regime=stop_overlay`
2. `optionality=repair_only`

That means:

1. favorite-size-up stayed off
2. underdog-overlay stayed off
3. only repair-style actions remained eligible

If the remaining drift is not repairable with a legal 5-share maker order, the bot just holds.

---

## Main Mismatch Against Sprint 3

Sprint 3 does not describe a "seed once, then mostly hold" wallet.

Sprint 3 intends a mode that:

1. holds both sides
2. spends more dollars on the favorite
3. ends with more shares on the underdog
4. keeps pair coverage high
5. keeps skew mild
6. keeps shaping through the market
7. then holds to settlement

The current canary is not yet doing item 6 reliably.

The current implementation is therefore:

1. correct as a mode boundary and early controller canary
2. incorrect as a final Sprint 3 behavioural match

---

## Exact Current Failure Modes

### Failure Mode 1: opening inventory is too small

Current behaviour:

1. the mode often starts from `5/5`
2. the next Sprint 3 shaping action then needs to clear a 5-share maker lot boundary
3. many target drifts after `5/5` are smaller than that

Observed effect:

1. `sub_min_best_action action=hold`

Result:

1. the bot seeds correctly
2. then it cannot legally place the next maker shaping order

### Failure Mode 2: target centers are treated as mandatory even when unreachable

Current behaviour:

1. `ShapeRepair` continues to measure drift against fixed centers
2. it does not first ask whether those centers are reachable under:
   - current price ratio
   - 5-share lot size
   - current budget
   - hard skew cap

Observed effect:

1. persistent `favorite_cost_drift`
2. repeated blocked repair attempts

Result:

1. the controller keeps wanting a trade
2. then rejects the only available next trade

### Failure Mode 3: hard-skew check blocks the only available next step

Current behaviour:

1. in some markets the only obvious way to reduce favorite-cost drift is to buy more underdog
2. but doing so from a tiny `5/5` book in 5-share steps can immediately jump the share ratio too far

Observed effect:

1. repeated `reason=hard_skew_breach`

Result:

1. the controller has no legal follow-up path
2. the run stalls after the opening seed

### Failure Mode 4: `stop_overlay` removes non-repair accumulation paths

Current behaviour:

1. when `market_snapshot_vwap_sum > 1.00`, the mode goes to `repair_only`
2. that shuts down optional builder paths

Observed effect:

1. no favorite-size-up
2. no underdog-overlay
3. only repair logic remains

Result:

1. if repair cannot act, nothing else acts

---

## Practical Interpretation

Right now the Sprint 3 canary should be interpreted as:

1. a proof that the new mode boundary works
2. a proof that settlement-shaper can seed and repair startup asymmetry
3. a proof that it can carry inventory to settlement ownership

It should not be interpreted as:

1. proof that the Sprint 3 inventory-building policy is complete
2. proof that the bot can actively shape through the full market
3. proof that the target final book logic is aligned with the wallet

---

## What Must Change Next

These are the concrete behaviour fixes still required for Sprint 3 alignment.

### 1. Feasible target envelope, not fixed unconditional center

The controller must stop treating `0.635 / 0.555` as always-live targets.

It needs a feasible target envelope derived from:

1. current favorite / underdog prices
2. 5-share maker lot size
3. available budget
4. hard skew cap

If the fixed center is infeasible, the controller should target the best feasible nearby shape instead.

### 2. Seed sizing must scale beyond `5/5`

Raising total budget alone is not enough if opening size remains fixed at `5`.

The mode needs to size the opening two-sided seed so that:

1. the next legal 5-share actions are still available
2. the initial book is already inside a reachable shaping region
3. the bot does not start from a book that is too tiny to shape further

### 3. No permanent drift reason when no legal next trade exists

If the next legal 5-share trade is impossible under the skew cap, the controller should not keep behaving like:

1. "target still demands action"
2. "but every action is blocked"

It needs an explicit state like:

1. feasible target reached
2. no legal next shaping block
3. pair resting until conditions change

### 4. PairResting should own normal accumulation once the book is healthy

After `5/5` with both sides live, normal shaping should not look like a permanent repair loop.

The normal path needs to become:

1. healthy two-sided book
2. `PairResting`
3. phase-appropriate size-up / shaping actions
4. back to resting

not:

1. healthy two-sided book
2. `ShapeRepair`
3. repeated hold logs forever

---

## Current Verdict

For Sprint 3, version `0.1.27` should be described as:

Status: `PARTIAL`

More precise description:

1. mode boundary is implemented
2. startup bootstrap is implemented
3. startup asymmetry repair is implemented
4. hold-to-resolution ownership is implemented
5. continuous Sprint 3 shaping is not implemented correctly yet

The most honest behavioural summary is:

1. the bot can now enter the market under Sprint 3
2. but it still does not behave like the intended settlement-shaping inventory builder
3. opening-only trading is still a real current limitation

---

## Short Operator Read

If the bot currently:

1. seeds at the open
2. repairs one missing leg
3. then stops trading for the rest of the market

that is not operator error.

That is the current Sprint 3 behaviour.

The remaining problem is inside the controller:

1. fixed targets
2. tiny opening size
3. 5-share maker floor
4. hard-skew block
5. `repair_only` gating

Until those are changed, increasing total budget by itself will not make the mode behave like the final Sprint 3 wallet.
