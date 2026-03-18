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

- Estimated requirement coverage in current scope: `66%`
- Estimated remaining gap in current scope: `34%`

### Work Status Snapshot

- Requirement analysis: `Done`
- Requirement-only backlog definition: `Done`
- `REQ-001` scope override recorded: `Done`
- Active implementation tasks completed: `9 / 24`
- Tasks ready to start now: `5 / 24`
- Tasks queued behind earlier work: `4 / 24`
- Tasks blocked by prerequisites: `6 / 24`
- Tasks marked not applicable: `1`

### Coverage by Requirement Area

| Requirement area | REQ scope | Coverage | Gap | Current reading |
|---|---|---:|---:|---|
| Market scope and paired lifecycle | `REQ-002` to `REQ-008` with `REQ-001` excluded | 82% | 18% | Pair identity, pair snapshots, pair-level runtime decisions, `AwaitSettlement`, buy-only hold-to-settlement flow, exact open-time seeding, and `AwaitSecondFill` with one-shot missing-side rescue are now in place. The remaining gap in this area is mostly the still-partial settlement-owned lifecycle polish under `REQ-003`. |
| Inventory, pricing, and residual discipline | `REQ-009` to `REQ-013`, `REQ-018` to `REQ-021` | 82% | 18% | Exact unmatched-fraction controls, exact marginal pair-cost zones, and explicit residual-direction blocks are now in place, including `7% / 12% / 20%` imbalance thresholds, `0.94 / 0.97 / 1.00 / 1.03` price bands, projected hard-disable blocks, repair-only routing, live order cancellation when imbalance or rebalance pricing shuts off further adds, and hard blocks on speculative one-sided adds or underdog-residual increases. The remaining gap in this area is mostly strict late-window behavior and the optional model-edge overlay. |
| Execution controls and clip policy | `REQ-014` to `REQ-017`, `REQ-034` | 80% | 20% | Maker-first execution, explicit taker exception reasons, wallet-aware market and daily taker-share caps, shared pending-taker accounting, crash-tolerant shared-state enforcement, and the exact env-configurable `12 / 20 / 40 / 80` clip ladder with green-gated `40 / 80` escalation are now in place. The remaining gap in this area is mostly refresh-rate controls under `REQ-034`. |
| Config, observability, and auditability | `REQ-023` to `REQ-025` | 18% | 82% | Config snapshotting exists, but no full config versioning, append-only decisions, or requirement-grade metrics and logs. |
| Replay, paper, and live safety modes | `REQ-026` to `REQ-029` | 10% | 90% | No true replay engine, importer parity, paper adapter, or shadow-first guarded live flow is present. |
| Reliability, stale data, and reconciliation | `REQ-030` to `REQ-033` | 24% | 76% | There are partial dedupe and reconcile pieces. Current direction keeps startup, reconnect, and post-settlement validation in scope, but excludes active-market 5-second position reconciliation from the target behavior. |
| Validation and launch gates | Section 10 | 8% | 92% | A small unit-test set exists, but replay, simulation, paper KPI, and shadow KPI gates are mostly absent. |

## Requirement-First Implementation Tasks

### P0 Blockers

| ID | Requirement mapping | Requirement status | Work status | Implementation task | Current gap in repo | Main files to touch |
|---|---|---|---|---|---|---|
| IMP-01 | `REQ-001` | N/A | N/A | Do not implement strict BTC 5-minute whitelist restrictions; treat this requirement as intentionally out of scope for the current product direction. | User direction explicitly rejects restricted market scope. | `TASKS.MD` only unless product docs are updated |
| IMP-02 | `REQ-002` | Done | Done | Make `pair_id` the mandatory decision key so strategy, risk, and ledger decisions operate on the pair as one instrument. | Implemented: pair identity, pair snapshots, pair-level runtime boundary, execution metadata, and pair persistence are now wired through the monolith. | `src/bot/shared.rs`, `src/bot/public_tail.rs`, `src/bot/runtime/*`, `src/bot/execution/*`, `src/db.rs`, `src/main.rs` |
| IMP-03 | `REQ-003`, `REQ-022` | Partial | Done | Enforce buy-only hold-to-settlement behavior and ensure settlement, not intrawindow sell logic, closes exposure. | Implemented: `AwaitSettlement`, terminal order drain, settlement-owned finalization, unresolved settlement persistence, and buy-only BOT runtime guard are now in place. Exact late-window timing still depends on `IMP-11`. | `src/bot/runtime/*`, `src/rtds.rs`, `src/db.rs`, `src/main.rs` |
| IMP-04 | `REQ-004`, `REQ-005`, `REQ-006` | Done | Done | Add pre-open warmup plus exact open-time seeding deadlines: both seed orders within 5 seconds of open and within 1 second of each other. | Implemented: `PreArm` now records readiness before open, `OpenBoth` uses explicit open-confirmed and first-tradable anchors, enforces exact seed timing KPIs, supports one controlled late unlock, and blocks seeding on pre-open cached quotes. | `src/bot/runtime/startup.rs`, `src/bot/runtime/loop.rs`, `src/bot/runtime/policy.rs`, `src/bot/runtime/config.rs`, `src/bot/runtime/state.rs`, `src/bot/runtime/tests.rs` |
| IMP-05 | `REQ-007`, `REQ-008` | Done | Done | Implement `AwaitSecondFill` completion mode with no scale-up before both sides fill, 15-second target, and 30-second hard stop for accumulation. | Implemented: one-sided first fills now route into `AwaitSecondFill`, normal accumulation stays blocked until both sides fill, the 15-second and 30-second clocks run from first fill, and a one-shot ask-aware missing-side taker rescue now exists after the hard deadline. | `src/bot/runtime/state.rs`, `src/bot/runtime/startup.rs`, `src/bot/runtime/loop.rs`, `src/bot/runtime/policy.rs`, `src/bot/runtime/pair_build/*`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-06 | `REQ-009`, `REQ-010` | Done | Done | Replace heuristic imbalance logic with exact unmatched-fraction controls: target below 7%, warning above 12%, disable or reduce-only at 20% or more. | Implemented: unmatched-fraction is now the authoritative imbalance metric, `7% / 12% / 20%` thresholds drive `Normal / Throttle / Warning / HardDisable`, projected hard-disable is blocked, and paired-growth orders are actively canceled when imbalance holds shut off normal adds. | `src/bot/runtime/config.rs`, `src/bot/runtime/state.rs`, `src/bot/runtime/policy.rs`, `src/bot/runtime/metrics.rs`, `src/bot/runtime/pair_build/*`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-07 | `REQ-011`, `REQ-012`, `REQ-013` | Done | Done | Implement exact marginal pair-cost zones using `0.94 / 0.97 / 1.00 / 1.03` thresholds and block all size-increasing intents at `>= 1.00`. | Implemented: balanced adds now use `y_bid + n_bid`, repairs now use heavier-side carried average plus lagging-side bid, `StopAdd` and `Danger` block new size, and live growth or repair orders are canceled when rebalance pricing turns forbidden. | `src/bot/runtime/pair_build/costs.rs`, `src/bot/runtime/pair_build/decision.rs`, `src/bot/runtime/pair_build/handler.rs`, `src/bot/runtime/pair_build/repair.rs`, `src/bot/runtime/taper.rs`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-08 | `REQ-014`, `REQ-015` | Done | Done | Make maker-first execution mandatory, require explicit taker exception reasons, and enforce sub-5% target and 10% hard cap for taker share. | Implemented: taker submits now require explicit exception reasons and cap policy, projected market and daily taker share are enforced with shared wallet-aware state, same-wallet sibling pending takers are counted for the daily cap but not leaked into other pairs' market caps, and shared-state locking now survives crash recovery. | `src/bot/execution/*`, `src/bot/runtime/*`, `src/bot/runtime/metrics.rs`, `src/helpers.rs`, `scripts/run-instance.sh` |
| IMP-09 | `REQ-016`, `REQ-017` | Done | Done | Align clip sizing to `12 / 20 / 40 / 80`, hard-cap single orders at 80, and allow 40 or 80 only when all green conditions are true. | Implemented: `BOT_CLIP_LADDER` is now the authoritative clip policy, seeds default to `12`, paired growth defaults to `20`, `40 / 80` escalate progressively only under green conditions, taper maintenance re-downshifts to the late-phase lot, and repair sizing now respects actual lagging-side spend plus min-notional-safe gap limits. | `src/bot/runtime/config.rs`, `src/bot/runtime/pair_build/decision.rs`, `src/bot/runtime/pair_build/repair.rs`, `src/bot/runtime/taper.rs`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-10 | `REQ-018`, `REQ-019` | Done | Done | Hard-block intentional single-side speculative adds and forbid any order that increases residual inventory on the underdog side. | Implemented: pair-build, taper, and `AwaitSecondFill` now classify favorite/underdog and residual direction explicitly, emit stable residual-direction hold reasons, cancel only blocked-side BOT orders when underdog residual would increase, and expose the new residual telemetry in runtime logs. | `src/bot/runtime/pair_build/decision.rs`, `src/bot/runtime/metrics.rs`, `src/bot/runtime/pair_build/orders.rs`, `src/bot/runtime/startup.rs`, `src/bot/runtime/taper_runtime.rs`, `src/bot/runtime/tests.rs` |
| IMP-11 | `REQ-021` | Partial | Ready | Add exact late-window controls: smaller clips after 180 seconds, balance-only at 225 seconds, and zero new orders after 240 seconds. | Current tapering starts later and still allows behavior outside the required time bands. | `src/bot/runtime/taper.rs`, `src/bot/runtime/loop.rs`, `src/bot/runtime/config.rs` |
| IMP-12 | `REQ-023` | Partial | Queued | Introduce versioned config with persisted snapshots, explicit `config_version` on decisions, orders, and fills, and rollback-safe reload behavior. | Config loads from env and snapshots exist, but there is no requirement-complete versioned config pipeline. | `src/config.rs`, `src/db.rs`, `src/main.rs` |
| IMP-13 | `REQ-024`, `REQ-025` | Missing | Queued | Build append-only decision logging and structured metrics for state transitions, risk blocks, intents, acks, fills, reconciliation, and settlement. | Current decision persistence is incomplete, legacy-shaped, and not append-only or fully wired into order flow. | `src/db.rs`, `src/logging.rs`, `src/main.rs`, `src/bot/runtime/*` |
| IMP-14 | `REQ-028`, `REQ-029` | Missing | Blocked | Add a true paper adapter and make `paper` or `shadow` the default startup mode until clean reconciliation and explicit live enablement pass. | `dry_run` exists, but it is not a real paper adapter, and live-start safeguards are not requirement-complete. | `src/main.rs`, `src/bot.rs`, `src/bot/execution/*`, new paper adapter module |
| IMP-15 | `REQ-030`, adjusted `REQ-031` scope | Partial | Ready | Enforce idempotent client order IDs, fill dedupe, dependency-failure pause, startup reconciliation, reconnect reconciliation, and post-settlement validation without active-market Data API position reconciliation. Use `TRADE_VALIDATE` for startup and post-settlement validation flows. | Some dedupe and reconciliation helpers exist, but the repo still lacks a requirement-complete safe-mode and startup or post-settlement validation policy under the new product direction. | `src/bot/execution/*`, `src/bot/runtime_ws.rs`, `src/db.rs`, `src/main.rs` |

### P1 Completion Work

| ID | Requirement mapping | Requirement status | Work status | Implementation task | Current gap in repo | Main files to touch |
|---|---|---|---|---|---|---|
| IMP-16 | `REQ-020` | Missing | Blocked | Add the optional model-edge overlay behind a feature flag and require `edge_model_minus_price > 0.02` plus all normal inventory and price guards. | No model-edge overlay implementation was found. | strategy and config layers |
| IMP-17 | `REQ-027` | Missing | Queued | Build the analytics importer for the provided parquet and csv inputs and reproduce the calibration metrics used by the requirement. | No importer or parity validation pipeline exists. | new importer tool, fixtures, tests |
| IMP-18 | `REQ-026` | Missing | Blocked | Build deterministic event replay using the same strategy, risk, and ledger code paths as live and paper. | No replay engine or deterministic clock abstraction exists. | new replay module, strategy, execution, persistence wiring |
| IMP-19 | `REQ-032` | Missing | Queued | Add per-market and portfolio-level gross cost caps enforced before any order approval. | No explicit market-level or portfolio-level gross exposure cap was found. | risk layer, config, runtime wiring |
| IMP-20 | `REQ-033` | Partial | Ready | Enforce stale-data policy exactly: block new adds after 2 seconds of staleness and cancel or pause after 5 seconds. | Current stale-data threshold is looser and mostly logged rather than strictly enforced. | `src/config.rs`, `src/bot/runtime/loop.rs`, `src/bot/runtime_ws.rs` |
| IMP-21 | `REQ-034` | Missing | Ready | Cap quote refresh or cancel-replace activity to one cycle per side per second by default. | No requirement-matching refresh-rate control was found. | `src/bot/execution/*`, scheduler logic |

### Validation and Launch Gates

| ID | Requirement mapping | Requirement status | Work status | Implementation task | Current gap in repo | Main files to touch |
|---|---|---|---|---|---|---|
| IMP-22 | Section 10 unit and property tests | Missing | Ready | Add invariant tests for no scale-up before both sides fill, no add at `>= 1.00`, no underdog residual increase, and quantity conservation. | Existing unit coverage is too small and misses the main behavioral guards. | new unit and property tests |
| IMP-23 | Section 10 replay and simulation tests | Missing | Blocked | Add deterministic replay scenarios for good opens, one-side lag, stale data, reconciliation mismatch, and late settlement. | No replay-driven certification suite exists yet. | replay tests, fixtures |
| IMP-24 | Section 10 paper and shadow KPIs | Missing | Blocked | Add KPI reporting for seed timing, second-fill timing, taker share, imbalance, price-zone mix, and reconciliation health. | No paper or shadow KPI gate exists today. | metrics, dashboards, reports |
| IMP-25 | Section 10 deployment and rollback gates | Missing | Blocked | Add launch gates so live canary is blocked until replay, paper, shadow, and reconciliation criteria pass. | No requirement-grade go or no-go gate is implemented. | startup checks, admin controls, deployment docs |

## Recommended Build Order

This sequence stays tied to the requirement instead of a refactor roadmap:

1. `IMP-11` late-window balance-only and cutoff logic
2. `IMP-12` config versioning
3. `IMP-13` append-only decision audit trail
4. `IMP-15` startup/reconnect/post-settlement reconciliation and dependency-failure pause
5. `IMP-14` true paper plus shadow-first startup safety
6. `IMP-17` analytics importer parity
7. `IMP-18` deterministic replay
8. `IMP-22` to `IMP-25` validation and launch gates

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

The biggest remaining requirement gaps are:

- late-window rules
- append-only decision and audit persistence
- true paper, replay, and shadow modes
- startup and post-settlement validation discipline plus strict stale-data behavior
- validation gates before any guarded live rollout

Intentional scope exception:

- `REQ-001` strict BTC 5-minute restriction is marked `Not Applicable` by current product direction.
- `REQ-031` active-market 5-second position reconciliation is intentionally not part of the current product direction; startup, reconnect, and post-settlement validation remain in scope.

Current estimate:

- Requirement coverage in current scope: `66%`
- Remaining requirement gap in current scope: `34%`
