# IMP-03 Plan: Settlement-Owned Hold Lifecycle

## Summary
Implement `IMP-03` as a lifecycle change, not a trading-logic rewrite. The bot should keep its current paired buy-first runtime, but once the terminal cutoff is reached it must stop trading, cancel all working orders, enter an explicit `AwaitSettlement` phase, and let settlement drive trade finalization.

This task will preserve the current “next market can start while post-market cleanup runs in background” architecture. The trading loop exits once the pair is safely in `AwaitSettlement`, and the background finalizer owns RTDS close, settlement result capture, trade finalization, and post-settlement validation.

## Key Changes
### Runtime lifecycle
- Replace `HoldSettleRollover` with `AwaitSettlement` in `BotRuntimePhase` and `BotRuntimeControlOwner`.
- Treat `AwaitSettlement` as a terminal non-trading phase:
  - no new orders
  - no taper maintenance
  - no rebalancing
  - no intrawindow exit logic
- Use the existing rollover / end-of-market trigger as the transition signal in this task.
  - Exact late-window timing remains `IMP-11`.
- On transition into `AwaitSettlement`, the runtime must:
  - cancel all strategy-owned working orders
  - request exchange-level cancel for any live pair orders
  - wait only until local order state is drained or a short bounded timeout expires
  - emit a stable exit reason of `AWAIT_SETTLEMENT`
- The runtime loop then stops trading and returns, but leaves the trade open for background settlement finalization.

### Buy-only enforcement
- Make normal BOT runtime order families explicitly buy-only:
  - open-both
  - seed-completion
  - pair-build
  - taper
- Add a runtime guard that rejects any normal BOT strategy submit carrying `SELL`.
- Keep recovery / manual unwind helpers out of normal strategy flow.
  - `_chunked_unwind_heavy_leg` and other taker-sell helpers remain recovery-only and must not be called by the standard paired lifecycle.
- Do not remove sell-capable execution primitives in this task; only wall them off from the MVP runtime path.

### Settlement-owned finalization
- In the background post-run path in `main.rs`, mark the trade as waiting for settlement before RTDS close.
  - Use existing `claim_status` for this task.
  - Set `claim_status = "AWAIT_SETTLEMENT"` once trading has stopped and the pair is handed off.
- Keep `RtdsService::close()` as the settlement wait primitive.
  - It already waits until the resolution boundary and persists a resolution snapshot.
- Final trade closure rule:
  - only finalize from settlement data
  - do not treat trading-loop exit as the final trade close
- When a resolution snapshot is available:
  - compute realized LP from the snapshot
  - write final trade result with `update_trade_result`
  - update settlement metadata with `claim_status = "SETTLED"`
  - store resolution details in `meta_data`
- When resolution data is still unavailable after RTDS close:
  - do not invent a pre-settlement fallback close for this task
  - leave the trade open with `claim_status = "AWAIT_SETTLEMENT"`
  - keep `validation_status` pending so startup / post-settlement validation can reconcile it later

### Validation and reconciliation handoff
- Keep `TRADE_VALIDATE` out of active-market reconciliation.
- Use `reconcile_unvalidated_trades_with_polymarket(...)` only:
  - on startup / before the first tradable market loop
  - after settlement finalization
- Do not add or retain any requirement for every-5-second active-market position reconciliation.
- Settlement lifecycle target:
  - `AwaitSettlement` after trading stops
  - `SETTLED` once RTDS or venue settlement is captured
  - existing validation flow then upgrades the accounting truth afterward

## Public Interface / Type Changes
- Rename runtime phase and control owner variant:
  - `HoldSettleRollover` -> `AwaitSettlement`
- Extend `BotRuntimeState` with explicit settlement handoff fields:
  - `await_settlement_started_ts`
  - `await_settlement_orders_cleared_ts`
  - `await_settlement_cancel_requested`
- Reuse existing DB fields instead of adding schema in this task:
  - `claim_status` for settlement lifecycle state
  - `meta_data` for settlement snapshot details
  - existing validation columns for post-settlement validation outcome
- Standard exit reason for the trading loop becomes `AWAIT_SETTLEMENT` instead of `ROLLOVER` when the pair reaches terminal hold. 

## Test Plan
- Runtime phase tests:
  - terminal phase maps to `AwaitSettlement`
  - owner routes to `AwaitSettlement` and no trading handler runs afterward
- Order lifecycle tests:
  - entering `AwaitSettlement` cancels all working pair orders
  - no new BOT runtime orders are submitted after `AwaitSettlement` begins
  - normal BOT runtime submit paths reject `SELL`
- Finalization tests:
  - when RTDS resolution exists, trade is finalized from settlement and `claim_status` becomes `SETTLED`
  - when RTDS resolution is missing, trade remains open with `claim_status = "AWAIT_SETTLEMENT"` and no fake LP close is written
- Validation tests:
  - startup `TRADE_VALIDATE` still processes pending settled trades
  - post-settlement validation runs after settlement finalization, not during active trading
- Regression tests:
  - main loop still advances to the next market after trading stops
  - manual or recovery sell helpers remain callable outside the standard BOT runtime path

## Assumptions and Defaults
- This task uses the current end-of-market / rollover trigger as the entry into `AwaitSettlement`; exact late-window timing stays in `IMP-11`.
- `AwaitSettlement` is a non-trading terminal phase, not a long-lived foreground loop.
- Existing background settlement architecture in `main.rs` stays in place.
- No new DB columns are required for `IMP-03`; existing `claim_status`, `meta_data`, and validation fields are sufficient.
- Recovery or manual sell-capable code remains in the repo, but it is explicitly out of the normal MVP paired lifecycle.
