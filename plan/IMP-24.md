# IMP-24 Plan: Section 10 Paper and Shadow KPI Gates

## Summary
- Add a dedicated KPI-gate subsystem that evaluates `paper` and effective-`shadow` BOT runs from structured audit records and settled trade rows.
- Keep trading behavior unchanged except for small observability-only audit enrichments needed to compute the Section 10 KPIs exactly and deterministically.
- Produce deterministic JSON summaries plus persisted KPI-gate rows, but stop short of automatic rollout blocking. `IMP-25` will consume these outputs for actual go/no-go enforcement.

## Important Changes
- Add `src/kpi_gate/` with:
  - a `KpiEventSource` trait for loading `trade`, `trade_decision_events`, and `trade_runtime_events`
  - metric extractors
  - policy evaluation
  - deterministic report types and JSON writer
- Add `src/bin/kpi_gate.rs` with:
  - `cargo run --bin kpi_gate -- --bot-id <id> --profile <paper|shadow> --start <iso> --end <iso> [--output-dir <path>]`
  - `DB_URL` reuse only; no new supported operator env
- Add Postgres persistence in `src/db.rs` for:
  - `kpi_gate_run`
  - `kpi_gate_metric`
- Add observability-only audit enrichments:
  - a terminal `run_summary` runtime event after post-run settlement finalization, carrying the current `BotRuntimeMetricsSnapshot` values, exit reason, entry reason, configured/effective order mode, and terminal settlement status
  - an `audit_drop` runtime event whenever decision/runtime audit enqueue or insert is dropped or fails
  - settlement payload enrichment with `paired_realized_pnl`, `residual_realized_pnl`, `paired_qty`, and `residual_qty`

## Implementation Changes
### Data Model and Evaluation Surface
- Use `trade_decision_events` and `trade_runtime_events` as the authoritative behavioral source.
- Use `trade` rows only for settled-run economics and reconciliation status.
- Filter the requested window by trade timestamps, then filter the profile by `run_summary.effective_order_mode`.
- Treat `shadow` profile as all runs whose effective mode stayed `shadow`, including configured-live runs temporarily downgraded to shadow.

### Sample Coverage and Overall Status
- Compute `distinct_trading_days` from persisted `trade.date`.
- Compute `settled_pairs` from trade rows with terminal settlement status.
- Use overall status precedence: `FAIL` > `INSUFFICIENT_SAMPLE` > `WARN` > `PASS`.
- Paper sample sufficiency is conservative: require both `distinct_trading_days >= 10` and `settled_pairs >= 500`.
- Shadow sample sufficiency requires `distinct_trading_days >= 3`.

### Paper KPI Definitions
- `seed_timing`:
  - denominator is runs with opening participation
  - use `run_summary.open_both_submit_delta_met` as the primary compliance flag
  - require compliance on at least `99%` of entered pairs
  - also report `open_both_seed_by_deadline_met`, worst seed delta, and deadline-miss count
- `no_scale_up_before_both_sides_filled`:
  - fail on any approved decision while `owner = "AwaitSecondFill"` unless it is the explicit second-side rescue path
  - explicit rescue is `one_side_exception_kind = "AwaitSecondFillRescue"` or the matching rescue entry reason
- `unmatched_fraction`:
  - compute per-run final unmatched fraction from `run_summary.unmatched_fraction`
  - require median `< 0.07`, p95 `< 0.12`, and max `< 0.20`
- `price_discipline`:
  - fail on any approved decision with `effective_marginal_pair_cost >= 1.00 - 1e-9`
- `underdog_residual`:
  - fail on any approved decision with `increases_underdog_residual = true`
- `taker_share`:
  - aggregate `fill` runtime events by `is_maker`
  - warn at `>= 0.05`, fail at `>= 0.10`
  - also report max daily share by UTC trade date
- `single_side_speculation`:
  - fail on any participating run with fills but `paired_qty <= 0`
- `settlement_reconciliation`:
  - all participating runs must have terminal settlement runtime events
  - all settled trade rows must match those settlement events
- `pnl_decomposition`:
  - compute `paired_qty = min(q_yes, q_no)`
  - compute `residual_qty = abs(q_yes - q_no)`
  - compute `paired_cost = 2 * paired_qty * cpp`
  - compute `paired_realized_pnl = paired_qty - paired_cost`
  - compute `residual_realized_pnl = lp - paired_realized_pnl`
  - require sample `paired_realized_pnl > 0`
  - require `abs(min(residual_realized_pnl, 0)) <= 0.5 * paired_realized_pnl`

### Shadow KPI Definitions
- `adapter_recovery`:
  - every websocket-related dependency pause must be followed by reconciliation returning healthy within the same run
- `startup_reconciliation`:
  - fail any run that ends with unresolved startup or reconnect reconciliation
- `decision_logging_integrity`:
  - fail on any `audit_drop`
  - fail if `run_summary.audit_decision_event_count` or `audit_runtime_event_count` disagrees with the rows loaded for that run
- `state_machine_progress`:
  - fail on `run_summary.await_second_fill_hard_paused = true`
  - fail on `run_summary.startup_completion_blocked_count > 0`
  - fail on an unresolved nonterminal safety gate or owner at the end of a participating run
  - flag `AwaitSecondFill` dwell over `30s` as a failure
- `hypothetical_price_and_imbalance_compliance`:
  - fail on any approved shadow decision with `effective_marginal_pair_cost >= 1.00 - 1e-9`
  - fail on any approved shadow decision with `price_zone in {"stop_add","danger"}`
  - fail on any approved shadow decision with `imbalance_state = "HardDisable"`
- `hypothetical_underdog_residual`:
  - fail on any approved shadow decision with `increases_underdog_residual = true`
- `settlement_observation`:
  - all participating shadow runs must emit terminal settlement events and match settled trade rows when settlement exists

### Report and Persistence
- Write deterministic JSON to `output/kpi_gate/<bot_id>/<profile>/<window>/summary.json`.
- Report sections are:
  - `metadata`
  - `sample_coverage`
  - `source_counts`
  - `metrics`
  - `evaluation`
  - `overall_status`
- Serialize floating-point metrics at fixed `6` decimal places.
- Write the JSON file first, then persist DB rows, matching the safer pattern used in `IMP-17`.

## Test Plan
- Unit tests for:
  - sample sufficiency
  - seed-timing compliance rate
  - second-fill dwell extraction
  - unmatched-fraction quantiles
  - price-zone and underdog-residual violation detection
  - taker-share aggregation
  - PnL decomposition helper
  - audit-drop and logging-gap detection
- Integration tests with an in-memory `KpiEventSource`:
  - clean paper sample that passes
  - paper sample with an approved `>= 1.00` add that fails
  - paper sample with bad unmatched-fraction distribution that fails
  - shadow sample with disconnect then successful reconciliation that passes recovery
  - shadow sample with `audit_drop` that fails logging integrity
  - shadow sample with unresolved reconnect or `AwaitSecondFill` stall that fails progress
  - insufficient paper and shadow samples that report `INSUFFICIENT_SAMPLE`
- Replay-backed integration coverage:
  - reuse the committed replay certification scenarios for stale, reconnect, and settlement semantics
  - add dedicated synthetic multi-run fixtures for sample-size and aggregate KPI math
- Optional ignored Postgres smoke test:
  - persist one KPI report and verify `kpi_gate_run` and `kpi_gate_metric`

## Assumptions
- No new supported operator env is introduced; `DB_URL` remains the only CLI dependency.
- `trade.date` is the v1 trading-day key and UTC date semantics are sufficient.
- Paper sufficiency is intentionally conservative and requires both the day and settled-pair thresholds.
- `paired_realized_pnl` and `residual_realized_pnl` are net-of-fees v1 values derived from persisted `lp` and `cpp`; if explicit fee fields become available later, the helper should preserve `paired + residual + fees = lp`.
- `IMP-24` produces measurement and status only; `IMP-25` is responsible for using those results as actual rollout gates.
