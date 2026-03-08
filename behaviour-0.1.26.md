# CURRENT BEHAVIOUR

## Version 0.1.26 Snapshot

This file is a release snapshot copied from `behaviour.md` and updated for `0.1.26`.

### Release-specific notes

1. `0.1.26` narrows the active recovery scorer so `taker_buy_light` is no longer allowed on tiny score gaps or in `early` / `mid` windows.
2. Pair-base taker rescue now gets one bounded immediate retry on explicit `FAK no orders found to match`, instead of waiting for the next normal loop without action.
3. Pair-base-scoped blocked taker rescue no longer falls through into generic maker replace logic.
4. `RECOVERY_SCORING_ACTIVE` should still be treated as canary-only. The code is safer than `0.1.25`, but the scorer is not signed off as the default Step 1 policy.

### Practical release interpretation

1. Safe default remains:
   - `RECOVERY_SHADOW_SCORING_ENABLED=true`
   - `RECOVERY_SCORING_ACTIVE=false`
2. If active scoring is enabled manually, `0.1.26` is intended to reduce the worst false-taker behavior, not to declare active scoring complete.
3. Step 1 status remains `PARTIAL`. This release improves the scorer guardrails; it does not complete Step 1 validation.

## Current Working Tree Delta After 0.1.26

This file now also reflects the current unversioned working tree after `0.1.26`.

The most important post-`0.1.26` runtime changes are:

1. `ENTRY_ACK_TIMEOUT_MS=1000` has now been validated as the better live canary setting versus `800ms`.
2. `RECOVERY_SCORING_ACTIVE=true` has been exercised in live canary runs without repeating the earlier bad early-taker behavior.
3. The current unversioned patch adds forced escalation out of long `negative_economics` stalls:
   - `Early`: force `RiskExitOnly` after `4 x RECOVERY_STALL_ESCALATION_MS`
   - `Mid`: force `RiskExitOnly` after `2 x RECOVERY_STALL_ESCALATION_MS`
   - `Late` / `Terminal`: immediate forced `RiskExitOnly`
4. The current working tree is intentionally more aggressive about flattening and control than about preserving maker purity once a recovery cycle is clearly bad.

### What the latest two `1000ms` canary runs proved

#### `btc-updown-5m-1772876400`

Observed outcome:

1. control pass
2. no first-leg timeout aborts
3. one cycle resolved by maker
4. one cycle stalled in `negative_economics` and required near-expiry taker rescue
5. final inventory returned to `qYES=10.00 qNO=10.00`
6. final row was negative: `lp=-1.7000 cost=11.7000 cpp=1.1700`

Important metrics:

1. `merge_success_rate=1.000`
2. `maker_recovery_success_rate=0.500`
3. `risk_exit_count=1`
4. `emergency_taker_attempts=1`
5. `negative_economics_s=149.91`
6. `settlement_pnl_net_of_fees=-1.7010`

Interpretation:

1. the `1000ms` entry timeout substantially reduced pair-entry churn
2. active scoring stayed safe
3. the real remaining weakness was the long `negative_economics` dwell time before rescue

#### `btc-updown-5m-1772876700`

Observed outcome:

1. control pass
2. first recovery cycle was fast and healthy
3. second cycle still had some entry asymmetry noise:
   - one first-leg timeout abort
   - two second-leg missing-oid aborts
4. second recovery opened `qYES=10.00 qNO=15.00`
5. that cycle then spent a very long time in `negative_economics`
6. near-expiry taker BUY finally flattened it to `qYES=15.00 qNO=15.00`
7. final row was still negative: `lp=-1.6500 cost=16.6500 cpp=1.1100`

Important metrics and observations:

1. first cycle resolved by maker in about `9.84s`
2. second cycle resolved by taker in about `172.28s`
3. repeated `merge: stop negative_economics ...`
4. repeated `[PAIR_BASE][FEE] label=merge_requote ...`
5. final flatten happened only after:
   - `phase MergePending -> RiskExitOnly`
   - `risk_exit_action trigger=near_expiry`
   - `TAKER_FAK_BUY`
   - later fill confirmation

Interpretation:

1. active scoring again behaved safely
2. `ENTRY_ACK_TIMEOUT_MS=1000` is still better than `800ms`
3. but long negative-economics stalls were still the dominant operational problem

### Cross-run interpretation

Across those two latest canary runs:

1. `ENTRY_ACK_TIMEOUT_MS=1000` is the correct canary setting for now
2. `RECOVERY_SCORING_ACTIVE=true` is behaving acceptably as a canary, not yet as a signed-off default
3. the main remaining Step 1 weakness before the current patch was no longer entry arming or scorer safety
4. the main remaining weakness was long `negative_economics` stall time before forced flattening

### What the current patch is intended to change

The current unversioned patch is meant to remove the exact long-stall pattern seen in those two runs.

Expected behavioural change:

1. the bot should stop sitting in `MergePending` for `150s+` once maker recovery is clearly uneconomic
2. `forced_negative_economics` should take terminal ownership earlier
3. `RiskExitOnly` should begin earlier in bad cycles, not only in the last ~45s near expiry window
4. the bot should flatten earlier, even if that sacrifices some profitability

Practical meaning:

1. control should improve
2. average mismatch duration should drop
3. near-expiry rescue should become a smaller share of total recovery
4. PnL may become less noisy because the bot stops waiting for late lucky completion

### Latest post-patch validation on 2026-03-08

Two more live runs were used to validate the current working tree after the post-`0.1.26` settlement-hold and forced-exit patches.

#### `btc-updown-5m-1772965200`

Observed outcome:

1. final inventory stayed flat through rollover: `qYES=25.00 qNO=25.00`
2. final row was positive: `lp=+1.2500 cost=23.7500 cpp=0.9500`
3. no `RiskExitOnly` rescue was required
4. the new settlement-hold path was exercised repeatedly after the book first appeared balanced

Key evidence:

1. logs showed:
   - `[PAIR_BASE] merge: settling_live_orders ...`
2. the engine stayed in `MergePending` while unresolved pair-base buy risk still existed
3. the old bad pattern did not occur:
   - apparent balance
   - transition out of recovery
   - late recovery fill reopening the book

Interpretation:

1. the stale recovery-order late-fill reopen bug is no longer reproducing in this path
2. this run is the strongest evidence that the settlement-hold fix is doing the intended job

#### `btc-updown-5m-1772965500`

Observed outcome:

1. final inventory stayed flat through rollover: `qYES=5.00 qNO=5.00`
2. final row was slightly negative: `lp=-0.2500 cost=5.2500 cpp=1.0500`
3. `forced_negative_economics` escalated at `17:26:19`
4. the taker-cap override applied immediately on that forced path
5. terminal taker BUY flattened the book well before the near-expiry stop buffer

Key evidence:

1. `[PAIR_BASE] merge: escalate_risk_exit reason=forced_negative_economics ...`
2. `[PAIR_BASE] forced-negative-economics taker cap override ... effective_cap=0.99 ...`
3. taker BUY was sent and later filled
4. the run then stayed under `risk_exit_latched` until rollover

Interpretation:

1. `forced_negative_economics` is now a real early flatten path, not just an early blocked-exit loop
2. the engine no longer has to wait for the last near-expiry override window to get a permissive cap in that path

#### Current verdict after these runs

These two runs change the practical Step 1 assessment:

1. the stale recovery-order reopen bug should no longer block progression
2. `forced_negative_economics` is operationally valid enough to keep
3. both runs finished flat through rollover
4. the remaining issues are now secondary:
   - repeated `risk_exit_action` warning spam while taker is inflight
   - metrics classification noise
   - occasional pair-entry timeout churn

Practical conclusion:

1. Step 1 is still formally `PARTIAL`
2. but this specific blocker is no longer strong enough to hold the next stage
3. Step 1 should remain the canary baseline while next-stage work begins

Date: 2026-03-08
Scope: observed runtime behaviour of the current `PAIR_BASE + RECOVERY + RISK_EXIT` Step 1 path
Reference: user-provided logs across many markets from 2026-03-06 through 2026-03-08

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

In the current unversioned working tree, there is one additional change:

1. if a recovery cycle stays uneconomic for too long, the bot now escalates out of `MergePending` earlier with `forced_negative_economics` instead of waiting deep into the late window

The main remaining weakness is not accounting anymore.
The main remaining weakness is that release-era maker recovery was still too patient and often finished only because the near-expiry taker path rescued it.

In short:

1. accounting is mostly fixed
2. state ownership is much better
3. terminal behaviour is much better
4. the current patch is explicitly trying to make recovery less patient and less profit-protective once the cycle is already bad
5. Step 1 is still `PARTIAL`, not complete

---

## Overall Status Against Target

Relative to `TARGET_GOAL_STATUS.md`, current behaviour is approximately:

1. `Milestone 0`: complete
2. `Step 1`: partial but runnable
3. estimated closeness to practical Step 1 target: about `75%`

Why it is not higher:

1. maker recovery still spends too much time in passive wait / requote cycles
2. near-expiry taker rescue is still doing too much of the real cleanup work in the pre-patch logs
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

The current unversioned patch changes this further:

1. `negative_economics` no longer means "just wait"
2. after a window-dependent stall duration, it can now escalate to `forced_negative_economics`
3. that forced path enters `RiskExitOnly` earlier than the old near-expiry-only rescue

This is the most important current behaviour change beyond `0.1.26`.

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

The latest current patch is explicitly intended to reduce this pattern by forcing earlier risk exit on long uneconomic cycles.

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

### Pattern A2: long uneconomic stall before terminal rescue

Observed in the latest two `ENTRY_ACK_TIMEOUT_MS=1000` runs.

Behaviour:

1. pair arms successfully
2. one side fills
3. maker recovery starts correctly
4. repeated `merge: stop negative_economics ...`
5. repeated fee snapshots with worsening or still-negative worst-case completion
6. cycle remains open for a long time
7. near-expiry taker finally finishes it

Interpretation:

1. this was the last clearly dominant control inefficiency in the current worktree before the newest patch
2. it is exactly what `forced_negative_economics` is meant to cut shorter

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
4. pre-patch, it still finished too many markets via late taker rescue instead of timely maker recovery
5. current patch direction is to flatten earlier once recovery quality is clearly bad, even at the expense of near-term profitability

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

The next question after the current patch is:

1. does `forced_negative_economics` materially reduce:
   - `negative_economics_s`
   - `avg_time_to_flat_s`
   - late-window rescue dependence
2. without breaking flatness at rollover
