# IMP-02 Plan: Make the Bot Pair-First

## Summary
- Implement `REQ-002` inside the current monolith, not as a crate refactor.
- Introduce a first-class `pair_id` and make it the canonical key for strategy, runtime, risk context, and pair-level persistence.
- Keep `trade_id` as the per-run summary/result row; do not replace it.
- Use `pair_id = slug.trim().to_ascii_lowercase()` in this codebase, and store `condition_id`, `yes_asset_id`, and `no_asset_id` as pair metadata.
- Primary touch points: `src/bot/core.rs`, `src/bot/runtime/*`, and `src/db.rs`.

## Key Changes
- Identity model
  - Add `PairIdentity { pair_id, market_slug, condition_id, yes_asset_id, no_asset_id }`.
  - Store it on `MakerHedgeCapBot`.
  - Build it during bot construction after market metadata fetch; if market metadata is incomplete, still derive `pair_id` from slug and leave the extra metadata optional until available.
- Pair-owned state
  - Add `PairPosition { q_yes, q_no, c_yes, c_no }`.
  - Add `PairSnapshot { identity, position, phase, t_into_s, total_cost, paired_size, unmatched_size, yes_quote, no_quote }`.
  - Keep `BotState`’s current storage shape, but treat it as pair-owned state only; do not introduce separate side strategy state.
- Runtime decision boundary
  - Change the top-level order-producing runtime paths to accept or immediately build `PairSnapshot` before making any decision:
    - open-both
    - seed-completion
    - pair-build
    - taper / late-window handling
  - Internal math helpers may keep scalar args, but only as decomposed fields from `PairSnapshot`.
  - Side-only helpers like `OutcomeSide`, `MakerOrderKey`, order slots, and asymmetry checks remain execution utilities and cannot be the sole input to a size-changing decision.
- Execution metadata
  - Extend `order_exec_context` and submit-timing metadata with `pair_id`, `market_slug`, `condition_id`, `yes_asset_id`, and `no_asset_id`.
  - Include `pair_id` in submit, ack, fill, reconciliation, and runtime hold logs.
- Persistence
  - `trade` table: add `pair_id TEXT NOT NULL`, `condition_id TEXT NULL`, `yes_asset_id TEXT NULL`, `no_asset_id TEXT NULL`.
  - Backfill existing rows with `pair_id = lower(trim(slug))`; leave new metadata null for historical rows.
  - Add a unique index on `(bot_id, pair_id)` and change pending-trade dedupe to use that key instead of `(bot_id, slug)`.
  - Change `create_pending_trade` to receive pair metadata instead of raw slug-only identity.
  - `trade_decisions` table: add the same pair fields now, but keep the current `trade_id` primary key in this task. Append-only redesign stays in `IMP-13`.
- Metrics and summaries
  - Extend `TradeMetrics` with `pair_id` and optional pair metadata.
  - Keep existing `q_yes`, `q_no`, `cpp`, and `lp` outputs unchanged.

## Public Interface / Type Changes
- New types
  - `PairIdentity`
  - `PairPosition`
  - `PairSnapshot`
- Updated types
  - `MakerHedgeCapBot` gains pair identity fields
  - `TradeMetrics` gains `pair_id`
  - `TradeRow` and `TradeDecisionUpsert` gain pair metadata
- Signature changes
  - `create_pending_trade(...)` takes pair metadata
  - top-level runtime decision methods consume `PairSnapshot` or a `PairDecisionContext` built from it
  - execution-context writers accept `pair_id`

## Test Plan
- Unit tests
  - `pair_id` is always derived from normalized slug and is present after bot construction.
  - `PairSnapshot` correctly reflects `q_yes`, `q_no`, `c_yes`, `c_no`, paired size, unmatched size, and quotes.
  - fill application updates one pair-owned position and never creates side-orphan state.
  - pending-trade dedupe uses `(bot_id, pair_id)`, not raw slug text variation.
- Runtime tests
  - open-both, seed-completion, pair-build, and taper paths all operate through pair context and emit logs containing `pair_id`.
  - no size-increasing decision entrypoint can be called from only `asset_id`, `OutcomeSide`, or side-local order-slot state.
- Migration / repository tests
  - existing `trade` rows backfill `pair_id` from slug.
  - existing `trade_decisions` rows remain readable after additive pair columns are added.
  - trade finalization still updates the same `trade_id` row while preserving `pair_id`.

## Assumptions and Defaults
- `trade_id` remains the run/result key; `pair_id` becomes the strategy/risk/ledger key.
- `pair_id` is slug-derived in this task; `condition_id` is stored as venue metadata, not used as the primary key yet.
- No workspace split or crate reorganization is part of `IMP-02`.
- No append-only decision-log redesign is part of `IMP-02`; that remains `IMP-13`.
- `REQ-001` stays out of scope, so this plan does not add market-family restrictions.
