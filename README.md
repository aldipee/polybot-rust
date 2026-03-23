# Polybot Canary Runbook

This README is a practical runbook for a supervised canary using the current repo state.

For the full operational manual, including `polybot`, replay, `kpi_gate`, `analysis_importer`, helper utilities, outputs, and troubleshooting, use [OPERATIONS.md](/c:/Works/aldipranata.com/bot-dev/OPERATIONS.md).

Important current constraint:

- `IMP-25` is not implemented yet.
- That means deployment gating and rollback are still manual.
- Recommended sequence is:
  - replay certification
  - shadow canary
  - shadow KPI review
  - optional supervised live canary

The recommended env starting point is [canary.env.txt](/c:/Works/aldipranata.com/bot-dev/canary.env.txt).

## Current Safety Model

What already exists:

- guarded `shadow`, `paper`, and `live` routing
- replay certification scenarios
- shadow and paper KPI summaries
- stale-data, reconciliation, validation, and gross-cap runtime safety
- paper-mode parity through `PairBuild` and `Taper`, so paper no longer freezes on `user_ws_disconnected` in those phases

What does not exist yet:

- automatic deployment gate enforcement from replay and KPI outputs
- automatic rollback orchestration for live canaries

So for now:

- `shadow` canary is the default and recommended first step
- `live` canary should be supervised and manually controlled
- rollback means manually returning the bot to `shadow` and restarting it

## Step 1: Fill Canary Env

Start from [canary.env.txt](/c:/Works/aldipranata.com/bot-dev/canary.env.txt) and set at least:

- `DB_URL`
- `POLYMARKET_PRIVATE_KEY`
- `POLYMARKET_FUNDER`
- optionally `POLYMARKET_WALLET_ADDRESS` if you want it explicit

Recommended starting mode in the file is already:

- `BOT_ORDER_MODE=shadow`
- `BOT_LIVE_ENABLED=false`

That gives you a zero-live-write canary with full runtime logic, replay capture, audit logs, and KPI-compatible persistence.

## Step 2: Load Env and Run Replay Certification

PowerShell session example:

```powershell
Get-Content .\canary.env.txt |
  Where-Object { $_ -and -not $_.Trim().StartsWith('#') } |
  ForEach-Object {
    $name, $value = $_ -split '=', 2
    [System.Environment]::SetEnvironmentVariable($name, $value, 'Process')
  }
```

Replay certification should be green before you trust a canary build:

```powershell
cargo test --test replay_certification -- --nocapture
```

Expected result:

- replay certification passes
- no failed scenario in:
  - good open
  - one-side lag
  - stale hold escalation
  - reconnect reconciliation mismatch
  - late settlement handoff

## Step 3: Start the Shadow Canary

Run the bot in `shadow` first:

```powershell
cargo run --release --bin polybot
```

Or build once and run the binary directly:

```powershell
cargo build --release
.\target\release\polybot.exe
```

Artifacts to watch during the run:

- logs under `LOG_DIR`
- replay capture bundles under `REPLAY_CAPTURE_DIR`
- runtime events and trades in the database

Recommended shadow-canary expectation:

- let it run long enough to cover at least `3` distinct trading days
- keep it supervised
- do not switch to live until KPI evidence is green

## Step 4: Run Shadow KPI Review

After the shadow window finishes, run the KPI gate for that exact bot id and time range:

```powershell
cargo run --bin kpi_gate -- `
  --bot-id canary-btc5m-01 `
  --profile shadow `
  --start 2026-03-23T00:00:00+07:00 `
  --end 2026-03-26T00:00:00+07:00
```

What to look for in the output:

- `overall_status=PASS`
- `distinct_trading_days >= 3`

And in the generated summary:

- no unresolved startup or reconnect reconciliation failures
- no `audit_drop`
- no missing `run_summary`
- no settlement/audit mismatches
- no state-machine stalls that end unresolved

The summary file is written under:

- `output/kpi_gate/<bot_id>/shadow/<window>/summary.json`

## Step 5: Optional Supervised Live Canary

Only do this after:

- replay certification is green
- shadow KPI is `PASS`
- you are ready to supervise the run manually

Change the mode lines from:

- `BOT_ORDER_MODE=shadow`
- `BOT_LIVE_ENABLED=false`

to:

- `BOT_ORDER_MODE=live`
- `BOT_LIVE_ENABLED=true`

Then restart the bot.

Important:

- live still stays guarded by existing runtime safety
- but there is no `IMP-25` deployment gate yet
- there is also no `IMP-25` automatic rollback yet

So live canary promotion is a manual operator decision.

## Step 6: Manual Rollback

If you want to stop live writes during canary:

1. change the mode back to:
   - `BOT_ORDER_MODE=shadow`
   - `BOT_LIVE_ENABLED=false`
2. restart the bot

This is the current manual rollback path.

What that means operationally:

- no new real venue submits after restart
- audit, reconciliation, and monitoring can keep running
- you can still inspect logs, replay captures, and KPI outputs

## Recommended Canary Policy

Good current operating policy for this repo state:

1. Keep the first canary `shadow` only.
2. Require replay certification to pass before trusting the build.
3. Require `shadow` KPI to pass before any live promotion.
4. Promote to `live` only while supervised.
5. Roll back manually by switching back to `shadow` and restarting.

## Notes

- [OPERATIONS.md](/c:/Works/aldipranata.com/bot-dev/OPERATIONS.md) is the full operations manual for runtime, replay, KPI, analysis import, helper tools, artifacts, and troubleshooting.
- [ENVIRONMENT.md](/c:/Works/aldipranata.com/bot-dev/ENVIRONMENT.md) is the supported env reference for the active `polybot` runtime.
- [TASKS.MD](/c:/Works/aldipranata.com/bot-dev/TASKS.MD) tracks requirement status; `IMP-25` is the remaining deployment/rollback gate task.
- [FINDINGS.MD](/c:/Works/aldipranata.com/bot-dev/FINDINGS.MD) records the hardening findings that were raised and fixed along the way.

## Offline Analysis Importer

The repo also includes an offline historical dataset importer for `IMP-17` and `REQ-027`.

Use this when you want to ingest the checked-in dataset under `dataset/`, persist the imported rows and pair rollups, and reproduce the committed calibration summary.

Entry points:

- binary: [src/bin/analysis_importer.rs](/c:/Works/aldipranata.com/bot-dev/src/bin/analysis_importer.rs)
- implementation: [src/analysis_import/mod.rs](/c:/Works/aldipranata.com/bot-dev/src/analysis_import/mod.rs)
- parity test: [tests/analysis_import.rs](/c:/Works/aldipranata.com/bot-dev/tests/analysis_import.rs)
- committed summary fixture: [tests/analysis_import/expected_summary.json](/c:/Works/aldipranata.com/bot-dev/tests/analysis_import/expected_summary.json)

### What It Reads

The importer expects the checked-in dataset directory:

- [dataset/vidarx_trade_profitable.parquet](/c:/Works/aldipranata.com/bot-dev/dataset/vidarx_trade_profitable.parquet)
- [dataset/vidarx_close_position_profitable.csv](/c:/Works/aldipranata.com/bot-dev/dataset/vidarx_close_position_profitable.csv)
- [dataset/dataset_schema.md](/c:/Works/aldipranata.com/bot-dev/dataset/dataset_schema.md)

The importer validates the exact parquet column list and CSV header list before it starts importing.

### Requirements

- `DB_URL` must be set, or available from `.env`
- the dataset files above must exist under [dataset](/c:/Works/aldipranata.com/bot-dev/dataset)
- Postgres schema initialization is handled automatically by the binary

### Quick Start

From the repo root:

```powershell
cargo run --bin analysis_importer -- dataset
```

To choose a custom output folder:

```powershell
cargo run --bin analysis_importer -- dataset --output-dir output/analysis_import
```

If `DB_URL` is only stored in `.env`, the binary will load it automatically before startup.

### What It Does

The importer performs these steps:

1. validates the exact parquet and CSV schemas
2. loads all source columns exactly as provided
3. computes pair-level rollups and calibration metrics
4. writes a deterministic `summary.json`
5. persists the import run plus raw rows and rollups into Postgres

### Output Files

By default the generated summary lands at:

- [output/analysis_import/summary.json](/c:/Works/aldipranata.com/bot-dev/output/analysis_import/summary.json)

The console output includes:

- `summary_path`
- `filtered_market_count`
- `closed_position_pair_count`
- `two_sided_participation_rate`
- `taker_share`
- `weighted_pair_sum_median`

The committed expected output for the checked-in dataset is:

- [tests/analysis_import/expected_summary.json](/c:/Works/aldipranata.com/bot-dev/tests/analysis_import/expected_summary.json)

### Database Tables

The importer persists into these tables:

- `analysis_import_run`
- `analysis_trade_row`
- `analysis_close_position_row`
- `analysis_pair_rollup`

### Verification

To verify the importer still reproduces the committed calibration summary:

```powershell
cargo test --test analysis_import -- --nocapture
```

That test checks the checked-in dataset against the committed parity fixture and the expected counts:

- `29342` parquet rows
- `209` close rows
- `105` filtered markets
- `105` closed-position pairs
- `104` two-sided close pairs

### Notes

- This is an offline helper binary, not part of the `polybot` runtime startup path.
- [ENVIRONMENT.md](/c:/Works/aldipranata.com/bot-dev/ENVIRONMENT.md) intentionally documents the supported `polybot` runtime env surface only, so `analysis_importer` is documented here instead.
