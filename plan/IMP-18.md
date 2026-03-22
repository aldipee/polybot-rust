# IMP-18 Plan: Deterministic Replay from Captured Event Tapes

## Summary
- Add a dedicated offline replay binary, not a new branch inside the live `main` loop.
- Use replay scenario folders backed by JSON: exact resolved config, copied initial companion state, one sorted `events.jsonl` tape, and optional oracle files for expected decisions/runtime events/final state.
- Reuse the same strategy, risk, reconciliation, and ledger handlers as live BOT runtime; swap only time, ID generation, venue/websocket I/O, and persistence targets.

## Important Changes
- Add `src/bin/replay.rs` as the first operator-facing entrypoint for replay. Initial UX is `cargo run --bin replay -- <scenario_dir>`. It exits nonzero on loader, determinism, or oracle mismatch.
- Add a new internal replay subsystem under `src/replay/` with:
  - `ReplaySource` over one sorted `events.jsonl`
  - `ReplayClock` for deterministic simulated time
  - deterministic ID provider for audit IDs, synthetic order IDs, and paper trade keys
  - scenario loader and runner
- Add runtime-only capture controls, not versioned trading config:
  - `REPLAY_CAPTURE_ENABLED` default `false`
  - `REPLAY_CAPTURE_DIR` default empty
  These only write sidecar files and do not change trading behavior.
- Capture bundles use:
  - `manifest.json`
  - `resolved_config.json` storing the exact `ResolvedVersionedConfigBundle`
  - `initial_state/` containing copies of the same local companion files the bot already uses
  - `events.jsonl` with normalized input events
  - optional `oracle_decisions.jsonl`, `oracle_runtime_events.jsonl`, and `oracle_final_state.json`

## Implementation Changes
- Deterministic substrate:
  - Introduce a shared clock abstraction and replace direct wall-clock reads in BOT runtime, execution, audit, and helper TTL paths with injected time access.
  - Introduce a deterministic ID source and replace direct `new_uuid()`, `now_ns()`-derived trade keys, and time-derived synthetic order IDs in replay-sensitive paths.
  - Extract a single-step runtime method from `_run_bot_runtime_loop()` so replay can drive the same logic one tick at a time.
- Replay scheduling:
  - Replay advances time from `manifest.start_ts_ns` to `manifest.end_ts_ns`.
  - Between external events, it runs internal BOT ticks at `min(loop_wait_seconds_maker, 0.5s)` so stale timers, gating, and completion windows behave like live.
  - At each event timestamp, replay sets the clock first, then feeds the event into the same existing handlers or adapter hooks.
- Event tape and capture:
  - Use one normalized `events.jsonl`, sorted by `(ts_ns, seq)`.
  - Event kinds are fixed to: `market_best_bid_ask`, `market_tick_size`, `user_order`, `user_trade`, `ws_open`, `ws_close`, and `reconcile_snapshot`.
  - Capture normalized events after current parsing/normalization, not raw websocket frames, so replay feeds the same shapes the bot already consumes.
  - Capture current audit-style outputs as oracle sidecars so canary/live logs can later become replay fixtures without redesigning the oracle format.
- Replay adapter behavior:
  - Replay does not start websocket threads and does not hit live CLOB APIs.
  - Market and user events feed existing `_handle_market_event` and `_handle_user_event` paths.
  - Venue reads used during reconciliation are answered from captured `reconcile_snapshot` events.
  - Replay does not use paper fill simulation for captured execution replay; if a fixture has no user fill/order event, replay does not invent one.
  - All replay state is isolated under a replay-specific temp/state root and never touches live DB rows or live shared companion files.

## Test Plan
- Loader and determinism tests:
  - reject unsorted tapes, duplicate `(ts_ns, seq)`, or missing required bundle files
  - deterministic clock and deterministic ID provider return stable results across repeated runs
- Capture roundtrip tests:
  - normalized market/user/reconcile events written by capture can be loaded by replay without schema loss
  - oracle sidecar normalization is stable across repeated serialization
- Replay integration scenarios:
  - simple open/seed/fill scenario run twice produces byte-equivalent normalized decision/runtime outputs and identical final state
  - reconnect scenario with `ws_close`, `ws_open`, and `reconcile_snapshot` reproduces the same reconciliation and safety-gate transitions
  - stale-gap scenario reproduces stale-data holds and emits no extra submits
  - replay of a captured scenario with oracle files matches expected decisions, runtime events, and final ledger/state
  - replay writes only replay-scoped state files and leaves live shared state/DB untouched

## Assumptions
- JSON/JSONL scenario folders are the primary replay format; the database remains a source and oracle, not the replay runtime store.
- `IMP-18` includes minimal normalized capture plus replay, not raw-frame archival. Raw websocket frame preservation can be added later if needed.
- Replay is a dedicated binary now; richer multi-command admin tooling stays for later tasks.
- The captured `resolved_config.json` is authoritative for replay; replay does not re-resolve env or DB config during the run.
- Execution replay consumes captured venue/order events and does not synthesize fills unless a later scenario class explicitly opts into paper-style simulation.
