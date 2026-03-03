# Binance-Based Sniper Momentum + Micro-Breakout Filters

## Summary

This project now supports Binance spot-driven sniper entry filters for BTC:

- Momentum confirmation filter
- Micro-breakout trigger filter
- JSON state persistence for candle/filter runtime state

The filters are enforced in sniper entry paths before order submit.

## Data Source and Scope

- Data source: Binance spot (`BTCUSDT`) via REST warmup + WS trades
- Initial asset scope: BTC only (controlled by symbol lists in env)
- Existing RTDS gates remain active and independent

## Entry Decision Order

For sniper entries, the flow is:

1. Build candidate from Polymarket quotes and apply base candidate checks
   - price band (`SNIPER_PRICE_MIN`/`SNIPER_PRICE_MAX`)
   - spread/parity/ROI windows
2. Apply RTDS entry gate
3. Apply Binance sniper filters
   - breakout + momentum combination logic
4. Submit order only if all active gates pass

If both momentum and breakout are disabled, Binance filter gating is bypassed.

## Momentum Filter

Enabled by:

- `SNIPER_MOMENTUM_CONFIRM_ENABLED=true`

Core checks (per side):

- Trend: `EMA(fast) > EMA(slow)` for YES, inverted for NO
- Slope: `EMA(fast)[t] - EMA(fast)[t-1]` sign matches side
- Candle body count in recent window

Pass threshold:

- `SNIPER_MOMENTUM_REQUIRED_CHECKS` (typically 2 of 3)

Fail-closed conditions when enabled:

- Stale Binance tick snapshot
- Insufficient completed candles

## Breakout Filter

Enabled by:

- `SNIPER_BREAKOUT_ENABLED=true`

Breakout levels from last `K` completed 1m candles:

- `Hk = max(high)`
- `Lk = min(low)`
- `buffer_up = Hk * (1 + bps/10000)`
- `buffer_dn = Lk * (1 - bps/10000)`

Trigger rules:

- YES requires price >= `buffer_up` for at least `persistence_ms`
- NO requires price <= `buffer_dn` for at least `persistence_ms`
- Trigger direction is latched into `active_trigger` until `rearm_ms` expires

Mode:

- `required` (default): side must match breakout trigger direction
- `assist`: if no breakout trigger, strict momentum fallback can allow entry

Fail-closed conditions when enabled:

- Stale Binance tick snapshot
- Insufficient candles / missing breakout levels

## JSON State Persistence

Persistence is controlled by:

- `SNIPER_FILTERS_PERSIST_STATE`
- `SNIPER_FILTERS_STATE_PATH`
- `SNIPER_FILTERS_STATE_WRITE_MIN_INTERVAL_MS`

Persisted data includes:

- Completed + current 1m candles
- Last tick timestamp/price
- Breakout levels/buffers and trigger timers
- `active_trigger`
- Momentum snapshots for both directions:
  - `momentum_yes`
  - `momentum_no`

State is loaded at bot startup (when enabled) and saved on state changes + bot stop.

## Logging

Filter decision logs:

- `[MOMENTUM] ...`
- `[BREAKOUT] ...`

Flat sniper status line now includes compact filter metrics suffix, for example:

`[SNIPER] t_left=   9.0s trades=0 pnl(mtm)=+0.0000 (flat) | mom[y=2/2:ok,n=1/2:checks_failed] brk[dir=NONE y:no_trigger n:no_trigger trig=false cd=0ms]`

## Recommended BTC Settings (Example)

```env
SNIPER_BINANCE_VENUE=GLOBAL
SNIPER_BINANCE_SYMBOL=BTCUSDT
SNIPER_BINANCE_QUOTE_ASSET=USDT

SNIPER_MOMENTUM_CONFIRM_ENABLED=true
SNIPER_MOMENTUM_SYMBOLS=btc
SNIPER_MOMENTUM_REQUIRED_CHECKS=2
SNIPER_MOMENTUM_EMA_FAST=3
SNIPER_MOMENTUM_EMA_SLOW=8
SNIPER_MOMENTUM_WINDOW_CANDLES=4
SNIPER_MOMENTUM_WINDOW_MIN_BULLISH=3
SNIPER_MOMENTUM_MAX_SNAPSHOT_AGE_SECONDS=1.0

SNIPER_BREAKOUT_ENABLED=true
SNIPER_BREAKOUT_SYMBOLS=btc
SNIPER_BREAKOUT_LEVEL_LOOKBACK_CANDLES=3
SNIPER_BREAKOUT_BUFFER_BPS=5
SNIPER_BREAKOUT_PERSISTENCE_MS=2800
SNIPER_BREAKOUT_REARM_MS=15000
SNIPER_BREAKOUT_MAX_SNAPSHOT_AGE_SECONDS=1.0
SNIPER_BREAKOUT_MODE=required
SNIPER_BREAKOUT_ASSIST_MOMENTUM_REQUIRED_CHECKS=3

SNIPER_FILTERS_PERSIST_STATE=true
SNIPER_FILTERS_STATE_PATH=state/sniper_filters_state_polybot_btc.json
SNIPER_FILTERS_STATE_WRITE_MIN_INTERVAL_MS=250
```
