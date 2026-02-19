# Polybot Native Rust Port

This repository is now on a native Rust migration path for `main.py`.

## Current native modules

- `src/main.rs`: native process control loop (env, market rollover, signal startup, DB lifecycle)
- `src/config.rs`: `BotConfig` and env/default mapping
- `src/helpers.rs`: segment/slug helpers, state load-save, numeric helpers
- `src/signal.rs`: signal data model, JSONL/CSV services, inbox queue, websocket hub
- `src/gamma.rs`: Gamma market fetch + token/condition parsing
- `src/bot.rs`: `MakerHedgeCapBot` core struct and runtime scaffolding
- `src/db.rs`: SQLite-backed schema/session/repository port for `db/models.py`, `db/session.py`, `db/repository.py`, and `db/utils.py`
- `src/logging.rs`: `setup_item_logger`-style per-item logger writing `app.log` + `app.json` under `LOG_DIR/<item_id>/`

## Build note

- `rustup` and `cargo` are installed.
- `cargo check` passes on this machine (`rustc 1.93.1`).

## Tracking

- Function/method parity tracker: `PORTING_STATUS.md`
- Current tracker snapshot: 167 symbols marked `Ported`, 0 symbols `Pending` (name parity from `main.py` to `src/*.rs`)
- Regenerate tracker:

```powershell
.\.venv\Scripts\python.exe .\scripts\generate_port_status.py
```

- Env contract extraction:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\extract_env_contract.ps1
```
