# IMP-11 Plan: Exact Late-Window Cutoff and Balance-Only Modes

## Summary
Implement `REQ-021` with exact late-window policy using env-configurable thresholds that default to `180 / 225 / 240`.

Chosen decisions:
- `180s`: enter reduced-clip late trading
- `225s`: switch to `balance_only`
- `240s`: immediately hand off into `AwaitSettlement`
- thresholds are env-configurable, but the requirement defaults remain `180 / 225 / 240`

This replaces the current loose `taper_start_seconds` / `final_quiet_seconds` behavior with explicit late-window phases that match the requirement.

## Key Changes
### 1. Runtime timing and mode model
- Add authoritative late-window config fields:
  - `late_reduce_start_seconds = 180`
  - `late_balance_only_start_seconds = 225`
  - `late_stop_new_orders_start_seconds = 240`
- Add env keys:
  - `BOT_LATE_REDUCE_START_SECONDS`
  - `BOT_LATE_BALANCE_ONLY_START_SECONDS`
  - `BOT_LATE_STOP_NEW_ORDERS_START_SECONDS`
- Validate:
  - `30 < late_reduce < late_balance_only < late_stop_new_orders <= 300`
- Make these new fields replace `taper_start_seconds` / `final_quiet_seconds` as the control inputs for late behavior.
- Update phase routing:
  - `OpenBoth`: `< 30s`
  - `PairBuild`: `30s` to `< late_reduce_start_seconds`
  - `Taper`: `late_reduce_start_seconds` to `< late_stop_new_orders_start_seconds`
  - `AwaitSettlement`: `>= late_stop_new_orders_start_seconds`
- Replace taper mode semantics with exact requirement modes:
  - `ReduceClips`
  - `BalanceOnly`

### 2. Late-window behavior
- `180s` to `<225s` (`ReduceClips`)
  - paired growth remains possible, but only at the existing maintenance one-lot clip
  - no normal `20/40/80` paired-growth clips in this band
  - lighter-side repair remains allowed under the existing imbalance, price-zone, taker-share, and residual-direction guards
  - keep the current late floor/tail policy as an additional veto
- `225s` to `<240s` (`BalanceOnly`)
  - block all `PairedGrowth` with a stable late hold reason
  - allow only `LighterSideFirst` repair actions that reduce imbalance and pass all existing `IMP-06`, `IMP-07`, `IMP-08`, and `IMP-10` guards
  - cancel any live paired-growth BOT orders when entering this band
- `>=240s`
  - phase owner becomes `AwaitSettlement`
  - reuse the existing `IMP-03` settlement handoff and order-drain path
  - no BOT create path may emit a new order after this point

### 3. Observability and metrics
- Update runtime logs to emit exact late mode and reason codes instead of the current loose taper wording.
- Replace misleading `after_240/270` late counters with explicit counters for:
  - fills after `180s`
  - fills after `225s`
  - new orders after `225s`
  - new orders after `240s`
- Final metrics should clearly prove:
  - reduced-clip late participation
  - balance-only behavior after `225s`
  - zero create events after `240s`

## Important Interface Changes
- `BotRuntimeConfigSnapshot` gains the three explicit late-window thresholds above.
- `BotRuntimeTaperMode` becomes requirement-shaped late modes:
  - `ReduceClips`
  - `BalanceOnly`
- `bot_runtime_phase_from_t_into_s(...)` changes its `Taper` / `AwaitSettlement` boundaries so `AwaitSettlement` starts at the configured stop-new-orders cutoff.

## Test Plan
- Config tests:
  - defaults are `180 / 225 / 240`
  - invalid ordering fails validation
- Phase and mode tests:
  - boundaries at `179.9 / 180 / 224.9 / 225 / 239.9 / 240`
  - owner routing enters `AwaitSettlement` at `240`
- Runtime behavior tests:
  - paired growth after `180` is downshifted to the maintenance clip
  - paired growth is blocked after `225`
  - lighter-side repair remains legal after `225` when it reduces imbalance
  - live paired-growth orders are canceled when `BalanceOnly` begins
  - no BOT order create events occur after `240`
  - existing `AwaitSettlement` order drain still clears working orders after the `240` handoff
- Regression tests:
  - `AwaitSecondFill` logic remains unchanged except for the earlier global handoff at `240`
  - existing price-zone, imbalance, taker-cap, and residual-direction guards still apply inside late repair mode

## Assumptions and Defaults
- The new late-window thresholds are env-configurable, but `180 / 225 / 240` are the requirement defaults.
- `240s` is the new terminal non-trading cutoff and immediately enters `AwaitSettlement`.
- Late paired-growth size reduction uses the existing maintenance one-lot clip.
- Late repair sizing is not forcibly shrunk before `225`; it stays governed by the normal repair policy because it reduces risk rather than creating fresh paired exposure.
