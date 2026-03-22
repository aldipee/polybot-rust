# IMP-17 Plan: Analysis Importer and Calibration Parity

## Summary
- Add a dedicated offline importer binary for the checked-in historical dataset at [c:\Works\aldipranata.com\bot-dev\dataset](c:\Works\aldipranata.com\bot-dev\dataset).
- Ingest both source files exactly, persist imported rows plus pair-level rollups, and emit a deterministic parity report that becomes the committed oracle for `REQ-027`.
- Keep this separate from live bot runtime. No trading-path behavior changes, no new operator env, and no dependency on replay or paper-mode code.

## Important Changes
- Add `src/bin/analysis_importer.rs` with initial UX:
  - `cargo run --bin analysis_importer -- <dataset_dir> [--output-dir <path>]`
  - `DB_URL` is required for persistence
  - default output root: `output/analysis_import`
- Add a new library module, recommended as `src/analysis_import/`, with:
  - dataset loaders and exact schema validation
  - deterministic metric engine
  - report types
  - a narrow sink interface so tests can run without Postgres
- Add Postgres-backed persistence in `src/db.rs` for:
  - `analysis_import_run`
  - `analysis_trade_row`
  - `analysis_close_position_row`
  - `analysis_pair_rollup`
- Add committed parity fixtures under `tests/analysis_import/`:
  - expected summary report
  - optional pair-rollup fixture excerpt if needed for debugging
  - integration test runner

## Implementation Changes
### 1. Dataset and Schema Locking
- Treat the checked-in dataset as the authoritative oracle source:
  - [c:\Works\aldipranata.com\bot-dev\dataset\vidarx_trade_profitable.parquet](c:\Works\aldipranata.com\bot-dev\dataset\vidarx_trade_profitable.parquet)
  - [c:\Works\aldipranata.com\bot-dev\dataset\vidarx_close_position_profitable.csv](c:\Works\aldipranata.com\bot-dev\dataset\vidarx_close_position_profitable.csv)
- Validate exact field names and order before import:
  - parquet: `60` columns
  - csv: `17` columns
- Preserve field names exactly, including `snapsot_*` spellings.
- Use the actual file schema as truth for physical types; use `dataset_schema.md` as human reference only, not a machine-parsed contract.
- Import all columns, even if only a subset drives the headline parity metrics.

### 2. Persistence Model
- `analysis_import_run`
  - one row per importer execution
  - stores dataset paths, started/completed timestamps, status, source file hashes or mtimes, and the final summary JSON
- `analysis_trade_row`
  - one row per parquet trade record
  - preserve all `60` source columns plus `import_run_id`
  - primary key: `(import_run_id, trade_identity_key)`
- `analysis_close_position_row`
  - one row per CSV close-position record
  - preserve all `17` source columns plus `import_run_id` and a stable row ordinal
- `analysis_pair_rollup`
  - one row per `conditionId`
  - stores pair-level normalized facts from both datasets:
    - `eventSlug`
    - sides traded in tape
    - sides present in close CSV
    - both-sided-close flag
    - total trade count
    - taker trade count
    - total notional
    - taker notional
    - close-side `avgPrice`, `totalBought`, `realizedPnl`, `curPrice` per side when present
- Use a local importer sink trait so automated tests can use an in-memory sink while the binary uses the Postgres sink.

### 3. Metric Definitions
- `filtered_market_count`
  - distinct `conditionId` in the parquet trade tape
- `closed_position_pair_count`
  - distinct `conditionId` in the close-position CSV
- `two_sided_participation_rate`
  - `distinct conditionId with both Up and Down rows in close CSV / distinct conditionId in close CSV`
  - keep incomplete close pairs in the denominator
  - with the current dataset, the plan should expect one incomplete close pair rather than assuming perfect pairing
- `taker_share`
  - headline metric uses trade-row share: `count(is_taker=true) / total trade rows`
  - also compute and persist supplemental notional taker share, but do not use it as the primary parity gate
- `historical_effective_pair_cost`
  - per trade row: executed `price` plus the opposite side’s `snapshot_last_trade_price_*`
  - fallback to opposite `snapshot_price_*` when last-trade price is null
  - drop the row from pair-cost metrics only if both opposite-side fields are null
- `weighted_pair_sum_median`
  - weighted median of `historical_effective_pair_cost`
  - weight by executed notional `size * price`
- `price_zone`
  - classify `historical_effective_pair_cost` using the live rule thresholds:
    - `preferred < 0.94`
    - `acceptable 0.94-<0.97`
    - `caution 0.97-<1.00`
    - `stop_add 1.00-<1.03`
    - `danger >= 1.03`
- `price_zone_outcome_summary`
  - one row per zone in the final report with:
    - `trade_count`
    - `trade_notional`
    - `taker_trade_rate`
    - `resolved_trade_count`
    - `winner_alignment_rate` where `outcome == final_outcome`
    - `skipped_pair_cost_count`
    - `skipped_outcome_count`
- Report metrics should serialize as fixed-scale decimals rounded to `6` decimal places so fixture comparison is exact and deterministic.

### 4. Binary and Report Output
- The binary should:
  1. validate dataset files and exact schemas
  2. import rows
  3. persist raw rows and pair rollups
  4. compute headline metrics
  5. write a deterministic `summary.json` under the chosen output dir
  6. print the summary path plus the headline metrics to stdout
- The report should include:
  - dataset file names
  - row counts
  - distinct market counts
  - headline parity metrics
  - zone summaries
  - coverage counters for null-driven skips

## Test Plan
- Add unit tests for:
  - exact parquet column list
  - exact CSV header list
  - pair-cost fallback logic
  - price-zone threshold boundaries
  - weighted median helper
- Add an integration test over the checked-in dataset that verifies:
  - parquet rows imported: `29342`
  - close rows imported: `209`
  - distinct `conditionId` in trade tape: `105`
  - distinct `conditionId` in close CSV: `105`
  - two-sided close pairs: `104`
  - the generated `summary.json` matches a committed fixture exactly at the chosen decimal precision
- Add an in-memory-sink test that asserts imported row counts and pair-rollup counts without requiring Postgres.
- Add one optional or ignored Postgres smoke test that:
  - runs `init_schema`
  - persists one real import run
  - verifies raw-row and rollup-row counts landed in the new tables

## Assumptions
- The checked-in dataset directory is the oracle source for `IMP-17`; no external notebook or manual metric sheet is required for v1 parity.
- The historical pair-cost proxy is locked to `executed price + opposite snapshot last-trade price`, with fallback to opposite snapshot price.
- The headline taker-share parity metric uses trade-row share, not notional share.
- The importer will preserve all source columns exactly, but only the defined subset above drives the required parity metrics.
- No new operator env is needed; this is a separate CLI tool using the existing `DB_URL` contract.
