# IMP-08 Plan: Maker-First Enforcement and Taker-Share Caps

## Summary
Implement `REQ-014` and `REQ-015` by making maker intent the default for all normal BOT strategy flow, requiring explicit reasoned exceptions for any taker submit, and enforcing taker-share limits at both market and daily scope.

This task will use:
- market-level taker accounting in runtime state
- daily taker accounting persisted in `BotState`
- UTC calendar day for the daily cap window
- bypass only for recovery/manual taker flows

The policy will be:
- normal BOT strategy is maker-first and post-only when supported
- approved BOT taker exceptions must carry a reason
- taker-share target `< 5%` is observability/warn only
- `>= 10%` market or daily taker share blocks new normal taker exceptions
- recovery/manual taker flows may bypass the cap, but must be explicitly tagged and logged

## Key Changes
### Execution intent and taker exceptions
- Add a first-class liquidity intent model to execution metadata:
  - `maker`
  - `taker_exception`
- Add stable taker-exception reasons:
  - `await_second_fill_rescue`
  - `rebalance_repair`
  - `recovery_bypass`
- Update taker submit entrypoints in `src/bot/execution/submit.rs` so every taker order requires:
  - explicit exception reason
  - whether the order is cap-enforced or recovery-bypass
- Keep maker submit paths passive:
  - normal BOT maker orders continue using post-only where supported
  - if post-only is unsupported or rejected, do not silently escalate to taker in normal BOT flow

### Market and daily taker-share accounting
- Extend `BotRuntimeState` and `BotRuntimeMetricsSnapshot` with explicit taker counters:
  - `taker_fill_events`
  - `taker_fill_shares`
  - `pair_taker_share`
  - `daily_maker_fill_shares`
  - `daily_taker_fill_shares`
  - `daily_taker_share`
- Extend persisted `BotState` with UTC-day aggregate fields:
  - `taker_day_key_utc`
  - `daily_maker_fill_shares`
  - `daily_taker_fill_shares`
- Reset daily counters automatically when the current UTC day key changes.
- Keep old state files compatible by defaulting new fields to zero and normalizing missing values on load.

### Taker-cap gating
- Add shared helpers for:
  - `pair_taker_share = taker_qty / (maker_qty + taker_qty)` if total > 0 else `0`
  - `daily_taker_share = daily_taker_qty / (daily_maker_qty + daily_taker_qty)` if total > 0 else `0`
  - projected pair/day taker share using current filled qty plus pending/requested taker qty
- Add a pending-taker-quantity helper alongside existing pending-taker notional tracking.
- Enforce:
  - `< 5%` target: warning/metrics only
  - `>= 10%` projected or current pair taker share: block new normal taker exceptions
  - `>= 10%` projected or current daily taker share: block new normal taker exceptions
- Recovery/manual taker flows may bypass both caps only with `recovery_bypass`.

### Runtime wiring and observability
- Gate the existing startup taker rescue in `src/bot/runtime/startup.rs` through the new taker-cap policy using `await_second_fill_rescue`.
- Gate any BOT rebalance taker path through the same policy using `rebalance_repair`.
- Keep recovery/manual taker flows in recovery paths as bypass-only.
- Route maker/taker fill accounting through the existing observed-fill hook so metrics stay consistent.
- Extend execution and runtime logs with:
  - `liquidity_intent`
  - `taker_exception_reason`
  - `pair_taker_share`
  - `daily_taker_share`
  - hold reasons:
    - `taker_cap_market`
    - `taker_cap_daily`
    - `taker_exception_reason_missing`
    - `taker_exception_reason_disallowed`

## Important Interface Changes
- `BotState` gains UTC-day daily maker/taker fill counters.
- `BotRuntimeState` and `BotRuntimeMetricsSnapshot` gain explicit market and daily taker-share fields.
- Taker submit APIs now require an explicit exception reason and bypass classification.
- Execution context and structured logs gain `liquidity_intent` and `taker_exception_reason`.

## Test Plan
- Share math tests:
  - pair and daily taker share compute correctly at `0`, normal mixed fills, and cap boundaries
  - projected share uses pending/requested taker size conservatively
- Persistence tests:
  - old state files load with zeroed daily counters
  - UTC day rollover resets daily counters and preserves market-local state
- Policy tests:
  - maker orders remain allowed without taker reason
  - taker submit fails without an exception reason
  - taker submit fails with a disallowed reason in normal BOT flow
  - startup rescue is allowed below cap and blocked at projected `>= 10%`
  - rebalance repair is allowed below cap and blocked at projected `>= 10%`
  - daily cap blocks new normal taker exceptions even if market cap is still below limit
  - recovery/manual taker bypass remains allowed above cap and emits explicit warning metadata
- Fill-accounting tests:
  - maker fills increment maker counters only
  - taker fills increment taker counters only
  - runtime metrics expose `pair_taker_share` and `daily_taker_share`
- Regression tests:
  - normal maker-first startup and pair-build behavior stays unchanged
  - no normal BOT taker callsite can bypass the cap without `recovery_bypass`

## Assumptions and Defaults
- Daily aggregate cap resets on UTC calendar day.
- The 10% hard cap applies only to normal BOT strategy takers.
- Recovery/manual taker flows are the only bypass-cap category in this task.
- Cap gating uses filled quantity plus pending/requested taker quantity for conservative projected enforcement.
- No DB schema changes are needed for `IMP-08`; daily aggregate persistence lives in `BotState`.
