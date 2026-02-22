# Collection Mode

This document explains how to run the bot as a data collector with RTDS (+ CLOB join), without placing live orders.

## Critical Runtime Variables

These are the non-optional items for stable collection mode in this codebase:

- `DRY_RUN=true`
- `RTDS_ENABLED=true`
- `MARKET_SLUG=<current market slug>` (for example `btc-updown-5m-1771642500`)  
  or set `MARKET_SYMBOL=<asset>` (fallback to `RTDS_SYMBOL`) for auto-generation when slug is empty
- `MARKET_SEGMENT=<5M|15M|1H|4H|1D>`
- `EXEC_MODE=SNIPER` or `EXEC_MODE=MAKER` (do not use SIGNAL mode unless SIGNAL WS is configured)
- `POLYMARKET_PRIVATE_KEY=<required by startup checks>`
- `POLYMARKET_FUNDER=<required by startup checks>`
- `SIGNATURE_TYPE=1` (standard Polymarket setup)
- `DB_URL=<postgres url>` (startup initializes DB schema even in dry run)

If `MARKET_SLUG` is empty, startup auto-generates one from current time using:

- `MARKET_SYMBOL` (or `RTDS_SYMBOL`)
- `MARKET_SEGMENT`
- `MARKET_STEP_SECONDS` (or segment default step)
- Optional `MARKET_SLUG_STYLE`:
  - `TIMESTAMP` (default)
  - `HUMAN_ET` (ET-based human slug for `1H`/`1D`; other segments remain timestamp style)

If neither `MARKET_SLUG` nor symbol hint (`MARKET_SYMBOL`/`RTDS_SYMBOL`) is provided, startup fails (unless signal-follow provides slug first).

## Segment and Slug Behavior (Important)

`MARKET_SEGMENT` controls defaults used for rollover:

- `5M`: duration `360s`, step `300s`
- `15M`: duration `900s`, step `900s`
- `1H`: duration `3600s`, step `3600s`
- `4H`: duration `14400s`, step `14400s`
- `1D`: duration `86400s`, step `86400s`

You can override with:

- `MARKET_DURATION_SECONDS`
- `MARKET_STEP_SECONDS`

Slug rollover uses `MARKET_STEP_SECONDS` on timestamp-based slugs.

Auto-generated slug format:

- `<asset>-updown-5m-<slot_ts>`
- `<asset>-updown-15m-<slot_ts>`
- `<asset>-updown-1h-<slot_ts>`
- `<asset>-updown-4h-<slot_ts>`
- `<asset>-updown-1d-<slot_ts>`

`<slot_ts>` is the current Unix timestamp rounded down to the configured step.

When `MARKET_SLUG_STYLE=HUMAN_ET`:

- `1H`: `<asset-name>-up-or-down-<month>-<day>-<hour><am|pm>-et`
  - example: `bitcoin-up-or-down-february-22-10pm-et`
- `1D`: `<asset-name>-up-or-down-on-<month>-<day>`
  - example: `bitcoin-up-or-down-on-february-22`

Human style is generated using current `America/New_York` (ET) time.

## Full Collector Profile (5M Rolling)

```env
# core runtime
DRY_RUN=true
EXEC_MODE=SNIPER
MARKET_SEGMENT=5M
MARKET_SLUG=btc-updown-5m-1771642500

# required startup identity/config checks
POLYMARKET_PRIVATE_KEY=0x...
POLYMARKET_FUNDER=0x...
SIGNATURE_TYPE=1
DB_URL=postgresql://postgres:postgres@localhost:5432/polybot

# RTDS collection
RTDS_ENABLED=true
RTDS_CLOB_JOIN_ENABLED=true
RTDS_CLOB_WS_URL=wss://ws-subscriptions-clob.polymarket.com/ws/market

# sink mode: file | clickhouse | both | none
RTDS_SINK=both
RTDS_WRITE_LATEST_FILE=true
RTDS_PERSIST_STATE_TO_FILE=true
```

## Sink Configuration

### File only

```env
RTDS_SINK=file
RTDS_WRITE_LATEST_FILE=true
RTDS_PERSIST_STATE_TO_FILE=true
```

Output files (default):

- `state/rtds_prices.jsonl`
- `state/rtds_latest.json`
- `state/rtds_price_to_beat_state.json`
- `state/rtds_resolution_state.json`

### ClickHouse only

```env
RTDS_SINK=clickhouse
RTDS_WRITE_LATEST_FILE=false
RTDS_PERSIST_STATE_TO_FILE=false

CLICKHOUSE_URL=http://localhost:8123
CLICKHOUSE_DATABASE=polybot
CLICKHOUSE_USER=default
CLICKHOUSE_PASSWORD=
CLICKHOUSE_TABLE_RTDS_PRICES=rtds_prices
CLICKHOUSE_TABLE_RTDS_PRICE_TO_BEAT=rtds_price_to_beat_state
CLICKHOUSE_TABLE_RTDS_RESOLUTION_STATE=rtds_resolution_state
RTDS_CLICKHOUSE_AUTO_CREATE_SCHEMA=true
RTDS_CLICKHOUSE_TIMEOUT_SECONDS=2
RTDS_CLICKHOUSE_ERROR_LOG_EVERY_MS=5000
RTDS_CLICKHOUSE_BATCH_MAX_ROWS=200
RTDS_CLICKHOUSE_BATCH_MAX_DELAY_MS=250
```

### Both file + ClickHouse

```env
RTDS_SINK=both
RTDS_WRITE_LATEST_FILE=true
RTDS_PERSIST_STATE_TO_FILE=true
```

Use the same ClickHouse variables above.

## Copy Collect RTDS Toggle

`copy_collect` now supports disabling RTDS price data ingestion/enrichment:

```env
COPY_COLLECT_INCLUDE_RTDS_DATA=false
```

When disabled:

- no RTDS price topic subscriptions are added
- `rtds_price` fields in `copy_trade` rows become `null`
- CLOB join enrichment still works if `COPY_COLLECT_CLOB_JOIN_ENABLED=true`

## Docker Compose

Add ClickHouse service to `docker-compose.yml`:

```yaml
  clickhouse:
    container_name: polybot-clickhouse
    image: clickhouse/clickhouse-server:24.12
    ports:
      - "8123:8123"
      - "9000:9000"
    volumes:
      - ./data/clickhouse:/var/lib/clickhouse
    environment:
      TZ: Asia/Jakarta
    restart: unless-stopped
```

Start stack:

```bash
docker compose up -d postgres clickhouse polybot-rust
docker compose logs -f polybot-rust
```

## Verification

ClickHouse rows:

```bash
docker exec -it polybot-clickhouse clickhouse-client --query "SELECT count() FROM polybot.rtds_prices"
```

Latest local tick:

```bash
tail -n 1 state/rtds_prices.jsonl
```

## Troubleshooting (Critical)

- `Missing MARKET_SLUG`: set `MARKET_SLUG` or provide `MARKET_SYMBOL` (or `RTDS_SYMBOL`) with `MARKET_SEGMENT`; signal-follow also works if configured.
- `Missing POLYMARKET_PRIVATE_KEY` / `Missing POLYMARKET_FUNDER`: these are required by startup checks even in dry run.
- `DB Init Error`: fix `DB_URL` / Postgres availability.
- No sink output: verify `RTDS_SINK` and sink-specific vars.
