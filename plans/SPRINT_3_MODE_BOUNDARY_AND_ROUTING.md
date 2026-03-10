### Sprint 3: Mode Boundary And Routing

**Summary**

Implement `EXEC_MODE=SETTLEMENT_SHAPER` as a new top-level runtime path, not as a branch inside the existing `MAKER_SKEW_ARB` flow. The first PR for this checklist should be a read-only canary: it must route correctly, own its own loop/state/logging, and place no orders. This is necessary because the current generic maker loop in [src/bot.rs](c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs) still contains near-expiry and exposure handling that would flatten inventory and violate Sprint 3’s hold-to-settlement objective.

**Interfaces And Runtime Changes**

- Add `SETTLEMENT_SHAPER` as a supported runtime mode value for `EXEC_MODE`.
- In [src/bot.rs](c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs), factor loop selection into a small helper or enum so `run()` dispatch is explicit and testable:
  - sniper-like modes -> existing sniper loops
  - `SETTLEMENT_SHAPER` -> new dedicated `_run_settlement_shaper_loop()`
  - `MAKER_SKEW_ARB` and other maker/taker modes -> existing paths unchanged
- Do not route `SETTLEMENT_SHAPER` through `_maker_skew_arb_step(...)`, `PAIR_BASE_ENABLED`, or any old maker-skew flag combination.
- Add internal Sprint 3 skeleton types in [src/bot.rs](c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs):
  - `SettlementShaperPhase` with the full phase set from the spec
  - `SettlementShaperRuntimeState` with at least current phase, state-enter timestamp, and a startup-log/arming marker
- Add a pure helper that maps `t_into_s` to the Sprint 3 phase windows. For this checklist it is used only for state/logging, not trading behavior.
- Add `_settlement_shaper_step(now, q_yes, q_no, total_cost)` as a no-trade skeleton:
  - read quotes, inventory, time-left, and current phase
  - update runtime state and emit phase-transition logs
  - never submit, replace, cancel, hedge, or redeem orders
  - never call pair-base recovery, skew overlay, maker arb, or risk-exit logic
- Add a one-time startup config log for the new mode, mirroring the existing `[PAIR_BASE][CFG]` style:
  - prefix: `[SETTLEMENT_SHAPER][CFG]`
  - include mode, default phase budget slices, default target bands, and `phase_controller=time_based`
- Add phase transition logs with a dedicated prefix, for example `[SETTLEMENT_SHAPER] phase <old> -> <new>`.

**Implementation Details**

- Keep `MAKER_SKEW_ARB` behavior byte-for-byte equivalent except for the new top-level dispatch arm. No logic inside `_maker_skew_arb_step(...)` should change for this checklist.
- Reuse existing maker loop cadence (`loop_wait_seconds_maker`) inside `_run_settlement_shaper_loop()`, but give the new loop its own control flow so it does not inherit generic maker flattening behavior.
- For this checklist, use Sprint 3 default budget/target values as code defaults in [src/bot.rs](c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs). Do not add the full Sprint 3 env surface yet.
- No change is required in [src/env_contract.rs](c:/Works/aldipranata.com/polybot-convert-rust/src/env_contract.rs) for `EXEC_MODE` itself because that key is already allowlisted.
- Leave Telegram startup messaging unchanged in `main.rs`; the detailed Sprint 3 startup snapshot should be normal bot logs only in this task.

**Test Plan**

- Unit test the mode-classification helper so `SETTLEMENT_SHAPER` dispatches to the dedicated loop and `MAKER_SKEW_ARB` still maps to the existing maker path.
- Unit test the phase mapping helper at the exact boundaries: `0`, `30`, `60`, `180`, `240`, and `300` seconds.
- Unit test the default Sprint 3 config snapshot used by the startup log so the logged budget slices and target bands match the spec values.
- Run `cargo check -q`.
- Run targeted Rust tests covering the new pure helpers; full behavior tests can wait until the mode starts placing orders.

**Assumptions**

- First boundary PR is intentionally read-only: `SETTLEMENT_SHAPER` is allowed to log and hold state, but it must not trade yet.
- Full env wiring and `ENVIRONMENT.md` updates are deferred until the later Config/Docs checklist, when the mode starts consuming operator-facing Sprint 3 settings.
- `SettlementRedeem`, `EntryRepair`, `ShapeRepair`, and target-gap scoring are explicitly out of scope for this checklist and should not leak into this PR.
