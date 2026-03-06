# TARGET GOAL: Pair-First, Merge-First, Maker-First Roadmap

Date: 2026-03-06
Status: Active target roadmap
Scope: Bitcoin Up or Down - 5 Minutes (Chainlink BTC/USD resolution)
Target wallet reference: `0x8e9cd5ec7a26d602b63b4bc4c193febb83c8ed64`

---

## Purpose

This file defines the execution roadmap for matching the observed `0x8e9c...` wallet behavior.

The target wallet is not a pure spread-capture bot. From the analyzed sample:

1. it bought both outcomes in every sampled window
2. it showed no sells in the sample
3. it built a bounded-floor, skewed-payoff profile
4. it behaved like a pair-first, merge/recovery-aware builder
5. maker execution appears to be the default path
6. directional stretch is secondary, not primary

So the goal is not:

- "prove pure maker spread capture first, then add everything later"

The real goal is:

- build both sides as maker
- keep downside bounded
- repair one-leg fills maker-first
- allow mild payoff skew only after the base pair builder is reliable
- use taker only for explicit risk exits

This roadmap is milestone-gated:

1. do not move to the next step until the current step passes
2. each step must be validated with explicit metrics
3. each step must be explainable from logs

---

## Strategy Summary

The target strategy inferred from the wallet is:

1. build both sides in the same 5-minute window
2. use maker orders as the normal source of edge
3. keep a downside floor by maintaining both-side exposure
4. shape payoff by adding more shares on the cheaper side
5. recover mismatches immediately and maker-first
6. settle / merge / redeem later rather than depending on frequent sells

Core operating priority:

1. risk exit
2. merge recovery
3. pair base builder
4. skew overlay
5. stretch overlay

### Execution State Ownership

The live engine should have explicit high-level ownership states:

1. `PAIR_BASE`
2. `RECOVERY`
3. `SKEW`
4. `RISK_EXIT`

Rules:

1. `RECOVERY` preempts `PAIR_BASE`
2. `SKEW` cannot open new risk while `RECOVERY` is active
3. `RISK_EXIT` preempts everything
4. exactly one missing-leg recovery quote may exist during `RECOVERY`

Taker policy:

1. taker is not part of normal flow
2. taker is allowed only for:
   - near-expiry unresolved imbalance
   - hard max-loss breach
   - feed / venue failure while carrying imbalance

---

## Important Correction to the Roadmap

The previous roadmap treated Step 1 as a pure maker spread-capture proof.

That is wrong for this wallet.

For this target, the first real proof is:

1. can the bot build both sides as maker
2. can it keep downside bounded
3. can it repair missing-leg fills without taker

Mismatch recovery is therefore not a late feature. It is base-engine infrastructure.

Also:

1. "pair base" is not the same thing as "pair arb"
2. Step 1 should allow pair base and maker recovery
3. Step 1 can still keep aggressive pair-arb triggers off

---

## Current Code Position

Current code still centers on `MAKER_SKEW_ARB`.

The recent `v0.1.24` changes are useful, but they belong to infrastructure hardening, not the true target Step 1.

Specifically:

1. quote-only fallback under `MAKER_SKEW_ENABLED=false` is a valid diagnostic tool
2. it is not the final Step 1 for this wallet
3. it should be treated as Milestone 0 support, not the actual pair-builder milestone

---

## Milestone 0: Make the Engine Separable and Explainable

### Objective

Stop patching one mixed execution loop.

Separate the current engine internally so the following behaviors can be owned independently:

1. `quote_engine`
2. `pair_base_engine`
3. `recovery_engine`
4. `skew_engine`
5. `stretch_overlay`

### Required outcome

The bot must support:

1. maker-only infrastructure without hidden kill-switch behavior
2. explicit idle reasons
3. explicit budget visibility
4. explicit ownership of pair, recovery, skew, and risk paths

### Notes

`v0.1.24` infrastructure work belongs here:

1. internal phase separation
2. quote-only fallback
3. quote-only-native idle reasons

### Pass condition

The code is modular enough that Step 1 can run as a pair-base builder with maker-first recovery, without fighting the mixed skew loop.

---

## Step 1: Maker-Only Pair Builder With Maker-Only Recovery

### Objective

Prove the base strategy:

1. quote both outcomes as maker
2. build both sides in the same window
3. keep downside bounded
4. repair one-leg fills maker-first
5. avoid taker in normal flow

This is the first real proof for the target wallet.

### Step 1 must do

1. quote both sides as maker using post-only resting orders
2. participate on both outcomes in the same window
3. maintain a pair-style inventory rather than a one-sided directional position
4. treat mismatch recovery as the highest-priority normal path
5. keep exactly one live missing-leg recovery quote during recovery
6. block new accumulation while recovery is open

### Step 1 must not do

1. no directional stretch bias
2. no aggressive taker rescue
3. no one-sided bootstrap as the normal strategy path
4. no ladder accumulation unrelated to pair build / merge repair
5. no hidden budget stop

### Step 1 configuration intent

This step should conceptually be:

```env
PAIR_BASE_ENABLED=true
PAIR_RECOVERY_ENABLED=true
MAKER_SKEW_ENABLED=false
MAKER_ARB_ENABLED=false
MAKER_STRETCH_BIAS_ENABLED=false
TAKER_ALLOWED=false
```

Important:

1. `PAIR_BASE` and `PAIR_ARB` are different concepts
2. Step 1 allows pair base and maker recovery
3. Step 1 keeps aggressive pair-arb behavior off
4. no Step 1 result is valid unless it is evaluated fee-net

### Step 1 metrics

Measure over 20+ markets:

1. both-side participation rate
2. pair coverage ratio:
   - `min(shares_up, shares_down) / max(shares_up, shares_down)`
3. worst-case PnL if Up wins
4. worst-case PnL if Down wins
5. median downside as % of window budget
6. hard downside cap as % of window budget
7. maker recovery success rate
8. taker count
9. actual settlement / merge PnL
10. merge success rate
11. residual unmerged inventory after resolution
12. time to flat after resolution
13. time to redeploy capital
14. settlement PnL net of fees

### Step 1 target thresholds

Use these as initial thresholds:

1. both-side participation: at least `90%` of windows
2. median pair coverage: at least `0.60`
3. median downside / window budget: no worse than about `-15%`
4. hard downside / window budget: no worse than about `-50%`
5. taker count: `0`
6. maker recovery success: high and explicit in logs

### Step 1 success criteria

1. no normal-path `10/0` or `0/10` bootstrap
2. mismatches are repaired maker-first
3. new accumulation does not continue while recovery is open
4. both-side participation is high
5. downside is bounded and measurable
6. normal taker usage is zero

### Advance rule

Do not proceed unless the maker-only pair builder can:

1. fill both sides reliably
2. keep downside bounded
3. repair mismatches without hidden taker usage

This is the actual gate for the target wallet.

---

## Step 2: Gentle Skew Around a Protected Floor

### Objective

Once the pair-base builder and maker-only recovery are stable, add mild skew to shape payoff more like the target wallet.

### Required config

```env
MAKER_SKEW_ENABLED=true
MAKER_SKEW_TARGET_RATIO=1.2
MAKER_SKEW_MAX_RATIO=2.2
```

### What this step must do

1. preserve both-side participation
2. bias incremental shares toward the cheaper side
3. maintain a protected downside floor
4. keep maker-first behavior
5. keep taker out of normal flow

### Expected behavioral shape

1. cost split remains roughly balanced
2. share split becomes mildly skewed
3. upside becomes materially larger than downside
4. downside remains bounded

### Metrics

1. skew ratio distribution
2. cost split by side
3. worst-case downside per window
4. best-case upside per window
5. both-side participation per window
6. pair coverage ratio after skew
7. taker count during normal operation
8. fee-net worst-case PnL
9. fee-net best-case PnL
10. fee-net pair cost

### Step 2 success criteria

1. skew is mild-to-moderate, not runaway
2. both sides still get filled
3. downside remains bounded
4. maker-only normal flow is preserved
5. no Step 2 result is valid unless it is evaluated fee-net

---

## Step 3: Advanced Mismatch, Expiry, and Failure Recovery

### Objective

Handle edge-case recovery once the normal pair-base and gentle-skew flows are already stable.

This is not the first appearance of mismatch recovery.
This step is for advanced recovery and failure-mode handling.

### This step covers

1. expiry-time repair
2. emergency-only taker rules
3. feed/venue failure while carrying imbalance
4. hard max-loss fallback
5. late-fill and stale-order edge cases

### Required behavior

1. heavy-leg orders are canceled immediately on mismatch
2. exactly one missing-leg quote is live
3. no new accumulation while recovery is open
4. taker is only allowed under explicit emergency policy

### Emergency taker semantics

When `RISK_EXIT` is active and taker is permitted:

1. emergency taker `BUY` must size by dollars
2. emergency taker `SELL` must size by shares
3. every emergency taker action must log:
   - trigger reason
   - intended imbalance reduction
   - actual fill result
   - resulting inventory state

### Metrics

1. mismatch-to-flat success rate
2. recovery duration
3. post-recovery overshoot count
4. number of emergency taker exits
5. late-fill damage frequency
6. max inventory imbalance during recovery

### Step 3 success criteria

1. recovery remains controlled under stress
2. taker remains rare and policy-bound
3. expiry and outage behavior are explicit, measurable, and bounded

---

## Step 4: Stretch Bias Overlay

### Objective

Add directional stretch only after the maker pair builder, maker recovery, and gentle skew are already stable.

Stretch is not the core strategy. It is a small overlay.

### Suggested config

```env
MAKER_STRETCH_BIAS_ENABLED=true
MAKER_STRETCH_RSI_OVERSOLD=35
MAKER_STRETCH_RSI_OVERBOUGHT=65
```

### Rules

1. stretch must not replace the pair-base engine
2. stretch must not replace recovery priority
3. stretch should consume only residual skew budget
4. stretch must use the correct market resolution context

### Step 4 success criteria

1. stretch improves skew quality without breaking pair coverage
2. recovery behavior remains unchanged in priority
3. taker use does not expand beyond explicit risk policy

---

## Polymarket Execution Requirements

The target implementation must respect venue rules:

1. maker-only normal flow uses post-only resting orders
2. post-only requires `GTC` or `GTD`
3. post-only must not be combined with `FAK` or `FOK`
4. the orderbook should be tracked from the real-time market channel, not slow polling
5. user fills and order status should be tracked from the real-time user channel
6. fee treatment must not be assumed blindly; market fee behavior must be logged and observed

Operationally, always log:

1. `fees_enabled`
2. estimated taker fee at current price
3. maker rebate eligibility if available
4. fee-net pair cost
5. fee-net worst-case PnL
6. fee-net best-case PnL
7. maker rebate assumed or realized

### Fee-net validity rule

No Step 1 or Step 2 result is valid unless it is evaluated fee-net.

---

## Acceptance Criteria for the Whole Roadmap

1. no `10/0` or `0/10` bootstrap as the normal path
2. no uncontrolled directional accumulation outside approved skew budget or recovery
3. every imbalance maps to one live missing leg
4. taker count is near zero and only from explicit risk exits
5. no hidden budget stop
6. balanced state returns to pair entry, not skew accumulation

---

## Implementation Order

1. finish Milestone 0 separation and visibility
2. wire Step 1 maker-only pair base
3. wire Step 1 maker-only recovery
4. disable normal skew / ladder behavior in Step 1 path
5. add Step 2 gentle skew
6. add Step 3 strict emergency taker rules and expiry handling
7. add Step 4 stretch overlay
8. replay recent bad logs against each milestone before advancing

---

## Bottom Line

The corrected roadmap is:

1. Milestone 0: infrastructure and explainability
2. Step 1: maker-only pair builder with maker-only recovery
3. Step 2: gentle skew around a protected floor
4. Step 3: advanced recovery / expiry / failure handling
5. Step 4: stretch bias overlay

This is the roadmap that matches the observed wallet behavior most closely.
