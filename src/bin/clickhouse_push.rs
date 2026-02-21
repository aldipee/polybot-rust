use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[derive(Debug, Clone)]
struct PushConfig {
    url: String,
    user: String,
    password: String,
    database: String,
    table_rtds_prices: String,
    table_copy_collect: String,
    table_price_to_beat: String,
    table_resolution_state: String,
    path_rtds_prices: PathBuf,
    path_copy_collect: PathBuf,
    path_price_to_beat: PathBuf,
    path_resolution_state: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
struct RtdsPricesRow {
    row_hash: String,
    market_slug: String,
    symbol: String,
    asset_id: String,
    kind: String,
    timestamp_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    timestamp_ms_utc: Option<DateTime<Utc>>,
    received_at_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    received_at_ms_utc: Option<DateTime<Utc>>,
    price: Option<f64>,
    value: Option<f64>,
    price_to_beat: Option<f64>,
    diff_vs_price_to_beat: Option<f64>,
    diff_vs_price_to_beat_percentage: Option<f64>,
    clob_join_target_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_join_target_ts_ms_utc: Option<DateTime<Utc>>,
    clob_up_asset_id: String,
    clob_up_best_bid_price: Option<f64>,
    clob_up_best_ask_price: Option<f64>,
    clob_up_mid_price: Option<f64>,
    clob_up_spread: Option<f64>,
    clob_up_exchange_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_up_exchange_ts_ms_utc: Option<DateTime<Utc>>,
    clob_up_recv_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_up_recv_ts_ms_utc: Option<DateTime<Utc>>,
    clob_down_asset_id: String,
    clob_down_best_bid_price: Option<f64>,
    clob_down_best_ask_price: Option<f64>,
    clob_down_mid_price: Option<f64>,
    clob_down_spread: Option<f64>,
    clob_down_exchange_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_down_exchange_ts_ms_utc: Option<DateTime<Utc>>,
    clob_down_recv_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_down_recv_ts_ms_utc: Option<DateTime<Utc>>,
    row_json: String,
    ingested_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
struct CopyCollectRow {
    row_hash: String,
    kind: String,
    collector_ts_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    collector_ts_ms_utc: Option<DateTime<Utc>>,
    wallet: String,
    market_slug: String,
    event_slug: String,
    symbol: String,
    direction: String,
    side: String,
    outcome: String,
    trade_ts_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    trade_ts_ms_utc: Option<DateTime<Utc>>,
    trade_price: Option<f64>,
    trade_size: Option<f64>,
    rtds_price: Option<f64>,
    rtds_price_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    rtds_price_ts_ms_utc: Option<DateTime<Utc>>,
    clob_join_target_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_join_target_ts_ms_utc: Option<DateTime<Utc>>,
    clob_up_asset_id: String,
    clob_up_best_bid_price: Option<f64>,
    clob_up_best_ask_price: Option<f64>,
    clob_up_mid_price: Option<f64>,
    clob_up_spread: Option<f64>,
    clob_up_exchange_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_up_exchange_ts_ms_utc: Option<DateTime<Utc>>,
    clob_up_recv_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_up_recv_ts_ms_utc: Option<DateTime<Utc>>,
    clob_down_asset_id: String,
    clob_down_best_bid_price: Option<f64>,
    clob_down_best_ask_price: Option<f64>,
    clob_down_mid_price: Option<f64>,
    clob_down_spread: Option<f64>,
    clob_down_exchange_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_down_exchange_ts_ms_utc: Option<DateTime<Utc>>,
    clob_down_recv_ts_ms: Option<i64>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    clob_down_recv_ts_ms_utc: Option<DateTime<Utc>>,
    row_json: String,
    ingested_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
struct PriceToBeatRow {
    row_hash: String,
    market_slug: String,
    price_to_beat: Option<f64>,
    updated_at_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    updated_at_ms_utc: Option<DateTime<Utc>>,
    row_json: String,
    ingested_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
struct ResolutionStateRow {
    row_hash: String,
    state_version: i64,
    state_updated_at_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    state_updated_at_ms_utc: Option<DateTime<Utc>>,
    market_slug: String,
    symbol: String,
    asset_id: String,
    resolution_ts_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    resolution_ts_ms_utc: Option<DateTime<Utc>>,
    source_ts_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    source_ts_ms_utc: Option<DateTime<Utc>>,
    resolution_price: Option<f64>,
    resolution_value: Option<f64>,
    capture_mode: String,
    price_to_beat: Option<f64>,
    diff_vs_price_to_beat: Option<f64>,
    diff_vs_price_to_beat_percentage: Option<f64>,
    captured_at_ms: i64,
    #[serde(with = "clickhouse::serde::chrono::datetime64::millis::option")]
    captured_at_ms_utc: Option<DateTime<Utc>>,
    row_json: String,
    ingested_at_ms: i64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn val_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn val_f64(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn val_i64(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .filter(|x| *x != 0),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok().filter(|x| *x != 0),
        _ => None,
    }
}

fn epoch_ms_to_utc(ms: i64) -> Option<DateTime<Utc>> {
    if ms == 0 {
        None
    } else {
        DateTime::<Utc>::from_timestamp_millis(ms)
    }
}

fn parse_utc_datetime(raw: &str) -> Option<DateTime<Utc>> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(ms) = t.parse::<i64>() {
        return epoch_ms_to_utc(ms);
    }
    if let Ok(secs) = t.parse::<f64>() {
        if secs.abs() >= 1_000_000_000_000.0 {
            return epoch_ms_to_utc(secs as i64);
        }
        return DateTime::<Utc>::from_timestamp_millis((secs * 1000.0) as i64);
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));
    }
    None
}

fn val_opt_datetime_utc(v: Option<&Value>) -> Option<DateTime<Utc>> {
    match v {
        Some(Value::String(s)) => parse_utc_datetime(s),
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                epoch_ms_to_utc(i)
            } else {
                n.as_f64()
                    .and_then(|f| DateTime::<Utc>::from_timestamp_millis((f * 1000.0) as i64))
            }
        }
        _ => None,
    }
}

fn row_hash_hex(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn validate_ident(raw: &str, key: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(anyhow!("{key} cannot be empty"));
    }
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        Ok(name.to_string())
    } else {
        Err(anyhow!(
            "{key} must contain only [A-Za-z0-9_], got '{name}'"
        ))
    }
}

fn quote_ident(raw: &str) -> String {
    format!("`{raw}`")
}

fn config_from_env() -> Result<PushConfig> {
    let url = env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string());
    let user = env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "default".to_string());
    let password = env::var("CLICKHOUSE_PASSWORD").unwrap_or_default();
    let database = validate_ident(
        &env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "polybot".to_string()),
        "CLICKHOUSE_DATABASE",
    )?;
    let table_rtds_prices = validate_ident(
        &env::var("CLICKHOUSE_TABLE_RTDS_PRICES").unwrap_or_else(|_| "rtds_prices".to_string()),
        "CLICKHOUSE_TABLE_RTDS_PRICES",
    )?;
    let table_copy_collect = validate_ident(
        &env::var("CLICKHOUSE_TABLE_COPY_COLLECT")
            .unwrap_or_else(|_| "copy_collect".to_string()),
        "CLICKHOUSE_TABLE_COPY_COLLECT",
    )?;
    let table_price_to_beat = validate_ident(
        &env::var("CLICKHOUSE_TABLE_RTDS_PRICE_TO_BEAT")
            .unwrap_or_else(|_| "rtds_price_to_beat_state".to_string()),
        "CLICKHOUSE_TABLE_RTDS_PRICE_TO_BEAT",
    )?;
    let table_resolution_state = validate_ident(
        &env::var("CLICKHOUSE_TABLE_RTDS_RESOLUTION_STATE")
            .unwrap_or_else(|_| "rtds_resolution_state".to_string()),
        "CLICKHOUSE_TABLE_RTDS_RESOLUTION_STATE",
    )?;

    Ok(PushConfig {
        url,
        user,
        password,
        database,
        table_rtds_prices,
        table_copy_collect,
        table_price_to_beat,
        table_resolution_state,
        path_rtds_prices: PathBuf::from(
            env::var("CLICKHOUSE_RTDS_PRICES_PATH")
                .unwrap_or_else(|_| "state/rtds_prices.jsonl".to_string()),
        ),
        path_copy_collect: PathBuf::from(
            env::var("CLICKHOUSE_COPY_COLLECT_PATH")
                .unwrap_or_else(|_| "state/copy_collect.jsonl".to_string()),
        ),
        path_price_to_beat: PathBuf::from(
            env::var("CLICKHOUSE_RTDS_PRICE_TO_BEAT_PATH")
                .unwrap_or_else(|_| "state/rtds_price_to_beat_state.json".to_string()),
        ),
        path_resolution_state: PathBuf::from(
            env::var("CLICKHOUSE_RTDS_RESOLUTION_STATE_PATH")
                .unwrap_or_else(|_| "state/rtds_resolution_state.json".to_string()),
        ),
    })
}

fn make_client(url: &str, user: &str, password: &str, database: &str) -> Client {
    Client::default()
        .with_url(url)
        .with_user(user)
        .with_password(password)
        .with_database(database)
}

async fn create_schema(root_client: &Client, db_client: &Client, cfg: &PushConfig) -> Result<()> {
    let db = quote_ident(&cfg.database);
    root_client
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .context("failed creating ClickHouse database")?;

    let t_rtds_prices = quote_ident(&cfg.table_rtds_prices);
    db_client
        .query(&format!(
            "CREATE TABLE IF NOT EXISTS {t_rtds_prices} (
                row_hash String,
                market_slug String,
                symbol String,
                asset_id String,
                kind String,
                timestamp_ms Int64,
                timestamp_ms_utc Nullable(DateTime64(3, 'UTC')),
                received_at_ms Int64,
                received_at_ms_utc Nullable(DateTime64(3, 'UTC')),
                price Nullable(Float64),
                value Nullable(Float64),
                price_to_beat Nullable(Float64),
                diff_vs_price_to_beat Nullable(Float64),
                diff_vs_price_to_beat_percentage Nullable(Float64),
                clob_join_target_ts_ms Nullable(Int64),
                clob_join_target_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_up_asset_id String,
                clob_up_best_bid_price Nullable(Float64),
                clob_up_best_ask_price Nullable(Float64),
                clob_up_mid_price Nullable(Float64),
                clob_up_spread Nullable(Float64),
                clob_up_exchange_ts_ms Nullable(Int64),
                clob_up_exchange_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_up_recv_ts_ms Nullable(Int64),
                clob_up_recv_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_down_asset_id String,
                clob_down_best_bid_price Nullable(Float64),
                clob_down_best_ask_price Nullable(Float64),
                clob_down_mid_price Nullable(Float64),
                clob_down_spread Nullable(Float64),
                clob_down_exchange_ts_ms Nullable(Int64),
                clob_down_exchange_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_down_recv_ts_ms Nullable(Int64),
                clob_down_recv_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                row_json String,
                ingested_at_ms Int64
            ) ENGINE = MergeTree
            ORDER BY (market_slug, timestamp_ms, row_hash)"
        ))
        .execute()
        .await
        .context("failed creating table for rtds_prices")?;

    let t_copy_collect = quote_ident(&cfg.table_copy_collect);
    db_client
        .query(&format!(
            "CREATE TABLE IF NOT EXISTS {t_copy_collect} (
                row_hash String,
                kind String,
                collector_ts_ms Int64,
                collector_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                wallet String,
                market_slug String,
                event_slug String,
                symbol String,
                direction String,
                side String,
                outcome String,
                trade_ts_ms Int64,
                trade_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                trade_price Nullable(Float64),
                trade_size Nullable(Float64),
                rtds_price Nullable(Float64),
                rtds_price_ts_ms Nullable(Int64),
                rtds_price_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_join_target_ts_ms Nullable(Int64),
                clob_join_target_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_up_asset_id String,
                clob_up_best_bid_price Nullable(Float64),
                clob_up_best_ask_price Nullable(Float64),
                clob_up_mid_price Nullable(Float64),
                clob_up_spread Nullable(Float64),
                clob_up_exchange_ts_ms Nullable(Int64),
                clob_up_exchange_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_up_recv_ts_ms Nullable(Int64),
                clob_up_recv_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_down_asset_id String,
                clob_down_best_bid_price Nullable(Float64),
                clob_down_best_ask_price Nullable(Float64),
                clob_down_mid_price Nullable(Float64),
                clob_down_spread Nullable(Float64),
                clob_down_exchange_ts_ms Nullable(Int64),
                clob_down_exchange_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                clob_down_recv_ts_ms Nullable(Int64),
                clob_down_recv_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                row_json String,
                ingested_at_ms Int64
            ) ENGINE = MergeTree
            ORDER BY (market_slug, collector_ts_ms, row_hash)"
        ))
        .execute()
        .await
        .context("failed creating table for copy_collect")?;

    let t_price_to_beat = quote_ident(&cfg.table_price_to_beat);
    db_client
        .query(&format!(
            "CREATE TABLE IF NOT EXISTS {t_price_to_beat} (
                row_hash String,
                market_slug String,
                price_to_beat Nullable(Float64),
                updated_at_ms Int64,
                updated_at_ms_utc Nullable(DateTime64(3, 'UTC')),
                row_json String,
                ingested_at_ms Int64
            ) ENGINE = ReplacingMergeTree(updated_at_ms)
            ORDER BY (market_slug, row_hash)"
        ))
        .execute()
        .await
        .context("failed creating table for price_to_beat state")?;

    let t_resolution_state = quote_ident(&cfg.table_resolution_state);
    db_client
        .query(&format!(
            "CREATE TABLE IF NOT EXISTS {t_resolution_state} (
                row_hash String,
                state_version Int64,
                state_updated_at_ms Int64,
                state_updated_at_ms_utc Nullable(DateTime64(3, 'UTC')),
                market_slug String,
                symbol String,
                asset_id String,
                resolution_ts_ms Int64,
                resolution_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                source_ts_ms Int64,
                source_ts_ms_utc Nullable(DateTime64(3, 'UTC')),
                resolution_price Nullable(Float64),
                resolution_value Nullable(Float64),
                capture_mode String,
                price_to_beat Nullable(Float64),
                diff_vs_price_to_beat Nullable(Float64),
                diff_vs_price_to_beat_percentage Nullable(Float64),
                captured_at_ms Int64,
                captured_at_ms_utc Nullable(DateTime64(3, 'UTC')),
                row_json String,
                ingested_at_ms Int64
            ) ENGINE = MergeTree
            ORDER BY (market_slug, symbol, resolution_ts_ms, row_hash)"
        ))
        .execute()
        .await
        .context("failed creating table for resolution state")?;

    Ok(())
}

fn open_reader(path: &PathBuf) -> Result<Option<BufReader<File>>> {
    if !path.exists() {
        println!("skip missing file: {}", path.display());
        return Ok(None);
    }
    let file = File::open(path).with_context(|| format!("failed opening {}", path.display()))?;
    Ok(Some(BufReader::new(file)))
}

async fn ingest_rtds_prices(client: &Client, cfg: &PushConfig) -> Result<usize> {
    let mut reader = match open_reader(&cfg.path_rtds_prices)? {
        Some(r) => r,
        None => return Ok(0),
    };
    let mut insert = client.insert(cfg.table_rtds_prices.as_str())?;
    let mut count = 0usize;
    let ingested_at_ms = now_ms();

    let mut line = String::new();
    let mut line_no = 0usize;
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .with_context(|| format!("failed reading {}", cfg.path_rtds_prices.display()))?;
        if n == 0 {
            break;
        }
        line_no += 1;
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let obj: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "skip invalid json line {} in {}: {e}",
                    line_no,
                    cfg.path_rtds_prices.display()
                );
                continue;
            }
        };
        let timestamp_ms = val_i64(obj.get("timestamp_ms")).unwrap_or(0);
        let received_at_ms = val_i64(obj.get("received_at_ms")).unwrap_or(0);
        let clob_join_target_ts_ms = val_i64(obj.get("clob_join_target_ts_ms"));
        let clob_up_exchange_ts_ms = val_i64(obj.get("clob_up_exchange_ts_ms"));
        let clob_up_recv_ts_ms = val_i64(obj.get("clob_up_recv_ts_ms"));
        let clob_down_exchange_ts_ms = val_i64(obj.get("clob_down_exchange_ts_ms"));
        let clob_down_recv_ts_ms = val_i64(obj.get("clob_down_recv_ts_ms"));
        let row = RtdsPricesRow {
            row_hash: row_hash_hex(raw),
            market_slug: val_str(obj.get("market_slug")),
            symbol: val_str(obj.get("symbol")),
            asset_id: val_str(obj.get("asset_id")),
            kind: val_str(obj.get("kind")),
            timestamp_ms,
            timestamp_ms_utc: val_opt_datetime_utc(obj.get("timestamp_ms_utc"))
                .or_else(|| epoch_ms_to_utc(timestamp_ms)),
            received_at_ms,
            received_at_ms_utc: val_opt_datetime_utc(obj.get("received_at_ms_utc"))
                .or_else(|| epoch_ms_to_utc(received_at_ms)),
            price: val_f64(obj.get("price")),
            value: val_f64(obj.get("value")),
            price_to_beat: val_f64(obj.get("price_to_beat")),
            diff_vs_price_to_beat: val_f64(obj.get("diff_vs_price_to_beat")),
            diff_vs_price_to_beat_percentage: val_f64(obj.get("diff_vs_price_to_beat_percentage")),
            clob_join_target_ts_ms,
            clob_join_target_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_join_target_ts_ms_utc"))
                .or_else(|| clob_join_target_ts_ms.and_then(epoch_ms_to_utc)),
            clob_up_asset_id: val_str(obj.get("clob_up_asset_id")),
            clob_up_best_bid_price: val_f64(obj.get("clob_up_best_bid_price")),
            clob_up_best_ask_price: val_f64(obj.get("clob_up_best_ask_price")),
            clob_up_mid_price: val_f64(obj.get("clob_up_mid_price")),
            clob_up_spread: val_f64(obj.get("clob_up_spread")),
            clob_up_exchange_ts_ms,
            clob_up_exchange_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_up_exchange_ts_ms_utc"))
                .or_else(|| clob_up_exchange_ts_ms.and_then(epoch_ms_to_utc)),
            clob_up_recv_ts_ms,
            clob_up_recv_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_up_recv_ts_ms_utc"))
                .or_else(|| clob_up_recv_ts_ms.and_then(epoch_ms_to_utc)),
            clob_down_asset_id: val_str(obj.get("clob_down_asset_id")),
            clob_down_best_bid_price: val_f64(obj.get("clob_down_best_bid_price")),
            clob_down_best_ask_price: val_f64(obj.get("clob_down_best_ask_price")),
            clob_down_mid_price: val_f64(obj.get("clob_down_mid_price")),
            clob_down_spread: val_f64(obj.get("clob_down_spread")),
            clob_down_exchange_ts_ms,
            clob_down_exchange_ts_ms_utc: val_opt_datetime_utc(
                obj.get("clob_down_exchange_ts_ms_utc"),
            )
            .or_else(|| clob_down_exchange_ts_ms.and_then(epoch_ms_to_utc)),
            clob_down_recv_ts_ms,
            clob_down_recv_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_down_recv_ts_ms_utc"))
                .or_else(|| clob_down_recv_ts_ms.and_then(epoch_ms_to_utc)),
            row_json: raw.to_string(),
            ingested_at_ms,
        };
        insert.write(&row).await?;
        count += 1;
    }
    insert.end().await?;
    Ok(count)
}

async fn ingest_copy_collect(client: &Client, cfg: &PushConfig) -> Result<usize> {
    let mut reader = match open_reader(&cfg.path_copy_collect)? {
        Some(r) => r,
        None => return Ok(0),
    };
    let mut insert = client.insert(cfg.table_copy_collect.as_str())?;
    let mut count = 0usize;
    let ingested_at_ms = now_ms();

    let mut line = String::new();
    let mut line_no = 0usize;
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .with_context(|| format!("failed reading {}", cfg.path_copy_collect.display()))?;
        if n == 0 {
            break;
        }
        line_no += 1;
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let obj: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "skip invalid json line {} in {}: {e}",
                    line_no,
                    cfg.path_copy_collect.display()
                );
                continue;
            }
        };
        let trade_ts_ms = val_i64(obj.get("trade_ts_ms")).unwrap_or_else(|| {
            val_i64(obj.get("trade_ts_sec"))
                .map(|x| x * 1000)
                .unwrap_or(0)
        });
        let collector_ts_ms = val_i64(obj.get("collector_ts_ms")).unwrap_or(0);
        let rtds_price_ts_ms = val_i64(obj.get("rtds_price_ts_ms"));
        let clob_join_target_ts_ms = val_i64(obj.get("clob_join_target_ts_ms"));
        let clob_up_exchange_ts_ms = val_i64(obj.get("clob_up_exchange_ts_ms"));
        let clob_up_recv_ts_ms = val_i64(obj.get("clob_up_recv_ts_ms"));
        let clob_down_exchange_ts_ms = val_i64(obj.get("clob_down_exchange_ts_ms"));
        let clob_down_recv_ts_ms = val_i64(obj.get("clob_down_recv_ts_ms"));
        let row = CopyCollectRow {
            row_hash: row_hash_hex(raw),
            kind: val_str(obj.get("kind")),
            collector_ts_ms,
            collector_ts_ms_utc: val_opt_datetime_utc(obj.get("collector_ts_ms_utc"))
                .or_else(|| epoch_ms_to_utc(collector_ts_ms)),
            wallet: val_str(obj.get("wallet")),
            market_slug: val_str(obj.get("market_slug")),
            event_slug: val_str(obj.get("event_slug")),
            symbol: val_str(obj.get("symbol")),
            direction: val_str(obj.get("direction")),
            side: val_str(obj.get("side")),
            outcome: val_str(obj.get("outcome")),
            trade_ts_ms,
            trade_ts_ms_utc: val_opt_datetime_utc(obj.get("trade_ts_ms_utc"))
                .or_else(|| epoch_ms_to_utc(trade_ts_ms)),
            trade_price: val_f64(obj.get("trade_price")),
            trade_size: val_f64(obj.get("trade_size")),
            rtds_price: val_f64(obj.get("rtds_price")),
            rtds_price_ts_ms,
            rtds_price_ts_ms_utc: val_opt_datetime_utc(obj.get("rtds_price_ts_ms_utc"))
                .or_else(|| rtds_price_ts_ms.and_then(epoch_ms_to_utc)),
            clob_join_target_ts_ms,
            clob_join_target_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_join_target_ts_ms_utc"))
                .or_else(|| clob_join_target_ts_ms.and_then(epoch_ms_to_utc)),
            clob_up_asset_id: val_str(obj.get("clob_up_asset_id")),
            clob_up_best_bid_price: val_f64(obj.get("clob_up_best_bid_price")),
            clob_up_best_ask_price: val_f64(obj.get("clob_up_best_ask_price")),
            clob_up_mid_price: val_f64(obj.get("clob_up_mid_price")),
            clob_up_spread: val_f64(obj.get("clob_up_spread")),
            clob_up_exchange_ts_ms,
            clob_up_exchange_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_up_exchange_ts_ms_utc"))
                .or_else(|| clob_up_exchange_ts_ms.and_then(epoch_ms_to_utc)),
            clob_up_recv_ts_ms,
            clob_up_recv_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_up_recv_ts_ms_utc"))
                .or_else(|| clob_up_recv_ts_ms.and_then(epoch_ms_to_utc)),
            clob_down_asset_id: val_str(obj.get("clob_down_asset_id")),
            clob_down_best_bid_price: val_f64(obj.get("clob_down_best_bid_price")),
            clob_down_best_ask_price: val_f64(obj.get("clob_down_best_ask_price")),
            clob_down_mid_price: val_f64(obj.get("clob_down_mid_price")),
            clob_down_spread: val_f64(obj.get("clob_down_spread")),
            clob_down_exchange_ts_ms,
            clob_down_exchange_ts_ms_utc: val_opt_datetime_utc(
                obj.get("clob_down_exchange_ts_ms_utc"),
            )
            .or_else(|| clob_down_exchange_ts_ms.and_then(epoch_ms_to_utc)),
            clob_down_recv_ts_ms,
            clob_down_recv_ts_ms_utc: val_opt_datetime_utc(obj.get("clob_down_recv_ts_ms_utc"))
                .or_else(|| clob_down_recv_ts_ms.and_then(epoch_ms_to_utc)),
            row_json: raw.to_string(),
            ingested_at_ms,
        };
        insert.write(&row).await?;
        count += 1;
    }
    insert.end().await?;
    Ok(count)
}

async fn ingest_price_to_beat(client: &Client, cfg: &PushConfig) -> Result<usize> {
    if !cfg.path_price_to_beat.exists() {
        println!("skip missing file: {}", cfg.path_price_to_beat.display());
        return Ok(0);
    }
    let raw = std::fs::read_to_string(&cfg.path_price_to_beat)
        .with_context(|| format!("failed reading {}", cfg.path_price_to_beat.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let obj: Value = serde_json::from_str(trimmed)
        .with_context(|| format!("invalid json in {}", cfg.path_price_to_beat.display()))?;
    let updated_at_ms = val_i64(obj.get("updated_at_ms")).unwrap_or(0);
    let row = PriceToBeatRow {
        row_hash: row_hash_hex(trimmed),
        market_slug: val_str(obj.get("market_slug")),
        price_to_beat: val_f64(obj.get("price_to_beat")),
        updated_at_ms,
        updated_at_ms_utc: val_opt_datetime_utc(obj.get("updated_at_ms_utc"))
            .or_else(|| epoch_ms_to_utc(updated_at_ms)),
        row_json: trimmed.to_string(),
        ingested_at_ms: now_ms(),
    };
    let mut insert = client.insert(cfg.table_price_to_beat.as_str())?;
    insert.write(&row).await?;
    insert.end().await?;
    Ok(1)
}

async fn ingest_resolution_state(client: &Client, cfg: &PushConfig) -> Result<usize> {
    if !cfg.path_resolution_state.exists() {
        println!("skip missing file: {}", cfg.path_resolution_state.display());
        return Ok(0);
    }
    let raw = std::fs::read_to_string(&cfg.path_resolution_state)
        .with_context(|| format!("failed reading {}", cfg.path_resolution_state.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let root: Value = serde_json::from_str(trimmed)
        .with_context(|| format!("invalid json in {}", cfg.path_resolution_state.display()))?;

    let state_version = val_i64(root.get("version")).unwrap_or(0);
    let state_updated_at_ms = val_i64(root.get("updated_at_ms")).unwrap_or(0);
    let mut insert = client.insert(cfg.table_resolution_state.as_str())?;
    let mut count = 0usize;
    let ingested_at_ms = now_ms();

    if let Some(records) = root.get("records").and_then(|v| v.as_array()) {
        for rec in records {
            let rec_raw = serde_json::to_string(rec).unwrap_or_else(|_| "{}".to_string());
            let resolution_ts_ms = val_i64(rec.get("resolution_ts_ms")).unwrap_or(0);
            let source_ts_ms = val_i64(rec.get("source_ts_ms")).unwrap_or(0);
            let captured_at_ms = val_i64(rec.get("captured_at_ms")).unwrap_or(0);
            let row = ResolutionStateRow {
                row_hash: row_hash_hex(&rec_raw),
                state_version,
                state_updated_at_ms,
                state_updated_at_ms_utc: val_opt_datetime_utc(rec.get("state_updated_at_ms_utc"))
                    .or_else(|| val_opt_datetime_utc(root.get("updated_at_ms_utc")))
                    .or_else(|| epoch_ms_to_utc(state_updated_at_ms)),
                market_slug: val_str(rec.get("market_slug")),
                symbol: val_str(rec.get("symbol")),
                asset_id: val_str(rec.get("asset_id")),
                resolution_ts_ms,
                resolution_ts_ms_utc: val_opt_datetime_utc(rec.get("resolution_ts_ms_utc"))
                    .or_else(|| epoch_ms_to_utc(resolution_ts_ms)),
                source_ts_ms,
                source_ts_ms_utc: val_opt_datetime_utc(rec.get("source_ts_ms_utc"))
                    .or_else(|| epoch_ms_to_utc(source_ts_ms)),
                resolution_price: val_f64(rec.get("resolution_price")),
                resolution_value: val_f64(rec.get("resolution_value")),
                capture_mode: val_str(rec.get("capture_mode")),
                price_to_beat: val_f64(rec.get("price_to_beat")),
                diff_vs_price_to_beat: val_f64(rec.get("diff_vs_price_to_beat")),
                diff_vs_price_to_beat_percentage: val_f64(
                    rec.get("diff_vs_price_to_beat_percentage"),
                ),
                captured_at_ms,
                captured_at_ms_utc: val_opt_datetime_utc(rec.get("captured_at_ms_utc"))
                    .or_else(|| epoch_ms_to_utc(captured_at_ms)),
                row_json: rec_raw,
                ingested_at_ms,
            };
            insert.write(&row).await?;
            count += 1;
        }
    }
    insert.end().await?;
    Ok(count)
}

async fn run() -> Result<()> {
    install_rustls_crypto_provider();
    let _ = dotenvy::dotenv();
    let cfg = config_from_env()?;

    let root_client = make_client(&cfg.url, &cfg.user, &cfg.password, "default");
    let db_client = make_client(&cfg.url, &cfg.user, &cfg.password, &cfg.database);
    create_schema(&root_client, &db_client, &cfg).await?;

    let n_rtds_prices = ingest_rtds_prices(&db_client, &cfg).await?;
    let n_copy_collect = ingest_copy_collect(&db_client, &cfg).await?;
    let n_price_to_beat = ingest_price_to_beat(&db_client, &cfg).await?;
    let n_resolution = ingest_resolution_state(&db_client, &cfg).await?;

    println!(
        "ClickHouse ingest done. db={} rtds_prices={} copy_collect={} price_to_beat={} resolution_records={}",
        cfg.database, n_rtds_prices, n_copy_collect, n_price_to_beat, n_resolution
    );

    Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("fatal: {e:#}");
        std::process::exit(1);
    }
}
