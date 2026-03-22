# IMP-23 Plan: Section 10 Replay Certification Suite

## Summary
- Build a requirement-grade certification suite on top of the existing `IMP-18` replay engine.
- Keep this task test-only: no intended runtime behavior changes, no live rollout gating yet.
- Certify five committed deterministic scenarios: good open, one-side lag, stale-data hold, reconnect reconciliation mismatch, and late settlement.

## Important Changes
- Add a new integration test runner at `tests/replay_certification.rs` that uses the existing `polybot::replay::run_replay_scenario()` path as the exact-oracle gate.
- Add committed replay fixture folders under `tests/replay/scenarios/<scenario_id>/` using the current replay bundle format:
  - `manifest.json`
  - `resolved_config.json`
  - `events.jsonl`
  - `initial_state/`
  - `oracle_decisions.jsonl`
  - `oracle_runtime_events.jsonl`
  - `oracle_final_state.json`
  - optional `resolution_snapshot.json`
  - short `README.md` describing the scenario intent and requirement mapping
- Do not add a new operator CLI or rollout gate in `IMP-23`; `cargo test` is the certification entrypoint, and `IMP-25` remains responsible for go/no-go deployment enforcement.

## Implementation Changes
- Certification harness:
  - Add a `ReplayCertificationCase` table in the test runner with one row per scenario.
  - Each case hard-codes: scenario directory, Section 10 coverage label, expected final `phase` or `owner` or `safety_gate`, minimum audit counts, required runtime events, and forbidden runtime events.
  - The harness first validates fixture completeness, then runs `run_replay_scenario()` for exact oracle comparison, then parses the committed oracle files for semantic assertions so the suite proves the fixture actually covers the intended behavior.
- Scenario set:
  - `good_open_paired_seed`
    - Healthy market open, paired maker seed, both sides filled.
    - Final state ends `Healthy` and back in normal trading control.
    - Forbid `risk_block`.
  - `one_side_lag_await_second_fill`
    - One side fills and the other lags.
    - Require a `state_transition` into `AwaitSecondFill`.
    - Final owner remains `AwaitSecondFill` so the scenario certifies “no scale-up before both sides fill.”
  - `stale_data_hold_escalation`
    - Quote gap crosses stale-data protection.
    - Require `risk_block` rows for stale gating.
    - End the scenario while still held or explicitly recovering; the case table must pin whichever behavior is intended.
  - `reconnect_reconciliation_mismatch`
    - Include `ws_close`, `ws_open`, and `reconcile_snapshot` events with mismatched local versus exchange state.
    - Require reconnect-scoped `risk_block` plus reconciliation runtime rows.
    - Final `safety_gate` returns to `Healthy`.
  - `late_settlement_handoff`
    - Near-expiry handoff into settlement with committed `resolution_snapshot.json`.
    - Require `await_settlement_handoff` and `settled`.
- Fixture authoring rules:
  - Use minimal normalized event tapes, not raw websocket frames.
  - Keep fixtures self-contained: no ambient env dependence, no live RTDS lookup, no live DB state.
  - Commit oracle files generated from the current intended behavior and review them into source control together with the case-table assertions.
  - Treat canary or live logs as future fixture sources, not as an automatic oracle rewrite mechanism.

## Test Plan
- `cargo test --test replay_certification -- --nocapture` passes repeatedly on the same machine and on a clean machine.
- Each scenario must pass both:
  - exact replay reproduction against committed oracle files
  - semantic certification assertions from the case table
- Required semantic checks:
  - good open: no `risk_block`, healthy finish
  - one-side lag: `AwaitSecondFill` transition present
  - stale data: stale `risk_block` present
  - reconnect mismatch: reconnect hold plus reconciliation event, then healthy finish
  - late settlement: settlement handoff and settled event present
- Add one fixture-integrity test that fails if:
  - a scenario directory is missing required replay files
  - a scenario directory exists without a matching `ReplayCertificationCase`
  - the case table references a scenario directory that does not exist

## Assumptions
- `IMP-23` is certification only; KPI thresholds and deployment gates stay in `IMP-24` and `IMP-25`.
- The certification suite uses committed deterministic fixtures, mostly hand-authored from normalized events, not a mandatory capture-from-canary workflow.
- No dedicated oracle-refresh CLI is added in this task; oracle updates remain an explicit developer action when intended behavior changes.
