# IMP-06 Plan: Exact Unmatched-Fraction Risk Controls

## Summary
Implement `REQ-009` and `REQ-010` by making unmatched inventory fraction the authoritative imbalance metric for runtime risk decisions.

Per your choice, this stays inside the current phase model: no new top-level `BalanceOnly` or `Disabled` phases now. Instead, `PairBuild` and `Taper` will use an explicit imbalance state to decide whether paired growth is allowed, whether only lagging-side repair is allowed, or whether the pair is hard-disabled for the rest of the market.

The new canonical thresholds will be:
- `Normal`: unmatched fraction `< 0.07`
- `Throttle`: unmatched fraction `>= 0.07` and `<= 0.12`
- `Warning`: unmatched fraction `> 0.12` and `< 0.20`
- `HardDisable`: unmatched fraction `>= 0.20`

`pair_coverage` and `share_skew_ratio` stay as telemetry, but they stop being the load-bearing imbalance gates.

## Key Changes
### 1. Canonical imbalance math and config
- Add to `BotRuntimeConfigSnapshot`:
  - `imbalance_target_fraction = 0.07`
  - `imbalance_warning_fraction = 0.12`
  - `imbalance_disable_fraction = 0.20`
- Add env keys:
  - `BOT_IMBALANCE_TARGET_FRACTION`
  - `BOT_IMBALANCE_WARNING_FRACTION`
  - `BOT_IMBALANCE_DISABLE_FRACTION`
- Validate `0 < target < warning < disable <= 1`.
- Add pure helpers for:
  - `unmatched_fraction = abs(q_yes - q_no) / (q_yes + q_no)` if total > 0 else `0`
  - `match_ratio = min(q_yes, q_no) / max(q_yes, q_no)` if max > 0 else `1`
  - imbalance-state classification from current inventory
  - projected unmatched fraction after a candidate order
  - whether a candidate order reduces imbalance

### 2. Runtime state and decision payloads
- Add `BotRuntimeImbalanceState { Normal, Throttle, Warning, HardDisable }`.
- Add to `BotRuntimeState`:
  - `imbalance_state`
  - `imbalance_state_enter_ts`
  - `imbalance_last_hold_reason`
- Extend `BotRuntimeMetricsSnapshot` with:
  - `unmatched_fraction`
  - `match_ratio`
  - `imbalance_state`
- Extend `BotRuntimePairBuildDecision` with:
  - `current_unmatched_fraction`
  - `projected_unmatched_fraction`
  - `match_ratio`
  - `imbalance_state`
  - `reduces_imbalance`
- Keep `pair_coverage` and `skew_ratio` in metrics and logs as secondary telemetry only.

### 3. PairBuild and Taper behavior
- Compute current imbalance state before planning any size-increasing action.
- If current state is `HardDisable`:
  - cancel BOT-owned pair-build and taper orders
  - emit a stable `hard_imbalance_disable` reason
  - submit no new orders for the rest of the market
  - continue passive monitoring and later settlement only
- If current state is `Throttle` or `Warning`:
  - do not allow normal paired growth
  - cancel any live paired-growth orders
  - allow only lighter-side / lagging-side repair that reduces imbalance
- Any candidate order must compute projected unmatched fraction before submit.
- Block any candidate order whose projected unmatched fraction is `>= 0.20`, even if current state is still below hard-disable.
- `bot_runtime_pair_build_materially_skewed(...)` stops being the authoritative switch for leaving normal accumulation.
  - paired growth eligibility now requires `current_unmatched_fraction < 0.07`
  - lighter-side repair remains allowed only when it improves imbalance
- `Taper` inherits the same imbalance gating:
  - no paired optional adds when state is not `Normal`
  - repair-only while in `Throttle` or `Warning`
  - no new orders in `HardDisable`

### 4. Logging and observability
- Update pair-build, taper, and loop metrics/logs to include:
  - `unmatched_fraction`
  - `projected_unmatched_fraction`
  - `match_ratio`
  - `imbalance_state`
  - whether the approved order reduces imbalance
- Emit stable transition or block reasons:
  - `imbalance_throttle`
  - `imbalance_warning`
  - `projected_hard_imbalance_block`
  - `hard_imbalance_disable`
- No DB schema changes in `IMP-06`; persisted decision/audit detail remains `IMP-13`.

## Important Interface Changes
- `runtime/config.rs`: add imbalance thresholds to the runtime config snapshot and validation.
- `runtime/state.rs`: add `BotRuntimeImbalanceState`, runtime state tracking, and metric/decision fields for unmatched fraction.
- `runtime/metrics.rs` and `runtime/pair_build/*`: compute and propagate current/projected unmatched fraction and match ratio into logs, decisions, and gating.
- `runtime/taper_runtime.rs`: apply the same imbalance-state gate as `PairBuild`.

## Test Plan
- Helper tests:
  - unmatched fraction is `0` when both sides are zero
  - unmatched fraction and match ratio are correct for balanced, one-sided, and partially imbalanced inventories
  - projected unmatched fraction is correct for paired-growth and lighter-side repair cases
  - config validation rejects invalid threshold ordering
- PairBuild tests:
  - `< 7%` can still approve paired growth when price rules pass
  - `>= 7%` suppresses paired growth and routes to repair-only behavior
  - `> 12%` reports `Warning` and still forbids normal paired growth
  - current `>= 20%` cancels working BOT orders and blocks all new adds
  - current `< 20%` but projected `>= 20%` blocks the candidate order
  - a repair order that does not reduce imbalance is rejected
  - `pair_coverage` or `skew_ratio` can no longer allow paired growth once unmatched fraction is above target
- Taper tests:
  - imbalance state suppresses optional paired adds in taper
  - hard-disable in taper cancels working orders and prevents new ones
- Metrics tests:
  - runtime metrics snapshot exposes exact unmatched fraction, match ratio, and imbalance state

## Assumptions and Defaults
- Keep the current phase model; do not add generic `BalanceOnly` or `Disabled` phases in `IMP-06`.
- `Normal` requires unmatched fraction strictly below `7%`.
- `Warning` begins only above `12%`; `20%` or more is hard-disable.
- Current imbalance state controls whether the runtime may remain in normal accumulation.
- Projected unmatched fraction is the final per-order safety gate.
- `pair_coverage` and `share_skew_ratio` remain available for diagnostics, but they are no longer the policy source of truth.
- No underdog-residual logic is added here; that remains `IMP-10`.
