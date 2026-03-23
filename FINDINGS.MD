# Findings By IMP

This document records post-implementation findings that were raised, validated, and fixed while hardening requirement-driven work.

Scope covered here:

- `IMP-13` (`REQ-024`, `REQ-025`)
- `IMP-14` (`REQ-028`, `REQ-029`)
- `IMP-15` (`REQ-030`, adjusted `REQ-031`)
- `IMP-17` (`REQ-027`)
- `IMP-18` (`REQ-026`)
- `IMP-19` (`REQ-032`)
- `IMP-20` (`REQ-033`)
- `IMP-21` (`REQ-034`)
- `IMP-24` (Section 10 KPI gates)
- `IMP-23` (Section 10 replay certification)

Status legend:

- `Fixed`: finding was validated and addressed in code
- `Runtime`: requirement-aligned production/runtime finding
- `Test-only`: test-isolation or harness issue, not a runtime requirement gap

## Summary

| IMP | Requirement mapping | Total findings | Runtime findings | Test-only findings | Status |
|---|---|---:|---:|---:|---|
| `IMP-13` | `REQ-024`, `REQ-025` | 5 | 5 | 0 | Fixed |
| `IMP-14` | `REQ-028`, `REQ-029` | 7 | 7 | 0 | Fixed |
| `IMP-15` | `REQ-030`, adjusted `REQ-031` | 5 | 5 | 0 | Fixed |
| `IMP-17` | `REQ-027` | 4 | 4 | 0 | Fixed |
| `IMP-18` | `REQ-026` | 12 | 12 | 0 | Fixed |
| `IMP-19` | `REQ-032` | 35 | 34 | 1 | Fixed |
| `IMP-20` | `REQ-033` | 5 | 5 | 0 | Fixed |
| `IMP-21` | `REQ-034` | 12 | 12 | 0 | Fixed |
| `IMP-24` | Section 10 KPI gates | 17 | 17 | 0 | Fixed |
| `IMP-23` | Section 10 replay certification | 2 | 0 | 2 | Fixed |
| **Total** |  | **104** | **101** | **3** | **Fixed** |

## IMP-13

Requirement mapping: `REQ-024`, `REQ-025`

1. `P1` Runtime-event audit inserts were synchronous on the live trading path. `Runtime`, `Fixed`
2. `P2` Pending-trade cleanup deleted append-only audit rows for no-fill or blocked markets. `Runtime`, `Fixed`
3. `P2` Maker fill audit events used receipt time instead of exchange match time. `Runtime`, `Fixed`
4. `P1` Decision-event writes still ran synchronously on the trading path. `Runtime`, `Fixed`
5. `P2` Phase-transition audit rows stamped the new owner into both `prev_owner` and `next_owner`. `Runtime`, `Fixed`

## IMP-14

Requirement mapping: `REQ-028`, `REQ-029`

1. `P1` Legacy flat-row `dry_run=true` configs could fail migration by validating shadow-mode credentials before the legacy paper mapping was restored. `Runtime`, `Fixed`
2. `P1` Paper `FAK` or `FOK` taker exits could leave phantom shared gross reservations behind after immediate local resolution. `Runtime`, `Fixed`
3. `P2` Local non-live cancels could leave stale taker tracking and pending-taker state behind for synthetic orders. `Runtime`, `Fixed`
4. `P1` Shadow intents could disappear during open-order reconciliation when venue reads overwrote the hypothetical working set. `Runtime`, `Fixed`
5. `P1` Guarded-live fallback sessions could try venue cancels for synthetic `SHADOW_INTENT_*` orders after live had been armed once. `Runtime`, `Fixed`
6. `P1` Shadow `FAK` or `FOK` takers could remain pending until TTL expiry even though shadow mode can never fill them. `Runtime`, `Fixed`
7. `P1` `PairBuild` and `Taper` still hard-required user websocket connectivity in paper mode even though paper is supposed to differ from live only in execution and fill simulation. `Runtime`, `Fixed`

## IMP-15

Requirement mapping: `REQ-030`, adjusted `REQ-031`

1. `P1` Database-triggered dependency pause was not latched until state persistence actually recovered. `Runtime`, `Fixed`
2. `P1` Pause-time BOT order cleanup missed direct BOT orders when single-inflight was disabled. `Runtime`, `Fixed`
3. `P2` Legacy seen-trade-key migration lost recency ordering and could shrink the dedupe window. `Runtime`, `Fixed`
4. `P1` Daily-liquidity persistence pauses were cleared by main-state recovery even if the daily-liquidity file was still failing. `Runtime`, `Fixed`
5. `P1` Direct BOT order cleanup depended on execution-latency logging being enabled. `Runtime`, `Fixed`

## IMP-17

Requirement mapping: `REQ-027`

1. `P2` The analysis importer binary read `DB_URL` before loading `.env`, so local project-root runs could fail even though the repo's other entrypoints worked. `Runtime`, `Fixed`
2. `P2` Analysis imports were marked `COMPLETED` in Postgres before `summary.json` was written, which could leave false successful runs on output-directory failures. `Runtime`, `Fixed`
3. `P2` Price-zone classification used strict threshold comparisons instead of the runtime's `1e-9` boundary tolerance, so parity buckets could drift at `0.94 / 0.97 / 1.00 / 1.03`. `Runtime`, `Fixed`
4. `P3` The live `polybot` binary compiled a duplicate copy of the `analysis_import` module solely to satisfy a crate-path dependency from `db.rs`. `Runtime`, `Fixed`

## IMP-18

Requirement mapping: `REQ-026`

1. `P1` Replay could lose the captured market schedule and reinterpret mid-market tapes as if they had just opened. `Runtime`, `Fixed`
2. `P1` Replay ticks could skip phase and owner transition runtime events that live capture emitted. `Runtime`, `Fixed`
3. `P1` Replay finalized before regenerating the post-run settlement events that live capture wrote after the main loop returned. `Runtime`, `Fixed`
4. `P2` Replay ticks could suppress `risk_block` runtime events during startup, reconnect, or stale-data holds. `Runtime`, `Fixed`
5. `P1` Replay settlement consulted the local machine RTDS state instead of a captured resolution snapshot. `Runtime`, `Fixed`
6. `P2` Replayed order acknowledgements could miss captured submit-timing metadata in `order_ack` payloads. `Runtime`, `Fixed`
7. `P2` Replay bundles persisted only the snapshot and recomputed a fresh resolved config bundle at load time. `Runtime`, `Fixed`
8. `P2` Event-only replay scenarios could lose real exchange order IDs when `oracle_runtime_events.jsonl` was absent. `Runtime`, `Fixed`
9. `P2` Event-only replay could swap paired maker YES and NO order IDs by consuming fallback acks in arrival order instead of submit order. `Runtime`, `Fixed`
10. `P2` Event-only replay scenarios could lose `active_trade_id` and undercount their own portfolio exposure against gross caps. `Runtime`, `Fixed`
11. `P2` Replay loader treated missing `events.jsonl` as an empty scenario instead of rejecting a corrupted bundle. `Runtime`, `Fixed`
12. `P2` Replay bundles could still depend on ambient machine env overrides for behavior-critical runtime branches. `Runtime`, `Fixed`

## IMP-19

Requirement mapping: `REQ-032`

1. `P1` Portfolio gross could double-count the restarted local trade until shared-state TTL expiry. `Runtime`, `Fixed`
2. `P1` Shared-state tests mutated a process-wide env var and made the test suite nondeterministic. `Test-only`, `Fixed`
3. `P1` Same-pair sibling trades on the same wallet were incorrectly collapsed out of portfolio gross. `Runtime`, `Fixed`
4. `P2` Gross-cap instance keys were derived from a non-unique relative state filename. `Runtime`, `Fixed`
5. `P1` Pair gross preapproval could undercount when one leg did not actually replace. `Runtime`, `Fixed`
6. `P2` Old versioned config snapshots could lose their `max_total_cost`-derived gross caps on load. `Runtime`, `Fixed`
7. `P1` Pair gross preapproval could overcount pure YES or NO replacement refreshes as additive exposure. `Runtime`, `Fixed`
8. `P1` Direct refresh legs were excluded from gross preview before the old live orders were actually canceled. `Runtime`, `Fixed`
9. `P1` Live pending gross reservations could age out of shared state before any periodic republish refreshed them. `Runtime`, `Fixed`
10. `P2` Failed shared gross-reservation writes were converted into dependency pause but not propagated back to submitters. `Runtime`, `Fixed`
11. `P2` BUY taker submits ignored shared gross-reservation publish failures. `Runtime`, `Fixed`
12. `P2` Expired taker records were still republished into shared gross state forever. `Runtime`, `Fixed`
13. `P2` Shared gross-reservation refresh cadence allowed TTLs shorter than the refresh floor. `Runtime`, `Fixed`
14. `P2` Dry-run local orders were republished into wallet-global shared gross state. `Runtime`, `Fixed`
15. `P2` Republish refreshed only one live maker order per asset instead of every live reservation. `Runtime`, `Fixed`
16. `P2` Dry-run single-inflight maker submits still published fake shared gross reservations. `Runtime`, `Fixed`
17. `P2` Republish reset maker `applied_size` to `0.0` and could resurrect already filled notional. `Runtime`, `Fixed`
18. `P2` Republish double-subtracted partial maker fills once local order state had already switched to remaining size. `Runtime`, `Fixed`
19. `P2` Contextless live BUY orders defaulted reservation `kind` to `maker` instead of preserving the existing kind. `Runtime`, `Fixed`
20. `P2` Size-only BUY order updates could undercount remaining shared gross after partial fills. `Runtime`, `Fixed`
21. `P2` Long restart gaps could cause a still-live taker GTC BUY to be republished as `maker`. `Runtime`, `Fixed`
22. `P2` Reconnect BUY order updates could rewrite live taker reservations as `maker` before republish. `Runtime`, `Fixed`
23. `P2` Taker-kind recovery depended on the pending-taker TTL even though a taker GTC BUY could rest much longer. `Runtime`, `Fixed`
24. `P2` Accepted taker GTC BUYs were not persisted into local durable open-order state with `kind="taker"`. `Runtime`, `Fixed`
25. `P2` Persisted taker BUY rows could be republished back into shared gross before startup reconciliation proved they were still live. `Runtime`, `Fixed`
26. `P2` Persisted maker BUY rows could also be republished back into shared gross before startup reconciliation. `Runtime`, `Fixed`
27. `P2` Reconnect-pending state was not covered by the unconfirmed-order filter. `Runtime`, `Fixed`
28. `P2` Stale in-memory `order_exec_context` entries were still trusted during reconnect pending. `Runtime`, `Fixed`
29. `P2` Valid persisted local BUY reservations could be dropped before reconciliation had a chance to confirm them. `Runtime`, `Fixed`
30. `P2` Reconciliation marked the gate clean after republishing, so recovered live BUYs could still be filtered out in the same loop iteration. `Runtime`, `Fixed`
31. `P2` Partial websocket reconnects could drop genuinely live maker BUY reservations before reconciliation was allowed to run. `Runtime`, `Fixed`
32. `P2` Preserved reservations in staggered reconnects were not refreshed, so TTL expiry could silently drop them anyway. `Runtime`, `Fixed`
33. `P2` Single-inflight maker submits could publish the same shared gross reservation twice, turning a transient second-write failure into a spurious cancel. `Runtime`, `Fixed`
34. `P2` Local taker BUY cancel helpers cleared `open_orders` only and left stale taker tracking that could recreate false pending gross. `Runtime`, `Fixed`
35. `P2` Single-inflight submit-ack failures were not propagated back to callers, so paired GTC flow could still treat leg one as consumed and continue leg two. `Runtime`, `Fixed`

## IMP-20

Requirement mapping: `REQ-033`

1. `P1` Hard-stale recovery latch could be lost if another dependency pause reason briefly replaced the stale pause reason. `Runtime`, `Fixed`
2. `P2` Legacy flat DB configs could silently lose their stored stale thresholds and snap to `2 / 5` on restart. `Runtime`, `Fixed`
3. `P1` Hard stale could overwrite an existing database pause and let trading resume without proving persistence had recovered. `Runtime`, `Fixed`
4. `P1` Deprecated `MARKET_DATA_STALE_SECONDS` env rejection could be bypassed by falling back to a previous or DB config bundle. `Runtime`, `Fixed`
5. `P2` Startup and readiness quote gates still used the legacy stale threshold instead of the new add-block threshold. `Runtime`, `Fixed`

## IMP-21

Requirement mapping: `REQ-034`

1. `P1` Direct-mode refresh-cap no-ops could be misclassified as successful new submits. `Runtime`, `Fixed`
2. `P2` Per-side refresh throttling did not hold across BOT family handoffs. `Runtime`, `Fixed`
3. `P1` Returning `None` for capped direct refreshes could make the runtime think no live direct order existed. `Runtime`, `Fixed`
4. `P2` Handoff throttling compared against the target family instead of the actual previous cycle family. `Runtime`, `Fixed`
5. `P1` Direct refreshes across BOT family handoffs could still bypass the per-side cadence cap. `Runtime`, `Fixed`
6. `P2` Pair-build submit bookkeeping advanced even when both pair legs were refresh-capped no-ops. `Runtime`, `Fixed`
7. `P1` Direct refresh cycles were recorded before repost success, creating phantom cadence blocks after failed reposts. `Runtime`, `Fixed`
8. `P1` Protective single-inflight cancels were incorrectly blocked by the cadence cap. `Runtime`, `Fixed`
9. `P2` Missing execution context after restart or pruning disabled direct refresh throttling. `Runtime`, `Fixed`
10. `P2` Missing-context direct throttling used a mutable order timestamp instead of stable submit time. `Runtime`, `Fixed`
11. `P2` Pair-submit helper re-tracked refresh-capped no-op legs and erased their no-op marker. `Runtime`, `Fixed`
12. `P2` Replacement user-order events could inherit `submit_ts` from a different order and weaken direct refresh throttling. `Runtime`, `Fixed`

## IMP-23

Requirement mapping: Section 10 replay certification

1. `P1` Empty `initial_state/` replay fixture directories were not preserved on a clean clone because git does not track empty directories. `Test-only`, `Fixed`
2. `P1` Replay certification fixture JSON files were still swallowed by the repo-wide `*.json` ignore rule, so required scenario bundles would disappear from normal commits and fresh clones. `Test-only`, `Fixed`

## IMP-24

Requirement mapping: Section 10 KPI gates

1. `P1` Terminal `run_summary` rows could be emitted before post-run validation had a chance to settle the trade, creating false settlement mismatches for otherwise healthy runs. `Runtime`, `Fixed`
2. `P2` Direct runtime-event inserts such as `run_summary` could fail without emitting a compensating `audit_drop`, which let KPI integrity silently miss lost audit rows. `Runtime`, `Fixed`
3. `P1` KPI profile selection could be widened from any per-event mode tag instead of staying anchored to the terminal `run_summary.effective_order_mode`. `Runtime`, `Fixed`
4. `P1` Trades missing terminal `run_summary` rows could still satisfy sample sufficiency thresholds even though multiple run-level KPIs skipped them. `Runtime`, `Fixed`
5. `P1` Missing-summary attribution could incorrectly blame the shadow profile from transient shadow-tagged events in runs whose terminal mode was actually different. `Runtime`, `Fixed`
6. `P1` Missing shadow summaries could be missed entirely when attribution fell back to `configured_order_mode` instead of stable effective-mode evidence. `Runtime`, `Fixed`
7. `P1` `run_summary` audit counts could freeze before queued audit writes drained, making healthy runs fail logging integrity nondeterministically. `Runtime`, `Fixed`
8. `P1` Later startup validation could settle a trade whose terminal audit trail had already been frozen at `AWAIT_SETTLEMENT`, causing false settlement-reconciliation failures. `Runtime`, `Fixed`
9. `P1` Startup validation repair events could sort before earlier unresolved terminal events when they shared the same second-precision timestamp. `Runtime`, `Fixed`
10. `P1` After-market validation repair could hit the same timestamp-ordering problem, leaving repaired trades nondeterministically interpreted as unresolved. `Runtime`, `Fixed`
11. `P1` Mixed-mode runs that finished shadow could be excluded from shadow missing-summary attribution if the fallback required all observed effective modes to match. `Runtime`, `Fixed`
12. `P1` The "last observed mode" fallback could still depend on random UUID ordering for same-second events. `Runtime`, `Fixed`
13. `P1` Same-second later live decisions could be overridden by earlier shadow state-transition runtime rows because the fallback hard-ranked all runtime rows after decisions. `Runtime`, `Fixed`
14. `P2` Same-second earlier shadow `risk_block` rows could be forced after later live decisions across loop iterations, misattributing missing summaries to the shadow profile. `Runtime`, `Fixed`
15. `P2` Loop-level `risk_block` payloads without `t_into_seconds` could still lose to earlier live decisions in same-second missing-summary attribution. `Runtime`, `Fixed`
16. `P2` Shadow adapter-recovery evaluation sorted same-second runtime rows by random `event_id`, making quick reconnect recovery nondeterministic. `Runtime`, `Fixed`
17. `P1` Repo-side validation repair plus shutdown settlement handling could double-publish `settled` runtime events and make `run_summary` audit counts disagree with persisted rows. `Runtime`, `Fixed`

## Notes

- This document records findings that were actually raised during implementation hardening, not hypothetical future risks.
- `IMP-19` generated the largest follow-up volume because it introduced wallet-global shared state, multi-instance coordination, restart recovery, TTL cleanup, maker and taker reservation accounting, and local-to-shared state republish logic all at once.
- `IMP-17` findings were narrower and focused on importer parity correctness, CLI ergonomics, summary completion semantics, and one binary-linkage hygiene issue rather than live trading-path behavior.
- `IMP-18` generated a second cluster of follow-up parity fixes because deterministic replay had to become self-contained across schedule metadata, settlement inputs, captured order IDs, trade identity, and machine-local env differences.
- `IMP-24` generated a concentrated KPI-hardening cluster around terminal settlement audit repair, profile attribution for missing summaries, and deterministic same-second ordering of runtime versus decision evidence.
- `IMP-14` also picked up a late paper-parity hardening fix: the remaining direct `REQUIRE_USER_WS_CONNECTED` gates in `PairBuild` and `Taper` were replaced with the shared paper-aware helper, and the affected replay oracle was refreshed to match the intended paper behavior.
- The non-runtime findings in this document are the shared-state test-isolation issue under `IMP-19` plus the replay certification fixture-portability issues under `IMP-23`.
