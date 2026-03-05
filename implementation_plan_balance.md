# Implementation Plan: Replace Balance-Based Reconciliation

## Problem
[get_balance_allowance](file:///c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs#5888-6036) (CLOB API) always returns `raw_balance=0` for YES tokens, causing the bot to zero out its position state on every reconciliation.

## Design: Multi-Source Position Verification

### Source Hierarchy (by reliability)

| Source | Latency | Reliability | Use |
|--------|---------|------------|-----|
| **Fill events** (CLOB WS) | Real-time (~ms) | High (already working) | Primary tracker |
| **Trade events** (User WS) | ~1-5s delay | High (CONFIRMED = on-chain) | Confirmation signal |
| **Data API** (`/positions`) | ~1-3s (HTTP) | Medium-High (actual position) | Verification/reconciliation |
| ~~Balance API~~ | N/A | **Broken** | **Remove** |

### New Reconciliation Strategy

```
_reconcile_state_from_balances (RENAMED: _reconcile_state_from_positions)
├── Query Data API for YES + NO position sizes
├── Compare with internal state (q_yes, q_no)
├── If Data API > internal → missed fills → cautiously ADD (use ask price for cost)
├── If Data API < internal → possible settlement lag
│   ├── First check: log warning, set "suspect" flag + timestamp
│   ├── If suspect persists for >N seconds → then reduce (conservative)
│   └── NEVER zero out in a single check
└── If Data API ≈ internal → no action (state is consistent)
```

### Key Safety Rails

1. **Never zero-out in one shot**: If Data API says 0 but internal says 15, don't immediately reset. Instead, require 2+ consecutive checks with a minimum gap of `RECONCILE_CONFIRM_DELAY_SECONDS` (default: 3s).

2. **Directional bias**: Trust "upward" corrections (missed fills) more readily than "downward" (phantom sells). Downward adjustments need stronger confirmation.

3. **Minimum delta threshold**: Don't reconcile for differences smaller than `min_shares` — those are likely rounding/timing.

4. **Rate limiting**: Don't reconcile more than once every `RECONCILE_MIN_INTERVAL_SECONDS` (default: 5s) to avoid thrashing.

## Files to Change

### [src/bot.rs](file:///c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs)

1. **Replace [_reconcile_state_from_balances](file:///c:/Works/aldipranata.com/polybot-convert-rust/main.py#2486-2585)** → use Data API instead of balance API
2. **Add suspect tracking fields** to [MakerHedgeCapBot](file:///c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs#232-302) struct  
3. **Update [_handle_exposure_mismatch](file:///c:/Works/aldipranata.com/polybot-convert-rust/src/bot.rs#7850-7909)** → trust fill tracking more, use Data API as tiebreaker
4. **Force-flatten loop** → use Data API, keep conservative behavior

## Changes

### Change 1: Add suspect tracking fields (struct)
Add two new fields to track suspected position discrepancies:
- `reconcile_suspect_yes: Arc<Mutex<Option<(f64, f64)>>>` — (timestamp, suspected_balance)
- `reconcile_suspect_no: Arc<Mutex<Option<(f64, f64)>>>` — (timestamp, suspected_balance)

### Change 2: Rewrite [_reconcile_state_from_balances](file:///c:/Works/aldipranata.com/polybot-convert-rust/main.py#2486-2585)
Replace the balance API calls with Data API calls. Add dual-confirmation logic.

### Change 3: Update env contract
Add new env vars:
- `RECONCILE_USE_DATA_API` (default: true)
- `RECONCILE_CONFIRM_DELAY_SECONDS` (default: 3.0)
- `RECONCILE_MIN_INTERVAL_SECONDS` (default: 5.0)
- `RECONCILE_NEVER_ZERO_WITHOUT_CONFIRM` (default: true)
