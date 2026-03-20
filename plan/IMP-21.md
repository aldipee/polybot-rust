# IMP-21 Plan: Per-Side Maker Refresh-Cycle Cap

## Summary
Implement `REQ-034` by making maker quote refreshes obey one authoritative per-side cadence cap in the supported BOT runtime.

Chosen decisions:
- reuse `MAKER_REPLACE_MIN_INTERVAL_SECONDS` as the authoritative refresh-cycle cap
- change its default to `1.0` second per side
- move that knob into the versioned config snapshot instead of reading it ad hoc from env
- keep `REPLACE_IF_PRICE_MOVES_TICKS` and `STALE_SECONDS` as the existing refresh triggers and order-aging inputs
- apply the cap only to voluntary quote refresh churn, not to emergency or terminal cancels

A “refresh cycle” means touching an existing BOT maker quote on one side to improve/reprice/repost it. In cancel-then-create flow, the cancel plus replacement counts as one cycle.

## Key Changes
### 1. Config and authoritative policy surface
- Add `maker_replace_min_interval_seconds` to the effective config bundle in `src/config.rs` and persist it through the `IMP-12` versioned snapshot path.
- Keep env name `MAKER_REPLACE_MIN_INTERVAL_SECONDS`; do not add a second overlapping refresh-cap env.
- Change the default from `0.5` to `1.0`.
- Keep explicit operator overrides allowed.
- Treat `REPLACE_IF_PRICE_MOVES_TICKS` and `STALE_SECONDS` as trigger knobs, but `MAKER_REPLACE_MIN_INTERVAL_SECONDS` becomes the actual per-side max refresh cadence.

### 2. Runtime definition of what is capped
- Scope the cap to supported BOT maker buy families:
  - `BOT_OPEN_BOTH`
  - `BOT_AWAIT_SECOND_FILL`
  - `BOT_PAIR_BUILD*`
  - `BOT_TAPER*`
- Do not apply the cap to taker orders.
- Do not apply the cap to unsupported ladder or legacy non-BOT strategy surfaces.
- Count these as refresh activity:
  - cancel/reprice of an existing BOT maker order because price moved, best-passive candidate changed, age threshold hit, or the runtime wants to repost the same side
  - timed-out live-order reposts in await-second-fill, pair-build, and taper
- Exempt these from the cap:
  - stale-data hard-pause drains
  - dependency-pause drains
  - settlement handoff drains
  - hard-disable / price-zone / residual / validation / reconciliation safety cancels
  - any transition where the requirement says stale working orders must not stay live across the state change

### 3. Per-side refresh-cycle state and enforcement
- Add pair-local per-side refresh-cycle state to `BotRuntimeState`, one for `YES` and one for `NO`.
- Track at least:
  - `last_cycle_started_ts`
  - `awaiting_repost`
  - `last_origin`
  - `last_reason`
- Use one shared helper to decide whether a new refresh cycle may start on a side.
- Starting a refresh cycle should happen when the bot intentionally requests a refresh-driven cancel on that side.
- The replacement submit that follows that cancel is part of the same cycle and must not be blocked by the cap.
- Initial first submits on a side with no live BOT maker order are not refresh cycles and stay allowed.
- Apply the same cap in both paths:
  - single-inflight slot path in `_maker_order_upsert_gtc`
  - direct-order fallback path when `MAKER_SINGLE_INFLIGHT_PER_SIDE` is off
- Keep the existing repost hysteresis and optional-growth pacing, but make the cadence cap a harder outer guard above them.

### 4. Logs, metrics, and stable reasons
- Emit a stable hold reason when the cap blocks a refresh, for example `refresh_cadence_cap:<side>:<remaining_seconds>`.
- Add runtime counters for:
  - refresh cycles started per side
  - refresh-cap blocks per side
- Expose those counters in `BotRuntimeMetricsSnapshot` and the periodic runtime log.
- Persist refresh-cap holds through the existing audit/risk-block path so operators can prove no side exceeded the configured cadence.

## Important Interface Changes
- `BotConfig` and the versioned config snapshot gain first-class `maker_replace_min_interval_seconds`.
- `BotRuntimeState` gains per-side refresh-cycle tracking.
- `BotRuntimeMetricsSnapshot` gains refresh-cycle and refresh-cap-block counters.
- `MAKER_REPLACE_MIN_INTERVAL_SECONDS` becomes the documented authoritative refresh-cap control, not just an internal slot helper.

## Test Plan
- Config tests:
  - default `MAKER_REPLACE_MIN_INTERVAL_SECONDS` resolves to `1.0`
  - persisted old config rows/snapshots without the new field hydrate safely
  - explicit overrides still load correctly
- Single-inflight tests:
  - replacing a working YES quote twice within `1s` blocks the second cycle
  - YES and NO are independent; blocking one does not block the other
  - initial first submit is not blocked
  - cancel-plus-repost inside one cycle is allowed
- Direct-order tests:
  - with `MAKER_SINGLE_INFLIGHT_PER_SIDE=false`, direct BOT quote refresh still respects the per-side cap
  - the cap does not depend on latency logging
- Runtime behavior tests:
  - await-second-fill stale-order refresh respects the cap
  - pair-build/taper refresh churn respects the cap
  - safety or terminal cancels bypass the cap and still drain immediately on state transitions
- Observability tests:
  - refresh-cap block emits the stable hold reason
  - metrics/log snapshot expose per-side cycle counts and block counts

## Assumptions and Defaults
- The requirement target is “one cycle per side per second by default,” so explicit operator overrides remain allowed.
- `STALE_SECONDS` remains the passive-order aging trigger, not the cadence cap itself.
- Emergency and terminal cleanup must always win over cadence throttling.
- The implementation should leave current price-move, age, and runtime-state refresh triggers intact unless they conflict with the new per-side cap.
