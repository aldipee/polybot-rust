# TARGET GOAL: Pair-First, Merge-First, Maker-Only Roadmap

Date: 2026-03-06
Status: Active target roadmap
Scope: Bitcoin Up or Down - 5 Minutes (Chainlink BTC/USD resolution)
Target wallet reference: `0x8e9cd5ec7a26d602b63b4bc4c193febb83c8ed64`

---

## Purpose

This file is the execution roadmap for building the bot toward the observed wallet behavior:

1. trade both outcomes in every window
2. rely on maker fills as the normal edge source
3. use skew as a secondary payoff-shaping tool, not the first proof of edge
4. resolve mismatches via maker-first recovery
5. only use taker in a few explicit emergency cases

This roadmap is milestone-gated:

- Do not move to the next step until the current step passes.
- Each step must be validated on live or replayed windows with explicit metrics.

---

## Strategy Summary

The target behavior inferred from the wallet is:

1. Build both sides in every window.
2. Capture spread and cheap inventory with maker orders whenever possible.
3. Shape payoff by buying more shares on the cheaper side while maintaining a downside floor.
4. Trigger pair buys when both sides can be bought cheaply enough.
5. Treat mismatch recovery as a first-class workflow, not an afterthought.
6. Use stretch bias only as a later overlay.

Core operating principle:

- Maker edge first
- Pair/merge control second
- Skew third
- Taker only for explicit risk exits

---

## Current Constraint

The current codebase cannot execute Step 1 correctly with:

```env
MAKER_SKEW_ENABLED=false
```

because that setting currently acts as a kill switch for the whole `MAKER_SKEW_ARB` loop.

So before Step 1 can be measured honestly, the engine must support a maker-only quoting path that still runs when directional skewing is disabled.

That is why this roadmap starts with a prerequisite milestone.

---

## Milestone 0: Make Step 1 Executable

### Objective

Separate the current mixed execution loop so the bot can run a maker-only base engine without directional skew, ladder drift, or hidden shutdown behavior.

### Required outcome

The bot must support a mode or internal phase configuration where:

1. maker quoting still runs
2. directional skew logic does not run
3. pair-arb does not run
4. stretch bias does not run
5. the bot can quote both sides and stay alive

### Non-negotiable requirements

1. Do not delete the current `MAKER_SKEW_ARB` path.
2. Separate internal logic into distinct phases/modules.
3. `MAKER_SKEW_ENABLED=false` must no longer imply "bot does nothing" if Step 1 mode is selected.
4. Logging must explicitly say why the bot is idle.

### Pass condition

Step 1 can be run with a real maker-only engine path and without relying on the current skew loop as a kill-switch hack.

---

## Step 1: Maker-Only Mode (Prove the Engine)

### Objective

Prove that the bot can make money or stay close to flat by capturing bid-ask spread with post-only maker fills, without skew, arb, or directional overlays.

### Required config

```env
MAKER_SKEW_ENABLED=false
MAKER_ARB_ENABLED=false
MAKER_STRETCH_BIAS_ENABLED=false
```

### What this step must do

1. Quote both outcomes as maker.
2. Capture spread using post-only fills.
3. Keep net inventory controlled over time.
4. Avoid taker orders entirely.

### What this step must not do

1. No skew logic
2. No pair-arb logic
3. No stretch bias
4. No directional accumulation beyond two-sided maker inventory building
5. No taker usage

### Metrics to measure over 20+ markets

1. Maker fill rate
2. Effective trading cost versus book midpoint
3. Net inventory over time
4. Order cancel rate
5. Windows with both-side fills
6. Taker count
7. Net P&L before settlement

### Step 1 success criteria

1. Maker fills happen consistently.
2. Trading cost is near the midpoint, not systematically worse.
3. Inventory drift is bounded.
4. Taker count is zero.
5. Spread capture is neutral to positive over the sample.

### Advance rule

If Step 1 loses money consistently, do not proceed to Step 2.

Reason:

If pure maker spread capture does not work, adding skew later only layers risk on top of a broken execution core.

---

## Step 2: Turn On Hedging and Balanced Accumulation

### Objective

Enable gentle skew so the bot still builds both sides, but starts shaping payoff asymmetrically in the same direction as the target wallet.

### Required config

```env
MAKER_SKEW_ENABLED=true
MAKER_SKEW_TARGET_RATIO=1.2
MAKER_SKEW_MAX_RATIO=2.2
```

### What this step must do

1. Accumulate both outcomes across the window.
2. Bias additional shares toward the cheaper side.
3. Keep downside bounded via hedge-side adds.
4. Preserve maker-first behavior.
5. Avoid taker orders during normal accumulation.

### Expected behavioral shape

1. Cost split remains near balanced.
2. Share split becomes mildly skewed toward the underdog.
3. Worst-case loss is controlled.
4. Upside is materially larger than downside.

### Metrics to measure over 20+ markets

1. Skew ratio distribution
2. Cost split by side
3. Worst-case downside per window
4. Best-case upside per window
5. Budget utilization
6. Both-side participation per window
7. Taker count during normal operation

### Step 2 success criteria

1. The bot shows small-to-moderate skew, not runaway one-sided accumulation.
2. Both sides still get filled in normal windows.
3. Net payoff profile becomes asymmetric, but downside remains bounded.
4. Taker stays at zero for normal flow.

---

## Step 3: Add Mismatch Logic Above Normal Flow

### Objective

When one leg fills and the other does not, recovery becomes the highest-priority path and is resolved via maker-only behavior whenever possible.

### Required behavior

1. Cancel heavy-leg orders on mismatch.
2. Quote the missing leg aggressively as maker.
3. Allow exactly one live missing-leg recovery quote.
4. Refresh that quote when stale or invalid.
5. Block new pair entries while mismatch is open.
6. Block normal directional accumulation while mismatch is open.
7. Unwind only if:
   - risk cap is hit, or
   - time runs out, or
   - feed/venue failure makes maker recovery impossible

### Reused internals

1. maker fill ledger
2. order-centric pair fill wait
3. pending imbalance tracking
4. recovery refresh logic

### Metrics to measure over 20+ markets

1. Mismatch-to-flat success rate
2. Recovery duration
3. Post-recovery overshoot count
4. Number of windows where taker was needed
5. Recovery quote refresh success
6. Max inventory imbalance during recovery

### Step 3 success criteria

1. Heavy leg is canceled immediately on mismatch.
2. Missing leg is the only active recovery quote.
3. No new accumulation runs during recovery.
4. Most or all mismatches flatten via maker.
5. Taker remains rare and only tied to explicit emergency conditions.

---

## Step 4: Enable Stretch Bias Only After Steps 1-3 Are Stable

### Objective

Add directional tilt as a secondary overlay after the maker engine, balanced accumulation, and mismatch recovery are all already reliable.

### Required config

Use stretch bias only after the first three steps pass.

Suggested thresholds:

```env
MAKER_STRETCH_BIAS_ENABLED=true
MAKER_STRETCH_RSI_OVERSOLD=35
MAKER_STRETCH_RSI_OVERBOUGHT=65
```

### What this step must do

1. Tilt skew directionally only when stretch conditions are strong.
2. Leave the core pair/merge and maker behavior intact.
3. Improve net P&L without degrading Step 2 and Step 3 stability.

### What this step must not do

1. It must not become the main driver of the strategy.
2. It must not interfere with mismatch recovery.
3. It must not increase taker usage materially.

### Metrics to measure

1. P&L with stretch ON versus OFF
2. Change in downside behavior
3. Change in skew ratio distribution
4. Change in taker count
5. Recovery success with stretch enabled

### Step 4 success criteria

1. Stretch improves P&L or risk-adjusted return.
2. Core maker engine remains stable.
3. Recovery behavior remains unchanged in quality.

If stretch does not improve results, disable it.

---

## Always-On Risk Policy

These rules apply across all milestones:

1. Taker is not part of normal edge generation.
2. Taker is only allowed for:
   - near-expiry unresolved imbalance
   - hard max-loss breach
   - feed/venue failure while carrying imbalance
3. All taker use must be explicit in logs.
4. Any hidden budget stop is a bug, not acceptable behavior.

---

## Logging Requirements

At every milestone, the bot must explain why it is idle or active.

Required log classes:

1. `idle: no_pair_edge`
2. `idle: spread_too_wide`
3. `idle: budget_too_small`
4. `idle: clip_below_min`
5. `merge: waiting_light_leg`
6. `merge: requoting_light_leg`
7. `risk_exit_only`

The engine must not silently "do nothing" without a concrete reason.

---

## Acceptance Criteria for the Full Roadmap

The roadmap is complete only when all are true:

1. No `10/0` or `0/10` bootstrap as the normal path
2. No directional accumulation outside merge/recovery
3. Every imbalance maps to one live missing leg
4. Taker usage is near zero and only for explicit risk exits
5. No hidden budget stop
6. Balanced state returns to pair entry, not random skew accumulation
7. Maker-only edge remains positive or near-flat before directional overlays

---

## Implementation Order

1. Milestone 0: make Step 1 executable
2. Step 1: maker-only spread capture
3. Step 2: balanced accumulation with gentle skew
4. Step 3: mismatch recovery above normal flow
5. Step 4: stretch bias as secondary overlay

Advance only after the prior milestone passes.

---

## Practical Interpretation

This roadmap means:

1. prove execution edge first
2. add skew second
3. add recovery dominance third
4. add directional intelligence last

That sequencing matches the observed wallet behavior better than turning on every engine at once.
