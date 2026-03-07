# Plan: Recovery Loss Minimization for Step 1 Pair Base

## Summary
Keep the current Step 1 state machine and ownership model unchanged:

- `Flat`
- `PairResting`
- `MergePending`
- `RiskExitOnly`
- `Balanced`

Change the recovery policy inside `MergePending` from “maker purity first” to “maximize worst-case fee-net floor after the next action.” Implement this in two stages:

1. add **shadow scoring + metrics** without changing behavior
2. then switch `MergePending` action selection to the new scorer

This plan is deliberately incremental. It preserves the current working ownership logic, terminal exit behavior, and Step 1 routing, while replacing the main remaining weakness: waiting too long in `covered_by_live_order`, `negative_economics`, and passive re-quote loops.

## Key Changes

### 1. Recovery Scoring Engine
Add a recovery scorer used only in `MergePending`.

Evaluate these candidate actions on every recovery tick:
1. `maker_buy_light`
2. `exact_sell_heavy`
3. `taker_buy_light`
4. `wait`

Each candidate gets a **resolution-adjusted floor score**, not an idealized full-fill score.

Required scoring outputs per candidate:
1. `floor_after_action`
2. `best_case_after_action`
3. `estimated_action_cost`
4. `expected_resolution_delay_ms`
5. `confidence_penalty`
6. `blocked_reason` if not executable

Scoring rule:
1. base score is worst-case fee-net PnL floor after the action
2. subtract a delay penalty for non-immediate actions
3. subtract a confidence penalty for passive maker paths
4. treat `wait` as an explicit action with its own estimated floor

Decision rule:
1. choose the executable action with the highest adjusted floor
2. break ties in this order:
   - `maker_buy_light`
   - `exact_sell_heavy`
   - `taker_buy_light`
   - `wait`
3. if no action improves the floor enough, choose `wait`

Important behavior rule:
1. `maker_buy_light` must be scored conservatively:
   - one score if it fills soon
   - one score if it does not fill before next decision
   - use the worse or penalized combination
2. `exact_sell_heavy` and taker actions are immediate-execution paths and should not get the same delay penalty.

### 2. Time-Aware Economics Policy
Replace the current static `negative_economics` stop with staged acceptance.

Windows:
1. Early: `t_left > 180s`
2. Mid: `90s < t_left <= 180s`
3. Late: `45s < t_left <= 90s`
4. Terminal: `t_left <= 45s`

Default epsilon policy:
1. Early: `0.05`
2. Mid: `0.15`
3. Late: `0.35`
4. Terminal: compare all executable paths directly, no maker-purity preference

Acceptance rule for maker completion:
1. allow `maker_buy_light` if:
   - `score(maker_buy_light) >= score(wait) - epsilon(window)`
2. if not, prefer the best non-maker rescue path before defaulting to `wait`

Additional rule:
1. use both `t_left` and `recovery_age_ms`
2. if recovery has already been stalled for a configured duration, escalate one window earlier

### 3. Fast Recovery and Coverage TTL
Tighten the logic that treats the light side as “already covered.”

A light-side order only counts as valid coverage if all are true:
1. it has a live `oid`
2. it is not cancel-pending
3. it is still competitive versus the current book
4. the market snapshot is fresh
5. it was acknowledged recently

Add dedicated Step 1 recovery controls:
1. `RECOVERY_TICK_MS=300`
2. `RECOVERY_LIVE_ORDER_TTL_MS=400`
3. `RECOVERY_REQUOTE_ON_BID_MOVE_TICKS=1`
4. `RECOVERY_REQUOTE_ON_STALE_BOOK_MS=500`

Behavior:
1. do not let `covered_by_live_order` survive multi-second stale periods
2. invalidate coverage immediately after:
   - cancel ack
   - adverse top-of-book move beyond threshold
   - quote age > TTL
   - stale book
3. refresh recovery decisions on:
   - top-of-book change
   - cancel ack
   - fill event
   - reject / no oid

### 4. Exact Heavy-Side SELL Preference
Promote `exact_sell_heavy` to the preferred non-maker recovery path.

Action priority is no longer hard-coded, but if scores are close:
1. prefer `exact_sell_heavy` over `taker_buy_light`

Only allow `exact_sell_heavy` if:
1. heavy-side bid depth is sufficient for intended size
2. book is fresh
3. balance / allowance is sufficient
4. no unresolved active exact-sell is already live for that phase
5. the sell improves floor more than waiting

Add parameters:
1. `RECOVERY_PREFER_EXACT_SELL=true`
2. `RECOVERY_EXACT_SELL_MIN_DEPTH_BUFFER=1.05`
3. `RECOVERY_EXACT_SELL_MAX_SLICES=3`

Slicing rule:
1. if exact heavy-side sell needs slicing, slice only within the configured max
2. each slice must preserve floor improvement versus wait

### 5. Entry Asymmetry Reduction
Reduce self-created recovery load at pair entry.

Add entry controls:
1. `ENTRY_ACK_TIMEOUT_MS=400`
2. `ENTRY_FIRST_CLIP_SCALE=0.5`
3. `ENTRY_REQUIRE_BOTH_ACKS=true`
4. `ENTRY_CANCEL_OTHER_ON_NO_OID=true`

Behavior:
1. initial pair clip is scaled by `ENTRY_FIRST_CLIP_SCALE`
2. a pair is only considered armed when both legs have either:
   - valid `oid`s, or
   - confirmed reject/no-oid outcomes
3. if one leg rejects/no-oid, immediately cancel the other
4. if one leg acked and the other has not after `ENTRY_ACK_TIMEOUT_MS`, cancel the acked leg

Optional implementation enhancement:
1. use Polymarket batch order submission for initial YES/NO pair when supported cleanly by existing client code
2. still keep asymmetric-submit cleanup, because batch is not a true atomic guarantee

### 6. Metrics and Logging
Extend Step 1 metrics from market-level summary to recovery-cycle decomposition.

For each recovery cycle, emit:
1. `entry_fee_net_edge`
2. `maker_recovery_cost`
3. `taker_recovery_cost`
4. `final_settlement_pnl`

Also track:
1. time spent in `covered_by_live_order`
2. time spent in `negative_economics`
3. recovery start-to-exit latency
4. percent of mismatches resolved before last `60s`
5. percent resolved by:
   - maker recovery
   - exact heavy-side sell
   - missing-leg taker buy
6. count of `FAK no orders found to match`
7. floor before and after each recovery action

Implementation detail:
1. add these as Step 1-native metrics state in `src/bot.rs`
2. emit per-cycle logs during recovery
3. emit end-of-market aggregates in the existing Step 1 metrics summary path

### 7. Rollout Strategy
Do not switch action selection immediately.

Phase 1:
1. implement all scoring and metrics in **shadow mode**
2. current recovery behavior remains active
3. logs must include:
   - chosen action
   - best scored action
   - score delta
   - reason current policy differed

Phase 2:
1. enable scored action selection for `MergePending`
2. keep terminal `RiskExitOnly` logic unchanged except where scorer explicitly routes there
3. keep all existing exact-exit single-flight protections

Default:
1. shadow mode enabled first
2. scored execution behind a dedicated Step 1 config flag

## Public / Config Additions
Add these environment variables and document them in `ENVIRONMENT.md`:

```env
ENTRY_ACK_TIMEOUT_MS=400
ENTRY_FIRST_CLIP_SCALE=0.5
ENTRY_REQUIRE_BOTH_ACKS=true
ENTRY_CANCEL_OTHER_ON_NO_OID=true

RECOVERY_TICK_MS=300
RECOVERY_LIVE_ORDER_TTL_MS=400
RECOVERY_REQUOTE_ON_BID_MOVE_TICKS=1
RECOVERY_REQUOTE_ON_STALE_BOOK_MS=500

RECOVERY_EPSILON_EARLY=0.05
RECOVERY_EPSILON_MID=0.15
RECOVERY_EPSILON_LATE=0.35
RECOVERY_TERMINAL_COMPARE_ALL_PATHS=true

RECOVERY_PREFER_EXACT_SELL=true
RECOVERY_EXACT_SELL_MIN_DEPTH_BUFFER=1.05
RECOVERY_EXACT_SELL_MAX_SLICES=3

RECOVERY_SHADOW_SCORING_ENABLED=true
RECOVERY_SCORING_ACTIVE=false
RECOVERY_STALL_ESCALATION_MS=15000

RISK_EXIT_TERMINAL_WINDOW_S=45
RISK_EXIT_ALLOW_TAKER_BUY=true
RISK_EXIT_ALLOW_TAKER_SELL=true
```

Defaults chosen:
1. `RECOVERY_SHADOW_SCORING_ENABLED=true`
2. `RECOVERY_SCORING_ACTIVE=false`
3. `ENTRY_REQUIRE_BOTH_ACKS=true`
4. `ENTRY_CANCEL_OTHER_ON_NO_OID=true`
5. `RECOVERY_PREFER_EXACT_SELL=true`

## Test Plan

### Unit / logic tests
1. recovery scorer ranks `exact_sell_heavy` above `wait` when maker completion is slightly negative and heavy-side sell improves floor
2. recovery scorer ranks `maker_buy_light` above `exact_sell_heavy` when maker completion is positive and fresh
3. `wait` loses to late-window rescue when delay-adjusted floor is worse
4. coverage TTL invalidates after:
   - cancel ack
   - stale-book timeout
   - adverse bid move
5. asymmetric pair entry:
   - one leg ack, one no-oid
   - acked leg is canceled within timeout
6. first-clip scaling applies only to initial pair arming path
7. shadow scoring logs chosen vs best-scored action without changing current behavior

### Integration / replay tests
1. replay logs where recovery sat in `covered_by_live_order` too long
   - verify faster re-quote eligibility
2. replay logs where small negative maker completion was refused and later terminal rescue was worse
   - verify scorer would choose earlier cheaper action
3. replay logs where early exact heavy-side sell was better than late missing-leg buy
   - verify scorer prefers exact sell
4. replay asymmetric submit/no-oid cases
   - verify reduced one-sided startup exposure
5. replay terminal near-expiry runs
   - verify existing `RiskExitOnly` behavior is preserved unless scorer explicitly routes there

### Acceptance criteria
1. median recovery start-to-exit latency decreases materially versus current logs
2. time spent in `covered_by_live_order` drops materially
3. time spent in `negative_economics` without action drops materially
4. more mismatches resolve before the last `60s`
5. exact heavy-side sell usage increases where it improves floor
6. late missing-leg taker buy frequency decreases without increasing unresolved rollover count
7. Step 1 remains `PARTIAL` until 20+ market validation passes, but Checkpoint C + recovery-policy work can be marked complete when shadow metrics and scored behavior are both live and validated

## Assumptions
1. Existing Step 1 state machine and ownership model remain unchanged.
2. Existing terminal exact-exit single-flight logic stays in place and is reused.
3. Existing maker/taker fill accounting is trusted enough to support per-cycle decomposition.
4. Polymarket market and user WebSocket streams remain the primary freshness and fill signals.
5. Batch order support is optional; if current client path makes it costly, implement the ack-timeout + cancel-other logic first and leave batching as a follow-up.
6. The implementation should favor behavior-level changes inside `src/bot.rs` and `src/env_contract.rs`, plus metrics wiring in `src/main.rs`, without broad architecture refactors.
