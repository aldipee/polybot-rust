# Plan: Fix Issue #4 by Unifying Maker Execution Dedupe and Fill Application

## Summary
The fix is to make maker fills go through one canonical execution-acceptance path before they touch inventory, cost, or pair-order progress.

Right now the bug exists because the maker path still has split truth:
- one dedupe concept for inventory (`state.seen_trade_keys` via `_apply_fill`)
- another dedupe concept for maker progress (`maker_seen_exec_keys` / `maker_exec_progress`)
- and a single event can arrive in more than one shape

That allows the same economic maker fill to be accepted once under a weaker key and then accepted again under a richer key before later duplicates are only logged as dropped.

The chosen approach is:
1. Make maker trade events the only source of truth for maker fill application.
2. Replace the current maker string-key dedupe with a canonical maker execution ledger that understands aliases.
3. Apply maker inventory/cost and maker per-order progress in one commit step after dedupe succeeds.
4. Keep taker fill handling unchanged.

This is a focused accounting fix. It does not change recovery thresholds, quote-invalidation behavior, or reject-backoff policy.

## Important Changes / Additions To Interfaces And Types

### Public / external interfaces
No external API changes.

### Internal runtime types
Add in `src/bot.rs`:

1. `MakerExecCandidate`
- Fields:
  - `order_id: String`
  - `asset_id: String`
  - `side: String`
  - `qty: f64`
  - `price: f64`
  - `tx_hash: Option<String>`
  - `trade_id: Option<String>`
  - `taker_order_id: Option<String>`
  - `match_time: Option<String>`

2. `MakerExecRecord`
- Fields:
  - `canonical_id: String`
  - `order_id: String`
  - `qty: f64`
  - `price: f64`
  - `asset_id: String`
  - `side: String`
  - `aliases: Vec<String>`
  - `applied_ts: f64`

3. `MakerExecLedger`
- Fields:
  - `alias_to_canonical: HashMap<String, String>`
  - `records: HashMap<String, MakerExecRecord>`
  - `per_order_applied: HashMap<String, MakerExecProgress>`

4. `MakerExecApplyResult`
- Variants:
  - `Applied { canonical_id: String }`
  - `Duplicate { canonical_id: String }`
  - `Conflict { canonical_id: String, reason: String }`
  - `DroppedWeakId { reason: String }`

### Bot fields
Replace:
- `maker_exec_progress`
- `maker_seen_exec_keys`

With:
- `maker_exec_ledger: Arc<Mutex<MakerExecLedger>>`

Keep:
- `pair_arb_pending_imbalance`

### Existing persisted state
Do not add a new persisted schema in this fix.

Keep using `BotState.seen_trade_keys` as the persisted accepted-key history, but only store the canonical maker execution id there for maker fills.

That gives exact-key restart protection without expanding the state format.

## Implementation Plan

### 1) Stop using `_maker_trade_exec_key` as the acceptance primitive
Replace `_maker_trade_exec_key(...) -> Option<String>` with:

1. `_maker_trade_exec_candidate(...) -> Option<MakerExecCandidate>`
- Parse only from the maker leg that matches our wallet
- Require:
  - non-empty maker `order_id`
  - valid `asset_id`
  - `side in {BUY, SELL}`
  - `qty > 0`
  - `price > 0`

2. `_maker_trade_exec_aliases(candidate: &MakerExecCandidate) -> Vec<String>`
- Build aliases in this exact priority order:
  - `maker_tx:{order_id}:{tx_hash}:{qty:.8}:{price:.8}` if `tx_hash` exists
  - `maker_trade:{order_id}:{trade_id}` if `trade_id` exists
  - `maker_match:{order_id}:{taker_order_id}:{match_time}:{qty:.8}:{price:.8}` if `taker_order_id` and `match_time` exist

Do not create acceptance aliases from `status` or other weak fields.

Reason:
- `status`-based fallback is too unstable
- the observed bug is almost certainly coming from fallback-vs-enriched key drift

### 2) Introduce one canonical maker execution commit path
Add a new method:

`_maker_commit_exec_fill(candidate: MakerExecCandidate) -> MakerExecApplyResult`

Behavior:
1. Build aliases from the candidate.
2. If no alias can be built:
   - return `DroppedWeakId`
   - do not mutate inventory
   - log a hard warning
3. Lock `maker_exec_ledger`.
4. Resolve canonical id:
   - if any alias already exists in `alias_to_canonical`, use the mapped canonical
   - otherwise choose the strongest available alias as canonical:
     - tx alias first
     - trade alias second
     - match alias third
5. Check whether that canonical already exists in `records`.
   - If yes:
     - verify `order_id`, `qty`, `price`, `asset_id`, `side` match within epsilon
     - if they match, return `Duplicate`
     - if they do not match, return `Conflict`
6. If new:
   - lock `state`
   - if `state.seen_trade_keys` already contains the canonical id, treat as `Duplicate`
   - otherwise:
     - apply inventory/cost once
     - append canonical id to `state.seen_trade_keys`
     - save state
   - then update ledger:
     - insert `MakerExecRecord`
     - register every alias to the canonical id
     - increment `per_order_applied[order_id].applied_qty += qty`
     - update `last_update_ts`

Critical rule:
- inventory mutation and per-order maker progress increment must happen in the same commit path
- there must be no maker path where inventory mutates first and progress is recorded later under a separate dedupe check

### 3) Split `_apply_fill` into generic and maker-safe forms
Current problem:
- `_apply_fill` performs its own dedupe through `state.seen_trade_keys`
- maker flow currently calls `_apply_fill` first and `_maker_record_exec_fill` second

Change it to:

1. Keep `_apply_fill(...)` for taker/generic use.
- No behavior change for taker path.

2. Add `_apply_fill_locked_nodedupe(...)`
- Internal helper
- Assumes the caller already performed dedupe
- Mutates:
  - `q_yes` / `q_no`
  - `c_yes` / `c_no`
  - entry-reason bookkeeping
  - cooldown reset
- Does not touch `state.seen_trade_keys`

3. `_maker_commit_exec_fill(...)` must use `_apply_fill_locked_nodedupe(...)`, not `_apply_fill(...)`.

This removes the split between:
- maker external dedupe
- generic `_apply_fill` dedupe

### 4) Remove the current maker split-dedupe flow from `_handle_user_trade_event`
In the maker branch of `_handle_user_trade_event`:

Current shape:
- derive key
- check `maker_seen_exec_keys`
- call `_apply_fill`
- then call `_maker_record_exec_fill`

Replace with:
- build `MakerExecCandidate`
- call `_maker_commit_exec_fill`
- branch on result:
  - `Applied`: record fill stats, latency, and order-fill telemetry
  - `Duplicate`: log `[FILL][MAKER_DEDUPE] drop ... canonical=...`
  - `Conflict`: log `[FILL][MAKER_CONFLICT] ...` and do not mutate
  - `DroppedWeakId`: log `[FILL][MAKER_DROP_WEAK] ...` and do not mutate

Delete or retire:
- `_maker_record_exec_fill`
- direct maker use of `maker_seen_exec_keys`

### 5) Keep pair fill waiting order-centric, but source it from the unified ledger
Keep `_wait_for_pair_order_fills(...)` conceptually the same.

Change only the source:
- `_maker_exec_applied_qty(order_id)` must read from `maker_exec_ledger.per_order_applied`
- that value must now reflect only canonical accepted maker executions

Expected result:
- for `btc-updown-5m-1772749200`, first pair wait should still report `fy=0 fn=5`
- later YES maker fills should move that order to `~5`
- but internal inventory should only move by the same accepted quantities, not double

### 6) Add explicit conflict and invariant checks
Add these checks inside `_maker_commit_exec_fill`:

1. Canonical conflict check
If an alias resolves to an existing canonical id but the new candidate differs materially in:
- `order_id`
- `qty`
- `price`
- `asset_id`
- `side`

then:
- log `[FILL][MAKER_CONFLICT]`
- do not mutate inventory
- do not mutate `per_order_applied`

2. Per-order progress invariant
After any applied maker fill:
- recompute `sum_qty_for_order = sum(record.qty for all records where record.order_id == candidate.order_id)`
- assert within epsilon:
  - `per_order_applied[order_id].applied_qty == sum_qty_for_order`

If not:
- log `[FILL][MAKER_INVARIANT]`
- fail closed for that event path
- do not try to “fix up” by clamping silently

This invariant is the direct guard against the current phantom pair-leg duplication.

### 7) Logging and observability
Add these logs:

1. On maker apply:
- `[FILL][MAKER_APPLY] oid=... canonical=... alias_kind=tx|trade|match qty=... px=...`

2. On maker duplicate:
- `[FILL][MAKER_DEDUPE] drop oid=... canonical=... alias_kind=... qty=... px=... trade_id=... tx=... taker_oid=... match_time=...`

3. On weak-id drop:
- `[FILL][MAKER_DROP_WEAK] oid=... reason=no_strong_alias ...`

4. On conflict:
- `[FILL][MAKER_CONFLICT] oid=... canonical=... reason=...`

5. On invariant failure:
- `[FILL][MAKER_INVARIANT] oid=... applied=... expected=...`

Use existing log behavior; do not add a new env key in this fix.

## Explicit Scope / Out Of Scope

### In scope
- Maker execution dedupe
- Maker inventory mutation path
- Maker per-order progress path
- Pair fill waiting correctness

### Out of scope
- Recovery threshold tuning (`4.99` vs `5.00`)
- Quote invalidation hedge bypass
- Reject backoff changes
- Mid-run restart continuity for alias promotion across different event shapes

## Test Cases And Scenarios

### Unit tests
1. Exact duplicate maker trade event
- same order, same tx, same qty, same price
- expected: one apply, one duplicate

2. Fallback then enriched shape for same maker fill
- first event contains only `trade_id`
- second event contains `trade_id + tx_hash + taker_order_id + match_time`
- expected: one apply only

3. Match-based then enriched shape for same maker fill
- first event contains `taker_order_id + match_time + qty + price`
- second event later adds `tx_hash`
- expected: one apply only

4. Two real partial fills on same maker order
- same order, different tx hashes, different qtys
- expected: both apply, cumulative progress equals sum

5. Weak maker event with no tx / no trade_id / no match tuple
- expected: no apply, weak-drop log

6. Conflict case
- same canonical alias resolves, but later event disagrees on qty or price
- expected: no second apply, conflict log

7. Per-order invariant
- after each accepted execution, `per_order_applied == sum(records.qty for order)`
- expected: always true

### Integration / replay scenarios
1. Replay `btc-updown-5m-1772749200`
- expected first pair behavior:
  - before pair: `qYES=10 qNO=10`
  - after NO leg only: near `qYES=10 qNO=15`
  - after YES leg completes: near `qYES=15 qNO=15`
- final expected:
  - `qYES=24.999352`
  - `qNO=20.000000`

2. Replay `btc-updown-5m-1772748900`
- expected final:
  - `qYES≈24.991175`
  - `qNO≈24.986630`

3. Replay known clean run `btc-updown-5m-1772748600`
- expected:
  - no behavior regression
  - trade-history totals still match internal state

4. Duplicate-delivery burst on one maker order
- same two maker trade events repeated many times
- expected:
  - one apply
  - all later repeats deduped
  - no progress drift

5. Taker path regression check
- taker fills and `_taker_order_fallback_on_order_event`
- expected:
  - unchanged behavior
  - no maker-ledger interaction

## Acceptance Criteria
1. A single maker economic fill can mutate inventory at most once.
2. For any maker order id, `per_order_applied` equals the sum of accepted canonical maker executions for that order.
3. Pair fill waits (`fy` / `fn`) match trade history for the relevant pair order ids.
4. `btc-updown-5m-1772749200` no longer ends at `qYES=30 qNO=25`; it ends near trade-history truth.
5. `btc-updown-5m-1772748900` no longer carries the phantom extra NO leg.
6. Existing control-plane behavior remains intact:
   - pending imbalance set/cleared
   - pair suppression while pending imbalance is active
   - no return to old runaway pair-arb behavior

## Assumptions And Defaults
1. Maker trade events are the canonical source of maker fill truth.
2. Taker order-event fallback remains taker-only and is not part of this bug.
3. Production maker trade events normally contain at least one strong identifier:
   - `transaction_hash`, or
   - `trade_id`, or
   - `taker_order_id + match_time`
4. Events that lack all strong identifiers will be dropped rather than risk double-counting.
5. Mid-run restart alias continuity is out of scope for this fix; exact canonical replay protection still comes from persisting canonical ids in `state.seen_trade_keys`.
6. No new env/config keys are added in this fix.
