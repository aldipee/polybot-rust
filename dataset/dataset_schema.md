# Schema Reference

This document describes the current output schema produced by `main.py`.

## Scope

- Row grain: one Polymarket BTC 5-minute trade record after enrichment.
- CSV output: the final report columns listed below.
- PostgreSQL output: the same columns as the CSV, plus one DB-only internal key: `trade_identity_key`.
- Exact field names matter. The current schema intentionally preserves the existing `snapsot_*` spelling.

## Sources

- Polymarket `/trades`: raw trade metadata.
- Binance 1-minute `BTCUSDT` klines: local model and technical indicators.
- PolyBackTest market lookup + snapshot-at: market state, snapshot prices, and top-of-book.
- Local derivations: time-window fields, taker flag, model outputs, and deltas.

## Notes

- Snapshot fields may be null when PolyBackTest market lookup fails, the snapshot is not found, or the API omits that field.
- `final_outcome` can still be filled even when no snapshot is found, because it may come from the PolyBackTest market payload.
- `snapsot_market_btc_price_to_beat` comes from PolyBackTest `market.btc_price_start`.
- `snapsot_btc_price_delta = snapsot_market_btc_price - snapsot_market_btc_price_to_beat`.

## Final CSV / Report Fields

### Polymarket Trade Fields

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `proxyWallet` | string | Polymarket | Wallet address associated with the trade row. | No |
| `side` | string | Polymarket | Trade side, typically `BUY` or `SELL`. | No |
| `asset` | string | Polymarket | Asset/token identifier for the traded outcome token. | No |
| `conditionId` | string | Polymarket | Polymarket condition identifier for the market. | No |
| `size` | number | Polymarket | Trade size in outcome shares/contracts. | No |
| `price` | number | Polymarket | Executed trade price. | No |
| `timestamp` | integer | Polymarket | Trade timestamp in Unix seconds. | No |
| `title` | string | Polymarket | Human-readable market title. | No |
| `slug` | string | Polymarket | Market slug from the trade payload. | Yes |
| `eventSlug` | string | Polymarket | Event slug from the trade payload; used to identify BTC 5-minute windows. | Yes |
| `outcome` | string | Polymarket | Outcome side for the trade, usually `Up` or `Down`. | Yes |
| `outcomeIndex` | integer | Polymarket | Numeric outcome index from Polymarket. | Yes |
| `transactionHash` | string | Polymarket | On-chain transaction hash for the fill. | Yes |
| `is_taker` | boolean | Local derivation | `True` if the trade also appears in `/trades?takerOnly=true`; otherwise `False`. | No |

### Time / Window Fields

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `window_start` | number | Local derivation | Start of the BTC 5-minute window in Unix seconds, parsed from `eventSlug`. | Yes |
| `window_end` | number | Local derivation | End of the BTC 5-minute window in Unix seconds, calculated as `window_start + 300`. | Yes |
| `t_remain_s` | number | Local derivation | Seconds remaining in the 5-minute market at trade time. | Yes |
| `t_into_s` | number | Local derivation | Seconds elapsed in the 5-minute market at trade time. | Yes |
| `trade_time_utc` | datetime | Local derivation | UTC datetime derived from `timestamp`. | No |

### Binance-Derived Market Fields

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `binance_btc_trade_px` | number | Binance | BTC spot price at or immediately before the trade timestamp from 1-minute Binance candles. | Yes |
| `binance_btc_start_px` | number | Binance | BTC spot price at or immediately before the market window start from 1-minute Binance candles. | Yes |
| `binance_delta_from_start` | number | Local derivation | `(binance_btc_trade_px / binance_btc_start_px) - 1`. | Yes |
| `binance_rsi14_at_trade` | number | Binance + local derivation | 14-period RSI from 1-minute BTC closes, sampled at trade time. | Yes |
| `binance_vol30m_1m_at_trade` | number | Binance + local derivation | Rolling 30-minute standard deviation of 1-minute BTC log returns at trade time. | Yes |
| `binance_up_model` | number | Local model | Estimated probability that BTC finishes above the window start price by expiry. | Yes |
| `binance_down_model` | number | Local model | Complement of `binance_up_model`, calculated as `1 - binance_up_model`. | Yes |
| `edge_model_minus_price` | number | Local model | Model edge versus paid price. Uses `binance_up_model - price` for `Up` and `binance_down_model - price` for `Down`. | Yes |

### Resolution / Snapshot Metadata

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `final_outcome` | string | PolyBackTest market/snapshot | Final resolved winner when available, typically `Up` or `Down`. | Yes |
| `snapshot_status` | string | Local derivation | Snapshot enrichment status: `matched`, `not_found`, `market_lookup_failed`, or `snapshot_error`. | No |
| `snapshot_requested_ts_ms` | integer | Local derivation | Requested snapshot timestamp in Unix milliseconds, derived from the trade time. | Yes |
| `snapshot_market_id` | string | PolyBackTest | PolyBackTest market identifier used for snapshot lookup. | Yes |
| `snapshot_time` | datetime/string | PolyBackTest | Timestamp of the matched snapshot returned by PolyBackTest. | Yes |
| `snapshot_match_delta_ms` | integer | Local derivation | `snapshot_time - snapshot_requested_ts_ms` in milliseconds. | Yes |
| `snapshot_id` | string | PolyBackTest | Snapshot identifier from the PolyBackTest payload. | Yes |

### Snapshot Prices, Sizing, and Deltas

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `snapsot_market_btc_price` | number | PolyBackTest snapshot | BTC price stored on the matched PolyBackTest snapshot. | Yes |
| `snapshot_price_up` | number | PolyBackTest snapshot | Snapshot price for the `Up` outcome. | Yes |
| `snapshot_price_down` | number | PolyBackTest snapshot | Snapshot price for the `Down` outcome. | Yes |
| `snapshot_last_trade_price_up` | number | PolyBackTest snapshot/market | Last traded `Up` price if available; may fall back to `snapshot_price_up`. | Yes |
| `snapshot_last_trade_price_down` | number | PolyBackTest snapshot/market | Last traded `Down` price if available; may fall back to `snapshot_price_down`. | Yes |
| `snapshot_min_order_size_up` | number | PolyBackTest market/snapshot | Minimum order size for the `Up` side when provided by PolyBackTest. | Yes |
| `snapshot_min_order_size_down` | number | PolyBackTest market/snapshot | Minimum order size for the `Down` side when provided by PolyBackTest. | Yes |
| `snapshot_tick_size_up` | number | PolyBackTest market/snapshot | Tick size for the `Up` side when provided by PolyBackTest. | Yes |
| `snapshot_tick_size_down` | number | PolyBackTest market/snapshot | Tick size for the `Down` side when provided by PolyBackTest. | Yes |
| `snapsot_market_btc_price_to_beat` | number | PolyBackTest market | Starting BTC price for the market, sourced from `btc_price_start`. | Yes |
| `snapsot_btc_price_delta` | number | Local derivation | `snapsot_market_btc_price - snapsot_market_btc_price_to_beat`. | Yes |

### Snapshot Order Book: Up Side

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `snapshot_orderbook_up_bid_count` | integer | PolyBackTest snapshot | Number of bid levels present in the `Up` order book. | Yes |
| `snapshot_orderbook_up_ask_count` | integer | PolyBackTest snapshot | Number of ask levels present in the `Up` order book. | Yes |
| `snapshot_orderbook_up_spread` | number | Local derivation | Best `Up` ask minus best `Up` bid. | Yes |
| `snapshot_orderbook_up_bid_1_price` | number | PolyBackTest snapshot | Best bid price for the `Up` order book. | Yes |
| `snapshot_orderbook_up_bid_1_size` | number | PolyBackTest snapshot | Best bid size for the `Up` order book. | Yes |
| `snapshot_orderbook_up_ask_1_price` | number | PolyBackTest snapshot | Best ask price for the `Up` order book. | Yes |
| `snapshot_orderbook_up_ask_1_size` | number | PolyBackTest snapshot | Best ask size for the `Up` order book. | Yes |

### Snapshot Order Book: Down Side

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `snapshot_orderbook_down_bid_count` | integer | PolyBackTest snapshot | Number of bid levels present in the `Down` order book. | Yes |
| `snapshot_orderbook_down_ask_count` | integer | PolyBackTest snapshot | Number of ask levels present in the `Down` order book. | Yes |
| `snapshot_orderbook_down_spread` | number | Local derivation | Best `Down` ask minus best `Down` bid. | Yes |
| `snapshot_orderbook_down_bid_1_price` | number | PolyBackTest snapshot | Best bid price for the `Down` order book. | Yes |
| `snapshot_orderbook_down_bid_1_size` | number | PolyBackTest snapshot | Best bid size for the `Down` order book. | Yes |
| `snapshot_orderbook_down_ask_1_price` | number | PolyBackTest snapshot | Best ask price for the `Down` order book. | Yes |
| `snapshot_orderbook_down_ask_1_size` | number | PolyBackTest snapshot | Best ask size for the `Down` order book. | Yes |

## PostgreSQL-Only Internal Field

| Field | Logical Type | Source | Description | Nullable |
| --- | --- | --- | --- | --- |
| `trade_identity_key` | string | Local derivation | Unique deduplication key stored only in PostgreSQL. Built from `transactionHash`, `timestamp`, `asset`, `conditionId`, `price`, `size`, `side`, and `outcomeIndex`. | No |

## Fields Removed From the Final Report

These source fields are intentionally dropped before CSV / final DB write:

- `icon`
- `name`
- `pseudonym`
- `bio`
- `profileImage`
- `profileImageOptimize`
- `profileImageOptimized`
