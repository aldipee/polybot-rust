# IMP-13 Plan: Append-Only Decision Audit and Structured Runtime Events

## Summary
Implement `REQ-024` and `REQ-025` by making audit persistence append-only and by giving every important runtime event a structured, queryable record.

Chosen design:
- add a new append-only decision table as the authoritative `REQ-025` store
- add a new append-only runtime-event table for `REQ-024`
- keep the existing `trade_decisions` row as a compatibility latest-summary mirror, not the source of truth
- use one generated `decision_event_id` to link a decision to later order intent, ack, fill, reconciliation, and settlement events

## Key Changes
### 1. Append-only persistence model
- Add `trade_decision_events` in `src/db.rs` with one row per decision attempt, never updated in place.
- Add `trade_runtime_events` in `src/db.rs` for append-only structured events:
  - `state_transition`
  - `risk_block`
  - `order_intent`
  - `order_ack`
  - `fill`
  - `reconciliation`
  - `settlement`
- Each row carries at least:
  - `event_id`
  - `trade_id`
  - `pair_id`
  - `config_version`
  - `event_kind`
  - `event_ts`
  - `decision_event_id` when applicable
  - `order_id` / `asset_id` / `side` when applicable
  - `reason_code`
  - canonical `payload_json`
- Keep `trade_decisions` as a latest-known mirror for compatibility reads, but make it explicitly non-authoritative.

### 2. Decision audit payload
- Introduce a typed decision-audit payload in the runtime layer that captures the load-bearing fields required by `REQ-025`.
- Persist, at minimum:
  - `t_into_seconds`
  - `t_left_seconds`
  - `effective_marginal_pair_cost`
  - `marginal_pair_sum` or rebalance equivalent inputs
  - `combined_avg_paid`
  - `unmatched_fraction`
  - `projected_unmatched_fraction`
  - `match_ratio`
  - `favorite_side`
  - `underdog_side`
  - `pair_taker_share`
  - `daily_taker_share`
  - `reason_code`
  - `approved_or_blocked`
  - `mode`
  - `price_zone`
  - `imbalance_state`
  - `phase` / `owner`
- Decision rows should also include pair metadata, `config_version`, and enough identifiers to join to later order and fill events.

### 3. Runtime and execution wiring
- Generate `decision_event_id` at the point a runtime decision is finalized, before any submit path.
- Carry `decision_event_id` into the existing order execution context so later submit, ack, and fill handlers can persist linked runtime events without re-deriving state.
- Persist append-only events on:
  - owner or phase transitions
  - risk holds and block reasons
  - submit intent creation
  - submit ack / post completion
  - maker and taker fills
  - reconciliation warnings or actions
  - settlement handoff and settlement completion
- Use the same pinned trade `config_version` from `IMP-12` for every decision and runtime event in that market.

### 4. Structured logs and metrics
- Add a small structured event helper in `src/logging.rs` so DB event rows and JSON logs are emitted from the same normalized payload shape.
- Keep human-readable text logs, but make JSON logs first-class and stable by event kind.
- Treat DB append-only events as the authoritative audit layer; existing latency JSONL remains optional local detail, not the primary audit store.
- Extend runtime metrics only enough to cover `REQ-024` event counts and key statuses; do not build dashboards in `IMP-13`.

## Important Interface Changes
- `TradeDecisionUpsert` becomes compatibility-only; add a new append-only decision insert API instead of reusing `upsert`.
- Add typed append-only repository methods, for example:
  - `insert_trade_decision_event(...)`
  - `insert_trade_runtime_event(...)`
- Extend in-memory execution context to carry `decision_event_id` and stable `reason_code`.
- Add normalized event-kind and reason-code enums or string helpers so DB rows and JSON logs do not drift.

## Test Plan
- DB tests:
  - multiple decisions for one trade append multiple rows instead of overwriting
  - runtime events append in order and preserve links to `decision_event_id`
  - compatibility mirror still updates without breaking legacy readers
- Decision tests:
  - approved decision persists full required input state
  - blocked decision persists the same required input state plus block reason
  - `config_version` is present on every decision row
- Execution linkage tests:
  - order intent event references the originating decision
  - ack event references the same decision and order
  - maker and taker fills reference the same decision when context exists
- Lifecycle tests:
  - state transition, reconciliation, and settlement events are appended with stable event kinds
  - an operator can trace one live order back to exactly one decision event
- Regression tests:
  - existing trade creation/finalization flow keeps working
  - append-only inserts do not require mutating earlier rows
  - legacy `trade_decisions` queries still return the latest summary

## Assumptions and Defaults
- New append-only tables are the source of truth; `trade_decisions` stays only as a compatibility summary.
- Use generated text IDs, not composite natural keys, for `decision_event_id` and runtime `event_id`.
- Existing optional latency file logs stay in place, but DB-backed audit records are the requirement-grade path.
- `IMP-13` does not add new dashboard UI; it delivers the persisted and structured data needed for later reporting.
