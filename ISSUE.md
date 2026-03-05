# Issue: Same-Side Maker GTC Accumulation (Duplicate In-Flight Orders)

## Status
- Date: 2026-03-05
- Severity: High (capital/risk control issue)
- Scope: `MAKER_SKEW_ARB` maker BUY flow
- Current state: Mitigated by lifecycle gate; additional hardening added; still monitor cancel-churn behavior.

## Problem Summary
The bot can place multiple maker GTC BUY orders on the same asset/side before prior orders are fully resolved. Under delayed ACK/fill/event timing, this stacks exposure, then fills in bursts, which can trigger emergency risk logic (`max-loss`, `force flatten`, taker unwind/sell).

## Observed Symptoms

### Earlier incident pattern (from previous markets)
- Repeated same-side maker submits on one leg while prior orders were still live/pending.
- Exposure mismatch escalated.
- Emergency hedge blocked by cap/ask constraints.
- Fallback to force-flatten via taker SELL, destroying edge.

### Provided run `btc-updown-5m-1772713500` (improved but still showing lifecycle stress)
- Same-side overlapping submits still occurred before previous submit fully settled:
  - `src log`: submit `0xe4d5e5c9..` then submit `0x18177b83..` on same asset (`400348`) before first fill.
  - `src log`: submit `0x89752ba1..` then submit `0xcfe66113..` before first fill.
- Evidence from `output/btc-updown-5m-1772713500/app.log`:
  - `19:26:30.545` submit `0xe4d5...` (BUY, maker)
  - `19:26:33.821` submit `0x1817...` (BUY, maker)
  - `19:26:34.874` fill `0xe4d5...`
  - `19:26:34.898` fill `0x1817...`
  - `19:26:50.069` submit `0x8975...`
  - `19:26:52.483` submit `0xcfe6...`
  - `19:26:52.494` fill `0xcfe6...`
  - `19:26:59.993` late fill `0x8975...`
- Long repeated cancel requests were also observed for same OID (`0x7c118665..`) near end of market.
- This run finished positive (`lp=2.2304`) and did not trigger force-flatten, but behavior is still not fully clean.

## Root Cause
This was a control-path/lifecycle consistency gap (not one bug):
1. Multiple maker submission paths could bypass strict in-flight gating.
2. Local tracking could be overwritten by newer OID while older live order still existed.
3. Event/reconcile timing races (WS/order list staleness) could temporarily make slot appear clear and allow another submit.
4. Seed decision originally used filled inventory only, not pending working BUY remainder.

## Implemented Fixes

### 1) Introduced explicit maker order lifecycle state machine
In `src/bot.rs`:
- Added `MakerOrderKey` (`asset_id`, `side`) at line ~138.
- Added `MakerOrderLifecycle` (`Idle | SubmitPending | Working | CancelPending`) at line ~153.
- Added `MakerOrderSlot` (state/order/price/size/remaining/timestamps/origin/replace target) at line ~169.
- Added maps:
  - `maker_order_slots` (key -> slot)
  - `maker_order_index` (order_id -> key)

### 2) Centralized maker GTC gate (single in-flight policy)
In `src/bot.rs`:
- Added `_maker_order_upsert_gtc(...)` at line ~4566.
- Enforced:
  - no new submit while `SubmitPending` within TTL,
  - no new submit while `CancelPending` within TTL,
  - replace via cancel-first (no blind submit),
  - submit only when slot is safe.

### 3) Rewired submit paths to use the gate
In `src/bot.rs`:
- Base seed path now uses gate (line ~5591).
- Pair-arb maker leg submits now use gate (lines ~5139-5140).
- Replace path delegates to gate (line ~9366).
- Ladder/strategy cancel flow moved toward gate-managed cancel API (`_maker_cancel_strategy_orders`, line ~4236).

### 4) Event-driven lifecycle updates + duplicate defense
In `src/bot.rs`:
- `_maker_order_on_user_event(...)` at line ~4398.
- Event hook connected in `_handle_user_order_event` (line ~7524).
- If duplicate working OID is detected while max active is 1, duplicate is canceled instead of adopted (`max_active` checks around line ~4512).

### 5) Exchange-truth reconciliation + deterministic de-dup
In `src/bot.rs`:
- `_maker_order_reconcile_asset(...)` at line ~4248.
- Before/around submits, reconcile open exchange orders for the asset and enforce max active BUY orders (`MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET`, line ~4257).
- Keep one canonical order (tracked or closest intended price), cancel extras.

### 6) Effective inventory now includes working open BUY remainder
In `src/bot.rs`:
- `_maker_order_open_buy_remaining(...)` at line ~4057.
- Seed sizing logic optionally uses effective quantity with open BUYs (lines ~5511 onward).
- Fixed overcount risk by using `remaining` as authoritative when available.

### 7) Additional hardening after latest log review
In `src/bot.rs`:
- Added transient-missing protection in reconcile:
  - `_maker_working_missing_ttl_seconds()` (line ~4035)
  - Do not clear `Working` slot immediately if exchange list is temporarily empty right after submit/cancel churn (lines ~4358-4368).
- Added cancel-throttle guard:
  - If already `CancelPending` and still within cancel TTL, skip repeated cancel requests (lines ~4179-4183).

### 8) Config keys added
In `src/env_contract.rs`:
- `MAKER_SINGLE_INFLIGHT_PER_SIDE`
- `MAKER_SUBMIT_PENDING_TTL_SECONDS`
- `MAKER_CANCEL_PENDING_TTL_SECONDS`
- `MAKER_WORKING_MISSING_TTL_SECONDS`
- `MAKER_REPLACE_MIN_INTERVAL_SECONDS`
- `MAKER_EFFECTIVE_Q_INCLUDE_OPEN_BUYS`
- `MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET`

## Suggested Runtime Defaults
- `MAKER_SINGLE_INFLIGHT_PER_SIDE=true`
- `MAKER_SUBMIT_PENDING_TTL_SECONDS=6.0`
- `MAKER_CANCEL_PENDING_TTL_SECONDS=3.0`
- `MAKER_WORKING_MISSING_TTL_SECONDS=12.0`
- `MAKER_REPLACE_MIN_INTERVAL_SECONDS=0.5`
- `MAKER_EFFECTIVE_Q_INCLUDE_OPEN_BUYS=true`
- `MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET=1`

## Validation Checklist
1. For each asset/side, exchange open BUY maker orders never exceed 1 after reconcile settles.
2. No `LIMIT_GTC_POSTONLY` burst on same asset before previous lifecycle resolves.
3. Delayed-fill scenarios do not produce exposure jumps from duplicate stacked maker orders.
4. No force-flatten triggered due to duplicate-order artifact.
5. Risk controls still trigger when true risk breaches occur.

## Notes for Next AI Model
- Primary files to inspect: `src/bot.rs`, `src/env_contract.rs`.
- Key methods: `_maker_order_upsert_gtc`, `_maker_order_reconcile_asset`, `_maker_order_on_user_event`, `_maker_order_request_cancel`, `_maker_order_open_buy_remaining`.
- Main remaining watchpoint: repeated cancel-request churn for same OID under quote-invalidation loops; ensure cancel state + exchange ACK transitions remain stable.
