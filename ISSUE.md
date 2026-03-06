# Issue #1: Same-Side Maker GTC Accumulation (Duplicate In-Flight Orders)

## Status
- Date: 2026-03-05
- Severity: High (capital/risk control issue)
- Scope: `MAKER_SKEW_ARB` maker BUY flow
- Current state: Mitigated by lifecycle gate; additional hardening added; still monitor cancel-churn behavior.

## Problem Summary
The bot can place multiple maker GTC BUY orders on the same asset/side before prior orders are fully resolved. Under delayed ACK/fill/event timing, this stacks exposure, then fills in bursts, which can trigger emergency risk logic (`max-loss`, `force flatten`, taker unwind/sell).

## Observed Symptoms

### Earlier incident pattern (from previous markets)
- Repeated same-side maker submits on one leg while prior orders were still live/pending.
- Exposure mismatch escalated.
- Emergency hedge blocked by cap/ask constraints.
- Fallback to force-flatten via taker SELL, destroying edge.

### Provided run `btc-updown-5m-1772713500` (improved but still showing lifecycle stress)
- Same-side overlapping submits still occurred before previous submit fully settled:
  - `src log`: submit `0xe4d5e5c9..` then submit `0x18177b83..` on same asset (`400348`) before first fill.
  - `src log`: submit `0x89752ba1..` then submit `0xcfe66113..` before first fill.
- Evidence from `output/btc-updown-5m-1772713500/app.log`:
  - `19:26:30.545` submit `0xe4d5...` (BUY, maker)
  - `19:26:33.821` submit `0x1817...` (BUY, maker)
  - `19:26:34.874` fill `0xe4d5...`
  - `19:26:34.898` fill `0x1817...`
  - `19:26:50.069` submit `0x8975...`
  - `19:26:52.483` submit `0xcfe6...`
  - `19:26:52.494` fill `0xcfe6...`
  - `19:26:59.993` late fill `0x8975...`
- Long repeated cancel requests were also observed for same OID (`0x7c118665..`) near end of market.
- This run finished positive (`lp=2.2304`) and did not trigger force-flatten, but behavior is still not fully clean.

## Root Cause
This was a control-path/lifecycle consistency gap (not one bug):
1. Multiple maker submission paths could bypass strict in-flight gating.
2. Local tracking could be overwritten by newer OID while older live order still existed.
3. Event/reconcile timing races (WS/order list staleness) could temporarily make slot appear clear and allow another submit.
4. Seed decision originally used filled inventory only, not pending working BUY remainder.

## Implemented Fixes

### 1) Introduced explicit maker order lifecycle state machine
In `src/bot.rs`:
- Added `MakerOrderKey` (`asset_id`, `side`) at line ~138.
- Added `MakerOrderLifecycle` (`Idle | SubmitPending | Working | CancelPending`) at line ~153.
- Added `MakerOrderSlot` (state/order/price/size/remaining/timestamps/origin/replace target) at line ~169.
- Added maps:
  - `maker_order_slots` (key -> slot)
  - `maker_order_index` (order_id -> key)

### 2) Centralized maker GTC gate (single in-flight policy)
In `src/bot.rs`:
- Added `_maker_order_upsert_gtc(...)` at line ~4566.
- Enforced:
  - no new submit while `SubmitPending` within TTL,
  - no new submit while `CancelPending` within TTL,
  - replace via cancel-first (no blind submit),
  - submit only when slot is safe.

### 3) Rewired submit paths to use the gate
In `src/bot.rs`:
- Base seed path now uses gate (line ~5591).
- Pair-arb maker leg submits now use gate (lines ~5139-5140).
- Replace path delegates to gate (line ~9366).
- Ladder/strategy cancel flow moved toward gate-managed cancel API (`_maker_cancel_strategy_orders`, line ~4236).

### 4) Event-driven lifecycle updates + duplicate defense
In `src/bot.rs`:
- `_maker_order_on_user_event(...)` at line ~4398.
- Event hook connected in `_handle_user_order_event` (line ~7524).
- If duplicate working OID is detected while max active is 1, duplicate is canceled instead of adopted (`max_active` checks around line ~4512).

### 5) Exchange-truth reconciliation + deterministic de-dup
In `src/bot.rs`:
- `_maker_order_reconcile_asset(...)` at line ~4248.
- Before/around submits, reconcile open exchange orders for the asset and enforce max active BUY orders (`MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET`, line ~4257).
- Keep one canonical order (tracked or closest intended price), cancel extras.

### 6) Effective inventory now includes working open BUY remainder
In `src/bot.rs`:
- `_maker_order_open_buy_remaining(...)` at line ~4057.
- Seed sizing logic optionally uses effective quantity with open BUYs (lines ~5511 onward).
- Fixed overcount risk by using `remaining` as authoritative when available.

### 7) Additional hardening after latest log review
In `src/bot.rs`:
- Added transient-missing protection in reconcile:
  - `_maker_working_missing_ttl_seconds()` (line ~4035)
  - Do not clear `Working` slot immediately if exchange list is temporarily empty right after submit/cancel churn (lines ~4358-4368).
- Added cancel-throttle guard:
  - If already `CancelPending` and still within cancel TTL, skip repeated cancel requests (lines ~4179-4183).

### 8) Config keys added
In `src/env_contract.rs`:
- `MAKER_SINGLE_INFLIGHT_PER_SIDE`
- `MAKER_SUBMIT_PENDING_TTL_SECONDS`
- `MAKER_CANCEL_PENDING_TTL_SECONDS`
- `MAKER_WORKING_MISSING_TTL_SECONDS`
- `MAKER_REPLACE_MIN_INTERVAL_SECONDS`
- `MAKER_EFFECTIVE_Q_INCLUDE_OPEN_BUYS`
- `MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET`

## Suggested Runtime Defaults
- `MAKER_SINGLE_INFLIGHT_PER_SIDE=true`
- `MAKER_SUBMIT_PENDING_TTL_SECONDS=6.0`
- `MAKER_CANCEL_PENDING_TTL_SECONDS=3.0`
- `MAKER_WORKING_MISSING_TTL_SECONDS=12.0`
- `MAKER_REPLACE_MIN_INTERVAL_SECONDS=0.5`
- `MAKER_EFFECTIVE_Q_INCLUDE_OPEN_BUYS=true`
- `MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET=1`

## Validation Checklist
1. For each asset/side, exchange open BUY maker orders never exceed 1 after reconcile settles.
2. No `LIMIT_GTC_POSTONLY` burst on same asset before previous lifecycle resolves.
3. Delayed-fill scenarios do not produce exposure jumps from duplicate stacked maker orders.
4. No force-flatten triggered due to duplicate-order artifact.
5. Risk controls still trigger when true risk breaches occur.

---

# Issue #2: Pair Arb Runaway Accumulation + Submit Reject Spam

## Status
- Date: 2026-03-06
- Severity: High (capital loss, wasted API calls)
- Scope: `MAKER_SKEW_ARB` pair arb flow + maker submit lifecycle
- Current state: Partially mitigated in v0.1.18-v0.1.20. Pair overlap/recovery controls improved, but maker pair-leg accounting is still intermittently wrong; not production-stable.

## Problem Summary
Three compounding issues discovered in production runs `btc-updown-5m-1772743500` (LP=+2.28, lucky) and `btc-updown-5m-1772743800` (LP=-2.30, loss):

1. **Pair arb runaway accumulation:** When one pair leg fills fast and the other is deferred (v0.1.17 Fix 2), the bot immediately fires another pair arb, compounding the YES/NO gap. Observed: YES=30 vs NO=10 (3:1 ratio) within seconds, requiring taker FAK at 0.70-0.74 to catch up.
2. **Submit reject spam:** 17 rejects over 100 seconds with flat 5s cooldown. Exchange consistently rejecting at a price level, but bot retries forever.
3. **Deferred leg orphaned:** When Fix 2 defers a hedge and the live leg is later cancelled by another path (quote invalidation, ladder mode), the gap is never explicitly closed.

## Observed Symptoms

### Run `btc-updown-5m-1772743500` (v0.1.17, LP=+2.28)
- `03:50:38` pair arb defers (NO still live), YES=15 NO=10
- `03:50:38` immediately fires ANOTHER pair arb
- `03:50:44` defers again, YES=30 NO=10 (3:1 ratio)
- `03:50:47` forced taker FAK BUY 5 NO @ 0.70
- `03:50:55` another taker FAK BUY 5 NO @ 0.74
- Got lucky: BTC went UP (YES wins), LP=+2.28

### Run `btc-updown-5m-1772743800` (v0.1.17, LP=-2.30)
- Same pair arb compounding pattern: YES=30 NO=10
- Two taker FAK at 0.70 and 0.74 to catch up
- 17 submit rejects on YES side (03:51:21 to 03:53:05)
- Emergency hedge blocked at expiry (ask=0.84 vs cap=0.52)
- BTC went DOWN: LP=-2.30

## Root Cause

### Pair arb compounding
No check for existing position imbalance before opening new pair arb. Each arb is individually valid (edge > 0), but aggregate position becomes dangerously one-sided when multiple pairs overlap with deferred legs.

### Submit reject spam
The 5s flat cooldown prevents rapid-fire rejects but allows infinite retries at the same interval. When the exchange has no liquidity at the desired price level, 5s is not enough backoff.

### Deferred leg orphan
v0.1.17 Fix 2 defers hedge when unfilled leg is "still live." But if another code path (quote invalidation, ladder mode) cancels that leg, the gap becomes permanent. No follow-up mechanism tracks the orphaned gap.

## Implemented Fixes (v0.1.18)

### Fix A: Pair arb imbalance guard
In `src/bot.rs` — `_maker_skew_try_arb` (line ~5372):
- Before submitting pair orders, check `|qYES - qNO| > PAIR_ARB_MAX_IMBALANCE_SHARES`.
- If imbalanced, suppress pair arb and log suppression.
- Base seed, ladder, and taker hedge paths remain free to rebalance.
- Default threshold: `max(clip_shares, min_shares)` (typically 8 shares).

### Fix B: Submit reject exponential backoff
In `src/bot.rs`:
- Added `consecutive_rejects` counter to `MakerOrderSlot` (line ~169).
- `_maker_order_on_submit_reject` increments counter (line ~4198).
- `_maker_order_on_submit_ack` resets counter to 0 (line ~4165).
- `_maker_order_upsert_gtc` cooldown uses `base * 2^(n-1)`, capped at max (line ~4668).
- Schedule: 5s → 10s → 20s → 40s → 60s (cap).
- Reduces 17 rejects to ~3-4 over same period.

### Fix C: Deferred leg follow-through
Covered by Fix A — imbalance guard prevents compounding. Existing ladder path (line ~5861) picks deficit side for rebalancing when `skew_ratio > ratio_max` or CPP soft cap hit.

### Config keys added
In `src/env_contract.rs`:
- `PAIR_ARB_MAX_IMBALANCE_SHARES` (default: `max(clip_shares, min_shares)`)
- `MAKER_SUBMIT_REJECT_COOLDOWN_SECONDS` (default: 5.0)
- `MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS` (default: 60.0)

## Validation (v0.1.18 test run `btc-updown-5m-1772745300`)
- Only 1 pair arb fired (vs 2-3 previously). Fix A would block second if attempted.
- Zero submit rejects (vs 17 previously). Fix B working.
- No cancel churn. Fix 1 (v0.1.17) holding.
- Deferred hedge correctly applied at 04:15:57.
- **New issue discovered:** see below.

---

# Issue #3: Quote Invalidation Blocks Hedge Rebalancing

## Status
- Date: 2026-03-06
- Severity: Medium-High (leaves imbalanced position exposed to expiry)
- Scope: `_quotes_invalidated` gate in `_maker_skew_main_loop`
- Current state: Identified, not yet implemented.

## Problem Summary
When the market spread is tight (YES bid + NO ask > 0.98), `_quotes_invalidated()` returns true and the bot cancels all orders and returns early from the main loop. This blocks ALL order activity including hedge/rebalancing orders on the deficit side. The bot sits idle with an imbalanced position until expiry.

## Observed Symptoms

### Run `btc-updown-5m-1772745300` (v0.1.18, LP=-3.89)
- At 04:16:16, position is YES=15, NO=24.99 (ratio=1.666, gap=9.99 shares).
- Spread tight: YES bid + NO ask ≈ 0.99 > 0.98 threshold.
- `_quotes_invalidated` returns true on EVERY loop iteration.
- Bot cancels (nothing to cancel) and returns. Never reaches ladder/hedge code.
- **3 minutes of total inactivity** (04:16:16 to 04:19:10) with exposed position.
- Emergency hedge blocked at expiry (ask=1.00 vs cap=0.58).
- Only ~5 YES shares grabbed by near-expiry maker order before cancel-all.

## Root Cause
The `_quotes_invalidated` gate (line ~5783 in `_maker_skew_main_loop`) is a blanket block. It correctly prevents accumulation orders when spreads are unfavorable, but it also blocks hedge/rebalancing orders that would **reduce** risk.

### Code flow when invalidated:
```
_quotes_invalidated() == true
  → _maker_ladder_cancel_all()     // cancels aggressive orders (correct)
  → _maker_cancel_strategy_orders() // cancels strategy orders (correct)
  → return                          // blocks ALL further logic (WRONG for hedge)
```

The ladder path at line ~5861 would pick the deficit side (YES) for rebalancing, but it's never reached.

## Proposed Fix: Hedge Bypass on Quote Invalidation
When quotes are invalidated BUT position is imbalanced beyond a threshold:
1. Still cancel existing aggressive orders (keep current behavior).
2. Do NOT return early — fall through to ladder path.
3. Force side = deficit side, role = "hedge".
4. Post maker order at bid price on deficit side.
5. New env key: `MAKER_SKEW_HEDGE_BYPASS_QUOTE_INVALIDATION` (default: true).
6. New env key: `MAKER_SKEW_HEDGE_BYPASS_MIN_GAP` (default: clip_shares).

### Constraints:
- Only deficit side allowed through (reduces risk, doesn't increase it).
- Only when gap > minimum threshold.
- Uses maker posting (not taker), so no overpaying.
- Still goes through `_maker_order_upsert_gtc` lifecycle gate.
- Reject backoff (Fix B) prevents spam if price level unavailable.

### Expected impact:
- In the test run, bot would have posted YES orders during the 3-minute idle window.
- Even slow fills would close the gap partially before expiry.
- No change to behavior when position is balanced.

## Notes for Next AI Model
- Primary files: `src/bot.rs`, `src/env_contract.rs`.
- Key methods for this issue: `_quotes_invalidated` (line ~5883), the invalidation gate (line ~5783), ladder side selection (line ~5861).
- The fix modifies the early-return block at line ~5784-5801 to conditionally fall through.
- Must ensure the bypass only allows deficit-side hedge orders, not new accumulation.
- Test with runs where spread is persistently tight (sum > 0.98) and position is imbalanced.

---

# Issue #4: Intermittent Maker Pair-Leg Fill Double-Application

## Status
- Date: 2026-03-06
- Severity: Critical (inventory/cost corruption drives wrong strategy decisions)
- Scope: Maker fill dedupe, `maker_exec_progress`, pair-arb fill accounting
- Current state: Reproduced in multiple v0.1.20 runs. Control-path fixes are mostly working, but accounting is still not trustworthy.

## Problem Summary
The remaining failure mode is no longer runaway pair-arb admission. The control logic now usually defers and suppresses correctly. The unresolved bug is that some maker pair-leg fills are still being applied twice to internal inventory/cost before later duplicate variants are only logged as deduped.

When this happens:
1. `qYES` / `qNO` diverges from actual exchange trade history.
2. `maker_exec_progress` for the affected pair order is effectively overstated.
3. Recovery mode, suppression, and expiry behavior are then driven by phantom inventory.

## What Works Now
- Recovery mode enters/exits more sanely than earlier versions.
- Heavy-side BUY blocking and pending-imbalance suppression are materially improved.
- Reject storms are no longer the dominant problem.
- Some runs match trade history exactly (for example `output/btc-updown-5m-1772748600/btc-updown-5m-1772748600_bot-v_0_1_20.log`).

## What Is Still Broken
- Pair-leg maker fills can still be double-applied under certain message-shape/lifecycle combinations.
- The bot can end a market with inventory and total cost materially higher than the actual fills in exchange trade history.
- This is intermittent, which makes it more dangerous: some runs look clean, some do not.

## Observed Symptoms

### Run `btc-updown-5m-1772748900`
Evidence file:
- `output/btc-updown-5m-1772748900/btc-updown-5m-1772748900.log`
- trade history pasted during analysis on 2026-03-06 (same market)

Key log evidence:
```text
2026-03-06 05:17:36.521|INFO| LP=+1.1994 CPP=0.940026 TotalCost=18.7992 qYES=20.00 qNO=20.00
2026-03-06 05:17:56.114|INFO| [LATENCY][FILL] submit->fill=16898ms oid=0xab04b2f5..
2026-03-06 05:17:57.172|INFO| [LATENCY][FILL] submit->fill=221ms oid=0xdc5dd810..
2026-03-06 05:17:57.367|INFO| LP=-0.2976 CPP=1.011903 TotalCost=25.2961 qYES=25.00 qNO=29.99
```

Final mismatch for the run:
- Bot final state: `qYES=25.00 qNO=29.99`
- Trade-history-supported state:
  - YES = `24.991175`
  - NO = `24.986630`

That run ended close to balanced in exchange history, but the bot carried a phantom extra NO leg of about `+5` shares internally.

### Run `btc-updown-5m-1772749200`
Evidence files:
- `output/btc-updown-5m-1772749200/btc-updown-5m-1772749200.log`
- trade history pasted during analysis on 2026-03-06 (same market)

Key log evidence:
```text
2026-03-06 05:20:43.643|INFO| LP=-0.5000 CPP=1.050000 TotalCost=10.5000 qYES=10.00 qNO=10.00
2026-03-06 05:20:47.998|INFO| [MAKER_SKEW][ARB] fill wait y_oid=0x0125037e n_oid=0x1c9d5845 fy=0.00 fn=5.00
2026-03-06 05:20:48.895|INFO| LP=-6.5000 CPP=1.650000 TotalCost=16.5000 qYES=10.00 qNO=20.00
2026-03-06 05:20:52.953|INFO| LP=-0.3008 CPP=1.015041 TotalCost=20.2995 qYES=20.00 qNO=20.00
2026-03-06 05:21:15.891|INFO| LP=-2.2995 CPP=1.091980 TotalCost=27.2995 qYES=30.00 qNO=25.00
```

Trade-history-supported fills for this market:
- YES:
  - `0x973a9b10` = `10`
  - `0x0125037e` = `2.040000 + 2.959352 = 4.999352`
  - `0x44f7efc4` = `0.420000 + 4.580000 = 5.000000`
  - `0x96d0a15f` = `5.000000`
  - Total YES = `24.999352`
- NO:
  - `0xcec5b939` = `8.770000 + 1.230000 = 10.000000`
  - `0x1c9d5845` = `5.000000`
  - `0x94183a3a` = `5.000000`
  - Total NO = `20.000000`

Bot final state for the same run:
- `qYES=30.00 qNO=25.00`
- `TotalCost=27.2995`

Expected state from trade history:
- `qYES=24.999352`
- `qNO=20.000000`

The excess again lines up with a duplicated first pair application:
- phantom YES `~5`
- phantom NO `5`

## Root Cause Hypothesis
The current dedupe key is still not stable across all maker message variants. The same economic execution can arrive in more than one shape:
1. one variant is applied to inventory and per-order progress,
2. a later variant is recognized and logged as duplicate,
3. but by then the internal state has already absorbed the fill more than once.

The pattern suggests the problem is not only logging duplication. It is an inventory mutation path problem.

## Why This Is The Main Remaining Blocker
The control plane is now mostly doing the right thing:
- pending imbalance is set and cleared,
- pair-arb suppression fires while imbalance is active,
- heavy-side admission is much better,
- reject storms are mostly contained.

But none of that is trustworthy if inventory is wrong. A bot with incorrect `qYES/qNO` can still:
- rebalance the wrong side,
- suppress valid trades for the wrong reason,
- carry phantom cost into expiry logic,
- misreport PnL and risk.

## Recommended Fix Direction

### 1) Use transaction-first maker execution dedupe
Primary identity should be:
- `maker_order_id + transaction_hash`

Only use weaker fallbacks if `transaction_hash` is absent, and log when fallback path is used.

### 2) Inventory mutation and `maker_exec_progress` must share the same ledger
Do not let one path dedupe inventory while another separately increments per-order progress. Both should be driven by the same accepted unique execution record.

### 3) Add hard invariant on per-order applied quantity
For each maker order:
- `applied_qty(order_id)` must never exceed the sum of unique transaction-backed matched amounts seen for that order.

If it would exceed:
- reject the mutation,
- emit a hard warning with `order_id`, `tx`, `qty`, `applied_qty_before`, `applied_qty_after`, `expected_max`.

### 4) Keep pair `fy/fn` fully order-centric
Continue using pair-order OIDs for pair wait logic, but source those values only from the deduped maker execution ledger.

## Validation Checklist
1. Replay `btc-updown-5m-1772748900` and confirm final state is near:
   - `qYES=24.991175`
   - `qNO=24.986630`
2. Replay `btc-updown-5m-1772749200` and confirm the first pair cycle lands near `qYES=15 qNO=15`, then final state is near:
   - `qYES=24.999352`
   - `qNO=20.000000`
3. No maker order's `applied_qty` exceeds the sum of unique transaction-backed fills for that order.
4. Duplicate user-trade events still log as deduped, but no longer mutate inventory/cost/progress.

## Notes for Next AI Model
- This is the highest-priority remaining issue.
- Fix accounting before making more strategy changes.
- Primary files: `src/bot.rs`, possibly `src/env_contract.rs` only if new diagnostic toggles are added.
