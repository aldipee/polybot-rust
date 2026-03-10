# AGENTS.md

## 1. Overview
- Rust is the active implementation. Main binary: `polybot`.
- Core trading engine lives in `src/bot.rs`.
- Best runtime starting point in code:
  - `_maker_skew_arb_step(...)` in `src/bot.rs`
  - from there, follow:
    - `_maker_pair_base_step(...)`
    - `_maker_pair_base_recovery_phase(...)`
    - `_maker_pair_base_risk_exit_step(...)`
- Current strategy architecture is layered inside `EXEC_MODE=MAKER_SKEW_ARB`:
  - `RiskExitOnly`
  - `MergePending` / recovery
  - `PairBase`
  - `Skew`
- Step 1 baseline is pair-base + recovery. Step 2 skew is a guarded overlay on top of that baseline.
- `main.py` is a legacy reference. Do not treat it as the source of truth for current behavior unless explicitly comparing ports.
- Roadmap docs:
  - `TARGET_GOAL.md`
  - `TARGET_GOAL_STATUS.md`
  - `plans/SPRINT_*.md`

## 2. Setup / run commands
- Build release binary:
  - `cargo build --release --locked`
- Multi-instance workflow from repo docs:
  - `bash scripts/create-instance.sh <instance-name> <cpu>`
  - `bash scripts/run-instance.sh <instance-name>`
- Instance launcher expects the release binary at:
  - `target/release/polybot`
- Docker compose exists for containerized runs.
  - Verify service names in `docker-compose.yml` before starting containers.
- Verify direct foreground local run command before using it; repo docs primarily use the built binary or the instance scripts.

## 3. Test / lint / build commands
- Fast compile check:
  - `cargo check -q`
- Full test suite:
  - `cargo test -q`
- Release build:
  - `cargo build --release --locked`
- Targeted Rust test examples already used in this repo:
  - `cargo test --bin polybot bot:: -- --nocapture`
  - `cargo test --bin polybot bot_priority1_tests`
  - `cargo test --bin polybot bot_priority2_tests`
- Verify before relying on `cargo fmt` or `cargo clippy`; they are not part of the documented project workflow today.
- Useful scan commands:
  - `Get-ChildItem -Force`
  - `rg -n "fn _maker_skew_arb_step|fn _maker_pair_base_step|fn _maker_pair_base_recovery_phase|fn _maker_pair_base_risk_exit_step" src/bot.rs`
  - `rg -n "PAIR_BASE|MergePending|RiskExitOnly|PAIR_BASE_RECOVERY|PAIR_BASE_SKEW" src/bot.rs src/main.rs src/env_contract.rs`
  - `rg -n "pair_base_metrics_snapshot|\\[PAIR_BASE\\]\\[METRICS\\]|Updated trade row" src/main.rs src/bot.rs`
  - `git status --short`

## 4. Coding conventions
- Prefer small helper functions over expanding the main loops in `src/bot.rs`.
- Preserve execution ownership order:
  - `RiskExitOnly > MergePending > PairBase > Skew`
- Keep normal flow maker-first.
- Use taker only for explicit risk-exit / terminal behavior, not for ordinary Step 1 or Step 2 flow.
- When adding env vars:
  - wire them in `src/env_contract.rs`
  - document them in `ENVIRONMENT.md`
- Keep edits ASCII unless the file already requires otherwise.
- Add or update Rust tests for math helpers, control-path guards, and state-machine transitions.
- For pair-base work, prefer fee-net evaluations over gross LP-only heuristics.

## 5. File-specific rules
- `src/bot.rs`
  - Core execution logic, pair-base state machine, recovery, risk exit, scoring, skew overlay.
  - New pair-base / Step 2 behavior should be implemented here, not by reviving the legacy generic skew path.
  - Best read order inside this file:
    - shared math/helpers
    - pair-base metrics helpers
    - pair-base phase/state helpers
    - `_maker_pair_base_recovery_phase(...)`
    - `_maker_pair_base_risk_exit_step(...)`
    - `_maker_pair_base_step(...)`
    - `_maker_skew_arb_step(...)`
- `src/main.rs`
  - End-of-market metrics and final trade-row logging.
  - Keep summary logs consistent with runtime metric fields.
  - This is the best place to verify market-end behavior and metric emission.
- `src/env_contract.rs`
  - Allowlist new env keys here whenever runtime config changes.
  - Any new env without an allowlist entry is incomplete work.
- `main.py`
  - Legacy reference only.
  - Do not port new Rust behavior back into Python unless explicitly asked.
- `ENVIRONMENT.md`
  - Operator reference for active env keys.
  - Do not copy raw secrets into docs.
- `behaviour-<version>.md`
  - Version-specific runtime behavior summaries.
- `TARGET_GOAL.md` / `TARGET_GOAL_STATUS.md`
  - Strategy roadmap and gating status.
  - Update status when a blocker is actually cleared.
- `CHANGE_LOG`, `Cargo.toml`, `Cargo.lock`
  - Update together only when intentionally cutting a release.
- `plans/SPRINT_*.md`
  - Sprint plans and checklist status.
  - Update checklist state when implementation materially changes.
- Runtime artifacts:
  - `state/`, `output/`, `logs/`, `signals/`, `data/`
  - `maker_hedgecap_state_*.json`
  - Treat as generated/runtime files. Do not edit manually unless the task is explicitly about state repair.

## 6. Workflow expectations
- Check the worktree before editing.
- Do not revert unrelated user changes.
- Recommended workflow for future sessions:
  1. `git status --short`
  2. read `TARGET_GOAL.md`
  3. read `TARGET_GOAL_STATUS.md`
  4. read the current version behavior doc, e.g. `behaviour-0.1.26.md`
  5. read `ENVIRONMENT.md` if env behavior matters
  6. scan the runtime starting points in `src/bot.rs`
  7. inspect `src/main.rs` for final metrics / finalization side effects
  8. implement behavior in small helper-first patches
  9. run `cargo check -q`
  10. run `cargo test -q` for behavior-changing edits
  11. update docs/status files that the change actually affects
- Best way to scan the codebase:
  - start from the roadmap docs, then the current behavior doc, then the runtime entry points
  - prefer `rg` over broad file-by-file reading
  - trace from top-level execution down into helpers rather than reading `src/bot.rs` linearly
  - inspect metrics/log sinks in `src/main.rs` before assuming a runtime field is surfaced
- Exact starting points for strategy work:
  - top-level route: `_maker_skew_arb_step(...)`
  - Step 1 core: `_maker_pair_base_step(...)`
  - recovery ownership: `_maker_pair_base_recovery_phase(...)`
  - terminal handling: `_maker_pair_base_risk_exit_step(...)`
  - env wiring: `src/env_contract.rs`
  - market-end reporting: `pair_base_metrics_snapshot(...)` in `src/bot.rs` and final logs in `src/main.rs`
- Validate behavior-changing code with:
  - `cargo check -q`
  - usually `cargo test -q`
- After behavior or config changes, update the relevant docs:
  - `ENVIRONMENT.md`
  - `TARGET_GOAL_STATUS.md`
  - `behaviour-<version>.md`
  - relevant `plans/SPRINT_*.md`
- Keep Step 1 baseline intact while testing overlays like Step 2.
- Prefer canary-first rollout for strategy changes.
- If something is uncertain, add a short `Verify:` note instead of guessing.

## 7. Things to avoid
- Do not write secret values into repo docs.
- Do not treat runtime logs/state files as source code.
- Do not add normal-flow taker usage to Step 1 or Step 2 unless the task explicitly changes risk policy.
- Do not bypass pair-base ownership by routing new strategy features through the old generic skew loop.
- Do not bump version/changelog for ordinary internal edits unless a release is intended.
- Do not rely on gross LP alone when evaluating strategy behavior; fee-net and downside-floor metrics matter.
