# Environment Reference

Date: 2026-03-19
Scope: supported operator env for the `polybot` binary only.

This repo now has one supported runtime:

- `EXEC_MODE=BOT`
- if `EXEC_MODE` is unset, startup defaults to `BOT`
- any other `EXEC_MODE` value fails fast at startup

This document intentionally covers the active `polybot` surface only. It does not document helper-binary-only env such as `COPYTRADE_*` or `COPY_COLLECT_*`, and it does not treat legacy compatibility aliases as part of the supported contract except where a migration alias is explicitly called out below.

## Runtime and Identity

- `EXEC_MODE`: only supported value is `BOT`
- `POLYBOT_PRINT_ENV_CONTRACT`: when set to `1`, prints the supported env contract and exits
- `BOT_ID`, `BOT_DESCRIPTION`, `ACCOUNT_NAME`: operator-facing run identity written into logs and summaries
- `DB_URL`: database connection used for trade/finalization persistence
- `LOG_DIR`: root directory for per-market bot logs and optional upload input

## Auth and Connectivity

- `POLYMARKET_PRIVATE_KEY`: primary trading credential
- `POLYMARKET_FUNDER`, `POLYMARKET_WALLET_ADDRESS`, `WALLET_ADDRESS`, `SIGNATURE_TYPE`: wallet/signing metadata and fallbacks
- `CHAIN_ID`, `CLOB_HOST`, `WS_BASE`, `CLOB_GAMMA_API_URL`: Polymarket API, websocket, and gamma connectivity
- `POLYMARKET_API_KEY`, `POLYMARKET_API_SECRET`, `POLYMARKET_API_PASSPHRASE`: optional explicit user-websocket auth fallback when derived creds are not used

## Market Selection

- `MARKET_SYMBOL`, `MARKET_SEGMENT`, `MARKET_DURATION_SECONDS`, `MARKET_STEP_SECONDS`: default market family selection
- `MARKET_SLUG`: explicit market override
- `MARKET_SLUG_STYLE`: slug parsing/generation style
- `AUTO_DETECT_MARKET_PARAMS`: allows market-param refresh from live market data

## Core Trading Controls

- `DRY_RUN`: simulation vs live placement
- `MIN_SHARES`, `CLIP_SHARES`, `MAX_TOTAL_COST`, `RESERVE_USD`: base sizing and budget caps
- `LOG_EVERY_SECONDS`, `LOOP_WAIT_SECONDS_MAKER`, `LOOP_WAIT_SECONDS_TAKER`: loop cadence and heartbeat logging
- `MARKET_DATA_STALE_ADD_BLOCK_SECONDS`, `MARKET_DATA_STALE_HARD_PAUSE_SECONDS`, `STOP_BUFFER_SECONDS`, `WARMUP_SECONDS`: timing and stale-data guards
- `ENTRY_EDGE_TICKS`, `MIN_ENTRY_EDGE_TICKS`, `MAX_SPREAD_TICKS`, `PARITY_TOLERANCE`: entry quality controls
- `HEDGE_BUFFER_TICKS`, `HEDGE_SLIPPAGE_TICKS`, `HEDGE_TAKER_ORDER_TYPE`: hedge and taker execution settings
- `MIN_MAKER_NOTIONAL`, `MIN_TAKER_NOTIONAL`, `FIRST_CLIP_SHARES`, `FIRST_HEDGE_FULL`, `UNHEDGED_TIMEOUT_SECONDS`: startup and exposure controls
- `TAKER_ORDER_TTL_SECONDS`, `TAKER_FILL_FALLBACK_FROM_ORDER_EVENTS`, `TAKER_STRICT_INFLIGHT`, `TAKER_HEDGE_MIN_INTERVAL`: taker lifecycle controls
- `IMPROVE_BID_TICKS`, `REPLACE_IF_PRICE_MOVES_TICKS`, `STALE_SECONDS`, `CLOB_ORDER_META_WARMUP`: order posting and repricing behavior
- `STALE_SECONDS` remains the order-management stale-order control; it is not the market-data add-block or hard-pause policy

## BOT Runtime Controls

- `BOT_PREARM_LEAD_SECONDS`: pre-arm lead before the market opens
- `BOT_CLIP_LADDER`: authoritative four-rung clip ladder for seed, normal, and green-gated large clips
- `BOT_REPAIR_RESERVE_BUFFER_USD`: reserve kept aside for lighter-side repair
- `BOT_BUDGET_SEED_MIN_FRACTION`, `BOT_BUDGET_SEED_MAX_FRACTION`: seed-phase budget band
- `BOT_BUDGET_EARLY_MIN_FRACTION`, `BOT_BUDGET_EARLY_MAX_FRACTION`: early pair-build budget band
- `BOT_BUDGET_MAIN_MIN_FRACTION`, `BOT_BUDGET_MAIN_MAX_FRACTION`: main pair-build budget band
- `BOT_BUDGET_LATE_MIN_FRACTION`, `BOT_BUDGET_LATE_MAX_FRACTION`: late pair-build budget band
- `BOT_BUDGET_TAPER_MIN_FRACTION`, `BOT_BUDGET_TAPER_MAX_FRACTION`: taper budget band
- `BOT_TARGET_BOTH_SIDES_BY_30S`, `BOT_TARGET_BOTH_SIDES_BY_60S`: two-sided progress canaries
- `BOT_LATE_REDUCE_START_SECONDS`, `BOT_LATE_BALANCE_ONLY_START_SECONDS`, `BOT_LATE_STOP_NEW_ORDERS_START_SECONDS`: authoritative late-window timing
- `BOT_TAPER_START_SECONDS`, `BOT_FINAL_QUIET_SECONDS`: accepted legacy compatibility aliases for late-window migration; `BOT_LATE_*` is preferred
- `BOT_TAIL_CAP_MID_START_SECONDS`, `BOT_TAIL_CAP_LATE_START_SECONDS`: tail-cap phase boundaries
- `BOT_TAIL_CAP_EARLY_FRACTION`, `BOT_TAIL_CAP_MID_FRACTION`, `BOT_TAIL_CAP_LATE_FRACTION`: tail inventory caps over time
- `BOT_BAD_REGIME_WINDOW_SECONDS`, `BOT_BAD_REGIME_EXPENSIVE_FRACTION`: expensive-regime protection
- `BOT_BUY_ONLY_NORMAL_FLOW`: must remain enabled for the supported runtime

## Market Data and Reconciliation

- `REQUIRE_USER_WS_CONNECTED`, `WS_PING_INTERVAL`, `WS_IO_TIMEOUT_SECONDS`, `DEBUG_THROTTLE_SECONDS`: websocket health and debug throttling
- `ORDERBOOK_HTTP_TIMEOUT`, `BOOK_CACHE_TTL_SECONDS`: orderbook snapshot fetch/cache behavior
- `RECONCILE_EXCHANGE_ORDERS`, `RECONCILE_INTERVAL_SECONDS`: exchange-order reconciliation cadence
- `RECONCILE_USE_DATA_API`, `MISMATCH_RECONCILE_FROM_BALANCE`: state-reconciliation source selection
- `RECONCILE_MIN_INTERVAL_SECONDS`, `RECONCILE_CONFIRM_DELAY_SECONDS`, `RECONCILE_NEVER_ZERO_WITHOUT_CONFIRM`, `RECONCILE_SELL_CREDIT_MULT`: reconciliation safety controls
- `UNWIND_CHUNK_SHARES`, `UNWIND_MAX_PASSES`, `UNWIND_WAIT_AFTER_ORDER_SECONDS`: chunked unwind behavior
- `UNWIND_DEPTH_GATE_ENABLED`, `DEPTH_GATE_LEVELS`, `DEPTH_GATE_MAX_AGE_SECONDS`: depth gating for unwind orders
- `MAKER_EXPOSURE_UNWIND_ORDER_TYPE`, `MAKER_EXPOSURE_UNWIND_SLIPPAGE_TICKS`: unwind order type and slippage

## Maker Order Lifecycle

- `MAKER_SINGLE_INFLIGHT_PER_SIDE`: enable single-slot maker order ownership
- `MAKER_SUBMIT_PENDING_TTL_SECONDS`, `MAKER_CANCEL_PENDING_TTL_SECONDS`, `MAKER_WORKING_MISSING_TTL_SECONDS`: local maker slot TTLs
- `MAKER_REPLACE_MIN_INTERVAL_SECONDS`: minimum replace spacing
- `MAKER_SUBMIT_REJECT_COOLDOWN_SECONDS`, `MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS`: submit reject backoff
- `MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET`: exchange-order cap enforced during reconciliation

## Fills, Guards, and Exit Plumbing

- `QUOTE_INVALIDATION_ENABLED`, `QUOTE_INVALIDATION_BUFFER_TICKS`: entry invalidation guard
- `OCO_ON_FILL`: cancel sibling side after a fill when applicable
- `POLY_DATA_API_BASE_URL`, `POSITIONS_API_TIMEOUT_SECONDS`, `POSITIONS_API_USER`, `POSITIONS_API_FILTER_MARKET`, `POSITIONS_API_DEBUG_ALL`: position lookup controls
- `POLY_CONDITIONAL_UNITS_PER_SHARE`, `BALANCE_ALLOWANCE_UPDATE_ENABLED`, `BALANCE_ALLOWANCE_DEBUG_ALL`: balance/allowance normalization
- `USER_TRADE_DEBUG`: verbose user trade-event logging
- `SIZE_DECIMALS`, `TAKER_EXIT_ALLOW_FRACTIONAL_SIZE`, `TAKER_EXIT_MIN_ORDER_SIZE`: taker exit sizing controls

## Execution Latency Logs

- `EXEC_LATENCY_LOG_ENABLED`: master switch for exec-latency logging
- `EXEC_LATENCY_FILE_LOG_ENABLED`, `EXEC_LATENCY_JSONL_ENABLED`, `EXEC_LATENCY_CSV_ENABLED`: file-output switches
- `EXEC_LATENCY_LOG_DIR`, `EXEC_LATENCY_JSONL_PATH`, `EXEC_LATENCY_CSV_PATH`: output paths
- `EXEC_LATENCY_FILE_LOG_SUBMIT_ALL_EVENTS`, `EXEC_LATENCY_FILE_LOG_SUBMIT_EVENTS`: submit-event capture scope
- `EXEC_LATENCY_CONTEXT_TTL_SECONDS`, `EXEC_LATENCY_MAX_CONTEXT_RECORDS`: in-memory context retention
- `EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE`, `EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE_MAKER`: console breakdown logging

## Reporting, Alerts, and Validation

- `DAILY_PNL_TAKE_PROFIT_USD`, `DAILY_PNL_STOP_LOSS_USD`: daily lockout thresholds
- `PNL_STATS_AT_END_ENABLED`, `TRADE_REALIZED_LOG_ENABLED`: end-of-market reporting toggles
- `TRADE_VALIDATION_ENABLED`, `TRADE_VALIDATION_AFTER_MARKET_ENABLED`, `TRADE_VALIDATION_POLL_SECONDS`: trade validation enablement and cadence
- `TRADE_VALIDATION_LOOKBACK_DAYS`, `TRADE_VALIDATION_MAX_TRADES_PER_POLL`, `TRADE_VALIDATION_PAGE_LIMIT`, `TRADE_VALIDATION_MAX_PAGES`, `TRADE_VALIDATION_API_TIMEOUT_SECONDS`: validation API window and paging
- `TRADE_VALIDATION_USER`, `TRADE_VALIDATION_USERS`: explicit validation wallet selection
- `TELEGRAM_BOT_ID`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`, `TELEGRAM_TIMEOUT_SECONDS`: Telegram alerting

## R2 Uploads

- `R2_UPLOAD_ENABLED`: enable upload before rollover
- `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_BUCKET`, `R2_REGION`, `R2_ENDPOINT`: R2 credentials and target
- `R2_UPLOAD_PREFIX`: object key prefix
- `R2_UPLOAD_INCLUDE_EXEC_LATENCY`: include exec-latency files in the upload set

## RTDS and ClickHouse

- `RTDS_ENABLED`: master switch
- `RTDS_PROVIDER`, `RTDS_SINK`, `RTDS_SUB_TYPE`, `RTDS_SYMBOL`, `RTDS_TOPIC`, `RTDS_WS_URL`: RTDS source and subscription settings
- `RTDS_WS_RECONNECT_MIN`, `RTDS_WS_RECONNECT_MAX`, `RTDS_WS_PING_INTERVAL`, `RTDS_WS_READ_TIMEOUT_SECONDS`: RTDS websocket transport settings
- `RTDS_LOG_REALTIME`, `RTDS_LOG_RAW`, `RTDS_LOG_TO_FILE`: RTDS logging toggles
- `RTDS_STATE_PATH`, `RTDS_STATE_MAX_RECORDS`, `RTDS_WRITE_LATEST_FILE`, `RTDS_PERSIST_STATE_TO_FILE`, `RTDS_LATEST_PATH`: RTDS state persistence
- `RTDS_PRICE_LOG_PATH`, `RTDS_PRICE_TO_BEAT`, `RTDS_PRICE_TO_BEAT_STATE_PATH`: price-to-beat tracking
- `RTDS_CLICKHOUSE_ENABLED`, `RTDS_CLICKHOUSE_TIMEOUT_SECONDS`, `RTDS_CLICKHOUSE_ERROR_LOG_EVERY_MS`, `RTDS_CLICKHOUSE_BATCH_MAX_ROWS`, `RTDS_CLICKHOUSE_BATCH_MAX_DELAY_MS`, `RTDS_CLICKHOUSE_AUTO_CREATE_SCHEMA`: RTDS ClickHouse sink controls
- `CLICKHOUSE_URL`, `CLICKHOUSE_DATABASE`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`: ClickHouse connection
- `CLICKHOUSE_TABLE_RTDS_PRICES`, `CLICKHOUSE_TABLE_RTDS_PRICE_TO_BEAT`, `CLICKHOUSE_TABLE_RTDS_RESOLUTION_STATE`: ClickHouse table names
- `RTDS_CHAINLINK_API_KEY`, `RTDS_CHAINLINK_API_SECRET`, `RTDS_CHAINLINK_REST_URL`, `RTDS_CHAINLINK_WS_URL`: Chainlink provider credentials and endpoints
- `RTDS_CHAINLINK_WS_HA`, `RTDS_CHAINLINK_WS_MAX_RECONNECT`, `RTDS_CHAINLINK_INSECURE_SKIP_VERIFY`, `RTDS_CHAINLINK_WS_READ_TIMEOUT_SECONDS`: Chainlink websocket behavior
- `RTDS_CHAINLINK_FEED_ID`, `RTDS_CHAINLINK_FEED_ID_BTC`, `RTDS_CHAINLINK_FEED_ID_ETH`, `RTDS_CHAINLINK_FEED_ID_SOL`, `RTDS_CHAINLINK_FEED_ID_XRP`, `RTDS_CHAINLINK_FEED_ID_DOGE`, `RTDS_CHAINLINK_FEED_ID_MATIC`: feed IDs
- `RTDS_CHAINLINK_PRICE_DECIMALS`: feed decimal normalization
- `RTDS_CLOB_JOIN_ENABLED`, `RTDS_CLOB_WS_URL`, `RTDS_CLOB_WS_RECONNECT_MIN`, `RTDS_CLOB_WS_RECONNECT_MAX`, `RTDS_CLOB_WS_PING_INTERVAL`, `RTDS_CLOB_WS_READ_TIMEOUT_SECONDS`: optional CLOB join stream
- `RTDS_CLOB_MATCH_MAX_AGE_MS`, `RTDS_CLOB_HISTORY_MAX_RECORDS`, `RTDS_CLOB_HISTORY_MAX_AGE_MS`: RTDS CLOB match/history retention

## Not Part of the Supported Operator Surface

- helper-binary-only env: `COPYTRADE_*`, `COPY_COLLECT_*`, `CLICKHOUSE_*_PATH`
- removed legacy mode families: `SETTLEMENT_SHAPER_*`, `SIGNAL_*`, `SNIPER_*`, `PAIR_BASE_*`, `PAIR_RECOVERY_*`, `PAIR_ARB_*`, `MAKER_SKEW_*`
- stale legacy control families removed from the contract: `FSM_*`, `MAX_LOSS_*`, `FORCE_FLATTEN_*`, `RISK_EXIT_*`, `RTDS_ENTRY_GATE_*`, `RTDS_GATE_*`
- deprecated stale-data alias `MARKET_DATA_STALE_SECONDS` is intentionally unsupported and startup now fails fast if it is set
- legacy compatibility aliases such as `API_KEY`, `API_SECRET`, `API_PASSPHRASE`, `CLOB_API_*`, `POLY_API_*`, and `GAMMA_HOST` are intentionally omitted from the supported contract
