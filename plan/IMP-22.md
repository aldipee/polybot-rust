# IMP-22 Plan: Section 10 Invariant and Property Tests

## Summary
- Add requirement-first validation around `REQ-007`, `REQ-013`, `REQ-019`, and the Section 10 quantity invariants.
- Keep this task test-first: no intended runtime behavior changes. Only allow tiny visibility-only changes if a pure helper cannot be reached from the existing test modules.
- Use deterministic table tests for discrete state-machine and risk boundaries, and `proptest` for arithmetic and ledger invariants.

## Important Changes
- `Cargo.toml`: add `[dev-dependencies] proptest = "1"`; no production dependency or runtime API change.
- `src/bot/runtime/tests.rs`: add owner-routing and fill-ledger invariant coverage using the existing BOT runtime test harness.
- `src/bot/runtime/pair_build/tests.rs`: add paired-cost and underdog-residual invariant sweeps using the current pure pair-build helpers.
- If a helper is not reachable from the existing test modules, widen visibility only to `pub(in crate::bot)`; do not add public APIs just for tests.

## Implementation Changes
- Owner-routing invariants:
  - Exhaustively check `OpenBoth`, `PairBuild`, and `Taper` with one-sided inventory on either side.
  - Assert the owner is always `AwaitSecondFill` until both sides have a fill.
  - Include both-zero, both-live, and startup-hard-pause boundary rows so the invariant is pinned to the real state machine.
- Price-zone invariants:
  - Sweep balanced-add and rebalance-add marginal costs around `0.999`, `1.000`, `1.029`, and `1.030`.
  - Assert `>= 1.00` always maps to `StopAdd` or `Danger` and produces a blocking hold reason.
  - Assert `< 1.00` never produces a stop-add hold in the pure price-zone helper.
- Underdog-residual invariants:
  - Add exact-gap, undershoot, overshoot, and existing-underdog cases for `LighterSideFirst`.
  - Assert no approved one-sided add may create or worsen residual on the underdog side.
  - Add a small property test over randomized `q_yes`, `q_no`, `clip`, and `side` inputs that compares projected residual side and magnitude with the hard-block helper.
- Quantity-conservation invariants:
  - Add a deterministic multi-fill sequence with BUY, SELL, and duplicate trade-key replay.
  - Assert side qty, side cost, total cost, paired qty, and unmatched qty remain internally consistent after every step.
  - Add a property test over randomized fill streams with dedupe keys to prove duplicate fills are idempotent and `paired_qty + residual_qty == total_side_qty`, with unmatched fraction always in `[0, 1]`.
- Keep property tests small and stable:
  - use explicit `ProptestConfig` with a modest case count such as `128`
  - use fixed seeds where reproducibility matters
  - avoid network or shared-file-state coupling inside generated tests

## Test Plan
- `cargo test --quiet` stays green with the new dev-dependency and generated cases.
- Required scenarios:
  - one-sided inventory on `YES` and `NO` in each pre-settlement phase
  - balanced-add and rebalance-add at `0.999`, `1.000`, `1.029`, `1.030`
  - favorite residual, no residual, exact-gap repair, overshoot to underdog, pre-existing underdog residual
  - duplicate fill replay after real fills
  - randomized fill streams mixing `BUY` and `SELL` without negative final inventory or cost state
- Acceptance criteria:
  - zero tests permit scale-up before both sides fill
  - zero tests permit a size-increasing add at `>= 1.00`
  - zero tests permit underdog residual increase
  - quantity and dedupe invariants hold across deterministic and generated cases

## Assumptions
- `IMP-22` stays scoped to unit and property tests only; replay, canary, paper, and shadow certification stay with `IMP-23` to `IMP-25`.
- Logs from later canary runs should create new fixtures or replay scenarios only after we decide the logged behavior is intended; they do not redefine these invariants automatically.
- If a pure helper is awkward to reach, prefer placing the test in the existing module test file over introducing a new cross-cutting harness.
