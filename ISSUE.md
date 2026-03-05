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
- Current state: Fixed in v0.1.18. Imbalance guard, reject backoff deployed. Quote invalidation bypass pending.

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
