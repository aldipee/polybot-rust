# IMP-25 Plan: Deployment Gate and Alert-Only Rollback Gate

## Summary
- Add a requirement-grade deployment gate that blocks configured `live` startup unless the current build has a passing replay certification result and the target bot has a recent passing `shadow` KPI report.
- Keep enforcement inside the `polybot` runtime, but make it explicitly toggleable and disabled by default.
- Add an alert-only rollback monitor for configured-live sessions: it records and surfaces “manual rollback recommended” events, but it does not auto-demote or auto-drain beyond the bot’s existing safety demotions.
- Do not require `paper` KPI for live gating in v1. `paper` remains informative; `shadow` is the required runtime evidence.

## Public Interfaces
- Add new supported env:
  - `BOT_DEPLOYMENT_GATE_ENABLED`
    - default `false`
    - when `true`, configured `live` can arm only if deployment-gate evaluation returns `PASS`
  - `BOT_RUNTIME_ROLLBACK_GATE_ENABLED`
    - default `false`
    - when `true`, configured-live runs emit alert-only rollback recommendations on hard integrity or live-health failures
  - `BOT_DEPLOYMENT_GATE_REPLAY_MAX_AGE_HOURS`
    - required when `BOT_DEPLOYMENT_GATE_ENABLED=true`
    - operator-supplied freshness window for replay certification artifacts
  - `BOT_DEPLOYMENT_GATE_SHADOW_MAX_AGE_HOURS`
    - required when `BOT_DEPLOYMENT_GATE_ENABLED=true`
    - operator-supplied freshness window for `shadow` KPI artifacts
- Add new binaries:
  - `src/bin/replay_certify.rs`
    - runs the committed Section 10 replay certification suite outside `cargo test`
    - writes deterministic summary JSON and persists certification results
  - `src/bin/deployment_gate.rs`
    - evaluates deployment readiness for one `bot_id` using persisted replay-cert and shadow-KPI results plus the current freshness policy
    - writes deterministic summary JSON and persists deployment-gate results
- Add new persisted report types:
  - `replay_cert_run`
  - `replay_cert_case`
  - `deployment_gate_run`
- Add new runtime-event kinds:
  - `deployment_gate_check`
  - `deployment_gate_alert`

## Implementation Changes
### Replay Certification as a Runtime-Consumable Artifact
- Move the committed replay-certification case table and evaluation logic out of `tests/replay_certification.rs` into a shared library module, then keep the test harness as a thin wrapper around that shared logic.
- `replay_certify` should execute the same five committed certification scenarios and persist:
  - current `GIT_COMMIT_ID`
  - overall status
  - per-scenario case status
  - failure reasons
  - summary JSON path
  - execution timestamp
- Require runtime live gating to match replay certification by `GIT_COMMIT_ID`, not just by “latest pass”.

### Deployment Gate Evaluator
- Add a new `src/deployment_gate/` module with:
  - `DeploymentGatePolicy`
    - built from the four new env values above
  - `DeploymentGateReport`
    - includes build commit, bot id, source timestamps, replay status, shadow KPI status, freshness checks, and overall status
  - `run_deployment_gate(...)`
    - loads the latest persisted replay-cert report for the current build
    - loads the latest persisted `shadow` KPI report for the target `bot_id`
    - applies operator-supplied freshness windows
    - returns `PASS` only when:
      - replay certification exists, matches the current build, is fresh enough, and is `PASS`
      - shadow KPI exists, is fresh enough, and is `PASS`
    - treats `WARN`, `FAIL`, `INSUFFICIENT_SAMPLE`, missing evidence, build mismatch, or stale evidence as blocking
- `deployment_gate.rs` should call that shared evaluator, write `summary.json`, and persist `deployment_gate_run`.
- Runtime startup should use the same shared evaluator logic, not a second hand-rolled policy path.

### Runtime Enforcement
- Extend `_bot_runtime_live_block_reason()` so configured `live` also blocks on deployment-gate failure when `BOT_DEPLOYMENT_GATE_ENABLED=true`.
- Use a stable block-reason family such as:
  - `deployment_gate:replay_missing`
  - `deployment_gate:replay_build_mismatch`
  - `deployment_gate:replay_stale`
  - `deployment_gate:replay_failed`
  - `deployment_gate:shadow_kpi_missing`
  - `deployment_gate:shadow_kpi_stale`
  - `deployment_gate:shadow_kpi_not_pass`
- Emit one `deployment_gate_check` runtime event during startup or live-arm evaluation with the resolved policy, source timestamps, and final decision.
- Startup enforcement is explicit and conservative:
  - if the deployment gate is red at startup, the bot stays effective `shadow` for that run
  - do not hot-promote the same process later if a new passing report appears; operator promotion remains manual via a fresh start or next run

### Alert-Only Rollback Gate
- When `BOT_RUNTIME_ROLLBACK_GATE_ENABLED=true` and the bot has already armed live once:
  - emit a one-shot `deployment_gate_alert` runtime event with `manual_rollback_recommended` on the first `audit_drop`
  - emit a one-shot `deployment_gate_alert` runtime event with `manual_rollback_recommended` on the first post-arm live-to-shadow fallback caused by an existing live block reason
- Do not add any new automatic demotion, cancel-all, or flatten behavior in v1.
- Existing safety-driven effective-shadow fallback remains unchanged; `IMP-25` only adds structured alerting and rollout policy around it.

## Test Plan
- Replay-cert artifact tests:
  - `replay_certify` reproduces the same five certification scenarios as the test harness
  - persisted overall and per-case statuses are deterministic
  - current-build commit id is recorded and compared correctly
- Deployment-gate evaluator tests:
  - `PASS` only when current-build replay certification is `PASS` and fresh, and shadow KPI is `PASS` and fresh
  - block on replay missing, stale, failed, or build mismatch
  - block on shadow KPI missing, stale, `WARN`, `FAIL`, or `INSUFFICIENT_SAMPLE`
  - disabled gate leaves live-arming behavior unchanged
- Runtime integration tests:
  - configured-live with gate disabled behaves exactly as before
  - configured-live with gate enabled and blocking evidence stays effective `shadow` with a deployment-gate block reason
  - configured-live with gate enabled and passing evidence can arm live once the normal reconciliation and freshness gates are healthy
  - `deployment_gate_check` runtime event is emitted with the final decision
- Alert-only rollback tests:
  - post-arm `audit_drop` emits one `deployment_gate_alert`
  - post-arm live-to-shadow fallback emits one `deployment_gate_alert`
  - repeated occurrences do not spam duplicate alerts in the same run
  - paper and shadow-only runs do not emit rollback alerts
- Optional Postgres smoke coverage:
  - persist one replay-cert run, one deployment-gate run, and verify DB rows plus summary files

## Assumptions
- Runtime-native enforcement is the desired v1 surface.
- Both new gate toggles default to `false`.
- Live deployment gating requires replay certification plus `shadow` KPI only; `paper` KPI stays informative.
- Freshness windows are operator-supplied env values and must be explicitly set when deployment gating is enabled.
- Promotion remains manual: a passing gate after startup does not hot-promote the already-running process.
- Rollback is alert-only in v1: operators act manually after alerts, while existing safety logic continues to control effective live vs shadow behavior.
