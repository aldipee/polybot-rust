# CURRENT BEHAVIOUR

Date: 2026-03-07
Scope: observed runtime behaviour of the current `PAIR_BASE + RECOVERY + RISK_EXIT` Step 1 path
Reference: user-provided logs across many markets from 2026-03-06 through 2026-03-07

---

## Purpose

This document describes what the bot actually does now, based on the logs provided across many live markets.

It is not a design target.
It is an observed-behaviour summary.

---

## Executive Summary

Current behaviour is materially better than the earlier versions, but it is still not the final target.

The current Step 1 path now behaves like this:

1. it is pair-first, not quote-only, during normal Step 1 operation
2. it can build balanced inventory through maker YES/NO pairs
3. it can detect one-leg fills and switch into maker recovery on the light side
4. it can stop maker recovery when completion economics go negative
5. it can escalate to `RiskExitOnly` near expiry and use taker to finish the pair
6. it now usually stays flat after terminal exit because `risk_exit_latched` blocks re-entry

The main remaining weakness is not accounting anymore.
The main remaining weakness is that maker recovery is still too patient and often finishes only because the near-expiry taker path rescues it.

In short:

1. accounting is mostly fixed
2. state ownership is much better
3. terminal behaviour is much better
4. maker recovery is still too slow / too conservative
5. Step 1 is still `PARTIAL`, not complete

---

## Overall Status Against Target

Relative to `TARGET_GOAL_STATUS.md`, current behaviour is approximately:

1. `Milestone 0`: complete
2. `Step 1`: partial but runnable
3. estimated closeness to practical Step 1 target: about `75%`

Why it is not higher:

1. maker recovery still spends too much time in passive wait / requote cycles
2. near-expiry taker rescue is still doing too much of the real cleanup work
3. empirical validation over the required 20+ markets is still pending
4. emergency taker semantics are much better now, but still share some lower-level helper behaviour

---

## Current Runtime Phases

Observed current Step 1 behaviour is:

1. `Flat`
2. `PairResting`
3. `MergePending`
4. `Balanced`
5. `RiskExitOnly`

These phases now show up explicitly in logs with:

1. `[PAIR_BASE] phase Flat -> PairResting`
2. `[PAIR_BASE] phase PairResting -> MergePending`
3. `[PAIR_BASE] phase MergePending -> RiskExitOnly`
4. `[PAIR_BASE] phase RiskExitOnly -> Balanced`

This is a real improvement over older logs where ownership was implicit and mixed with the skew loop.

---

## What The Bot Does Now

### 1) Market start / warmup behaviour

At market start the bot now typically:

1. warms order metadata for both YES and NO assets
2. waits for feed and CLOB connectivity
3. often shows `t_into=0.0s` until the market is actually live
4. pauses correctly on feed stale and resumes on feed recovery

This is normal.
The `t_into=0.0s` behaviour seen in several runs is not a trading bug by itself.

### 2) Pair entry behaviour

When the pair entry checks pass, the bot:

1. logs a fee-net pair snapshot
2. submits YES and NO maker GTC orders
3. enters `PairResting`

Observed log pattern:

1. `[PAIR_BASE][FEE] label=pair_entry ...`
2. `[LATENCY][SUBMIT] ... origin=PAIR_BASE_GTC_YES`
3. `[LATENCY][SUBMIT] ... origin=PAIR_BASE_GTC_NO`
4. `[PAIR_BASE] phase Flat -> PairResting ...`

### 3) Asymmetric submit is still real

Pair entry is still sequential in practice, not atomic.

That means:

1. YES can submit while NO rejects
2. one leg can fill before the other is even acknowledged
3. the bot still has to repair that asymmetry afterward

Observed repeated pattern:

1. YES submit succeeds
2. NO returns `post_order returned no oid`
3. YES is canceled
4. sometimes YES fills before cancel completes
5. recovery begins from there

This is still one of the main reasons the bot ends up in `MergePending`.

### 4) Recovery now owns the imbalance

Once one side fills and the other side does not, the current engine now does the right high-level thing:

1. it enters `MergePending`
2. it stops normal pair growth
3. it works only the light side
4. it does not continue adding heavy-side BUY risk

This part is significantly improved compared with older runs.

Observed log pattern:

1. `recovery enter gap=... heavy=... light=...`
2. `phase PairResting -> MergePending`
3. `merge: waiting_light_leg ...`
4. `merge: requoting_light_leg ...`

### 5) Recovery is still too patient

This is the main current behaviour problem.

In many recent runs, recovery spends a long time cycling through:

1. `waiting_light_leg reason=unsettled`
2. `waiting_light_leg reason=covered_by_live_order`
3. `requoting_light_leg reason=...`
4. `recovery remain ...`

That means the bot is often waiting because:

1. it still counts an old or canceling light-side order as usable coverage
2. it wants to remain maker-only
3. it is trying not to overtrade

The result is:

1. long mismatch windows
2. many recovery loops
3. eventual near-expiry taker rescue

This is the biggest gap between current behaviour and the target wallet style.

### 6) Negative-economics stop now works

This is a genuine improvement and is clearly visible in the logs.

When maker completion becomes uneconomic, the bot now logs:

1. `[PAIR_BASE] merge: stop negative_economics ...`

and does not keep blindly reposting maker recovery orders.

This behaviour is correct.
It prevents the bot from repeatedly paying for bad completion attempts.

The tradeoff is:

1. if maker recovery stops
2. and the position is still imbalanced
3. then the bot increasingly depends on the near-expiry taker path

### 7) Near-expiry `RiskExitOnly` now works much better

This is another real improvement.

Current observed behaviour:

1. the bot enters `RiskExitOnly` earlier than the final stop buffer
2. it logs the reason and the intended taker action
3. it uses the near-expiry taker cap override
4. it sends the taker order
5. it latches terminal ownership
6. after flattening it stays `risk_exit_latched` until rollover

Observed log pattern:

1. `[PAIR_BASE] phase MergePending -> RiskExitOnly`
2. `[PAIR_BASE] risk_exit_only reason=near_expiry ...`
3. `[PAIR_BASE] risk_exit_action ... action=taker_buy` or `action=taker_sell`
4. `[PAIR_BASE] near-expiry taker cap override ...`
5. `EMERGENCY HEDGE ...`
6. `[TAKER FAK] sent ...`
7. `[LATENCY][FILL] ...`
8. `[PAIR_BASE] phase RiskExitOnly -> Balanced`
9. `[PAIR_BASE] idle: risk_exit_latched`

This behaviour is much closer to the intended bounded-downside policy.

### 8) Sub-min tail handling is now explicit

Small unresolved tails are no longer ignored in the same way they were earlier.

Current behaviour now supports:

1. `hold` policy for sub-min gaps
2. `taker_immediate` policy for sub-min gaps
3. exact heavy-side `SELL` terminal behaviour for small tails

This is a major improvement over the older behaviour where sub-min tails could simply drift into expiry.

### 9) Accounting appears stable now

Across the later runs, the old phantom double-application behaviour does not appear to be the main issue anymore.

Current observable behaviour:

1. maker fills are applied once
2. later duplicates are logged as deduped drops
3. final inventory is generally consistent with the observed fills

This is one of the most important areas that improved from the earlier runs.

---

## What Is Clearly Better Than Before

Based on the logs across many markets, the following old issues appear materially improved or fixed:

1. maker fill double-application / phantom +5 or +10 inventory jumps
2. repeated overlapping pair-arb compounding
3. heavy-side BUY accumulation during recovery
4. re-entry after near-expiry terminal exit
5. blocked near-expiry exit caused only by the old cap path
6. silent sub-min terminal no-op behaviour

These changes are visible in the newer logs where:

1. final `qYES` / `qNO` often return to balance
2. `risk_exit_latched` holds the market flat until rollover
3. emergency taker exits actually send and fill

---

## What Is Still Not Good Enough

### 1) Recovery is still too slow

This is the main current weakness.

Observed behaviour:

1. one side fills quickly
2. the missing side takes tens of seconds or more to finish
3. recovery often spends too long in:
   - `covered_by_live_order`
   - `requoting_light_leg`
   - `negative_economics`

### 2) The bot still relies too much on terminal taker rescue

Many runs now end flat, but too often they end flat because:

1. maker recovery failed to finish in time
2. near-expiry `RiskExitOnly` took over
3. a taker BUY or exact SELL finished the pair in the last 30-50 seconds

That is safer than before, but it is still not ideal Step 1 behaviour.

### 3) Pair entry is still asymmetry-prone

The engine is still vulnerable to:

1. YES submit succeeds, NO reject
2. NO submit succeeds, YES reject
3. one leg fills before the other leg is actually live

This is still a structural source of recovery work.

### 4) The bot can still get stuck in long negative-economics wait cycles

When the light-side maker quote would complete the pair at a bad price, current behaviour is:

1. do not submit
2. log repeated fee snapshots
3. keep waiting
4. eventually near-expiry taker takes over

That is rational, but operationally it means the bot is not yet finishing many mismatches early.

---

## Recurring Log Patterns And What They Mean

### Healthy pair start

1. `[PAIR_BASE][FEE] label=pair_entry ...`
2. YES/NO `PAIR_BASE_GTC_*` submits
3. `phase Flat -> PairResting`

Meaning:

1. pair entry passed
2. the bot is attempting a normal maker pair build

### Healthy recovery start

1. `recovery enter gap=... heavy=... light=...`
2. `phase PairResting -> MergePending`

Meaning:

1. one leg filled
2. the bot is now explicitly in repair mode

### Too-patient recovery

1. `merge: waiting_light_leg reason=covered_by_live_order`
2. `recovery remain ...`
3. `merge: requoting_light_leg ...`
4. repeated without fill

Meaning:

1. the engine still believes passive maker completion is possible
2. it is preserving maker discipline, but slowly

### Uneconomic recovery pause

1. repeated `[PAIR_BASE][FEE] label=merge_requote ... fee_net_worst_case_pnl=-...`
2. `merge: stop negative_economics ...`

Meaning:

1. maker recovery is intentionally paused
2. the bot is waiting for a better price or a later risk-exit trigger

### Terminal rescue

1. `phase MergePending -> RiskExitOnly`
2. `risk_exit_action ...`
3. `near-expiry taker cap override ...`
4. `EMERGENCY HEDGE ...`
5. taker send/fill
6. `phase RiskExitOnly -> Balanced`
7. `idle: risk_exit_latched`

Meaning:

1. maker recovery did not finish
2. terminal risk policy took over
3. the bot flattened and then froze correctly

---

## Representative Behaviour From The Provided Logs

### Pattern A: clean near-expiry save

Observed in multiple later runs.

Behaviour:

1. pair starts
2. one side fills
3. maker recovery stalls or becomes uneconomic
4. near-expiry taker BUY or exact SELL completes the pair
5. `risk_exit_latched` prevents re-entry
6. final inventory is flat at rollover

Interpretation:

1. control is much safer than before
2. but the bot still often needs terminal rescue

### Pattern B: long maker recovery stall before eventual recovery

Observed repeatedly in the logs.

Behaviour:

1. first light-side submit happens quickly
2. quote gets invalidated or canceled
3. bot spends a long time in `covered_by_live_order`
4. then spends more time in `negative_economics`
5. eventually price improves and it submits again

Interpretation:

1. this is why recovery feels slow
2. the delay is decision logic, not raw submit latency

### Pattern C: balanced outcome, but only after rescue

Observed in several later passes.

Behaviour:

1. final `qYES` and `qNO` end equal
2. final row is valid
3. but most of the actual repair happened in the near-expiry taker window

Interpretation:

1. outcome is acceptable
2. path is still too reactive

---

## Current Behavioural Strengths

The bot is now good at:

1. keeping explicit ownership of pair / merge / risk-exit state
2. avoiding heavy-side accumulation during recovery
3. preventing re-entry after near-expiry terminal exit
4. applying maker fills consistently
5. logging enough state to explain most decisions

---

## Current Behavioural Weaknesses

The bot is still weak at:

1. quickly filling the light side during recovery
2. escaping `covered_by_live_order` quickly after recovery cancel/replace
3. completing pairs early enough without near-expiry rescue
4. avoiding sequential pair-entry asymmetry

---

## What The Current Behaviour Is Closest To

The current engine is closest to this operational style:

1. maker pair builder with explicit maker recovery
2. passive and conservative during recovery
3. guarded by fee-net economics
4. rescued by taker near expiry when maker completion did not happen

It is not yet closest to:

1. fast maker merge completion
2. near-flat inventory throughout the market
3. high-confidence balanced completion before the terminal window

---

## Current Practical Assessment

If the question is "what does it do now?", the best short answer is:

1. it can build and repair pairs
2. it usually stays logically correct
3. it often finishes flat
4. but it still finishes too many markets via late taker rescue instead of timely maker recovery

If the question is "is it stable enough to move to Step 2?", the current answer remains:

1. not yet
2. Step 1 still needs empirical validation and better recovery-quality evidence first

---

## What To Measure Next

The new `[PAIR_BASE][METRICS]` output should now be used to measure:

1. `merge_success_rate`
2. `maker_recovery_success_rate`
3. `pair_coverage_avg` / `pair_coverage_min`
4. `residual_gap_avg` / `residual_gap_max`
5. `avg_time_to_flat_s`
6. `avg_time_to_redeploy_s`
7. `emergency_taker_attempts`
8. `settlement_pnl_net_of_fees`

The most important practical question is:

1. how many markets finish because maker recovery works,
2. versus how many finish only because `RiskExitOnly` rescues them near expiry.

That is the clearest remaining gap between current behaviour and the target roadmap.
