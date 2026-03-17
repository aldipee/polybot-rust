use crate::config::BotConfig;
use crate::env_utils::{env_bool, env_float, env_int};
use crate::gamma::{fetch_market_by_slug, parse_tokens_and_condition};
use crate::helpers::iso_to_epoch;
use crate::logging::LogLike;
use crate::latency_log::JsonlFileService;
use anyhow::{anyhow, Result};
use chainlink_data_streams_report::feed_id::ID as ChainlinkFeedId;
use chainlink_data_streams_report::report::{
    decode_full_report as chainlink_decode_full_report, v1::ReportDataV1 as ChainlinkReportDataV1,
    v10::ReportDataV10 as ChainlinkReportDataV10, v11::ReportDataV11 as ChainlinkReportDataV11,
    v12::ReportDataV12 as ChainlinkReportDataV12, v13::ReportDataV13 as ChainlinkReportDataV13,
    v2::ReportDataV2 as ChainlinkReportDataV2, v3::ReportDataV3 as ChainlinkReportDataV3,
    v4::ReportDataV4 as ChainlinkReportDataV4, v5::ReportDataV5 as ChainlinkReportDataV5,
    v6::ReportDataV6 as ChainlinkReportDataV6, v7::ReportDataV7 as ChainlinkReportDataV7,
    v8::ReportDataV8 as ChainlinkReportDataV8, v9::ReportDataV9 as ChainlinkReportDataV9,
    Report as ChainlinkReport,
};
use chainlink_data_streams_sdk::config::{
    Config as ChainlinkConfig, InsecureSkipVerify as ChainlinkInsecureSkipVerify,
    WebSocketHighAvailability as ChainlinkWebSocketHighAvailability,
};
use chainlink_data_streams_sdk::stream::{
    Stream as ChainlinkStream, WebSocketReport as ChainlinkWebSocketReport,
};
use chrono::{TimeZone, Utc};
use rand::Rng;
use reqwest::blocking::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn row_hash_hex(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn val_as_f64(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn val_as_i64_ms(v: Option<&Value>) -> Option<i64> {
    val_as_i64_ms_with_precision(v).0
}

fn val_as_i64_ms_with_precision(v: Option<&Value>) -> (Option<i64>, &'static str) {
    match v {
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                if i > 1_000_000_000_000 {
                    (Some(i), "ms")
                } else if i > 1_000_000_000 {
                    (Some(i * 1000), "s")
                } else {
                    (None, "none")
                }
            } else if let Some(f) = n.as_f64() {
                if f > 1_000_000_000_000.0 {
                    (Some(f as i64), "ms")
                } else if f > 1_000_000_000.0 {
                    (Some((f * 1000.0) as i64), "s")
                } else {
                    (None, "none")
                }
            } else {
                (None, "none")
            }
        }
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return (None, "none");
            }
            if let Ok(i) = t.parse::<i64>() {
                if i > 1_000_000_000_000 {
                    (Some(i), "ms")
                } else if i > 1_000_000_000 {
                    (Some(i * 1000), "s")
                } else {
                    (None, "none")
                }
            } else if let Ok(f) = t.parse::<f64>() {
                if f > 1_000_000_000_000.0 {
                    (Some(f as i64), "ms")
                } else if f > 1_000_000_000.0 {
                    (Some((f * 1000.0) as i64), "s")
                } else {
                    (None, "none")
                }
            } else {
                (None, "none")
            }
        }
        _ => (None, "none"),
    }
}

fn to_asset_id(s: &str) -> String {
    let mut out = s.trim().to_ascii_lowercase();
    if out.contains('/') {
        out = out.split('/').next().unwrap_or("").to_string();
    } else if out.contains('-') {
        out = out.split('-').next().unwrap_or("").to_string();
    } else if out.ends_with("usd") && out.len() > 3 {
        out = out[..out.len() - 3].to_string();
    }
    match out.as_str() {
        "bitcoin" => "btc".to_string(),
        "ethereum" => "eth".to_string(),
        "solana" => "sol".to_string(),
        "ripple" => "xrp".to_string(),
        "polygon" => "matic".to_string(),
        _ => out,
    }
}

fn to_symbol(s: &str) -> String {
    let aid = to_asset_id(s);
    if aid.is_empty() {
        String::new()
    } else {
        format!("{aid}/usd")
    }
}

fn infer_symbol_from_question(question: &str) -> Option<String> {
    let q = question.to_ascii_lowercase();
    let mapped = if q.contains("bitcoin") || q.contains("btc") {
        "btc"
    } else if q.contains("ethereum") || q.contains("eth") {
        "eth"
    } else if q.contains("solana") || q.contains("sol") {
        "sol"
    } else if q.contains("xrp") || q.contains("ripple") {
        "xrp"
    } else {
        ""
    };
    (!mapped.is_empty()).then(|| format!("{mapped}/usd"))
}

fn infer_symbol_from_resolution_source(source: &str) -> Option<String> {
    let src = source.to_ascii_lowercase();
    for hint in [
        "btc-usd",
        "eth-usd",
        "sol-usd",
        "xrp-usd",
        "doge-usd",
        "matic-usd",
    ] {
        if src.contains(hint) {
            return Some(hint.replace('-', "/"));
        }
    }
    None
}

fn infer_symbol_from_slug(slug: &str) -> Option<String> {
    let head = slug
        .split('-')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if head.is_empty() {
        return None;
    }
    let aid = to_asset_id(&head);
    if aid.is_empty() || !aid.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("{aid}/usd"))
}

fn parse_env_f64_opt(name: &str) -> Option<f64> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .and_then(|v| v.parse::<f64>().ok())
}

fn diff_percentage(price: f64, price_to_beat: Option<f64>) -> Option<f64> {
    let ptb = price_to_beat?;
    if ptb.abs() <= 1e-12 {
        return None;
    }
    Some(((price - ptb) / ptb) * 100.0)
}

fn val_as_i64(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|x| x as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn val_as_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn format_utc_ms(ms: i64) -> Option<String> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

fn format_clickhouse_utc_ms(ms: i64) -> Option<String> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
}

fn infer_unix_ms_for_key(key: &str, v: &Value) -> Option<i64> {
    if v.is_null() {
        return None;
    }
    let k = key.trim().to_ascii_lowercase();
    if k.is_empty() || k.ends_with("_utc") {
        return None;
    }

    let is_unix_ts_key = k.contains("timestamp")
        || k == "ts_ms"
        || k == "ts_sec"
        || k.ends_with("_ts_ms")
        || k.ends_with("_ts_sec")
        || k.ends_with("_at_ms")
        || k.ends_with("_at_sec");
    if !is_unix_ts_key {
        return None;
    }

    let raw = val_as_i64(Some(v))?;
    if k == "ts_sec"
        || k.ends_with("_ts_sec")
        || k.ends_with("_at_sec")
        || k.contains("timestamp_sec")
    {
        return Some(raw.saturating_mul(1000));
    }
    if k == "ts_ms" || k.ends_with("_ts_ms") || k.ends_with("_at_ms") || k.contains("timestamp_ms")
    {
        return Some(raw);
    }
    if raw > 1_000_000_000_000 {
        Some(raw)
    } else if raw > 1_000_000_000 {
        Some(raw.saturating_mul(1000))
    } else {
        None
    }
}

fn append_utc_timestamp_columns(row: &mut Value) {
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    let keys: Vec<String> = obj.keys().cloned().collect();
    for key in keys {
        let out_key = format!("{key}_utc");
        if obj.contains_key(&out_key) {
            continue;
        }
        let Some(v) = obj.get(&key) else {
            continue;
        };
        let Some(ms) = infer_unix_ms_for_key(&key, v) else {
            continue;
        };
        let Some(iso) = format_utc_ms(ms) else {
            continue;
        };
        obj.insert(out_key, Value::String(iso));
    }
}

fn validate_ident(raw: &str, key: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(anyhow!("{key} cannot be empty"));
    }
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
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

#[derive(Debug, Clone)]
struct RtdsSinkSelection {
    file: bool,
    clickhouse: bool,
    mode_label: String,
}

fn parse_rtds_sink_selection() -> RtdsSinkSelection {
    let raw = env::var("RTDS_SINK")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if raw.is_empty() {
        let file = env_bool("RTDS_LOG_TO_FILE", true);
        let clickhouse = env_bool("RTDS_CLICKHOUSE_ENABLED", false);
        return RtdsSinkSelection {
            file,
            clickhouse,
            mode_label: "legacy".to_string(),
        };
    }
    match raw.as_str() {
        "file" => RtdsSinkSelection {
            file: true,
            clickhouse: false,
            mode_label: raw,
        },
        "clickhouse" | "ch" => RtdsSinkSelection {
            file: false,
            clickhouse: true,
            mode_label: raw,
        },
        "both" | "all" => RtdsSinkSelection {
            file: true,
            clickhouse: true,
            mode_label: raw,
        },
        "none" | "off" => RtdsSinkSelection {
            file: false,
            clickhouse: false,
            mode_label: raw,
        },
        _ => {
            let file = env_bool("RTDS_LOG_TO_FILE", true);
            let clickhouse = env_bool("RTDS_CLICKHOUSE_ENABLED", false);
            RtdsSinkSelection {
                file,
                clickhouse,
                mode_label: format!("legacy_invalid:{raw}"),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RtdsProvider {
    Polymarket,
    Chainlink,
}

impl RtdsProvider {
    fn from_env() -> Self {
        let raw = env::var("RTDS_PROVIDER")
            .unwrap_or_else(|_| "POLYMARKET".to_string())
            .trim()
            .to_ascii_uppercase();
        match raw.as_str() {
            "CHAINLINK" | "CHAINLINK_WS" | "CHAINLINK_STREAM" => Self::Chainlink,
            _ => Self::Polymarket,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Polymarket => "POLYMARKET",
            Self::Chainlink => "CHAINLINK",
        }
    }
}

#[derive(Debug, Clone)]
struct ChainlinkRtdsConfig {
    feed_id: ChainlinkFeedId,
    feed_id_hex: String,
    price_decimals: u32,
    api_key: String,
    api_secret: String,
    rest_url: String,
    ws_url: String,
    ws_ha: bool,
    ws_max_reconnect: usize,
    insecure_skip_verify: bool,
    read_timeout: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PriceTick {
    symbol: String,
    asset_id: String,
    price: f64,
    value: Option<f64>,
    timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionSnapshot {
    pub market_slug: String,
    pub symbol: String,
    pub asset_id: String,
    pub resolution_ts_ms: i64,
    pub source_ts_ms: i64,
    pub resolution_price: f64,
    pub resolution_value: Option<f64>,
    pub capture_mode: String,
    pub price_to_beat: Option<f64>,
    pub diff_vs_price_to_beat: Option<f64>,
    pub diff_vs_price_to_beat_percentage: Option<f64>,
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ResolutionStateFile {
    version: i64,
    updated_at_ms: i64,
    records: Vec<ResolutionSnapshot>,
    last_by_symbol: HashMap<String, ResolutionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PriceToBeatStateFile {
    market_slug: String,
    price_to_beat: Option<f64>,
    updated_at_ms: i64,
}

#[derive(Debug, Default)]
struct ClickhouseBatchBuffer {
    rows: Vec<String>,
    first_enqueue_ms: i64,
}

#[derive(Debug, Default)]
struct ClickhouseBatchState {
    by_table: HashMap<String, ClickhouseBatchBuffer>,
}

struct RtdsClickhouseSink {
    client: HttpClient,
    url: String,
    user: String,
    password: String,
    database: String,
    table_rtds_prices: String,
    table_price_to_beat: String,
    table_resolution_state: String,
    logger: Arc<dyn LogLike>,
    error_log_every_ms: i64,
    last_error_log_ms: Mutex<i64>,
    batch_max_rows: usize,
    batch_max_delay_ms: i64,
    batch_state: Mutex<ClickhouseBatchState>,
}

impl RtdsClickhouseSink {
    fn from_env(logger: Arc<dyn LogLike>) -> Result<Self> {
        let url = env::var("CLICKHOUSE_URL")
            .unwrap_or_else(|_| "http://localhost:8123".to_string())
            .trim()
            .to_string();
        if url.is_empty() {
            return Err(anyhow!("CLICKHOUSE_URL is empty"));
        }

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
        let timeout_seconds = env_float("RTDS_CLICKHOUSE_TIMEOUT_SECONDS", 2.0).max(0.2);
        let error_log_every_ms = env_int("RTDS_CLICKHOUSE_ERROR_LOG_EVERY_MS", 5000).max(500);
        let batch_max_rows = env_int("RTDS_CLICKHOUSE_BATCH_MAX_ROWS", 200).max(1) as usize;
        let batch_max_delay_ms = env_int("RTDS_CLICKHOUSE_BATCH_MAX_DELAY_MS", 250).max(1) as i64;
        let client = HttpClient::builder()
            .timeout(Duration::from_secs_f64(timeout_seconds))
            .build()
            .map_err(|e| anyhow!("failed creating ClickHouse HTTP client: {e}"))?;

        Ok(Self {
            client,
            url,
            user,
            password,
            database,
            table_rtds_prices,
            table_price_to_beat,
            table_resolution_state,
            logger,
            error_log_every_ms: error_log_every_ms as i64,
            last_error_log_ms: Mutex::new(0),
            batch_max_rows,
            batch_max_delay_ms,
            batch_state: Mutex::new(ClickhouseBatchState::default()),
        })
    }

    fn preview_text(s: &str, max: usize) -> String {
        if s.len() <= max {
            s.to_string()
        } else {
            format!("{}...", &s[..max])
        }
    }

    fn execute_query(&self, query: &str, body: Option<String>, with_database: bool) -> Result<()> {
        let mut req = self.client.post(&self.url);
        if with_database {
            req = req.query(&[("database", self.database.as_str()), ("query", query)]);
        } else {
            req = req.query(&[("query", query)]);
        }
        if !self.user.trim().is_empty() {
            req = req.basic_auth(self.user.clone(), Some(self.password.clone()));
        }
        req = req.body(body.unwrap_or_default());
        let resp = req
            .send()
            .map_err(|e| anyhow!("ClickHouse request failed: {e}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if status.is_success() {
            return Ok(());
        }
        Err(anyhow!(
            "ClickHouse query failed status={} query='{}' body='{}'",
            status,
            query,
            Self::preview_text(&text, 400)
        ))
    }

    fn ensure_schema(&self) -> Result<()> {
        let db = quote_ident(&self.database);
        self.execute_query(&format!("CREATE DATABASE IF NOT EXISTS {db}"), None, false)?;

        let t_rtds_prices = quote_ident(&self.table_rtds_prices);
        self.execute_query(
            &format!(
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
            ),
            None,
            true,
        )?;

        let t_price_to_beat = quote_ident(&self.table_price_to_beat);
        self.execute_query(
            &format!(
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
            ),
            None,
            true,
        )?;

        let t_resolution = quote_ident(&self.table_resolution_state);
        self.execute_query(
            &format!(
                "CREATE TABLE IF NOT EXISTS {t_resolution} (
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
            ),
            None,
            true,
        )?;

        Ok(())
    }

    fn insert_json_rows(&self, table: &str, rows: &[String]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let query = format!(
            "INSERT INTO {} SETTINGS input_format_skip_unknown_fields=1 FORMAT JSONEachRow",
            quote_ident(table)
        );
        let mut payload = rows.join("\n");
        payload.push('\n');
        self.execute_query(&query, Some(payload), true)
    }

    fn enqueue_row_best_effort(&self, table: &str, row: &Value, force_flush: bool) {
        let row_raw = match serde_json::to_string(row) {
            Ok(v) => v,
            Err(e) => {
                self.warn_throttled(&format!(
                    "[RTDS][CH] serialize row failed table={table}: {e}"
                ));
                return;
            }
        };

        let now = now_ms();
        let mut flush_rows: Option<Vec<String>> = None;
        if let Ok(mut state) = self.batch_state.lock() {
            let entry = state
                .by_table
                .entry(table.to_string())
                .or_insert_with(ClickhouseBatchBuffer::default);
            if entry.rows.is_empty() {
                entry.first_enqueue_ms = now;
            }
            entry.rows.push(row_raw);
            let age_ms = now.saturating_sub(entry.first_enqueue_ms);
            let should_flush = force_flush
                || entry.rows.len() >= self.batch_max_rows
                || age_ms >= self.batch_max_delay_ms;
            if should_flush {
                flush_rows = Some(std::mem::take(&mut entry.rows));
                entry.first_enqueue_ms = 0;
            }
        } else {
            self.warn_throttled("[RTDS][CH] batch lock poisoned");
            return;
        }

        if let Some(rows) = flush_rows {
            if let Err(e) = self.insert_json_rows(table, &rows) {
                self.warn_throttled(&format!(
                    "[RTDS][CH] batch insert failed table={} rows={} err={e:#}",
                    table,
                    rows.len()
                ));
            }
        }
    }

    fn flush_table_best_effort(&self, table: &str) {
        let mut rows = Vec::new();
        if let Ok(mut state) = self.batch_state.lock() {
            if let Some(entry) = state.by_table.get_mut(table) {
                if !entry.rows.is_empty() {
                    rows = std::mem::take(&mut entry.rows);
                    entry.first_enqueue_ms = 0;
                }
            }
        } else {
            self.warn_throttled("[RTDS][CH] batch lock poisoned on flush");
            return;
        }
        if rows.is_empty() {
            return;
        }
        if let Err(e) = self.insert_json_rows(table, &rows) {
            self.warn_throttled(&format!(
                "[RTDS][CH] flush batch failed table={} rows={} err={e:#}",
                table,
                rows.len()
            ));
        }
    }

    fn flush_all_best_effort(&self) {
        self.flush_table_best_effort(&self.table_rtds_prices);
        self.flush_table_best_effort(&self.table_price_to_beat);
        self.flush_table_best_effort(&self.table_resolution_state);
    }

    fn warn_throttled(&self, msg: &str) {
        let now = now_ms();
        if let Ok(mut last) = self.last_error_log_ms.lock() {
            if now - *last >= self.error_log_every_ms {
                self.logger.warning(msg);
                *last = now;
            }
        } else {
            self.logger.warning(msg);
        }
    }

    fn insert_rtds_price_best_effort(&self, source_row: &Value) {
        let row_json = match serde_json::to_string(source_row) {
            Ok(v) => v,
            Err(e) => {
                self.warn_throttled(&format!("[RTDS][CH] serialize rtds row failed: {e}"));
                return;
            }
        };
        let now = now_ms();
        let timestamp_ms = val_as_i64(source_row.get("timestamp_ms")).unwrap_or(0);
        let received_at_ms = val_as_i64(source_row.get("received_at_ms")).unwrap_or(now);
        let clob_join_target_ts_ms = val_as_i64(source_row.get("clob_join_target_ts_ms"));
        let clob_up_exchange_ts_ms = val_as_i64(source_row.get("clob_up_exchange_ts_ms"));
        let clob_up_recv_ts_ms = val_as_i64(source_row.get("clob_up_recv_ts_ms"));
        let clob_down_exchange_ts_ms = val_as_i64(source_row.get("clob_down_exchange_ts_ms"));
        let clob_down_recv_ts_ms = val_as_i64(source_row.get("clob_down_recv_ts_ms"));
        let row = json!({
            "row_hash": row_hash_hex(&row_json),
            "market_slug": val_as_string(source_row.get("market_slug")),
            "symbol": val_as_string(source_row.get("symbol")),
            "asset_id": val_as_string(source_row.get("asset_id")),
            "kind": val_as_string(source_row.get("kind")),
            "timestamp_ms": timestamp_ms,
            "timestamp_ms_utc": format_clickhouse_utc_ms(timestamp_ms),
            "received_at_ms": received_at_ms,
            "received_at_ms_utc": format_clickhouse_utc_ms(received_at_ms),
            "price": val_as_f64(source_row.get("price")),
            "value": val_as_f64(source_row.get("value")),
            "price_to_beat": val_as_f64(source_row.get("price_to_beat")),
            "diff_vs_price_to_beat": val_as_f64(source_row.get("diff_vs_price_to_beat")),
            "diff_vs_price_to_beat_percentage": val_as_f64(source_row.get("diff_vs_price_to_beat_percentage")),
            "clob_join_target_ts_ms": clob_join_target_ts_ms,
            "clob_join_target_ts_ms_utc": clob_join_target_ts_ms.and_then(format_clickhouse_utc_ms),
            "clob_up_asset_id": val_as_string(source_row.get("clob_up_asset_id")),
            "clob_up_best_bid_price": val_as_f64(source_row.get("clob_up_best_bid_price")),
            "clob_up_best_ask_price": val_as_f64(source_row.get("clob_up_best_ask_price")),
            "clob_up_mid_price": val_as_f64(source_row.get("clob_up_mid_price")),
            "clob_up_spread": val_as_f64(source_row.get("clob_up_spread")),
            "clob_up_exchange_ts_ms": clob_up_exchange_ts_ms,
            "clob_up_exchange_ts_ms_utc": clob_up_exchange_ts_ms.and_then(format_clickhouse_utc_ms),
            "clob_up_recv_ts_ms": clob_up_recv_ts_ms,
            "clob_up_recv_ts_ms_utc": clob_up_recv_ts_ms.and_then(format_clickhouse_utc_ms),
            "clob_down_asset_id": val_as_string(source_row.get("clob_down_asset_id")),
            "clob_down_best_bid_price": val_as_f64(source_row.get("clob_down_best_bid_price")),
            "clob_down_best_ask_price": val_as_f64(source_row.get("clob_down_best_ask_price")),
            "clob_down_mid_price": val_as_f64(source_row.get("clob_down_mid_price")),
            "clob_down_spread": val_as_f64(source_row.get("clob_down_spread")),
            "clob_down_exchange_ts_ms": clob_down_exchange_ts_ms,
            "clob_down_exchange_ts_ms_utc": clob_down_exchange_ts_ms.and_then(format_clickhouse_utc_ms),
            "clob_down_recv_ts_ms": clob_down_recv_ts_ms,
            "clob_down_recv_ts_ms_utc": clob_down_recv_ts_ms.and_then(format_clickhouse_utc_ms),
            "row_json": row_json,
            "ingested_at_ms": now,
        });
        self.enqueue_row_best_effort(&self.table_rtds_prices, &row, false);
    }

    fn insert_price_to_beat_best_effort(&self, state: &PriceToBeatStateFile) {
        let row_json = match serde_json::to_string(state) {
            Ok(v) => v,
            Err(e) => {
                self.warn_throttled(&format!("[RTDS][CH] serialize price_to_beat failed: {e}"));
                return;
            }
        };
        let row = json!({
            "row_hash": row_hash_hex(&row_json),
            "market_slug": state.market_slug,
            "price_to_beat": state.price_to_beat,
            "updated_at_ms": state.updated_at_ms,
            "updated_at_ms_utc": format_clickhouse_utc_ms(state.updated_at_ms),
            "row_json": row_json,
            "ingested_at_ms": now_ms(),
        });
        self.enqueue_row_best_effort(&self.table_price_to_beat, &row, true);
    }

    fn insert_resolution_best_effort(
        &self,
        snapshot: &ResolutionSnapshot,
        state_version: i64,
        state_updated_at_ms: i64,
    ) {
        let row_json = match serde_json::to_string(snapshot) {
            Ok(v) => v,
            Err(e) => {
                self.warn_throttled(&format!("[RTDS][CH] serialize resolution failed: {e}"));
                return;
            }
        };
        let row = json!({
            "row_hash": row_hash_hex(&row_json),
            "state_version": state_version,
            "state_updated_at_ms": state_updated_at_ms,
            "state_updated_at_ms_utc": format_clickhouse_utc_ms(state_updated_at_ms),
            "market_slug": snapshot.market_slug,
            "symbol": snapshot.symbol,
            "asset_id": snapshot.asset_id,
            "resolution_ts_ms": snapshot.resolution_ts_ms,
            "resolution_ts_ms_utc": format_clickhouse_utc_ms(snapshot.resolution_ts_ms),
            "source_ts_ms": snapshot.source_ts_ms,
            "source_ts_ms_utc": format_clickhouse_utc_ms(snapshot.source_ts_ms),
            "resolution_price": snapshot.resolution_price,
            "resolution_value": snapshot.resolution_value,
            "capture_mode": snapshot.capture_mode,
            "price_to_beat": snapshot.price_to_beat,
            "diff_vs_price_to_beat": snapshot.diff_vs_price_to_beat,
            "diff_vs_price_to_beat_percentage": snapshot.diff_vs_price_to_beat_percentage,
            "captured_at_ms": snapshot.captured_at_ms,
            "captured_at_ms_utc": format_clickhouse_utc_ms(snapshot.captured_at_ms),
            "row_json": row_json,
            "ingested_at_ms": now_ms(),
        });
        self.enqueue_row_best_effort(&self.table_resolution_state, &row, true);
    }
}

#[derive(Debug, Clone, Default)]
struct RuntimeState {
    latest: Option<PriceTick>,
    before_resolution: Option<PriceTick>,
    first_after_resolution: Option<PriceTick>,
    price_to_beat: Option<f64>,
    finalized: bool,
}

#[derive(Debug, Clone)]
struct ClobTopOfBook {
    asset_id: String,
    best_bid_price: Option<f64>,
    best_ask_price: Option<f64>,
    mid_price: Option<f64>,
    spread: Option<f64>,
    exchange_ts_ms: Option<i64>,
    exchange_ts_precision: String,
    recv_ts_ms: i64,
}

#[derive(Debug, Clone, Default)]
struct ClobTopOfBookState {
    up_asset_id: String,
    down_asset_id: String,
    latest_by_asset: HashMap<String, ClobTopOfBook>,
    history_by_asset: HashMap<String, VecDeque<ClobTopOfBook>>,
}

#[derive(Debug, Clone)]
struct ClobMatchedTopOfBook {
    sample: ClobTopOfBook,
    match_mode: String,
    match_delta_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtdsLiveSnapshot {
    pub market_slug: String,
    pub symbol: String,
    pub asset_id: String,
    pub timestamp_ms: i64,
    pub price: f64,
    pub value: Option<f64>,
    pub price_to_beat: Option<f64>,
    pub diff_vs_price_to_beat: Option<f64>,
    pub diff_vs_price_to_beat_percentage: Option<f64>,
    pub received_at_ms: i64,
    pub updated_at_ms: i64,
}

static LIVE_SNAPSHOTS_BY_MARKET: OnceLock<Mutex<HashMap<String, RtdsLiveSnapshot>>> =
    OnceLock::new();

fn live_snapshots_by_market() -> &'static Mutex<HashMap<String, RtdsLiveSnapshot>> {
    LIVE_SNAPSHOTS_BY_MARKET.get_or_init(|| Mutex::new(HashMap::new()))
}

fn upsert_live_snapshot(snapshot: RtdsLiveSnapshot) {
    if let Ok(mut m) = live_snapshots_by_market().lock() {
        m.insert(snapshot.market_slug.clone(), snapshot);
    }
}

fn clear_live_snapshot(market_slug: &str) {
    if market_slug.trim().is_empty() {
        return;
    }
    if let Ok(mut m) = live_snapshots_by_market().lock() {
        m.remove(market_slug.trim());
    }
}

pub fn get_live_snapshot_for_market(market_slug: &str) -> Option<RtdsLiveSnapshot> {
    if market_slug.trim().is_empty() {
        return None;
    }
    live_snapshots_by_market()
        .lock()
        .ok()
        .and_then(|m| m.get(market_slug.trim()).cloned())
}

pub fn get_resolution_snapshot_for_market(market_slug: &str) -> Option<ResolutionSnapshot> {
    let slug = market_slug.trim();
    if slug.is_empty() {
        return None;
    }
    let state_path = env::var("RTDS_STATE_PATH")
        .unwrap_or_else(|_| "state/rtds_resolution_state.json".to_string());
    let raw = fs::read_to_string(&state_path).ok()?;
    let state = serde_json::from_str::<ResolutionStateFile>(&raw).ok()?;
    state
        .records
        .iter()
        .filter(|r| r.market_slug == slug)
        .max_by_key(|r| (r.resolution_ts_ms, r.source_ts_ms, r.captured_at_ms))
        .cloned()
        .or_else(|| {
            state
                .last_by_symbol
                .values()
                .filter(|r| r.market_slug == slug)
                .max_by_key(|r| (r.resolution_ts_ms, r.source_ts_ms, r.captured_at_ms))
                .cloned()
        })
}

pub struct RtdsService {
    market_slug: String,
    symbol: String,
    asset_id: String,
    resolution_ts_ms: i64,
    provider: RtdsProvider,
    chainlink_cfg: Option<ChainlinkRtdsConfig>,
    ws_url: String,
    topic: String,
    sub_type: String,
    reconnect_min: f64,
    reconnect_max: f64,
    ping_interval: f64,
    read_timeout: f64,
    log_realtime: bool,
    log_raw: bool,
    write_latest_file: bool,
    persist_state_to_file: bool,
    state_path: PathBuf,
    price_to_beat_state_path: PathBuf,
    latest_path: PathBuf,
    max_records: usize,
    tick_log: Option<Arc<JsonlFileService>>,
    clickhouse_sink: Option<Arc<RtdsClickhouseSink>>,
    clob_join_enabled: bool,
    clob_ws_url: String,
    clob_reconnect_min: f64,
    clob_reconnect_max: f64,
    clob_ping_interval: f64,
    clob_read_timeout: f64,
    clob_match_max_age_ms: i64,
    clob_state: Arc<Mutex<ClobTopOfBookState>>,
    runtime: Arc<Mutex<RuntimeState>>,
    stop_event: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    clob_thread: Mutex<Option<JoinHandle<()>>>,
    logger: Arc<dyn LogLike>,
}

impl RtdsService {
    fn resolve_chainlink_feed_id(symbol: &str) -> Option<String> {
        let mut keys = Vec::<String>::new();
        let asset = to_asset_id(symbol).to_ascii_uppercase();
        if !asset.trim().is_empty() {
            keys.push(format!("RTDS_CHAINLINK_FEED_ID_{asset}"));
        }
        keys.push("RTDS_CHAINLINK_FEED_ID".to_string());
        keys.into_iter().find_map(|key| {
            env::var(&key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
    }

    fn build_chainlink_rtds_config(symbol: &str) -> Result<ChainlinkRtdsConfig> {
        let feed_id_hex = Self::resolve_chainlink_feed_id(symbol).ok_or_else(|| {
            anyhow!(
                "RTDS_PROVIDER=CHAINLINK requires RTDS_CHAINLINK_FEED_ID or RTDS_CHAINLINK_FEED_ID_<ASSET>"
            )
        })?;
        let feed_id = ChainlinkFeedId::from_hex_str(&feed_id_hex)
            .map_err(|e| anyhow!("invalid Chainlink feed id '{feed_id_hex}': {e}"))?;
        let api_key = env::var("RTDS_CHAINLINK_API_KEY")
            .unwrap_or_default()
            .trim()
            .to_string();
        if api_key.is_empty() {
            return Err(anyhow!(
                "RTDS_PROVIDER=CHAINLINK requires RTDS_CHAINLINK_API_KEY"
            ));
        }
        let api_secret = env::var("RTDS_CHAINLINK_API_SECRET")
            .unwrap_or_default()
            .trim()
            .to_string();
        if api_secret.is_empty() {
            return Err(anyhow!(
                "RTDS_PROVIDER=CHAINLINK requires RTDS_CHAINLINK_API_SECRET"
            ));
        }
        let rest_url = env::var("RTDS_CHAINLINK_REST_URL")
            .unwrap_or_else(|_| "https://api.testnet-dataengine.chain.link".to_string())
            .trim()
            .to_string();
        if rest_url.is_empty() {
            return Err(anyhow!(
                "RTDS_PROVIDER=CHAINLINK requires RTDS_CHAINLINK_REST_URL"
            ));
        }
        let ws_url = env::var("RTDS_CHAINLINK_WS_URL")
            .unwrap_or_else(|_| "wss://ws.testnet-dataengine.chain.link".to_string())
            .trim()
            .to_string();
        if ws_url.is_empty() {
            return Err(anyhow!(
                "RTDS_PROVIDER=CHAINLINK requires RTDS_CHAINLINK_WS_URL"
            ));
        }
        let price_decimals = env_int("RTDS_CHAINLINK_PRICE_DECIMALS", 8).clamp(0, 24) as u32;
        let ws_ha = env_bool("RTDS_CHAINLINK_WS_HA", false);
        let ws_max_reconnect = env_int("RTDS_CHAINLINK_WS_MAX_RECONNECT", 5).max(1) as usize;
        let insecure_skip_verify = env_bool("RTDS_CHAINLINK_INSECURE_SKIP_VERIFY", false);
        let read_timeout = env_float(
            "RTDS_CHAINLINK_WS_READ_TIMEOUT_SECONDS",
            env_float("RTDS_WS_READ_TIMEOUT_SECONDS", 1.0),
        )
        .max(0.1);

        Ok(ChainlinkRtdsConfig {
            feed_id,
            feed_id_hex,
            price_decimals,
            api_key,
            api_secret,
            rest_url,
            ws_url,
            ws_ha,
            ws_max_reconnect,
            insecure_skip_verify,
            read_timeout,
        })
    }

    pub fn for_market(
        market_slug: &str,
        cfg: &BotConfig,
        logger: Arc<dyn LogLike>,
    ) -> Result<Option<Arc<Self>>> {
        if !env_bool("RTDS_ENABLED", true) {
            return Ok(None);
        }

        let market = fetch_market_by_slug(market_slug, Some(&logger))
            .ok()
            .flatten();

        let symbol = env::var("RTDS_SYMBOL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| to_symbol(&s))
            .filter(|s| !s.is_empty())
            .or_else(|| {
                market
                    .as_ref()
                    .and_then(|m| m.get("resolutionSource").and_then(|v| v.as_str()))
                    .and_then(infer_symbol_from_resolution_source)
            })
            .or_else(|| {
                market
                    .as_ref()
                    .and_then(|m| m.get("question").and_then(|v| v.as_str()))
                    .and_then(infer_symbol_from_question)
            })
            .or_else(|| infer_symbol_from_slug(market_slug));

        let symbol = match symbol {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                logger.warning(&format!(
                    "[RTDS] unable to infer symbol for market={market_slug}; skipping RTDS"
                ));
                return Ok(None);
            }
        };
        let asset_id = to_asset_id(&symbol);
        if asset_id.is_empty() {
            logger.warning(&format!(
                "[RTDS] invalid inferred symbol={symbol} for market={market_slug}; skipping RTDS"
            ));
            return Ok(None);
        }

        let resolution_ts_ms = market
            .as_ref()
            .and_then(|m| {
                m.get("endDate")
                    .or_else(|| m.get("end_date"))
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_epoch)
            })
            .map(|v| v * 1000)
            .or_else(|| {
                market_slug
                    .split('-')
                    .last()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|start| (start + cfg.market_duration_seconds) * 1000)
            });
        let resolution_ts_ms = match resolution_ts_ms {
            Some(v) if v > 0 => v,
            _ => {
                logger.warning(&format!(
                    "[RTDS] unable to infer resolution timestamp for market={market_slug}; skipping RTDS"
                ));
                return Ok(None);
            }
        };

        let provider = RtdsProvider::from_env();
        let chainlink_cfg = if provider == RtdsProvider::Chainlink {
            let cfg = Self::build_chainlink_rtds_config(&symbol)?;
            logger.info(&format!(
                "[RTDS] provider={} market={} symbol={} feed_id={} decimals={} ws_ha={} ws_max_reconnect={}",
                provider.as_str(),
                market_slug,
                symbol,
                cfg.feed_id_hex,
                cfg.price_decimals,
                cfg.ws_ha,
                cfg.ws_max_reconnect
            ));
            Some(cfg)
        } else {
            logger.info(&format!(
                "[RTDS] provider={} market={} symbol={}",
                provider.as_str(),
                market_slug,
                symbol
            ));
            None
        };

        let state_path = env::var("RTDS_STATE_PATH")
            .unwrap_or_else(|_| "state/rtds_resolution_state.json".to_string())
            .trim()
            .to_string();
        let state_path = PathBuf::from(state_path);
        let price_to_beat_state_path = env::var("RTDS_PRICE_TO_BEAT_STATE_PATH")
            .unwrap_or_else(|_| "state/rtds_price_to_beat_state.json".to_string())
            .trim()
            .to_string();
        let price_to_beat_state_path = PathBuf::from(price_to_beat_state_path);
        let latest_path = env::var("RTDS_LATEST_PATH")
            .unwrap_or_else(|_| "state/rtds_latest.json".to_string())
            .trim()
            .to_string();
        let latest_path = PathBuf::from(latest_path);

        let tick_log_path = env::var("RTDS_PRICE_LOG_PATH")
            .unwrap_or_else(|_| "state/rtds_prices.jsonl".to_string())
            .trim()
            .to_string();
        let sink_selection = parse_rtds_sink_selection();
        let tick_log_enabled = sink_selection.file && !tick_log_path.is_empty();
        let tick_log = if tick_log_enabled {
            Some(Arc::new(JsonlFileService::new(tick_log_path, true)))
        } else {
            None
        };
        let write_latest_file = env_bool("RTDS_WRITE_LATEST_FILE", sink_selection.file);
        let persist_state_to_file = env_bool("RTDS_PERSIST_STATE_TO_FILE", sink_selection.file);

        let mut clickhouse_sink: Option<Arc<RtdsClickhouseSink>> = None;
        if sink_selection.clickhouse {
            match RtdsClickhouseSink::from_env(logger.clone()) {
                Ok(sink) => {
                    if env_bool("RTDS_CLICKHOUSE_AUTO_CREATE_SCHEMA", true) {
                        if let Err(e) = sink.ensure_schema() {
                            if !sink_selection.file {
                                return Err(anyhow!(
                                    "RTDS sink requires ClickHouse, but schema init failed: {e:#}"
                                ));
                            }
                            logger.warning(&format!(
                                "[RTDS] clickhouse schema init failed; continuing file sink only: {e:#}"
                            ));
                        } else {
                            clickhouse_sink = Some(Arc::new(sink));
                        }
                    } else {
                        clickhouse_sink = Some(Arc::new(sink));
                    }
                }
                Err(e) => {
                    if !sink_selection.file {
                        return Err(anyhow!(
                            "RTDS sink requires ClickHouse, but configuration is invalid: {e:#}"
                        ));
                    }
                    logger.warning(&format!(
                        "[RTDS] clickhouse config invalid; continuing file sink only: {e:#}"
                    ));
                }
            }
        }
        logger.info(&format!(
            "[RTDS] sink mode={} file={} clickhouse={} write_latest_file={} persist_state_to_file={}",
            sink_selection.mode_label,
            sink_selection.file,
            clickhouse_sink.is_some(),
            write_latest_file,
            persist_state_to_file
        ));
        if let Some(ch) = &clickhouse_sink {
            logger.info(&format!(
                "[RTDS] clickhouse batching rows={} max_delay_ms={}",
                ch.batch_max_rows, ch.batch_max_delay_ms
            ));
        }
        if tick_log.is_none() && clickhouse_sink.is_none() {
            logger.warning("[RTDS] no tick sink is active (file and clickhouse are both disabled)");
        }

        let (up_asset_id, down_asset_id) = match market
            .as_ref()
            .and_then(|m| parse_tokens_and_condition(m).ok())
        {
            Some((up, down, _)) => (up, down),
            None => (String::new(), String::new()),
        };

        let clob_ws_url = env::var("RTDS_CLOB_WS_URL")
            .unwrap_or_else(|_| format!("{}/ws/market", cfg.ws_base.trim_end_matches('/')))
            .trim()
            .to_string();
        let clob_join_enabled = env_bool("RTDS_CLOB_JOIN_ENABLED", true)
            && !up_asset_id.trim().is_empty()
            && !down_asset_id.trim().is_empty()
            && !clob_ws_url.trim().is_empty();
        let clob_state = Arc::new(Mutex::new(ClobTopOfBookState {
            up_asset_id: up_asset_id.clone(),
            down_asset_id: down_asset_id.clone(),
            latest_by_asset: HashMap::new(),
            history_by_asset: HashMap::new(),
        }));
        if clob_join_enabled {
            logger.info(&format!(
                "[RTDS] CLOB join enabled market={} up_asset={} down_asset={}",
                market_slug, up_asset_id, down_asset_id
            ));
        } else {
            logger.info(&format!(
                "[RTDS] CLOB join disabled market={} up_asset_present={} down_asset_present={} ws_url_present={}",
                market_slug,
                !up_asset_id.trim().is_empty(),
                !down_asset_id.trim().is_empty(),
                !clob_ws_url.trim().is_empty()
            ));
        }

        let mut runtime = RuntimeState::default();
        let store = Self::load_state_file(&state_path);
        let prev = Self::find_previous_snapshot(&store, &symbol, market_slug, resolution_ts_ms);
        let slug_ptb = Self::load_price_to_beat_state(&price_to_beat_state_path)
            .filter(|s| s.market_slug.trim() == market_slug.trim())
            .and_then(|s| s.price_to_beat);
        let configured_ptb = parse_env_f64_opt("RTDS_PRICE_TO_BEAT");
        runtime.price_to_beat = configured_ptb
            .or(slug_ptb)
            .or_else(|| prev.as_ref().map(|s| s.resolution_price));
        let initial_ptb_state = PriceToBeatStateFile {
            market_slug: market_slug.to_string(),
            price_to_beat: runtime.price_to_beat,
            updated_at_ms: now_ms(),
        };
        if persist_state_to_file {
            let _ = Self::save_price_to_beat_state(&price_to_beat_state_path, &initial_ptb_state);
        }
        if let Some(ch) = &clickhouse_sink {
            ch.insert_price_to_beat_best_effort(&initial_ptb_state);
        }

        if let Some(p) = runtime.price_to_beat {
            logger.info(&format!(
                "[RTDS] market={market_slug} symbol={symbol} price_to_beat={p:.6}"
            ));
        } else {
            logger.info(&format!(
                "[RTDS] market={market_slug} symbol={symbol} no previous price_to_beat found"
            ));
        }

        let svc = Arc::new(Self {
            market_slug: market_slug.to_string(),
            symbol: symbol.clone(),
            asset_id,
            resolution_ts_ms,
            provider,
            chainlink_cfg,
            ws_url: env::var("RTDS_WS_URL")
                .unwrap_or_else(|_| "wss://ws-live-data.polymarket.com".to_string())
                .trim()
                .to_string(),
            topic: env::var("RTDS_TOPIC")
                .unwrap_or_else(|_| "crypto_prices_chainlink".to_string())
                .trim()
                .to_string(),
            sub_type: env::var("RTDS_SUB_TYPE")
                .unwrap_or_else(|_| "*".to_string())
                .trim()
                .to_string(),
            reconnect_min: env_float("RTDS_WS_RECONNECT_MIN", 1.0).max(0.1),
            reconnect_max: env_float("RTDS_WS_RECONNECT_MAX", 20.0).max(0.5),
            ping_interval: env_float("RTDS_WS_PING_INTERVAL", 5.0).max(0.0),
            read_timeout: env_float("RTDS_WS_READ_TIMEOUT_SECONDS", 1.0).max(0.1),
            log_realtime: env_bool("RTDS_LOG_REALTIME", false),
            log_raw: env_bool("RTDS_LOG_RAW", false),
            write_latest_file,
            persist_state_to_file,
            state_path,
            price_to_beat_state_path,
            latest_path,
            max_records: env_int("RTDS_STATE_MAX_RECORDS", 2000).max(100) as usize,
            tick_log,
            clickhouse_sink,
            clob_join_enabled,
            clob_ws_url,
            clob_reconnect_min: env_float("RTDS_CLOB_WS_RECONNECT_MIN", 1.0).max(0.1),
            clob_reconnect_max: env_float("RTDS_CLOB_WS_RECONNECT_MAX", 20.0).max(0.5),
            clob_ping_interval: env_float("RTDS_CLOB_WS_PING_INTERVAL", 5.0).max(0.0),
            clob_read_timeout: env_float("RTDS_CLOB_WS_READ_TIMEOUT_SECONDS", 1.0).max(0.1),
            clob_match_max_age_ms: env_int("RTDS_CLOB_MATCH_MAX_AGE_MS", 2500).max(100) as i64,
            clob_state,
            runtime: Arc::new(Mutex::new(runtime)),
            stop_event: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
            clob_thread: Mutex::new(None),
            logger,
        });

        clear_live_snapshot(market_slug);
        Ok(Some(svc))
    }

    pub fn start(self: &Arc<Self>) {
        if let Ok(mut slot) = self.thread.lock() {
            if slot.as_ref().map(|h| !h.is_finished()).unwrap_or(false) {
                return;
            }
            let this = Arc::clone(self);
            *slot = Some(thread::spawn(move || this.run_loop()));
        }
        if self.clob_join_enabled {
            if let Ok(mut slot) = self.clob_thread.lock() {
                if slot.as_ref().map(|h| !h.is_finished()).unwrap_or(false) {
                    return;
                }
                let this = Arc::clone(self);
                *slot = Some(thread::spawn(move || this.run_clob_loop()));
            }
        }
    }

    pub fn close(&self) {
        self.wait_for_resolution_before_close();
        self.stop_event.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.thread.lock() {
            if let Some(handle) = slot.take() {
                let _ = handle.join();
            }
        }
        if let Ok(mut slot) = self.clob_thread.lock() {
            if let Some(handle) = slot.take() {
                let _ = handle.join();
            }
        }
        let _ = self.persist_resolution_snapshot();
        if let Some(ch) = &self.clickhouse_sink {
            ch.flush_all_best_effort();
        }
        clear_live_snapshot(&self.market_slug);
    }

    fn wait_for_resolution_before_close(&self) {
        let settle_after_ms = 2500i64;
        let now = now_ms();
        let target_ms = self.resolution_ts_ms.saturating_add(settle_after_ms);
        let remaining_ms = target_ms.saturating_sub(now);
        if remaining_ms <= 0 {
            return;
        }
        self.logger.info(&format!(
            "[RTDS] waiting for resolution before close market={} wait_ms={} target_ts_ms={}",
            self.market_slug, remaining_ms, target_ms
        ));
        while !self.stop_event.load(Ordering::SeqCst) && now_ms() < target_ms {
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn run_loop(self: Arc<Self>) {
        match self.provider {
            RtdsProvider::Polymarket => self.run_polymarket_loop(),
            RtdsProvider::Chainlink => self.run_chainlink_loop(),
        }
    }

    fn run_polymarket_loop(self: Arc<Self>) {
        if self.ws_url.trim().is_empty() {
            self.logger.error("[RTDS] missing RTDS_WS_URL");
            return;
        }
        let mut backoff = self.reconnect_min.max(0.1);
        while !self.stop_event.load(Ordering::SeqCst) {
            let conn = connect(&self.ws_url);
            let (mut ws, _) = match conn {
                Ok(v) => v,
                Err(e) => {
                    self.logger.warning(&format!("[RTDS] connect error: {e}"));
                    let sleep_for = (backoff.min(self.reconnect_max))
                        * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
                    thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                    backoff = (backoff * 2.0).min(self.reconnect_max);
                    continue;
                }
            };

            backoff = self.reconnect_min.max(0.1);
            self.configure_socket_timeouts(&mut ws);
            if let Err(e) = self.send_subscription(&mut ws) {
                self.logger.warning(&format!("[RTDS] subscribe error: {e}"));
                let _ = ws.close(None);
                thread::sleep(Duration::from_secs_f64(backoff));
                continue;
            }

            self.logger.info(&format!(
                "[RTDS] connected market={} symbol={} resolution_ts_ms={}",
                self.market_slug, self.symbol, self.resolution_ts_ms
            ));
            let mut last_ping = Instant::now();
            while !self.stop_event.load(Ordering::SeqCst) {
                if self.ping_interval > 0.0
                    && last_ping.elapsed() >= Duration::from_secs_f64(self.ping_interval)
                {
                    let _ = ws.send(Message::Text("ping".into()));
                    last_ping = Instant::now();
                }

                let msg = match ws.read() {
                    Ok(m) => m,
                    Err(tungstenite::Error::Io(e))
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                    {
                        continue
                    }
                    Err(e) => {
                        self.logger.warning(&format!("[RTDS] ws read error: {e}"));
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => self.process_text_message(&text),
                    Message::Binary(bin) => {
                        if let Ok(text) = String::from_utf8(bin.to_vec()) {
                            self.process_text_message(&text);
                        }
                    }
                    Message::Close(frame) => {
                        let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(0);
                        let reason = frame
                            .as_ref()
                            .map(|f| f.reason.to_string())
                            .unwrap_or_default();
                        self.logger
                            .warning(&format!("[RTDS] ws closed code={code} reason={reason}"));
                        break;
                    }
                    _ => {}
                }
            }
            let _ = ws.close(None);
            if self.stop_event.load(Ordering::SeqCst) {
                break;
            }
            let sleep_for =
                (backoff.min(self.reconnect_max)) * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
            self.logger
                .warning(&format!("[RTDS] reconnecting in {sleep_for:.1}s"));
            thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
            backoff = (backoff * 2.0).min(self.reconnect_max);
        }
    }

    fn run_chainlink_loop(self: Arc<Self>) {
        let Some(cfg) = self.chainlink_cfg.clone() else {
            self.logger
                .error("[RTDS][CHAINLINK] provider selected but config is missing");
            return;
        };

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                self.logger
                    .error(&format!("[RTDS][CHAINLINK] runtime init error: {e}"));
                return;
            }
        };

        let mut backoff = self.reconnect_min.max(0.1);
        while !self.stop_event.load(Ordering::SeqCst) {
            let this = Arc::clone(&self);
            let cfg_clone = cfg.clone();
            let session_result =
                runtime.block_on(async move { this.run_chainlink_session(cfg_clone).await });
            if self.stop_event.load(Ordering::SeqCst) {
                break;
            }
            if let Err(e) = session_result {
                self.logger
                    .warning(&format!("[RTDS][CHAINLINK] stream error: {e:#}"));
            } else {
                self.logger
                    .warning("[RTDS][CHAINLINK] stream closed, reconnecting");
            }
            let sleep_for =
                (backoff.min(self.reconnect_max)) * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
            self.logger.warning(&format!(
                "[RTDS][CHAINLINK] reconnecting in {sleep_for:.1}s"
            ));
            thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
            backoff = (backoff * 2.0).min(self.reconnect_max);
        }
    }

    async fn run_chainlink_session(self: Arc<Self>, cfg: ChainlinkRtdsConfig) -> Result<()> {
        let mut builder = ChainlinkConfig::new(
            cfg.api_key.clone(),
            cfg.api_secret.clone(),
            cfg.rest_url.clone(),
            cfg.ws_url.clone(),
        )
        .with_ws_max_reconnect(cfg.ws_max_reconnect);
        if cfg.ws_ha {
            builder = builder.with_ws_ha(ChainlinkWebSocketHighAvailability::Enabled);
        }
        if cfg.insecure_skip_verify {
            builder = builder.with_insecure_skip_verify(ChainlinkInsecureSkipVerify::Enabled);
        }
        let ds_cfg = builder
            .build()
            .map_err(|e| anyhow!("[RTDS][CHAINLINK] invalid config: {e}"))?;
        let mut stream = ChainlinkStream::new(&ds_cfg, vec![cfg.feed_id])
            .await
            .map_err(|e| anyhow!("[RTDS][CHAINLINK] stream create failed: {e}"))?;
        stream
            .listen()
            .await
            .map_err(|e| anyhow!("[RTDS][CHAINLINK] stream listen failed: {e}"))?;
        self.logger.info(&format!(
            "[RTDS][CHAINLINK] connected market={} symbol={} feed_id={}",
            self.market_slug, self.symbol, cfg.feed_id_hex
        ));

        let mut session_error: Option<anyhow::Error> = None;
        while !self.stop_event.load(Ordering::SeqCst) {
            let read_result =
                tokio::time::timeout(Duration::from_secs_f64(cfg.read_timeout), stream.read())
                    .await;
            match read_result {
                Ok(Ok(response)) => {
                    if let Err(e) = self.process_chainlink_report_message(response, &cfg) {
                        self.logger
                            .warning(&format!("[RTDS][CHAINLINK] parse error: {e:#}"));
                    }
                }
                Ok(Err(e)) => {
                    session_error = Some(anyhow!("[RTDS][CHAINLINK] read failed: {e}"));
                    break;
                }
                Err(_) => {
                    continue;
                }
            }
        }

        if let Err(e) = stream.close().await {
            self.logger
                .warning(&format!("[RTDS][CHAINLINK] close warning: {e}"));
        }
        if let Some(e) = session_error {
            return Err(e);
        }
        Ok(())
    }

    fn run_clob_loop(self: Arc<Self>) {
        if !self.clob_join_enabled {
            return;
        }
        if self.clob_ws_url.trim().is_empty() {
            self.logger.error("[RTDS][CLOB] missing RTDS_CLOB_WS_URL");
            return;
        }
        let mut backoff = self.clob_reconnect_min.max(0.1);
        while !self.stop_event.load(Ordering::SeqCst) {
            let conn = connect(&self.clob_ws_url);
            let (mut ws, _) = match conn {
                Ok(v) => v,
                Err(e) => {
                    self.logger
                        .warning(&format!("[RTDS][CLOB] connect error: {e}"));
                    let sleep_for = (backoff.min(self.clob_reconnect_max))
                        * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
                    thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                    backoff = (backoff * 2.0).min(self.clob_reconnect_max);
                    continue;
                }
            };

            backoff = self.clob_reconnect_min.max(0.1);
            Self::configure_socket_timeouts_with(&mut ws, self.clob_read_timeout);
            if let Err(e) = self.send_clob_subscription(&mut ws) {
                self.logger
                    .warning(&format!("[RTDS][CLOB] subscribe error: {e}"));
                let _ = ws.close(None);
                thread::sleep(Duration::from_secs_f64(backoff));
                continue;
            }

            let (up, down) = self.clob_asset_pair();
            self.logger.info(&format!(
                "[RTDS][CLOB] connected market={} up_asset={} down_asset={}",
                self.market_slug, up, down
            ));
            let mut last_ping = Instant::now();
            while !self.stop_event.load(Ordering::SeqCst) {
                if self.clob_ping_interval > 0.0
                    && last_ping.elapsed() >= Duration::from_secs_f64(self.clob_ping_interval)
                {
                    let _ = ws.send(Message::Text("ping".into()));
                    last_ping = Instant::now();
                }

                let msg = match ws.read() {
                    Ok(m) => m,
                    Err(tungstenite::Error::Io(e))
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                    {
                        continue
                    }
                    Err(e) => {
                        self.logger
                            .warning(&format!("[RTDS][CLOB] ws read error: {e}"));
                        break;
                    }
                };

                match msg {
                    Message::Text(text) => self.process_clob_text_message(&text),
                    Message::Binary(bin) => {
                        if let Ok(text) = String::from_utf8(bin.to_vec()) {
                            self.process_clob_text_message(&text);
                        }
                    }
                    Message::Close(frame) => {
                        let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(0);
                        let reason = frame
                            .as_ref()
                            .map(|f| f.reason.to_string())
                            .unwrap_or_default();
                        self.logger.warning(&format!(
                            "[RTDS][CLOB] ws closed code={code} reason={reason}"
                        ));
                        break;
                    }
                    _ => {}
                }
            }
            let _ = ws.close(None);
            if self.stop_event.load(Ordering::SeqCst) {
                break;
            }
            let sleep_for = (backoff.min(self.clob_reconnect_max))
                * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
            self.logger
                .warning(&format!("[RTDS][CLOB] reconnecting in {sleep_for:.1}s"));
            thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
            backoff = (backoff * 2.0).min(self.clob_reconnect_max);
        }
    }

    fn process_clob_text_message(&self, raw: &str) {
        let text = raw.trim();
        if text.is_empty() {
            return;
        }
        if text.eq_ignore_ascii_case("pong") || text.eq_ignore_ascii_case("ping") {
            return;
        }
        let msg: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                self.logger
                    .warning(&format!("[RTDS][CLOB] parse error: {e}; payload={text}"));
                return;
            }
        };
        self.process_clob_json_value(&msg);
    }

    fn process_clob_json_value(&self, v: &Value) {
        match v {
            Value::Array(items) => {
                for item in items {
                    self.process_clob_json_value(item);
                }
            }
            Value::Object(map) => {
                self.process_clob_event(v);
                if let Some(payload) = map.get("payload") {
                    self.process_clob_json_value(payload);
                }
                if let Some(data) = map.get("data") {
                    self.process_clob_json_value(data);
                }
                if let Some(events) = map.get("events") {
                    self.process_clob_json_value(events);
                }
            }
            _ => {}
        }
    }

    fn process_clob_event(&self, msg: &Value) {
        let et = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !et.is_empty() && et != "best_bid_ask" {
            return;
        }

        let asset_id = msg
            .get("asset_id")
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("asset"))
            .or_else(|| msg.get("token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if asset_id.is_empty() {
            return;
        }
        let (up, down) = self.clob_asset_pair();
        if asset_id != up && asset_id != down {
            return;
        }

        let best_bid_price = val_as_f64(
            msg.get("best_bid_price")
                .or_else(|| msg.get("best_bid"))
                .or_else(|| msg.get("bid"))
                .or_else(|| msg.get("b")),
        );
        let best_ask_price = val_as_f64(
            msg.get("best_ask_price")
                .or_else(|| msg.get("best_ask"))
                .or_else(|| msg.get("ask"))
                .or_else(|| msg.get("a")),
        );
        if best_bid_price.is_none() && best_ask_price.is_none() {
            return;
        }
        let (exchange_ts_ms, exchange_ts_precision) = val_as_i64_ms_with_precision(
            msg.get("timestamp_ms")
                .or_else(|| msg.get("timestampMs"))
                .or_else(|| msg.get("timestamp"))
                .or_else(|| msg.get("ts"))
                .or_else(|| msg.get("t"))
                .or_else(|| msg.get("time"))
                .or_else(|| msg.get("updated_at")),
        );
        let recv_ts_ms = now_ms();
        let mid_price = match (best_bid_price, best_ask_price) {
            (Some(b), Some(a)) if b > 0.0 && a > 0.0 => Some((b + a) * 0.5),
            _ => None,
        };
        let spread = match (best_bid_price, best_ask_price) {
            (Some(b), Some(a)) if b > 0.0 && a > 0.0 => Some(a - b),
            _ => None,
        };

        self.upsert_clob_top_of_book(ClobTopOfBook {
            asset_id,
            best_bid_price,
            best_ask_price,
            mid_price,
            spread,
            exchange_ts_ms,
            exchange_ts_precision: exchange_ts_precision.to_string(),
            recv_ts_ms,
        });
    }

    fn send_clob_subscription(&self, ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<()> {
        let (up, down) = self.clob_asset_pair();
        if up.trim().is_empty() || down.trim().is_empty() {
            return Err(anyhow!("missing CLOB up/down asset ids"));
        }
        let sub = json!({
            "assets_ids": [up, down],
            "type": "market",
            "custom_feature_enabled": true
        });
        ws.send(Message::Text(sub.to_string().into()))
            .map_err(|e| anyhow!("send CLOB subscribe failed: {e}"))?;
        Ok(())
    }

    fn configure_socket_timeouts(&self, ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
        Self::configure_socket_timeouts_with(ws, self.read_timeout);
    }

    fn configure_socket_timeouts_with(
        ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        timeout_seconds: f64,
    ) {
        let timeout = Some(Duration::from_secs_f64(timeout_seconds.max(0.1)));
        match ws.get_mut() {
            MaybeTlsStream::Plain(s) => {
                let _ = s.set_read_timeout(timeout);
                let _ = s.set_write_timeout(timeout);
            }
            MaybeTlsStream::Rustls(s) => {
                let _ = s.sock.set_read_timeout(timeout);
                let _ = s.sock.set_write_timeout(timeout);
            }
            _ => {}
        }
    }

    fn send_subscription(&self, ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<()> {
        let filters = json!({ "symbol": self.symbol });
        let sub = json!({
            "action": "subscribe",
            "subscriptions": [
                {
                    "topic": self.topic,
                    "type": self.sub_type,
                    "filters": serde_json::to_string(&filters)?
                }
            ]
        });
        ws.send(Message::Text(sub.to_string().into()))
            .map_err(|e| anyhow!("send subscribe failed: {e}"))?;
        Ok(())
    }

    fn process_text_message(&self, raw: &str) {
        let text = raw.trim();
        if text.is_empty() {
            return;
        }
        if text.eq_ignore_ascii_case("pong") || text.eq_ignore_ascii_case("ping") {
            return;
        }
        let msg: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                self.logger
                    .warning(&format!("[RTDS] parse error: {e}; payload={text}"));
                return;
            }
        };

        if self.log_raw {
            self.append_tick_log(&json!({
                "kind": "raw",
                "ts_ms": now_ms(),
                "market_slug": self.market_slug,
                "symbol": self.symbol,
                "payload": msg,
            }));
        }

        let mut ticks = Vec::<PriceTick>::new();
        Self::collect_ticks(&msg, None, &mut ticks);
        for tick in ticks.into_iter().filter(|t| self.tick_matches(t)) {
            self.on_tick(tick);
        }
    }

    fn process_chainlink_report_message(
        &self,
        response: ChainlinkWebSocketReport,
        cfg: &ChainlinkRtdsConfig,
    ) -> Result<()> {
        if self.log_raw {
            self.append_tick_log(&json!({
                "kind": "raw_chainlink",
                "ts_ms": now_ms(),
                "market_slug": self.market_slug,
                "symbol": self.symbol,
                "payload": &response,
            }));
        }
        let tick = Self::chainlink_report_to_tick(
            &response.report,
            &self.symbol,
            &self.asset_id,
            cfg.price_decimals,
        )?;
        self.on_tick(tick);
        Ok(())
    }

    fn chainlink_report_to_tick(
        report: &ChainlinkReport,
        symbol: &str,
        asset_id: &str,
        price_decimals: u32,
    ) -> Result<PriceTick> {
        let price = Self::chainlink_decode_price(report, price_decimals)?;
        let ts_seconds = report
            .observations_timestamp
            .max(report.valid_from_timestamp);
        let ts_ms = i64::try_from(ts_seconds)
            .unwrap_or(i64::MAX)
            .saturating_mul(1000);
        Ok(PriceTick {
            symbol: symbol.to_string(),
            asset_id: asset_id.to_string(),
            price,
            value: Some(price),
            timestamp_ms: ts_ms,
        })
    }

    fn chainlink_decode_price(report: &ChainlinkReport, price_decimals: u32) -> Result<f64> {
        let raw_full_report = report.full_report.trim();
        let raw_hex = raw_full_report
            .strip_prefix("0x")
            .or_else(|| raw_full_report.strip_prefix("0X"))
            .unwrap_or(raw_full_report);
        if raw_hex.is_empty() {
            return Err(anyhow!("empty full_report payload"));
        }
        let full_report_payload =
            hex::decode(raw_hex).map_err(|e| anyhow!("full_report hex decode failed: {e}"))?;
        let (_, report_blob) = chainlink_decode_full_report(&full_report_payload)
            .map_err(|e| anyhow!("full report decode failed: {e}"))?;
        let version = Self::chainlink_feed_version(&report.feed_id);
        let scaled_price = match version {
            1 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV1::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v1 failed: {e}"))?
                    .benchmark_price,
                price_decimals,
            ),
            2 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV2::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v2 failed: {e}"))?
                    .benchmark_price,
                price_decimals,
            ),
            3 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV3::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v3 failed: {e}"))?
                    .benchmark_price,
                price_decimals,
            ),
            4 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV4::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v4 failed: {e}"))?
                    .price,
                price_decimals,
            ),
            5 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV5::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v5 failed: {e}"))?
                    .rate,
                price_decimals,
            ),
            6 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV6::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v6 failed: {e}"))?
                    .price,
                price_decimals,
            ),
            7 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV7::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v7 failed: {e}"))?
                    .exchange_rate,
                price_decimals,
            ),
            8 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV8::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v8 failed: {e}"))?
                    .mid_price,
                price_decimals,
            ),
            9 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV9::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v9 failed: {e}"))?
                    .nav_per_share,
                price_decimals,
            ),
            10 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV10::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v10 failed: {e}"))?
                    .price,
                price_decimals,
            ),
            11 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV11::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v11 failed: {e}"))?
                    .mid,
                price_decimals,
            ),
            12 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV12::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v12 failed: {e}"))?
                    .nav_per_share,
                price_decimals,
            ),
            13 => Self::scaled_price_from_bigint(
                &ChainlinkReportDataV13::decode(&report_blob)
                    .map_err(|e| anyhow!("decode v13 failed: {e}"))?
                    .last_traded_price,
                price_decimals,
            ),
            other => {
                return Err(anyhow!(
                    "unsupported Chainlink report schema version={other}"
                ));
            }
        }
        .ok_or_else(|| anyhow!("failed to scale Chainlink price"))?;
        if !scaled_price.is_finite() {
            return Err(anyhow!("scaled Chainlink price is not finite"));
        }
        Ok(scaled_price)
    }

    fn chainlink_feed_version(feed_id: &ChainlinkFeedId) -> u16 {
        ((feed_id.0[0] as u16) << 8) | feed_id.0[1] as u16
    }

    fn scaled_price_from_bigint<T: ToString>(raw: &T, decimals: u32) -> Option<f64> {
        let raw_f64 = raw.to_string().trim().parse::<f64>().ok()?;
        let scale = 10f64.powi(decimals.min(24) as i32);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        Some(raw_f64 / scale)
    }

    fn tick_matches(&self, tick: &PriceTick) -> bool {
        if !tick.asset_id.is_empty() && tick.asset_id == self.asset_id {
            return true;
        }
        if tick.symbol.is_empty() {
            return true;
        }
        let sym = to_symbol(&tick.symbol);
        sym == self.symbol || to_asset_id(&tick.symbol) == self.asset_id
    }

    fn on_tick(&self, tick: PriceTick) {
        let (price_to_beat, diff, diff_percentage_value) = {
            let mut st = match self.runtime.lock() {
                Ok(v) => v,
                Err(_) => return,
            };
            st.latest = Some(tick.clone());
            // Treat exact boundary tick (timestamp == resolution_ts_ms) as the
            // resolution tick for the just-finished market.
            if tick.timestamp_ms < self.resolution_ts_ms {
                let should = st
                    .before_resolution
                    .as_ref()
                    .map(|t| tick.timestamp_ms >= t.timestamp_ms)
                    .unwrap_or(true);
                if should {
                    st.before_resolution = Some(tick.clone());
                }
            } else {
                let should = st
                    .first_after_resolution
                    .as_ref()
                    // Keep the earliest timestamp >= resolution_ts_ms.
                    // If timestamps are equal, keep the first seen sample.
                    .map(|t| tick.timestamp_ms < t.timestamp_ms)
                    .unwrap_or(true);
                if should {
                    st.first_after_resolution = Some(tick.clone());
                }
            }
            let ptb = st.price_to_beat;
            let diff = ptb.map(|p| tick.price - p);
            let diff_percentage_value = diff_percentage(tick.price, ptb);
            (ptb, diff, diff_percentage_value)
        };

        if self.log_realtime {
            let value_text = tick
                .value
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "-".to_string());
            let diff_text = diff
                .map(|d| format!("{d:+.6}"))
                .unwrap_or_else(|| "-".to_string());
            let diff_pct_text = diff_percentage_value
                .map(|d| format!("{d:+.6}%"))
                .unwrap_or_else(|| "-".to_string());
            self.logger.info(&format!(
                "[RTDS] market={} symbol={} ts_ms={} price={:.6} value={} diff_vs_price_to_beat={} diff_vs_price_to_beat_percentage={}",
                self.market_slug, self.symbol, tick.timestamp_ms, tick.price, value_text, diff_text, diff_pct_text
            ));
        }

        let t_now_ms = now_ms();
        let live_snapshot = RtdsLiveSnapshot {
            market_slug: self.market_slug.clone(),
            symbol: self.symbol.clone(),
            asset_id: self.asset_id.clone(),
            timestamp_ms: tick.timestamp_ms,
            price: tick.price,
            value: tick.value,
            price_to_beat,
            diff_vs_price_to_beat: diff,
            diff_vs_price_to_beat_percentage: diff_percentage_value,
            received_at_ms: t_now_ms,
            updated_at_ms: t_now_ms,
        };
        upsert_live_snapshot(live_snapshot.clone());
        self.write_latest_snapshot(&live_snapshot);
        let mut row = json!({
            "kind": "tick",
            "market_slug": self.market_slug,
            "symbol": self.symbol,
            "asset_id": tick.asset_id,
            "timestamp_ms": tick.timestamp_ms,
            "price": tick.price,
            "value": tick.value,
            "price_to_beat": price_to_beat,
            "diff_vs_price_to_beat": diff,
            "diff_vs_price_to_beat_percentage": diff_percentage_value,
            "received_at_ms": now_ms(),
        });
        self.apply_clob_join_to_tick_row(&mut row, tick.timestamp_ms);
        self.append_tick_log(&row);
    }

    fn write_latest_snapshot(&self, snapshot: &RtdsLiveSnapshot) {
        if !self.write_latest_file {
            return;
        }
        if let Some(parent) = self.latest_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let raw = match serde_json::to_string(snapshot) {
            Ok(v) => v,
            Err(_) => return,
        };
        let tmp = PathBuf::from(format!("{}.tmp", self.latest_path.to_string_lossy()));
        if fs::write(&tmp, &raw).is_ok() {
            let _ = fs::rename(&tmp, &self.latest_path);
        } else {
            let _ = fs::write(&self.latest_path, &raw);
        }
    }

    fn append_tick_log(&self, obj: &Value) {
        let mut out = obj.clone();
        append_utc_timestamp_columns(&mut out);
        if let Some(fs) = &self.tick_log {
            fs.append(&out);
        }
        if let Some(ch) = &self.clickhouse_sink {
            ch.insert_rtds_price_best_effort(&out);
        }
    }

    fn clob_asset_pair(&self) -> (String, String) {
        self.clob_state
            .lock()
            .map(|s| (s.up_asset_id.clone(), s.down_asset_id.clone()))
            .unwrap_or_else(|_| (String::new(), String::new()))
    }

    fn upsert_clob_top_of_book(&self, snapshot: ClobTopOfBook) {
        if !self.clob_join_enabled {
            return;
        }
        let max_records = env_int("RTDS_CLOB_HISTORY_MAX_RECORDS", 800).max(32) as usize;
        let max_age_ms = env_int("RTDS_CLOB_HISTORY_MAX_AGE_MS", 45_000).max(1000) as i64;
        if let Ok(mut state) = self.clob_state.lock() {
            let queue = state
                .history_by_asset
                .entry(snapshot.asset_id.clone())
                .or_insert_with(VecDeque::new);
            queue.push_back(snapshot.clone());
            while queue.len() > max_records {
                let _ = queue.pop_front();
            }
            while queue
                .front()
                .map(|s| snapshot.recv_ts_ms - s.recv_ts_ms > max_age_ms)
                .unwrap_or(false)
            {
                let _ = queue.pop_front();
            }
            state
                .latest_by_asset
                .insert(snapshot.asset_id.clone(), snapshot);
        }
    }

    fn pick_best_clob_snapshot(
        history: Option<&VecDeque<ClobTopOfBook>>,
        latest: Option<&ClobTopOfBook>,
        target_ts_ms: i64,
        max_age_ms: i64,
    ) -> Option<ClobMatchedTopOfBook> {
        let mut best_ms: Option<(i64, ClobTopOfBook)> = None;
        let mut best_s: Option<(i64, ClobTopOfBook)> = None;
        if let Some(hist) = history {
            for sample in hist {
                if let Some(exchange_ts_ms) = sample.exchange_ts_ms {
                    if sample.exchange_ts_precision == "s" {
                        let delta_s = ((target_ts_ms / 1000) - (exchange_ts_ms / 1000)).abs();
                        let replace = best_s.as_ref().map(|(d, _)| delta_s < *d).unwrap_or(true);
                        if replace {
                            best_s = Some((delta_s, sample.clone()));
                        }
                    } else {
                        let delta_ms = (target_ts_ms - exchange_ts_ms).abs();
                        let replace = best_ms.as_ref().map(|(d, _)| delta_ms < *d).unwrap_or(true);
                        if replace {
                            best_ms = Some((delta_ms, sample.clone()));
                        }
                    }
                }
            }
        }

        if let Some((delta_ms, sample)) = best_ms {
            if delta_ms <= max_age_ms {
                return Some(ClobMatchedTopOfBook {
                    sample,
                    match_mode: "exchange_ms".to_string(),
                    match_delta_ms: Some(delta_ms),
                });
            }
        }

        if let Some((delta_s, sample)) = best_s {
            let max_age_s = (max_age_ms / 1000).max(1);
            if delta_s <= max_age_s {
                return Some(ClobMatchedTopOfBook {
                    sample,
                    match_mode: "exchange_second".to_string(),
                    match_delta_ms: Some(delta_s * 1000),
                });
            }
        }

        let latest = latest
            .cloned()
            .or_else(|| history.and_then(|h| h.back().cloned()));
        if let Some(sample) = latest {
            let ts_ref = sample.exchange_ts_ms.unwrap_or(sample.recv_ts_ms);
            let delta_ms = (target_ts_ms - ts_ref).abs();
            if delta_ms <= max_age_ms {
                let mode = if sample.exchange_ts_ms.is_some() {
                    "exchange_fallback"
                } else {
                    "recv_fallback"
                };
                return Some(ClobMatchedTopOfBook {
                    sample,
                    match_mode: mode.to_string(),
                    match_delta_ms: Some(delta_ms),
                });
            }
        }
        None
    }

    fn match_clob_for_tick(
        &self,
        target_ts_ms: i64,
    ) -> (Option<ClobMatchedTopOfBook>, Option<ClobMatchedTopOfBook>) {
        if !self.clob_join_enabled {
            return (None, None);
        }
        let state = match self.clob_state.lock() {
            Ok(v) => v,
            Err(_) => return (None, None),
        };
        let up = if state.up_asset_id.trim().is_empty() {
            None
        } else {
            Self::pick_best_clob_snapshot(
                state.history_by_asset.get(&state.up_asset_id),
                state.latest_by_asset.get(&state.up_asset_id),
                target_ts_ms,
                self.clob_match_max_age_ms,
            )
        };
        let down = if state.down_asset_id.trim().is_empty() {
            None
        } else {
            Self::pick_best_clob_snapshot(
                state.history_by_asset.get(&state.down_asset_id),
                state.latest_by_asset.get(&state.down_asset_id),
                target_ts_ms,
                self.clob_match_max_age_ms,
            )
        };
        (up, down)
    }

    fn apply_clob_join_to_tick_row(&self, row: &mut Value, tick_ts_ms: i64) {
        let (up_match, down_match) = self.match_clob_for_tick(tick_ts_ms);
        let Some(obj) = row.as_object_mut() else {
            return;
        };
        obj.insert("clob_join_target_ts_ms".to_string(), json!(tick_ts_ms));
        Self::insert_clob_fields(obj, "clob_up", up_match);
        Self::insert_clob_fields(obj, "clob_down", down_match);
    }

    fn insert_clob_fields(
        obj: &mut serde_json::Map<String, Value>,
        prefix: &str,
        matched: Option<ClobMatchedTopOfBook>,
    ) {
        let keys = [
            "asset_id",
            "best_bid_price",
            "best_ask_price",
            "mid_price",
            "spread",
            "exchange_ts_ms",
            "recv_ts_ms",
            "exchange_ts_precision",
            "match_mode",
            "match_delta_ms",
        ];
        if let Some(m) = matched {
            obj.insert(
                format!("{prefix}_asset_id"),
                json!(m.sample.asset_id.clone()),
            );
            obj.insert(
                format!("{prefix}_best_bid_price"),
                json!(m.sample.best_bid_price),
            );
            obj.insert(
                format!("{prefix}_best_ask_price"),
                json!(m.sample.best_ask_price),
            );
            obj.insert(format!("{prefix}_mid_price"), json!(m.sample.mid_price));
            obj.insert(format!("{prefix}_spread"), json!(m.sample.spread));
            obj.insert(
                format!("{prefix}_exchange_ts_ms"),
                json!(m.sample.exchange_ts_ms),
            );
            obj.insert(format!("{prefix}_recv_ts_ms"), json!(m.sample.recv_ts_ms));
            obj.insert(
                format!("{prefix}_exchange_ts_precision"),
                json!(m.sample.exchange_ts_precision.clone()),
            );
            obj.insert(format!("{prefix}_match_mode"), json!(m.match_mode));
            obj.insert(format!("{prefix}_match_delta_ms"), json!(m.match_delta_ms));
            return;
        }
        for key in keys {
            obj.insert(format!("{prefix}_{key}"), Value::Null);
        }
    }

    fn collect_ticks(v: &Value, inherited_symbol: Option<&str>, out: &mut Vec<PriceTick>) {
        match v {
            Value::Array(items) => {
                for item in items {
                    Self::collect_ticks(item, inherited_symbol, out);
                }
            }
            Value::Object(map) => {
                let local_symbol = map
                    .get("symbol")
                    .or_else(|| map.get("asset_id"))
                    .or_else(|| map.get("assetId"))
                    .or_else(|| map.get("asset"))
                    .and_then(|vv| vv.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| inherited_symbol.map(str::to_string));

                let price = val_as_f64(
                    map.get("price")
                        .or_else(|| map.get("px"))
                        .or_else(|| map.get("value")),
                );
                let value = val_as_f64(map.get("value"));
                let timestamp_ms = val_as_i64_ms(
                    map.get("timestamp_ms")
                        .or_else(|| map.get("timestampMs"))
                        .or_else(|| map.get("timestamp"))
                        .or_else(|| map.get("ts"))
                        .or_else(|| map.get("time"))
                        .or_else(|| map.get("updated_at")),
                );

                if let (Some(price), Some(timestamp_ms)) = (price, timestamp_ms) {
                    let symbol_raw = local_symbol.clone().unwrap_or_default();
                    let symbol = if symbol_raw.is_empty() {
                        String::new()
                    } else {
                        to_symbol(&symbol_raw)
                    };
                    let asset_id = if symbol_raw.is_empty() {
                        String::new()
                    } else {
                        to_asset_id(&symbol_raw)
                    };
                    out.push(PriceTick {
                        symbol,
                        asset_id,
                        price,
                        value,
                        timestamp_ms,
                    });
                }

                for child in map.values() {
                    match child {
                        Value::Array(_) | Value::Object(_) => {
                            Self::collect_ticks(child, local_symbol.as_deref(), out)
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn load_state_file(path: &Path) -> ResolutionStateFile {
        let raw = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(_) => return ResolutionStateFile::default(),
        };
        serde_json::from_str::<ResolutionStateFile>(&raw).unwrap_or_default()
    }

    fn save_state_file(path: &Path, state: &ResolutionStateFile) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(state)?;
        fs::write(path, raw)?;
        Ok(())
    }

    fn load_price_to_beat_state(path: &Path) -> Option<PriceToBeatStateFile> {
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str::<PriceToBeatStateFile>(&raw).ok()
    }

    fn save_price_to_beat_state(path: &Path, state: &PriceToBeatStateFile) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(state)?;
        fs::write(path, raw)?;
        Ok(())
    }

    fn find_previous_snapshot(
        state: &ResolutionStateFile,
        symbol: &str,
        current_slug: &str,
        current_resolution_ts_ms: i64,
    ) -> Option<ResolutionSnapshot> {
        let mut rows: Vec<ResolutionSnapshot> = state
            .records
            .iter()
            .filter(|r| r.symbol == symbol && r.market_slug != current_slug)
            .cloned()
            .collect();
        rows.sort_by_key(|r| r.resolution_ts_ms);
        rows.iter()
            .rev()
            .find(|r| r.resolution_ts_ms <= current_resolution_ts_ms)
            .cloned()
            .or_else(|| rows.last().cloned())
            .or_else(|| {
                state
                    .last_by_symbol
                    .get(symbol)
                    .filter(|r| r.market_slug != current_slug)
                    .cloned()
            })
    }

    fn persist_resolution_snapshot(&self) -> Result<Option<ResolutionSnapshot>> {
        let (chosen_tick, mode, price_to_beat) = {
            let mut rt = self
                .runtime
                .lock()
                .map_err(|_| anyhow!("runtime lock poisoned"))?;
            if rt.finalized {
                return Ok(None);
            }
            let chosen = if let Some(t) = rt.first_after_resolution.clone() {
                (Some(t), "first_after_resolution".to_string())
            } else if let Some(t) = rt.before_resolution.clone() {
                self.logger.warning(&format!(
                    "[RTDS] fallback persist market={} symbol={} reason=missing_post_resolution_tick source_ts_ms={} mode=last_before_resolution_fallback",
                    self.market_slug, self.symbol, t.timestamp_ms
                ));
                (Some(t), "last_before_resolution_fallback".to_string())
            } else {
                self.logger.warning(&format!(
                    "[RTDS] skip persist market={} symbol={} reason=missing_post_resolution_tick_and_pre_resolution_tick",
                    self.market_slug, self.symbol
                ));
                rt.finalized = true;
                return Ok(None);
            };
            rt.finalized = true;
            (chosen.0, chosen.1, rt.price_to_beat)
        };

        let chosen_tick = match chosen_tick {
            Some(v) => v,
            None => {
                self.logger.warning(&format!(
                    "[RTDS] no tick captured for market={} symbol={} (cannot persist resolution)",
                    self.market_slug, self.symbol
                ));
                return Ok(None);
            }
        };
        let diff = price_to_beat.map(|p| chosen_tick.price - p);
        let diff_percentage_value = diff_percentage(chosen_tick.price, price_to_beat);
        let snapshot = ResolutionSnapshot {
            market_slug: self.market_slug.clone(),
            symbol: self.symbol.clone(),
            asset_id: self.asset_id.clone(),
            resolution_ts_ms: self.resolution_ts_ms,
            source_ts_ms: chosen_tick.timestamp_ms,
            resolution_price: chosen_tick.price,
            resolution_value: chosen_tick.value,
            capture_mode: mode,
            price_to_beat,
            diff_vs_price_to_beat: diff,
            diff_vs_price_to_beat_percentage: diff_percentage_value,
            captured_at_ms: now_ms(),
        };

        let mut state_version = 1i64;
        let mut state_updated_at_ms = now_ms();
        if self.persist_state_to_file {
            let mut state = Self::load_state_file(&self.state_path);
            state.version = 1;
            state.updated_at_ms = now_ms();
            if let Some(existing) = state
                .records
                .iter_mut()
                .find(|r| r.market_slug == snapshot.market_slug && r.symbol == snapshot.symbol)
            {
                *existing = snapshot.clone();
            } else {
                state.records.push(snapshot.clone());
            }
            state.records.sort_by_key(|r| r.resolution_ts_ms);
            if state.records.len() > self.max_records {
                let keep_from = state.records.len().saturating_sub(self.max_records);
                state.records = state.records[keep_from..].to_vec();
            }
            state
                .last_by_symbol
                .insert(self.symbol.clone(), snapshot.clone());
            Self::save_state_file(&self.state_path, &state)?;
            state_version = state.version;
            state_updated_at_ms = state.updated_at_ms;
        }

        self.logger.info(&format!(
            "[RTDS] persisted resolution market={} symbol={} price={:.6} source_ts_ms={} mode={} diff_vs_price_to_beat={} diff_vs_price_to_beat_percentage={}",
            snapshot.market_slug,
            snapshot.symbol,
            snapshot.resolution_price,
            snapshot.source_ts_ms,
            snapshot.capture_mode,
            snapshot
                .diff_vs_price_to_beat
                .map(|d| format!("{d:+.6}"))
                .unwrap_or_else(|| "-".to_string()),
            snapshot
                .diff_vs_price_to_beat_percentage
                .map(|d| format!("{d:+.6}%"))
                .unwrap_or_else(|| "-".to_string())
        ));
        let ptb_state = PriceToBeatStateFile {
            market_slug: self.market_slug.clone(),
            price_to_beat: snapshot.price_to_beat,
            updated_at_ms: now_ms(),
        };
        if self.persist_state_to_file {
            let _ = Self::save_price_to_beat_state(&self.price_to_beat_state_path, &ptb_state);
        }
        if let Some(ch) = &self.clickhouse_sink {
            ch.insert_resolution_best_effort(&snapshot, state_version, state_updated_at_ms);
            ch.insert_price_to_beat_best_effort(&ptb_state);
        }
        Ok(Some(snapshot))
    }
}
