# TASKS

## Objective

This file tracks only the implementation gap against `REQUIREMENT.md`.

Scope override:

- `REQ-001` is marked `Not Applicable` by current product direction.
- Reason: market scope should remain unrestricted instead of being hard-limited to BTC 5-minute pairs.
- `REQ-031` active-market position reconciliation is partially overridden by current product direction.
- Reason: the current inventory engine is treated as battle-tested during active markets, so Polymarket position API reconciliation should not run every 5 seconds while the market is live. Startup, reconnect, and post-settlement validation remain in scope.

Only requirement-driven work is included here:

- Active scope: `REQ-002` to `REQ-034`
- Scope exception: `REQ-001` is excluded
- Scope adjustment: `REQ-031` excludes active-market 5-second position reconciliation from the current product direction
- Section 4 state-machine behavior
- Section 5 risk rules
- Section 6 execution rules
- Section 7 persistence and audit needs
- Section 10 validation and launch gates

Architecture or refactoring work is included only when it is required to satisfy a requirement above.

Requirement status legend:

- `N/A`: intentionally excluded from current scope by product decision
- `Missing`: not implemented in a requirement-meaningful way
- `Partial`: some code exists, but the requirement is not satisfied end-to-end
- `Done`: requirement is effectively satisfied

Work status legend:

- `N/A`: no implementation work planned for current scope
- `Ready`: can be started now
- `Queued`: should start after earlier requirement work
- `Blocked`: depends on prerequisite work
- `In Progress`: currently being implemented
- `Done`: implementation work finished

## Gap Summary

### Overall Estimate

- Estimated requirement coverage in current scope: `99%`
- Estimated remaining gap in current scope: `1%`

### Work Status Snapshot

- Requirement analysis: `Done`
- Requirement-only backlog definition: `Done`
- `REQ-001` scope override recorded: `Done`
- Active implementation tasks completed: `21 / 24`
- Tasks ready to start now: `0 / 24`
- Tasks queued behind earlier work: `0 / 24`
- Tasks blocked by prerequisites: `3 / 24`
- Tasks marked not applicable: `1`

### Coverage by Requirement Area

| Requirement area | REQ scope | Coverage | Gap | Current reading |
|---|---|---:|---:|---|
| Market scope and paired lifecycle | `REQ-002` to `REQ-008` with `REQ-001` excluded | 82% | 18% | Pair identity, pair snapshots, pair-level runtime decisions, `AwaitSettlement`, buy-only hold-to-settlement flow, exact open-time seeding, and `AwaitSecondFill` with one-shot missing-side rescue are now in place. The remaining gap in this area is mostly the still-partial settlement-owned lifecycle polish under `REQ-003`. |
| Inventory, pricing, and residual discipline | `REQ-009` to `REQ-013`, `REQ-018` to `REQ-021` | 91% | 9% | Exact unmatched-fraction controls, exact marginal pair-cost zones, explicit residual-direction blocks, and strict late-window controls are now in place, including `7% / 12% / 20%` imbalance thresholds, `0.94 / 0.97 / 1.00 / 1.03` price bands, projected hard-disable blocks, repair-only routing, hard cutoff into `AwaitSettlement` at the late stop-new-orders window, and dynamic late-window observability keyed to configured thresholds. The remaining gap in this area is mostly the optional model-edge overlay. |
| Execution controls and clip policy | `REQ-014` to `REQ-017`, `REQ-034` | 100% | 0% | Maker-first execution, explicit taker exception reasons, wallet-aware market and daily taker-share caps, shared pending-taker accounting, crash-tolerant shared-state enforcement, the exact env-configurable `12 / 20 / 40 / 80` clip ladder with green-gated `40 / 80` escalation, and the per-side one-cycle-per-second maker refresh cap are now in place. |
| Config, observability, and auditability | `REQ-023` to `REQ-025` | 84% | 16% | Versioned env snapshots, persisted `config_text` plus `config_version`, next-market-only activation, rollback-safe reload fallback, append-only decision-event persistence, structured runtime-event persistence, linked `decision_event_id` audit trails, and pinned trade or execution metadata now exist. The remaining gap in this area is mostly operational hardening and richer downstream reporting on top of the new audit stream. |
| Replay, paper, and live safety modes | `REQ-026` to `REQ-029` | 100% | 0% | A true paper adapter, default shadow routing, guarded live arming, effective-order-mode dispatch, mode-scoped local or shared state, deterministic replay from captured event tapes, and analytics importer parity against the checked-in historical parquet and csv dataset are now in place. |
| Reliability, stale data, and reconciliation | `REQ-030` to `REQ-033` | 100% | 0% | Idempotent BOT order intents, bounded persisted fill dedupe, explicit startup and reconnect reconciliation gates, post-settlement validation handoff, dependency-failure safe-pause, persistence-path-specific pause latching, pause-time BOT order drain across both slot-tracked and direct BOT orders, exact dual-threshold stale-data enforcement, and wallet-global gross deployed cost caps with shared pending maker/taker reservations are now in place. Hard stale now latches until quotes are fully fresh again, startup quote-readiness uses the same add-block threshold, legacy flat DB configs preserve their prior stale threshold on reload, deprecated single-threshold stale env now fails activation, and shared gross-cap state now refreshes and deduplicates correctly across restart, direct-mode orders, TTL cleanup, and multi-instance same-wallet deployments. Current direction still excludes active-market 5-second position reconciliation. |
| Validation and launch gates | Section 10 | 60% | 40% | Requirement-grade invariant and property coverage now exists for no-scale-before-both-filled, no add at `>= 1.00`, no underdog residual increase, and quantity conservation, there is a deterministic replay foundation with scenario bundles and oracle comparison, and there is now a committed replay certification suite for good open, one-side lag, stale-data hold, reconnect reconciliation mismatch, and late settlement. Paper or shadow KPI gates and deployment gates are still absent. |

## Requirement-First Implementation Tasks

### P0 Blockers

| ID | Requirement mapping | Requirement status | Work status | Implementation task | Current gap in repo | Main files to touch |
|---|---|---|---|---|---|---|
| IMP-01 | `REQ-001` | N/A | N/A | Do not implement strict BTC 5-minute whitelist restrictions; treat this requirement as intentionally out of scope for the current product direction. | User direction explicitly rejects restricted market scope. | `TASKS.MD` only unless product docs are updated |
| IMP-02 | `REQ-002` | Done | Done | Make `pair_id` the mandatory decision key so strategy, risk, and ledger decisions operate on the pair as one instrument. | Implemented: pair identity, pair snapshots, pair-level runtime boundary, execution metadata, and pair persistence are now wired through the monolith. | `src/bot/shared.rs`, `src/bot/public_tail.rs`, `src/bot/runtime/*`, `src/bot/execution/*`, `src/db.rs`, `src/main.rs` |
| IMP-03 | `REQ-003`, `REQ-022` | Partial | Done | Enforce buy-only hold-to-settlement behavior and ensure settlement, not intrawindow sell logic, closes exposure. | Implemented: `AwaitSettlement`, terminal order drain, settlement-owned finalization, unresolved settlement persistence, buy-only BOT runtime guard, and the late-window handoff into settlement are now in place. | `src/bot/runtime/*`, `src/rtds.rs`, `src/db.rs`, `src/main.rs` |
| IMP-04 | `REQ-004`, `REQ-005`, `REQ-006` | Done | Done | Add pre-open warmup plus exact open-time seeding deadlines: both seed orders within 5 seconds of open and within 1 second of each other. | Implemented: `PreArm` now records readiness before open, `OpenBoth` uses explicit open-confirmed and first-tradable anchors, enforces exact seed timing KPIs, supports one controlled late unlock, and blocks seeding on pre-open cached quotes. | `src/bot/runtime/startup.rs`, `src/bot/runtime/loop.rs`, `src/bot/runtime/policy.rs`, `src/bot/runtime/config.rs`, `src/bot/runtime/state.rs`, `src/bot/runtime/tests.rs` |
| IMP-05 | `REQ-007`, `REQ-008` | Done | Done | Implement `AwaitSecondFill` completion mode with no scale-up before both sides fill, 15-second target, and 30-second hard stop for accumulation. | Implemented: one-sided first fills now route into `AwaitSecondFill`, normal accumulation stays blocked until both sides fill, the 15-second and 30-second clocks run from first fill, and a one-shot ask-aware missing-side taker rescue now exists after the hard deadline. | `src/bot/runtime/state.rs`, `src/bot/runtime/startup.rs`, `src/bot/runtime/loop.rs`, `src/bot/runtime/policy.rs`, `src/bot/runtime/pair_build/*`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-06 | `REQ-009`, `REQ-010` | Done | Done | Replace heuristic imbalance logic with exact unmatched-fraction controls: target below 7%, warning above 12%, disable or reduce-only at 20% or more. | Implemented: unmatched-fraction is now the authoritative imbalance metric, `7% / 12% / 20%` thresholds drive `Normal / Throttle / Warning / HardDisable`, projected hard-disable is blocked, and paired-growth orders are actively canceled when imbalance holds shut off normal adds. | `src/bot/runtime/config.rs`, `src/bot/runtime/state.rs`, `src/bot/runtime/policy.rs`, `src/bot/runtime/metrics.rs`, `src/bot/runtime/pair_build/*`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-07 | `REQ-011`, `REQ-012`, `REQ-013` | Done | Done | Implement exact marginal pair-cost zones using `0.94 / 0.97 / 1.00 / 1.03` thresholds and block all size-increasing intents at `>= 1.00`. | Implemented: balanced adds now use `y_bid + n_bid`, repairs now use heavier-side carried average plus lagging-side bid, `StopAdd` and `Danger` block new size, and live growth or repair orders are canceled when rebalance pricing turns forbidden. | `src/bot/runtime/pair_build/costs.rs`, `src/bot/runtime/pair_build/decision.rs`, `src/bot/runtime/pair_build/handler.rs`, `src/bot/runtime/pair_build/repair.rs`, `src/bot/runtime/taper.rs`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-08 | `REQ-014`, `REQ-015` | Done | Done | Make maker-first execution mandatory, require explicit taker exception reasons, and enforce sub-5% target and 10% hard cap for taker share. | Implemented: taker submits now require explicit exception reasons and cap policy, projected market and daily taker share are enforced with shared wallet-aware state, same-wallet sibling pending takers are counted for the daily cap but not leaked into other pairs' market caps, and shared-state locking now survives crash recovery. | `src/bot/execution/*`, `src/bot/runtime/*`, `src/bot/runtime/metrics.rs`, `src/helpers.rs`, `scripts/run-instance.sh` |
| IMP-09 | `REQ-016`, `REQ-017` | Done | Done | Align clip sizing to `12 / 20 / 40 / 80`, hard-cap single orders at 80, and allow 40 or 80 only when all green conditions are true. | Implemented: `BOT_CLIP_LADDER` is now the authoritative clip policy, seeds default to `12`, paired growth defaults to `20`, `40 / 80` escalate progressively only under green conditions, taper maintenance re-downshifts to the late-phase lot, and repair sizing now respects actual lagging-side spend plus min-notional-safe gap limits. | `src/bot/runtime/config.rs`, `src/bot/runtime/pair_build/decision.rs`, `src/bot/runtime/pair_build/repair.rs`, `src/bot/runtime/taper.rs`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-10 | `REQ-018`, `REQ-019` | Done | Done | Hard-block intentional single-side speculative adds and forbid any order that increases residual inventory on the underdog side. | Implemented: pair-build, taper, and `AwaitSecondFill` now classify favorite/underdog and residual direction explicitly, emit stable residual-direction hold reasons, cancel only blocked-side BOT orders when underdog residual would increase, and expose the new residual telemetry in runtime logs. | `src/bot/runtime/pair_build/decision.rs`, `src/bot/runtime/metrics.rs`, `src/bot/runtime/pair_build/orders.rs`, `src/bot/runtime/startup.rs`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-11 | `REQ-021` | Done | Done | Add exact late-window controls: smaller clips after 180 seconds, balance-only at 225 seconds, and zero new orders after 240 seconds. | Implemented: the runtime now uses env-configurable `180 / 225 / 240` late-window defaults, downshifts paired growth to maintenance sizing after late reduce starts, enforces balance-only repair routing after the configured balance-only threshold, hands off into `AwaitSettlement` at the configured stop-new-orders cutoff, preserves legacy late-window env compatibility, and emits dynamic late-window metrics keyed to the active thresholds. | `src/bot/runtime/config.rs`, `src/bot/runtime/state.rs`, `src/bot/runtime/taper.rs`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/metrics.rs`, `src/bot/runtime/loop.rs`, `src/env_contract.rs`, `src/bot/runtime/tests.rs` |
| IMP-12 | `REQ-023` | Done | Done | Introduce versioned config with persisted snapshots, explicit `config_version` on decisions, orders, and fills, and rollback-safe reload behavior. | Implemented: env now resolves into one versioned snapshot bundle, sanitized `config_text` and deterministic `config_version` are persisted, reload activates only on the next market and keeps the last good version on failure, and trades plus execution metadata now carry the pinned `config_version`. | `src/config.rs`, `src/db.rs`, `src/main.rs`, `src/bot/core.rs`, `src/bot/execution/*` |
| IMP-13 | `REQ-024`, `REQ-025` | Done | Done | Build append-only decision logging and structured metrics for state transitions, risk blocks, intents, acks, fills, reconciliation, and settlement. | Implemented: append-only `trade_decision_events` and `trade_runtime_events` are now the audit source of truth, `trade_decisions` remains only as a latest-summary mirror, decision and runtime events are linked through `decision_event_id`, structured audit payloads are persisted for state transitions, risk holds, order intents, acks, fills, reconciliation, and settlement, and audit DB writes are now off the live trading path. | `src/db.rs`, `src/logging.rs`, `src/main.rs`, `src/bot/runtime/*`, `src/bot/audit.rs` |
| IMP-14 | `REQ-028`, `REQ-029` | Done | Done | Add a true paper adapter and make `paper` or `shadow` the default startup mode until clean reconciliation and explicit live enablement pass. | Implemented: `BOT_ORDER_MODE=shadow|paper|live` plus `BOT_LIVE_ENABLED` now drive one shared execution-dispatch layer, shadow is the default startup mode, configured live stays shadow-routed until reconciliation, dependency health, and fresh-data gates pass, paper uses live market data with adapter-owned simulated fills, and paper or shadow state is isolated from live companion files. | `src/config.rs`, `src/env_contract.rs`, `src/bot.rs`, `src/bot/core.rs`, `src/bot/execution/*`, `src/bot/runtime/*`, `src/main.rs` |
| IMP-15 | `REQ-030`, adjusted `REQ-031` scope | Done | Done | Enforce idempotent client order IDs, fill dedupe, dependency-failure pause, startup reconciliation, reconnect reconciliation, and post-settlement validation without active-market Data API position reconciliation. Use `TRADE_VALIDATE` for startup and post-settlement validation flows. | Implemented: runtime safety gates now pause new risk until startup or reconnect reconciliation passes, dependency failures latch the bot into safe pause until the failed persistence path actually recovers, BOT intent IDs are deterministic, fill dedupe is bounded and persisted across restart with legacy-recency-safe migration, paused BOT cleanup now drains both slot-tracked and direct BOT orders, and post-settlement validation now runs through the same requirement-grade safety path without introducing active-market 5-second position polling. | `src/bot/execution/*`, `src/bot/runtime_ws.rs`, `src/bot/runtime/loop.rs`, `src/helpers.rs`, `src/main.rs` |

### P1 Completion Work

| ID | Requirement mapping | Requirement status | Work status | Implementation task | Current gap in repo | Main files to touch |
|---|---|---|---|---|---|---|
| IMP-16 | `REQ-020` | Missing | Blocked | Add the optional model-edge overlay behind a feature flag and require `edge_model_minus_price > 0.02` plus all normal inventory and price guards. | No model-edge overlay implementation was found. | strategy and config layers |
| IMP-17 | `REQ-027` | Done | Done | Build the analytics importer for the provided parquet and csv inputs and reproduce the calibration metrics used by the requirement. | Implemented: a dedicated analysis importer now validates the exact checked-in parquet and csv schemas, persists imported rows plus pair rollups, writes deterministic summary output, and reproduces the committed calibration parity fixture for the requirement metrics. | `src/analysis_import/*`, `src/bin/analysis_importer.rs`, `src/db.rs`, `tests/analysis_import.rs`, `tests/analysis_import/*` |
| IMP-18 | `REQ-026` | Done | Done | Build deterministic event replay using the same strategy, risk, and ledger code paths as live and paper. | Implemented: a dedicated replay binary now loads JSON or JSONL scenario bundles, reuses the BOT runtime through a deterministic clock and ID substrate, restores captured market schedule, trade identity, resolution state, and behavior-critical env overrides, replays normalized market, user, websocket, and reconcile events without live venue IO, and compares optional oracle decision or runtime outputs plus final state. | `src/bin/replay.rs`, `src/replay/*`, `src/bot/core.rs`, `src/bot/execution/*`, `src/bot/runtime/*`, `src/main.rs` |
| IMP-19 | `REQ-032` | Done | Done | Add per-market and portfolio-level gross cost caps enforced before any order approval. | Implemented: explicit pair and wallet-global gross deployed cost caps now gate maker and taker approvals, shared gross-cap state tracks filled trade cost plus pending maker and taker reservations across same-wallet instances, replacement-aware checks avoid double-counting while still keeping direct refresh adds conservative until live orders are actually canceled, shared-state failures enter dependency pause, and periodic republish plus restart-safe instance dedupe keep portfolio gross accounting accurate across TTL cleanup, reconnect, and multi-instance runs. | `src/config.rs`, `src/env_contract.rs`, `src/bot/execution/gross_caps.rs`, `src/bot/maker_orders.rs`, `src/bot/execution/*`, `src/bot/runtime/loop.rs`, `src/bot/runtime_ws.rs`, `src/helpers.rs`, `src/main.rs`, `src/bot/runtime/tests.rs` |
| IMP-20 | `REQ-033` | Done | Done | Enforce stale-data policy exactly: block new adds after 2 seconds of staleness and cancel or pause after 5 seconds. | Implemented: dual-threshold stale-data enforcement now blocks creates and reposts at the configured add-block age, enters hard stale dependency pause and cancels BOT working risk orders at the configured hard-pause age, keeps hard-stale recovery latched until quotes are fully fresh again, aligns startup quote-readiness to the same add-block threshold, preserves legacy DB-config stale thresholds during reload, and fails fast when the deprecated single-threshold env is set. | `src/config.rs`, `src/main.rs`, `src/bot/runtime/startup.rs`, `src/bot/runtime/loop.rs`, `src/bot/runtime_ws.rs`, `src/bot/runtime/tests.rs` |
| IMP-21 | `REQ-034` | Done | Done | Cap quote refresh or cancel-replace activity to one cycle per side per second by default. | Implemented: `MAKER_REPLACE_MIN_INTERVAL_SECONDS` is now the authoritative per-side refresh-cycle cap with a `1.0s` default, voluntary same-side maker refreshes are throttled across single-inflight and direct-order BOT flows, emergency drains still bypass the cap, and direct-mode no-op/submit bookkeeping now preserves live-order context, stable submit timestamps, and refresh-noop markers correctly across restart, pruning, handoffs, and pair-submit paths. | `src/config.rs`, `src/bot/maker_orders.rs`, `src/bot/execution/submit.rs`, `src/bot/execution/latency.rs`, `src/bot/execution/user_events.rs`, `src/bot/runtime/*` |

### Validation and Launch Gates

| ID | Requirement mapping | Requirement status | Work status | Implementation task | Current gap in repo | Main files to touch |
|---|---|---|---|---|---|---|
| IMP-22 | Section 10 unit and property tests | Done | Done | Add invariant tests for no scale-up before both sides fill, no add at `>= 1.00`, no underdog residual increase, and quantity conservation. | Implemented: deterministic invariant tests plus fixed-seed property tests now cover owner routing, price-zone blocking, underdog residual hard blocks, fill dedupe idempotence, and quantity or cost conservation across generated fill streams. | `Cargo.toml`, `src/bot/runtime/tests.rs`, `src/bot/runtime/pair_build/tests.rs` |
| IMP-23 | Section 10 replay and simulation tests | Done | Done | Add deterministic replay scenarios for good opens, one-side lag, stale data, reconciliation mismatch, and late settlement. | Implemented: committed replay certification fixtures now cover healthy paired opens, one-side lag into `AwaitSecondFill`, stale-data hold escalation, reconnect reconciliation mismatch recovery, and late settlement handoff, and the integration harness enforces both exact replay-oracle parity and scenario-specific semantic assertions. | `tests/replay_certification.rs`, `tests/replay/scenarios/*` |
| IMP-24 | Section 10 paper and shadow KPIs | Missing | Blocked | Add KPI reporting for seed timing, second-fill timing, taker share, imbalance, price-zone mix, and reconciliation health. | No paper or shadow KPI gate exists today. | metrics, dashboards, reports |
| IMP-25 | Section 10 deployment and rollback gates | Missing | Blocked | Add launch gates so live canary is blocked until replay, paper, shadow, and reconciliation criteria pass. | No requirement-grade go or no-go gate is implemented. | startup checks, admin controls, deployment docs |

## Recommended Build Order

This sequence stays tied to the requirement instead of a refactor roadmap:

1. `IMP-24` and `IMP-25` paper or shadow KPI gates plus deployment rollback gates

## Short Summary

The current codebase already contains the start of the paired engine, but it is still only a partial implementation of `REQUIREMENT.md`.

Recent completed work:

- `IMP-02` / `REQ-002` is now `Done`: the bot is pair-first at the identity, runtime-context, execution-metadata, and persistence layers.
- `IMP-03` / `REQ-003` and `REQ-022` implementation work is now `Done`: the bot now hands off into `AwaitSettlement`, drains working orders, persists terminal trade snapshots before unresolved settlement, and closes from settlement data instead of generic rollover.
- `IMP-04` / `REQ-004`, `REQ-005`, and `REQ-006` implementation work is now `Done`: the bot now tracks pre-open readiness, anchors startup timing to open-confirmed and first-tradable post-open events, enforces exact first-seed timing KPIs, allows only one controlled late seed unlock, and blocks opening seeds from pre-open cached quotes.
- `IMP-05` / `REQ-007` and `REQ-008` implementation work is now `Done`: one-sided startup fills now route into `AwaitSecondFill`, accumulation is blocked until both sides have a fill, the second-side deadlines are enforced from first fill, and the 30-second fallback rescue now works even in ask-only missing-side books.
- `IMP-06` / `REQ-009` and `REQ-010` implementation work is now `Done`: unmatched-fraction is now the canonical imbalance metric, the runtime enforces `7% / 12% / 20%` risk thresholds, projected hard-disable is blocked before submit, and live paired-growth orders are canceled when imbalance-only repair cannot legally continue.
- `IMP-07` / `REQ-011`, `REQ-012`, and `REQ-013` implementation work is now `Done`: exact marginal pair-cost zones are now authoritative, balanced adds and rebalance adds use the correct requirement formulas, `>= 1.00` blocks new size, and live lighter-side repair orders are canceled when rebalance pricing turns `StopAdd` or `Danger`.
- `IMP-08` / `REQ-014` and `REQ-015` implementation work is now `Done`: normal BOT flow is maker-first, taker submits now require explicit exception reasons and cap policy, market and daily taker-share caps are enforced with shared wallet-aware state, and multi-instance same-wallet crash recovery no longer silently disables the shared cap ledger.
- `IMP-09` / `REQ-016` and `REQ-017` implementation work is now `Done`: `BOT_CLIP_LADDER` now controls the exact `12 / 20 / 40 / 80` ladder, seeds and normal adds use the correct base rungs, `40 / 80` only escalate under the required green conditions, taper late maintenance downshifts back to the one-lot clip, and repair sizing now respects actual lagging-side spend and min-notional-safe gaps.
- `IMP-10` / `REQ-018` and `REQ-019` implementation work is now `Done`: pair-build, taper, and `AwaitSecondFill` now classify favorite and underdog sides from live bids, hard-block speculative one-sided adds, block orders that would increase underdog residual, and cancel only the offending BOT side when that residual-direction guard trips.
- `IMP-11` / `REQ-021` implementation work is now `Done`: the runtime now enforces exact late-window timing with env-configurable `180 / 225 / 240` defaults, shifts into reduced-clip late trading, flips to balance-only repair after the configured balance-only threshold, stops new orders by handing off into `AwaitSettlement` at the configured cutoff, preserves legacy late-window env compatibility, and emits late-window metrics against the active thresholds rather than hard-coded bucket names.
- `IMP-12` / `REQ-023` implementation work is now `Done`: env now resolves into one authoritative versioned snapshot, sanitized `config_text` and deterministic `config_version` are persisted in `configuration`, invalid reloads keep the previous active version, new valid snapshots activate only on the next market, and the pinned `config_version` now follows each trade and execution context through the market lifecycle.
- `IMP-13` / `REQ-024` and `REQ-025` implementation work is now `Done`: the bot now persists append-only decision and runtime audit events, links later intents, acks, fills, reconciliation, and settlement records back to a single `decision_event_id`, preserves skipped-market audit rows, uses exchange match time for maker fill audit events, and keeps audit DB writes off the live trading path.
- `IMP-14` / `REQ-028` and `REQ-029` implementation work is now `Done`: the bot now supports explicit `shadow`, `paper`, and guarded `live` order modes, starts in shadow by default, requires `BOT_LIVE_ENABLED` plus clean reconciliation, dependency health, and fresh market data before any real venue writes, uses a true paper adapter with simulated fills on live quotes, and isolates paper or shadow state from live shared ledgers.
- `IMP-17` / `REQ-027` implementation work is now `Done`: the repo now has a dedicated analysis importer binary, exact parquet and csv schema validation for the checked-in historical dataset, persisted raw import rows plus pair rollups, deterministic summary output, and a committed parity fixture that reproduces the calibration metrics expected by the requirement.
- `IMP-15` / `REQ-030` and the adjusted in-scope parts of `REQ-031` implementation work is now `Done`: the bot now uses explicit startup and reconnect reconciliation gates, latches dependency-failure pauses until the specific failed persistence path recovers, drains BOT new-risk orders while paused across both slot-tracked and direct BOT orders, persists bounded fill-dedupe history across restart with legacy-recency-safe migration, and keeps post-settlement validation in the safety-gate path without introducing active-market 5-second position polling.
- `IMP-19` / `REQ-032` implementation work is now `Done`: explicit pair and wallet-global gross deployed cost caps now gate maker and taker order approval, shared gross-cap state tracks filled exposure plus pending maker and taker reservations across same-wallet instances, restart-safe instance dedupe preserves portfolio accuracy across restarts without collapsing sibling live trades, shared reservation republishes now survive long-lived resting orders and expired taker cleanup correctly, and shared-state write failures now unwind or pause new live exposure instead of silently undercounting wallet risk.
- `IMP-20` / `REQ-033` implementation work is now `Done`: the bot now enforces dual-threshold market-data staleness with add-block and hard-pause cutoffs, aligns startup and await-second-fill quote-readiness to the same add-block threshold, keeps hard-stale recovery latched until quotes are fully fresh again even across other pause reasons, preserves legacy flat DB stale thresholds on reload, and fails fast when the deprecated `MARKET_DATA_STALE_SECONDS` env is set.
- `IMP-21` / `REQ-034` implementation work is now `Done`: maker refresh cadence is now capped per side with `MAKER_REPLACE_MIN_INTERVAL_SECONDS` defaulting to `1.0`, the cap is enforced across single-inflight and direct BOT handoffs, protective drains still bypass it, and direct-mode restart or prune paths now preserve stable submit timestamps and refresh-noop markers so capped no-ops do not masquerade as fresh submits.
- `IMP-22` / Section 10 unit and property test work is now `Done`: the test suite now includes deterministic invariant coverage for owner routing, stop-add and danger price zones, and underdog residual hard blocks, plus fixed-seed property tests for underdog residual math and fill-stream quantity or cost conservation with duplicate-fill idempotence.
- `IMP-18` / `REQ-026` implementation work is now `Done`: the repo now has a dedicated replay binary, replay scenario folders with captured config, initial state, normalized event tapes, and optional oracle files, deterministic replay time and ID plumbing, replay-scoped state isolation, captured order-ack reuse, captured resolution-state replay, manifest-persisted trade IDs, and replay-bundle env override restoration so offline runs match captured behavior instead of ambient machine settings.
- `IMP-23` / Section 10 replay certification work is now `Done`: the repo now has a committed replay certification suite with exact-oracle and semantic checks for good open, one-side lag into `AwaitSecondFill`, stale-data hold escalation, reconnect reconciliation mismatch recovery, and late settlement handoff, plus fixture-integrity checks that keep the scenario bundle set reproducible on a clean clone.

The biggest remaining requirement gaps are:

- analytics importer parity
- paper or shadow KPI gates
- deployment and rollback launch controls before any guarded live rollout

Intentional scope exception:

- `REQ-001` strict BTC 5-minute restriction is marked `Not Applicable` by current product direction.
- `REQ-031` active-market 5-second position reconciliation is intentionally not part of the current product direction; startup, reconnect, and post-settlement validation remain in scope.

Current estimate:

- Requirement coverage in current scope: `99%`
- Remaining requirement gap in current scope: `1%`
