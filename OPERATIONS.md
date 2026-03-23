# Operations Manual

Date: 2026-03-23
Scope: practical operator documentation for the current repo state.

This file is the detailed operational manual for:

- the main `polybot` runtime
- replay certification and replay scenarios
- KPI reporting
- the offline analysis importer
- helper utilities that operators may run alongside the bot

Important current constraint:

- `IMP-25` is not implemented yet
- deployment gating and rollback are still manual
- recommended rollout flow is:
  - replay certification
  - shadow canary
  - shadow KPI review
  - optional supervised live canary

## Documentation Map

Use these docs together:

- [README.md](/c:/Works/aldipranata.com/bot-dev/README.md): quick canary-oriented entrypoint
- [OPERATIONS.md](/c:/Works/aldipranata.com/bot-dev/OPERATIONS.md): this full operations manual
- [ENVIRONMENT.md](/c:/Works/aldipranata.com/bot-dev/ENVIRONMENT.md): supported `polybot` runtime env reference
- [RUN_MULTI_INSTANCE.md](/c:/Works/aldipranata.com/bot-dev/RUN_MULTI_INSTANCE.md): systemd and multi-instance setup
- [TASKS.MD](/c:/Works/aldipranata.com/bot-dev/TASKS.MD): implementation and requirement status
- [FINDINGS.MD](/c:/Works/aldipranata.com/bot-dev/FINDINGS.MD): hardening history

## Main Operator Surfaces

The repo currently has these operator-facing binaries and flows:

### 1. Main Bot Runtime

Primary executable:

- crate root binary: `cargo run --release --bin polybot`
- built binary: `.\target\release\polybot.exe`

Purpose:

- runs the guarded maker and taker strategy
- persists trade, decision, and runtime audit rows
- can run in `shadow`, `paper`, or configured `live`

Main supporting files:

- [canary.env.txt](/c:/Works/aldipranata.com/bot-dev/canary.env.txt)
- [ENVIRONMENT.md](/c:/Works/aldipranata.com/bot-dev/ENVIRONMENT.md)

### 2. Replay Certification

Entry command:

```powershell
cargo test --test replay_certification -- --nocapture
```

Purpose:

- certifies the committed replay scenarios before trusting a build

Fixtures:

- [tests/replay/scenarios](/c:/Works/aldipranata.com/bot-dev/tests/replay/scenarios)

### 3. Replay Scenario Runner

Entry command:

```powershell
cargo run --bin replay -- tests/replay/scenarios/good_open_paired_seed
```

Purpose:

- replays one scenario folder through the deterministic replay engine

Scenario bundle requirements:

- `manifest.json`
- `resolved_config.json`
- `events.jsonl`
- `initial_state/`
- optional:
  - `oracle_decisions.jsonl`
  - `oracle_runtime_events.jsonl`
  - `oracle_final_state.json`
  - `resolution_snapshot.json`

### 4. KPI Gate

Entry command:

```powershell
cargo run --bin kpi_gate -- `
  --bot-id canary-btc5m-01 `
  --profile shadow `
  --start 2026-03-23T00:00:00+07:00 `
  --end 2026-03-26T00:00:00+07:00
```

Purpose:

- evaluates `paper` or `shadow` runs from persisted structured audit rows and settled trade rows

Summary output:

- `output/kpi_gate/<bot_id>/<profile>/<window>/summary.json`

Persistence:

- `kpi_gate_run`
- `kpi_gate_metric`

### 5. Analysis Importer

Entry command:

```powershell
cargo run --bin analysis_importer -- dataset
```

Purpose:

- ingests the checked-in historical dataset
- persists imported rows and pair rollups
- reproduces the committed calibration summary for `REQ-027`

Summary output:

- `output/analysis_import/summary.json`

Persistence:

- `analysis_import_run`
- `analysis_trade_row`
- `analysis_close_position_row`
- `analysis_pair_rollup`

### 6. Helper Utilities

These are real binaries in the repo, but they are not the main supported `polybot` runtime surface:

- `cargo run --bin copy_collect`
- `cargo run --bin clickhouse_push`

Use them only when you intentionally need their side workflows.

## Prerequisites

Before running anything operationally, make sure you have:

- Rust toolchain installed
- Postgres reachable through `DB_URL`
- a filled env file, preferably [canary.env.txt](/c:/Works/aldipranata.com/bot-dev/canary.env.txt) as the starting point
- valid Polymarket credentials if you are using `shadow` or `live`
- the checked-in historical dataset present under [dataset](/c:/Works/aldipranata.com/bot-dev/dataset) if you plan to use `analysis_importer`

Recommended first build:

```powershell
cargo build --release
```

## Recommended Env Starting Point

Use [canary.env.txt](/c:/Works/aldipranata.com/bot-dev/canary.env.txt) as the recommended operator baseline.

It is intentionally:

- `shadow` first
- replay-capture capable
- conservative on gross-cap and validation defaults
- aligned with the current supported runtime surface

At minimum, set:

- `DB_URL`
- `POLYMARKET_PRIVATE_KEY`
- `POLYMARKET_FUNDER`
- optionally `POLYMARKET_WALLET_ADDRESS`

Recommended starting mode:

- `BOT_ORDER_MODE=shadow`
- `BOT_LIVE_ENABLED=false`

## Loading Env in PowerShell

Project-root example:

```powershell
Get-Content .\canary.env.txt |
  Where-Object { $_ -and -not $_.Trim().StartsWith('#') } |
  ForEach-Object {
    $name, $value = $_ -split '=', 2
    [System.Environment]::SetEnvironmentVariable($name, $value, 'Process')
  }
```

Notes:

- `analysis_importer` and `kpi_gate` also load `.env` automatically
- the main `polybot` runtime follows the runtime env contract documented in [ENVIRONMENT.md](/c:/Works/aldipranata.com/bot-dev/ENVIRONMENT.md)

## Runtime Modes

Supported routing modes:

- `shadow`: full runtime logic, no real venue writes
- `paper`: simulated execution path
- `live`: real venue writes, still guarded by runtime safety checks
- `paper` uses live market data and the same runtime strategy flow as non-live modes, but with simulated execution instead of venue writes
- `paper` `PairBuild` and `Taper` do not require user websocket connectivity; `shadow` and `live` still honor `REQUIRE_USER_WS_CONNECTED`

Important:

- `BOT_LIVE_ENABLED=true` is required in addition to `BOT_ORDER_MODE=live`
- `shadow` is the recommended canary mode before any supervised live run
- because `IMP-25` is not implemented, promotion to `live` is a manual operator decision

## Standard Supervised Canary Procedure

### Step 1. Run Replay Certification

Before you trust a build:

```powershell
cargo test --test replay_certification -- --nocapture
```

Expected result:

- all committed replay certification scenarios pass

Those scenarios currently cover:

- good open
- one-side lag
- stale-data hold escalation
- reconnect reconciliation mismatch
- late settlement handoff

### Step 2. Start a Shadow Canary

Run the bot in `shadow`:

```powershell
cargo run --release --bin polybot
```

Or:

```powershell
cargo build --release
.\target\release\polybot.exe
```

What to watch:

- `LOG_DIR`
- replay bundles under `REPLAY_CAPTURE_DIR`
- persisted rows in:
  - `trade`
  - `trade_decisions`
  - `trade_decision_events`
  - `trade_runtime_events`

Recommended shadow-canary expectation:

- let it run long enough to cover at least `3` distinct trading days
- keep the run supervised
- do not switch to live until KPI evidence is green

### Step 3. Run Shadow KPI Review

After the shadow window completes:

```powershell
cargo run --bin kpi_gate -- `
  --bot-id canary-btc5m-01 `
  --profile shadow `
  --start 2026-03-23T00:00:00+07:00 `
  --end 2026-03-26T00:00:00+07:00
```

What to look for:

- `overall_status=PASS`
- `distinct_trading_days >= 3`

In the summary JSON, confirm:

- no unresolved startup or reconnect reconciliation failures
- no `audit_drop`
- no missing terminal `run_summary`
- no settlement versus trade-row mismatches
- no unresolved state-machine stalls

### Step 4. Optional Supervised Live Canary

Only do this after:

- replay certification is green
- shadow KPI is `PASS`
- you are ready to supervise the run manually

Switch:

- `BOT_ORDER_MODE=live`
- `BOT_LIVE_ENABLED=true`

Then restart the bot.

Important:

- existing runtime safeguards still apply
- there is no automatic deployment gate yet
- there is no automatic rollback yet
- live promotion is manual

### Step 5. Manual Rollback

To stop live writes:

1. switch env back to:
   - `BOT_ORDER_MODE=shadow`
   - `BOT_LIVE_ENABLED=false`
2. restart the bot

That is the current rollback path.

Operational meaning:

- no new real venue submits after restart
- audit, reconciliation, monitoring, and replay capture can keep running

## Replay Operations

Replay exists in 2 forms:

- certification via `cargo test --test replay_certification -- --nocapture`
- one-scenario execution via `cargo run --bin replay -- <scenario_dir>`

### Replay Capture Bundles

When:

- `REPLAY_CAPTURE_ENABLED=true`
- `REPLAY_CAPTURE_DIR` is set

the bot writes capture bundles containing:

- `manifest.json`
- `resolved_config.json`
- `initial_state/`
- `events.jsonl`
- optional:
  - `oracle_decisions.jsonl`
  - `oracle_runtime_events.jsonl`
  - `oracle_final_state.json`
  - `resolution_snapshot.json`

These bundles are the source material for deterministic replay.

### Running a Captured Scenario

Example:

```powershell
cargo run --bin replay -- .\output\replay-captures\canary-btc5m-01\<scenario_name>
```

Behavior:

- replay loads the captured config and initial state
- replays normalized input events from `events.jsonl`
- compares output against any committed or captured oracle files if present
- fails fast on missing required files or unsorted event tapes

## KPI Gate Operations

The KPI gate is the current measurement layer for `paper` and `shadow` health.

### Profiles

Supported values:

- `paper`
- `shadow`

Shadow means runs whose terminal effective mode stayed `shadow`, including guarded configured-live runs that never armed live.

### Key Sample Thresholds

Current v1 expectations:

- `shadow`: at least `3` distinct trading days
- `paper`: at least `10` distinct trading days and `500` settled pairs

Overall status precedence:

- `FAIL`
- `INSUFFICIENT_SAMPLE`
- `WARN`
- `PASS`

### Important KPI Outputs

The summary includes:

- `metadata`
- `sample_coverage`
- `source_counts`
- `metrics`
- `evaluation`
- `overall_status`

For shadow canaries, the most important checks are:

- adapter recovery
- startup reconciliation
- decision logging integrity
- state machine progress
- hypothetical price and imbalance compliance
- hypothetical underdog residual behavior
- settlement observation

For paper windows, important checks include:

- seed timing
- no scale-up before both sides filled
- unmatched fraction
- price discipline
- underdog residual behavior
- taker share
- single-side speculation
- settlement reconciliation
- PnL decomposition

## Analysis Importer Operations

The analysis importer is offline and separate from runtime bot operation.

### Required Dataset Files

The dataset directory must contain:

- [dataset/vidarx_trade_profitable.parquet](/c:/Works/aldipranata.com/bot-dev/dataset/vidarx_trade_profitable.parquet)
- [dataset/vidarx_close_position_profitable.csv](/c:/Works/aldipranata.com/bot-dev/dataset/vidarx_close_position_profitable.csv)
- [dataset/dataset_schema.md](/c:/Works/aldipranata.com/bot-dev/dataset/dataset_schema.md)

### Run Command

```powershell
cargo run --bin analysis_importer -- dataset
```

Custom output location:

```powershell
cargo run --bin analysis_importer -- dataset --output-dir output/analysis_import
```

### What It Does

The importer:

1. validates the exact parquet and CSV schemas
2. loads all source columns exactly as provided
3. computes pair rollups and calibration metrics
4. writes deterministic `summary.json`
5. persists the import run plus raw rows and pair rollups

### Expected Output

Default summary path:

- `output/analysis_import/summary.json`

Committed parity fixture:

- [tests/analysis_import/expected_summary.json](/c:/Works/aldipranata.com/bot-dev/tests/analysis_import/expected_summary.json)

Verification command:

```powershell
cargo test --test analysis_import -- --nocapture
```

Current expected checked-in dataset counts:

- `29342` parquet rows
- `209` close rows
- `105` filtered markets
- `105` closed-position pairs
- `104` two-sided close pairs

## Files, Directories, and Artifacts

Common operator-facing paths:

- [canary.env.txt](/c:/Works/aldipranata.com/bot-dev/canary.env.txt): recommended env template
- [dataset](/c:/Works/aldipranata.com/bot-dev/dataset): checked-in analysis dataset
- `LOG_DIR`: main runtime logs
- `REPLAY_CAPTURE_DIR`: replay capture bundles
- `output/kpi_gate`: KPI reports
- `output/analysis_import`: analysis importer summaries
- `state/`: RTDS and other local state, when enabled
- `signals/`: signal or helper outputs, when configured

## Database Tables

Main runtime tables:

- `trade`
- `trade_decisions`
- `trade_decision_events`
- `trade_runtime_events`

Analysis importer tables:

- `analysis_import_run`
- `analysis_trade_row`
- `analysis_close_position_row`
- `analysis_pair_rollup`

KPI tables:

- `kpi_gate_run`
- `kpi_gate_metric`

## Multi-Instance Operation

For OS-level multi-instance deployment, use:

- [RUN_MULTI_INSTANCE.md](/c:/Works/aldipranata.com/bot-dev/RUN_MULTI_INSTANCE.md)

That doc covers:

- instance directory creation
- per-instance `.env`
- state and data directories
- systemd unit installation
- service lifecycle commands

## Helper Utility Appendix

### copy_collect

Binary:

- [src/bin/copy_collect.rs](/c:/Works/aldipranata.com/bot-dev/src/bin/copy_collect.rs)

Entry command:

```powershell
cargo run --bin copy_collect
```

Purpose:

- collects copy-trade and optional RTDS-adjacent market data feeds

Important env families:

- `COPY_COLLECT_OUT_PATH`
- `COPY_COLLECT_WS_URL`
- `COPY_COLLECT_PRICE_TOPICS`
- `COPY_COLLECT_PRICE_SYMBOLS`
- `COPY_COLLECT_RUN_SECONDS`
- `COPY_COLLECT_MAX_TRADES`
- `COPY_COLLECT_CLOB_JOIN_ENABLED`

This helper is intentionally not part of the supported `polybot` runtime env contract in [ENVIRONMENT.md](/c:/Works/aldipranata.com/bot-dev/ENVIRONMENT.md).

### clickhouse_push

Binary:

- [src/bin/clickhouse_push.rs](/c:/Works/aldipranata.com/bot-dev/src/bin/clickhouse_push.rs)

Entry command:

```powershell
cargo run --bin clickhouse_push
```

Purpose:

- ingests RTDS and copy-collect JSON or JSONL files into ClickHouse

Important env families:

- `CLICKHOUSE_URL`
- `CLICKHOUSE_DATABASE`
- `CLICKHOUSE_USER`
- `CLICKHOUSE_PASSWORD`
- `CLICKHOUSE_RTDS_PRICES_PATH`
- `CLICKHOUSE_COPY_COLLECT_PATH`
- `CLICKHOUSE_RTDS_PRICE_TO_BEAT_PATH`
- `CLICKHOUSE_RTDS_RESOLUTION_STATE_PATH`

This helper is also intentionally outside the supported `polybot` runtime env contract.

## Troubleshooting

### The bot never writes live orders

Check:

- `BOT_ORDER_MODE=live`
- `BOT_LIVE_ENABLED=true`
- reconciliation is healthy
- dependency and stale-data safety are healthy

If any of those fail, the bot can remain effectively shadowed.

### Replay capture is not writing bundles

Check:

- `REPLAY_CAPTURE_ENABLED=true`
- `REPLAY_CAPTURE_DIR` is set and writable

### KPI gate says `INSUFFICIENT_SAMPLE`

This usually means:

- not enough distinct trading days
- or, for `paper`, not enough settled pairs

### analysis_importer fails immediately

Check:

- `DB_URL` is present
- `dataset/` contains all 3 required files
- the parquet and CSV schemas still match the committed exact lists

### Replay fails on a scenario folder

Check:

- `manifest.json` exists
- `resolved_config.json` exists
- `events.jsonl` exists
- event rows are strictly sorted by `(ts_ns, seq)`

## Current Recommended Operating Policy

1. Use [canary.env.txt](/c:/Works/aldipranata.com/bot-dev/canary.env.txt) as the starting point.
2. Run replay certification before trusting a build.
3. Run the first canary in `shadow`.
4. Require shadow KPI `PASS` before any live promotion.
5. Promote to `live` only while supervised.
6. Roll back manually by switching back to `shadow` and restarting.
