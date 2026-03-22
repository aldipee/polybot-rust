# Findings By IMP

This document records post-implementation findings that were raised, validated, and fixed while hardening requirement-driven work.

Scope covered here:

- `IMP-13` (`REQ-024`, `REQ-025`)
- `IMP-15` (`REQ-030`, adjusted `REQ-031`)
- `IMP-19` (`REQ-032`)
- `IMP-20` (`REQ-033`)
- `IMP-21` (`REQ-034`)

Status legend:

- `Fixed`: finding was validated and addressed in code
- `Runtime`: requirement-aligned production/runtime finding
- `Test-only`: test-isolation or harness issue, not a runtime requirement gap

## Summary

| IMP | Requirement mapping | Total findings | Runtime findings | Test-only findings | Status |
|---|---|---:|---:|---:|---|
| `IMP-13` | `REQ-024`, `REQ-025` | 5 | 5 | 0 | Fixed |
| `IMP-15` | `REQ-030`, adjusted `REQ-031` | 5 | 5 | 0 | Fixed |
| `IMP-19` | `REQ-032` | 35 | 34 | 1 | Fixed |
| `IMP-20` | `REQ-033` | 5 | 5 | 0 | Fixed |
| `IMP-21` | `REQ-034` | 12 | 12 | 0 | Fixed |
| **Total** |  | **62** | **61** | **1** | **Fixed** |

## IMP-13

Requirement mapping: `REQ-024`, `REQ-025`

1. `P1` Runtime-event audit inserts were synchronous on the live trading path. `Runtime`, `Fixed`
2. `P2` Pending-trade cleanup deleted append-only audit rows for no-fill or blocked markets. `Runtime`, `Fixed`
3. `P2` Maker fill audit events used receipt time instead of exchange match time. `Runtime`, `Fixed`
4. `P1` Decision-event writes still ran synchronously on the trading path. `Runtime`, `Fixed`
5. `P2` Phase-transition audit rows stamped the new owner into both `prev_owner` and `next_owner`. `Runtime`, `Fixed`

## IMP-15

Requirement mapping: `REQ-030`, adjusted `REQ-031`

1. `P1` Database-triggered dependency pause was not latched until state persistence actually recovered. `Runtime`, `Fixed`
2. `P1` Pause-time BOT order cleanup missed direct BOT orders when single-inflight was disabled. `Runtime`, `Fixed`
3. `P2` Legacy seen-trade-key migration lost recency ordering and could shrink the dedupe window. `Runtime`, `Fixed`
4. `P1` Daily-liquidity persistence pauses were cleared by main-state recovery even if the daily-liquidity file was still failing. `Runtime`, `Fixed`
5. `P1` Direct BOT order cleanup depended on execution-latency logging being enabled. `Runtime`, `Fixed`

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

## Notes

- This document records findings that were actually raised during implementation hardening, not hypothetical future risks.
- `IMP-19` generated the largest follow-up volume because it introduced wallet-global shared state, multi-instance coordination, restart recovery, TTL cleanup, maker and taker reservation accounting, and local-to-shared state republish logic all at once.
- The only finding in this document that was not a runtime requirement gap was the shared-state test-isolation issue under `IMP-19`.
