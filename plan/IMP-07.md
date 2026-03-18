# IMP-07 Plan: Exact Marginal Pair-Cost Zones

## Summary
Implement `REQ-011`, `REQ-012`, and `REQ-013` by making one exact marginal-cost evaluation the authoritative price gate for every size-increasing BOT decision.

The rule set will be:
- `Preferred`: `< 0.94`
- `Acceptable`: `0.94` to `< 0.97`
- `Caution`: `0.97` to `< 1.00`
- `StopAdd`: `>= 1.00` to `< 1.03`
- `Danger`: `>= 1.03` or non-finite

In MVP, any size-increasing intent in `StopAdd` or `Danger` is blocked. `cpp_hint`, `inventory_vwap_sum`, and `market_snapshot_vwap_sum` remain as secondary telemetry or pacing hints only; they stop being the authoritative price-discipline gate.

## Key Changes
### 1. Canonical marginal-cost model
- Add one shared helper in `pair_build/costs.rs` that computes:
  - `BalancedAdd`: `effective_marginal_pair_cost = y_bid + n_bid`
  - `RebalanceAdd`: `effective_marginal_pair_cost = residual_unit_cost + lagging_side_bid`
- For `RebalanceAdd`, define `residual_unit_cost` as the carried average paid on the heavier side: `cost_heavy / q_heavy`.
- Add one shared classifier that maps the effective marginal cost into the exact requirement zones above.
- Keep the existing 5-slot occupancy arrays, but relabel the band semantics to the requirement zones instead of the old `strong/normal/reduced/repair/freeze` model.

### 2. Decision and telemetry payloads
- Extend `BotRuntimePairBuildDecision` with:
  - `marginal_cost_mode` (`BalancedAdd` or `RebalanceAdd`)
  - `effective_marginal_pair_cost`
  - `price_zone`
  - `residual_unit_cost: Option<f64>`
  - `lagging_side_quote: Option<f64>`
- Keep `pair_sum` on the decision only as the balanced-add quote sum field.
- Keep `cpp_hint`, `inventory_vwap_sum`, and `market_snapshot_vwap_sum` on the decision for telemetry, but do not use them to authorize or deny size-increasing orders.
- Update pair-build and taper submit logs so every approved or blocked decision emits:
  - `price_zone`
  - `effective_marginal_pair_cost`
  - `marginal_cost_mode`
  - for balanced adds: `yes_quote`, `no_quote`, `marginal_pair_sum`
  - for rebalance adds: `residual_unit_cost`, `lagging_side_quote`, `heavier_side`

### 3. PairBuild and Taper behavior
- In `decision.rs`, paired growth is evaluated only from `BalancedAdd` marginal cost.
- In `decision.rs`, lighter-side repair is evaluated only from `RebalanceAdd` effective marginal cost.
- Block any paired growth or lighter-side repair when the zone is `StopAdd` or `Danger`.
- Replace old price hold reasons like `pair_sum_too_high`, `lighter_side_completion_core_too_expensive`, and projected-inventory-cost band holds with stable zone-based reasons:
  - `price_zone_stop_add:balanced_add:<cost>`
  - `price_zone_danger:balanced_add:<cost>`
  - `price_zone_stop_add:rebalance_add:<cost>`
  - `price_zone_danger:rebalance_add:<cost>`
- Remove projected inventory-VWAP cost bands from price authorization. They may remain in telemetry, but they must not block a zone-valid order or allow a zone-invalid order.
- Keep taper using the same shared decision path, so late optional adds and lighter repairs inherit the same price-zone rules automatically.

### 4. Observability and compatibility
- Update `BotRuntimePairedCostBand` labels and all summaries in `metrics.rs` and loop metrics to the requirement zones:
  - `preferred`, `acceptable`, `caution`, `stop_add`, `danger`
- Update `paired_cost_observation` and `paired_size_delta_by_state` to track the new zone semantics.
- Preserve the existing metrics field names to avoid wider churn; only the zone meaning and labels change.
- No DB schema changes and no new env/config knobs in `IMP-07`. These thresholds are fixed MVP constants; config-versioned threshold control stays for `IMP-12`.

## Important Interface Changes
- `BotRuntimePairedCostBand` becomes the exact requirement-zone enum.
- `BotRuntimePairBuildDecision` gains `marginal_cost_mode`, `effective_marginal_pair_cost`, `price_zone`, `residual_unit_cost`, and `lagging_side_quote`.
- `BotRuntimePairedGrowthPolicy.projected_paired_cost` becomes the submitted marginal-cost value for the approved paired add, and its `band` uses the new exact zone semantics.

## Test Plan
- Zone-classifier boundary tests:
  - `0.939 -> Preferred`
  - `0.94 -> Acceptable`
  - `0.97 -> Caution`
  - `1.00 -> StopAdd`
  - `1.03 -> Danger`
- Balanced-add tests:
  - `y_bid + n_bid < 1.00` allows paired growth
  - `y_bid + n_bid >= 1.00` blocks paired growth
  - high `inventory_vwap_sum` with cheap next-unit quotes still allows growth if marginal cost is `< 1.00`
- Rebalance-add tests:
  - heavier-side carried average plus lagging bid is used as the effective repair cost
  - repair is blocked at `>= 1.00` even if it improves average inventory VWAP
  - repair is allowed below `1.00` even when legacy projected-VWAP heuristics would have blocked it
- Taper tests:
  - taper paired adds respect the same zone gate
  - taper lighter-side repair respects the same rebalance zone gate
- Metrics/logging tests:
  - zone labels appear in summaries and submit logs
  - approved decisions carry the correct mode and exact cost inputs
- Regression tests:
  - no BOT size-increasing intent is approved with `effective_marginal_pair_cost >= 1.00`
  - `AwaitSecondFill` rescue behavior remains unchanged unless it adopts the shared classifier without behavior change

## Assumptions and Defaults
- Use the passive maker buy quotes already used by the runtime: `y_bid` and `n_bid`.
- For repair, use heavier-side carried average paid as the residual-unit cost because the current ledger is aggregate, not lot-level.
- `cpp_hint` stays in place for non-price-authoritative pacing or clip shaping until `IMP-09`, but it cannot override the price-zone gate.
- `AwaitSecondFill` rescue is not refactored in this task; if the shared zone helper is reused there, behavior must stay identical to the current `>= 1.00` guard.
