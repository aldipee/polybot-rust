# Collection Mode

This document explains how to run the bot as a data collector with RTDS (+ CLOB join), without placing live orders.

## Critical Runtime Variables

These are the non-optional items for stable collection mode in this codebase:

- `DRY_RUN=true`
- `RTDS_ENABLED=true`
- `MARKET_SLUG=<current market slug>` (for example `btc-updown-5m-1771642500`)
- `MARKET_SEGMENT=<5M|15M|1H|4H|1D>`
- `EXEC_MODE=SNIPER` or `EXEC_MODE=MAKER` (do not use SIGNAL mode unless SIGNAL WS is configured)
- `POLYMARKET_PRIVATE_KEY=<required by startup checks>`
- `POLYMARKET_FUNDER=<required by startup checks>`
- `SIGNATURE_TYPE=1` (standard Polymarket setup)
- `DB_URL=<postgres url>` (startup initializes DB schema even in dry run)

If `MARKET_SLUG` is empty, startup fails unless using signal-follow mode (`SIGNAL_FOLLOW_SLUG=true` with signal WS configured).

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

- `Missing MARKET_SLUG`: set `MARKET_SLUG` or configure signal-follow correctly.
- `Missing POLYMARKET_PRIVATE_KEY` / `Missing POLYMARKET_FUNDER`: these are required by startup checks even in dry run.
- `DB Init Error`: fix `DB_URL` / Postgres availability.
- No sink output: verify `RTDS_SINK` and sink-specific vars.
