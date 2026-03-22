# IMP-14 Plan: Paper Adapter and Shadow-First Live Arming

## Summary
- Keep `EXEC_MODE=BOT`; add a separate order-routing mode `BOT_ORDER_MODE=shadow|paper|live`, default `shadow`, plus `BOT_LIVE_ENABLED=false` by default.
- Replace the current scattered `dry_run` branches with one internal execution-dispatch layer so strategy, risk, reconciliation, and ledger logic stay shared.
- Treat guarded live as “configured live, effective shadow until armed”: no real submit or cancel writes until explicit live enablement, clean reconciliation, dependency health, and fresh market data all pass.
- Implement a true paper adapter that uses live market data and the same decision engine, differing only in local order handling and fill simulation.

## Public Interface Changes
- Add `BOT_ORDER_MODE` as the operator-facing routing mode; accepted values are `SHADOW`, `PAPER`, and `LIVE`, case-insensitive, with `SHADOW` as the default.
- Add `BOT_LIVE_ENABLED` as the explicit live-send arm switch; default `false`. Real exchange writes require both `BOT_ORDER_MODE=LIVE` and `BOT_LIVE_ENABLED=true`.
- Keep `EXEC_MODE=BOT` as the only supported runtime family.
- Keep `DRY_RUN` only as a deprecated compatibility alias:
  - if `BOT_ORDER_MODE` is unset and `DRY_RUN=true`, resolve to `PAPER`
  - if `BOT_ORDER_MODE` is unset and `DRY_RUN=false`, resolve to the new default `SHADOW`
  - if both are set inconsistently, fail fast
- Extend the versioned execution snapshot with `order_mode` and `live_enabled`. Old snapshots backfill from legacy `dry_run`.
- Make config validation mode-aware:
  - `PAPER` requires live market-data connectivity, but not live trading credentials
  - `SHADOW` and configured `LIVE` keep the current wallet/auth requirements because they observe real account state and can promote to live

## Implementation Changes
- Add an internal `BotOrderMode` enum and a single `_bot_runtime_effective_order_mode()` helper.
  - `PAPER` stays `PAPER`
  - `SHADOW` stays `SHADOW`
  - `LIVE` downgrades to effective `SHADOW` until all live-arm conditions pass
- Live-arm conditions are:
  - `BOT_LIVE_ENABLED=true`
  - startup or reconnect reconciliation is clean
  - dependency health is green
  - market data is fresh under the existing stale-data policy
- Use one internal execution-dispatch layer below the current high-level order helpers.
  - Live adapter wraps the current CLOB submit/cancel/list behavior
  - Paper adapter owns local working-order state and simulated fills
  - Shadow adapter owns hypothetical working-order state and cancel/replace behavior, but never emits fills and never writes to the venue
- Do not fork strategy or risk logic by mode. Existing maker/taker approval paths should continue to call the same high-level helpers; only the adapter path changes underneath.
- Make startup and reconnect write-safe:
  - while effective mode is not `LIVE`, startup reconciliation may read venue state but must not send venue submits or venue cancels
  - `cancel_all_on_start` and other startup drains become read-only until live is actually armed
  - once a previously armed live session falls back to shadow because of reconnect, stale data, or dependency pause, new live submits stop immediately but protective drains for already-live orders are still allowed
- Add mode-scoped state isolation.
  - Live keeps the current state and shared-companion filenames for backward continuity
  - Paper and shadow use mode-suffixed local state, daily-liquidity, pending-taker, and gross-cap files so simulated or hypothetical exposure never contaminates live wallet state
- Paper fill model is conservative and entirely adapter-owned.
  - Maker orders only fill on observed touch/cross or trade-through evidence from live quotes/trades
  - Filled size is capped by visible top-of-book evidence for that update and never assumes optimistic queue priority
  - Taker FAK/FOK fills immediately only if the observed opposite side satisfies the price cap; otherwise the unfilled remainder cancels per order type
  - Taker GTC can remain as a live paper order in the paper ledger and then follow the normal lifecycle
- Keep existing safety controls active in paper mode.
  - stale-data blocks, refresh caps, gross caps, underdog residual guards, and reconciliation-style local consistency checks all still run
  - paper and shadow use their own mode-scoped pending/gross state rather than the live wallet-global files
- Extend logs and audit payloads with both `configured_order_mode` and `effective_order_mode`, plus a stable live-block reason when configured live is still shadow-routed.

## Test Plan
- Config and migration tests:
  - default `BOT_ORDER_MODE` resolves to `shadow`
  - `BOT_LIVE_ENABLED` defaults to `false`
  - `DRY_RUN=true` backfills to `paper`
  - inconsistent `DRY_RUN` and `BOT_ORDER_MODE` fails
  - old snapshots without the new execution fields backfill correctly
- Live-arm gating tests:
  - `BOT_ORDER_MODE=live` with `BOT_LIVE_ENABLED=false` sends zero real submits
  - startup reconciliation pending keeps configured live in effective shadow
  - reconnect, dependency pause, or hard stale demotes effective live back to shadow for new writes
  - once reconciliation is clean and data is fresh, the same configured live bot is allowed to send real orders
- Paper adapter tests:
  - maker submit creates paper working order and simulated ack without venue IO
  - maker fill only occurs on observed touch/cross evidence
  - taker FAK/FOK obeys price-cap semantics
  - open-to-settlement lifecycle works in paper with the existing BOT runtime
- Shadow mode tests:
  - hypothetical submits and cancels are logged and inspectable
  - shadow emits zero fills and changes zero real exchange state
  - shadow and paper do not publish into live shared gross or pending-taker files
- Safety regression tests:
  - existing stale-data, gross-cap, refresh-cap, and reconciliation guards still apply in paper mode
  - startup default config sends zero live orders and zero live cancels

## Assumptions
- Chosen operator UX: single `BOT_ORDER_MODE` enum with `shadow` as the default.
- Shadow mode is inspectable hypothetical-order mode, not simulated-fill mode; paper is the only simulated-fill mode.
- `EXEC_MODE` remains `BOT`; no new top-level runtime family is introduced.
- KPI dashboards, admin controls, replay, and canary launch gates remain later work under `IMP-23` to `IMP-25`.
