# IMP-15 Plan: Safe-Pause Reconciliation Gates and Failure Containment

## Summary
Implement `REQ-030` and the adjusted in-scope parts of `REQ-031` by turning reconciliation into explicit runtime gates instead of scattered helper calls.

Chosen decisions:
- keep the current product-direction exception: no active-market 5-second full position reconciliation loop
- enforce reconciliation only at startup, after websocket reconnect gaps, and after settlement
- use existing `TRADE_VALIDATE` only for startup and post-settlement trade validation, not as an in-market position source
- when required dependencies fail, stop new risk immediately and enter a pair-safe paused state until validation succeeds or the market is handed to settlement

The goal is to make the bot safe under restarts, reconnects, duplicate events, and dependency failure without replacing the current local inventory engine.

## Key Changes
### 1. One explicit safety gate for “can trade”
- Add a small runtime safety state in the BOT runtime, e.g. `BotRuntimeSafetyGate { Healthy, StartupReconPending, ReconnectReconPending, ValidationFailed, DependencyPaused }`.
- Track:
  - `safety_gate`
  - `safety_gate_reason`
  - `last_clean_reconcile_ts`
  - `last_reconnect_reconcile_ts`
  - `last_validation_ts`
  - `dependency_pause_started_ts`
- New-risk actions (`OpenBoth`, `AwaitSecondFill`, `PairBuild`, `Taper`) must early-return when the safety gate is not `Healthy`.
- `AwaitSettlement` remains allowed even while paused, so the bot can still drain or settle safely.
- Emit stable reasons such as:
  - `startup_reconciliation_pending`
  - `reconnect_reconciliation_pending`
  - `dependency_pause:market_ws`
  - `dependency_pause:user_ws`
  - `dependency_pause:adapter`
  - `dependency_pause:database`
  - `dependency_pause:reconciliation`
  - `reconciliation_mismatch`

### 2. Idempotent order intent and stronger fill dedupe
- Make BOT client order IDs deterministic from one intent key instead of best-effort/random generation:
  - include `trade_id`, `pair_id`, `origin`, `side`, intended price bucket / clip, and a monotonic attempt suffix when replacement is intentional
  - retries of the same submit intent must reuse the same client order ID
- Keep current maker fill dedupe, but unify the dedupe contract across maker and taker fills:
  - one canonical fill identity key per venue event
  - `seen_trade_keys` becomes a bounded dedupe store with timestamp/size trimming instead of an ever-growing plain vector
  - duplicate fill events must be dropped before inventory mutation and before liquidity counters are updated
- Persist enough dedupe context in local state so restart/reconnect does not immediately forget recently applied fills for the active market.

### 3. Startup, reconnect, and post-settlement reconciliation
- Add one shared reconciliation runner that checks:
  - open BOT orders known locally vs live venue open orders for the pair
  - local pair inventory vs venue/Data API position state for the pair
  - settlement result / closed-trade validation when running post-settlement
- Startup gate:
  - before the market is allowed to trade, run pair reconciliation
  - if clean, move safety gate to `Healthy`
  - if mismatched, keep the bot paused and log/audit the mismatch
- Reconnect gate:
  - when market or user websocket reconnects after a disconnect, set `ReconnectReconPending`
  - do not resume new trading until reconciliation completes cleanly
  - reconnect reconciliation must run once per recovered gap, not every loop
- Post-settlement gate:
  - after market exit / settlement completion, run the existing `TRADE_VALIDATE` flow and settlement-state validation
  - if unresolved mismatch remains, persist it and keep the trade in a non-clean terminal state instead of silently treating it as finished

### 4. Dependency-failure pause policy
- Treat these as required dependencies for live trading:
  - market websocket freshness
  - user websocket connectivity when required by config
  - venue adapter submit/cancel/ack path
  - DB persistence for load-bearing state updates
  - reconciliation source during startup/reconnect/post-settlement gates
- On required dependency failure:
  - block all new orders immediately
  - cancel working BOT growth orders if the failure leaves live exposure uncertain
  - keep only passive monitoring, reconciliation, and settlement-safe actions
- Recovery rule:
  - resume only after the failed dependency is healthy again and the relevant reconciliation gate has passed cleanly
- Do not widen this task into the stricter `REQ-033` 2s/5s stale-data policy; `IMP-15` should use the current freshness check only as a dependency-health input, while exact stale-data thresholds remain `IMP-20`.

## Important Interface Changes
- `BotRuntimeState` gains the safety-gate fields above.
- Execution submit helpers gain a deterministic BOT client-order-id builder for idempotent submit retries.
- Local persisted state gains bounded fill-dedupe history for the active market.
- Audit/log payloads gain:
  - `safety_gate`
  - `safety_gate_reason`
  - `reconcile_scope`
  - `reconcile_clean`
  - `dependency_pause_kind`

## Test Plan
- Idempotency:
  - retrying the same BOT intent reuses the same client order ID
  - intentional replacement generates a new attempt suffix
- Fill dedupe:
  - duplicate taker fill event does not change inventory twice
  - duplicate maker fill alias/replay does not change inventory twice
  - restart with recent seen fill keys still drops replayed active-market fills
- Startup reconciliation:
  - clean startup reconciliation unlocks trading
  - mismatched startup reconciliation keeps the bot paused and emits a reconciliation event
- Reconnect reconciliation:
  - websocket disconnect moves the runtime into dependency pause
  - reconnect does not resume trading until reconciliation succeeds
  - clean reconnect reconciliation restores `Healthy`
- Dependency failure:
  - adapter or DB failure blocks new-risk order creation
  - uncertain live-order state cancels BOT growth orders and leaves settlement handling intact
- Post-settlement:
  - clean `TRADE_VALIDATE` marks the trade cleanly validated
  - unresolved post-settlement mismatch remains persisted and visible in audit/logs
- Regression:
  - normal healthy trading path is unchanged
  - no new active-market 5-second position reconciliation loop is introduced

## Assumptions and Defaults
- Active-market 5-second full position reconciliation remains intentionally out of scope.
- Startup and reconnect reconciliation are hard gates for new trading.
- Post-settlement validation continues to use the existing `TRADE_VALIDATE` path plus settlement-specific cleanup checks.
- Required-dependency pauses block new risk, but do not block settlement-safe cleanup.
- Exact stale-data timing hardening remains `IMP-20`, not `IMP-15`.
