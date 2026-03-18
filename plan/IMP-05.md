# IMP-05 Plan: AwaitSecondFill and No Scale Before Both Filled

## Summary
Implement `REQ-007` and `REQ-008` by turning the current startup completion logic into an explicit `AwaitSecondFill` mode. The bot keeps the exact-open seeding from `IMP-04`, but once the first local fill lands on only one side it freezes any further adds on the filled side, works only the missing side, measures completion from first-fill time, and only unlocks normal accumulation after both sides have at least one fill.

This task will enforce three hard behaviors:
- no scale-up before both sides have a fill
- second-side completion is measured against `15s` and `30s` from first fill, not from market open
- after `30s`, the bot gets exactly one missing-side taker rescue, then permanently stops startup accumulation for that market if the pair is still one-sided

## Key Changes
### Runtime state and routing
- Rename `BotRuntimeControlOwner::SeedCompletion` to `AwaitSecondFill`.
- Keep the coarse phase model from `IMP-04`; do not add the full generic `BalanceOnly` / `Paused` phase system yet.
- Add `BotRuntimeState` fields for:
  - `await_second_fill_started_ts`
  - `await_second_fill_missing_side`
  - `await_second_fill_target_missed_ts`
  - `await_second_fill_second_fill_ts`
  - `await_second_fill_rescue_used`
  - `await_second_fill_rescue_attempted_ts`
  - `await_second_fill_hard_paused`
  - `second_side_by_15s`
  - `second_side_by_30s`
  - `first_fill_to_second_fill_ms`
- Detect first-fill timing from local inventory after fills are applied:
  - first transition to exactly one positive side latches `await_second_fill_started_ts`
  - first transition of the missing side from zero to positive latches `await_second_fill_second_fill_ts`
  - if both sides become positive in the same observation, skip `AwaitSecondFill` and hand off directly to `PairBuild`

### AwaitSecondFill behavior
- On entering `AwaitSecondFill`, cancel any live normal BOT order on the already-filled side and suppress all replacements on that side.
- While either side has zero filled quantity, normal accumulation is forbidden:
  - `PairBuild` and `Taper` must early-return and create no new size
  - only `OpenBoth`, `AwaitSecondFill`, and the one taker rescue may submit orders
- Before `first_fill + 15s`:
  - keep the current maker-first missing-side completion behavior
  - keep only the missing-side completion order live
- At `first_fill + 15s`:
  - mark `second_side_by_15s = false` if the missing side is still zero
  - stay in degraded `AwaitSecondFill` mode
  - continue missing-side-only completion, but still do not allow normal accumulation
- At `first_fill + 30s`:
  - cancel any live missing-side maker completion order
  - if the pair is still one-sided and no rescue was used, allow exactly one taker buy on the missing side
  - rescue size is `min(current completion repair size, unmatched filled quantity, visible ask size)`
  - rescue is allowed only if it improves balance and the carried average cost of the filled side plus the missing-side taker price keeps marginal pair sum `< 1.00`
  - rescue is one shot only, with no retry loop
- If the rescue is skipped, creates no fill, or the pair remains one-sided afterward:
  - set `await_second_fill_hard_paused = true`
  - stop all new startup accumulation for the rest of the market
  - continue only passive monitoring, fill reconciliation, and later settlement handling for the residual

### Metrics, logs, and interfaces
- Keep `IMP-04` submit KPIs unchanged; add separate fill-completion KPIs.
- Replace control-flow use of the legacy `both_by_30s` / `both_by_60s` completion checks with:
  - `second_side_by_15s`
  - `second_side_by_30s`
  - `first_fill_to_second_fill_ms`
  - `await_second_fill_rescue_used`
  - `await_second_fill_hard_paused`
- Emit stable logs for:
  - first-side fill detected
  - second-side completion detected
  - 15-second target missed
  - 30-second hard deadline reached
  - taker rescue attempted / skipped / succeeded / failed
  - hard pause engaged
- Rename the missing-side startup order origin from `BOT_SEED_COMPLETION` to `BOT_AWAIT_SECOND_FILL` so logs and execution metadata match the new lifecycle.
- No DB schema changes in `IMP-05`.
- No new env/config knobs in `IMP-05`; `15s`, `30s`, and the single-rescue policy are fixed requirement constants.

## Test Plan
- One-sided first fill routes to `AwaitSecondFill` and latches first-fill time.
- Entering `AwaitSecondFill` cancels any live filled-side startup order.
- `PairBuild` and `Taper` submit nothing while one side still has zero fill.
- Second side completes within `15s` and the runtime hands off to `PairBuild`.
- Second side completes after `15s` but before `30s`; `second_side_by_15s = false`, `second_side_by_30s = true`, and normal accumulation starts only after both sides are actually filled.
- One-sided state at `30s` triggers exactly one taker rescue on the missing side when the projected marginal pair sum stays below `1.00`.
- Rescue is skipped when it would not improve balance, when no ask is available, or when projected marginal pair sum is `>= 1.00`.
- After a failed or skipped rescue, the pair becomes hard-paused and no further startup accumulation orders are emitted.
- If both sides first fill in the same loop tick, `AwaitSecondFill` is skipped and both completion KPIs are marked true.
- Legacy `both_by_30s` / `both_by_60s` no longer drive behavior.

## Assumptions and defaults
- “First fill” means the first locally applied fill that makes exactly one side positive; the timer is not anchored to submit time.
- The `15s` and `30s` thresholds are measured from `await_second_fill_started_ts`, not market open.
- The single `30s` rescue is buy-only and only targets the missing side.
- Once `await_second_fill_hard_paused` is engaged, the market does not re-enter normal accumulation later in the same market; late fills are only absorbed into inventory.
- The broader generic `BalanceOnly` and `Paused` phase system remains future work; `IMP-05` delivers the required behavior inside the existing runtime with an explicit `AwaitSecondFill` owner and hard-stop policy.
