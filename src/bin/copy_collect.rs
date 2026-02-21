use anyhow::{anyhow, Result};
use chrono::DateTime;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn now_s() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn env_first(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(v) = env::var(key) {
            let vv = v.trim();
            if !vv.is_empty() {
                return Some(vv.to_string());
            }
        }
    }
    None
}

fn env_bool(keys: &[&str], default: bool) -> bool {
    let raw = match env_first(keys) {
        Some(v) => v,
        None => return default,
    };
    matches!(raw.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "y" | "on")
}

fn env_f64(keys: &[&str], default: f64) -> f64 {
    env_first(keys)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(default)
}

fn env_i64(keys: &[&str], default: i64) -> i64 {
    env_first(keys)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

fn parse_csv_set(keys: &[&str]) -> HashSet<String> {
    env_first(keys)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_csv_vec(keys: &[&str]) -> Vec<String> {
    env_first(keys)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn as_f64(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn as_i64_ms(v: Option<&Value>) -> Option<i64> {
    as_i64_ms_with_precision(v).0
}

fn as_i64_ms_with_precision(v: Option<&Value>) -> (Option<i64>, &'static str) {
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
            } else if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
                (Some(dt.timestamp_millis()), "ms")
            } else {
                (None, "none")
            }
        }
        _ => (None, "none"),
    }
}

fn val_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn to_asset_id(s: &str) -> String {
    let mut out = s.trim().to_ascii_lowercase();
    if out.contains('/') {
        out = out.split('/').next().unwrap_or("").to_string();
    } else if out.contains('-') {
        out = out.split('-').next().unwrap_or("").to_string();
    } else if out.ends_with("usdt") && out.len() > 4 {
        out = out[..out.len() - 4].to_string();
    } else if out.ends_with("usd") && out.len() > 3 {
        out = out[..out.len() - 3].to_string();
    }
    match out.as_str() {
        "bitcoin" => "btc".to_string(),
        "ethereum" => "eth".to_string(),
        "solana" => "sol".to_string(),
        "ripple" => "xrp".to_string(),
        "dogecoin" => "doge".to_string(),
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

fn infer_symbol_from_text(s: &str) -> Option<String> {
    let t = s.to_ascii_lowercase();
    let aid = if t.contains("bitcoin") || t.contains("btc") {
        "btc"
    } else if t.contains("ethereum") || t.contains("eth") {
        "eth"
    } else if t.contains("solana") || t.contains("sol") {
        "sol"
    } else if t.contains("xrp") || t.contains("ripple") {
        "xrp"
    } else if t.contains("doge") || t.contains("dogecoin") {
        "doge"
    } else if t.contains("matic") || t.contains("polygon") {
        "matic"
    } else {
        ""
    };
    (!aid.is_empty()).then(|| format!("{aid}/usd"))
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
    let symbol = to_symbol(&head);
    if symbol.is_empty() {
        None
    } else {
        Some(symbol)
    }
}

fn normalize_direction(outcome: &str) -> Option<String> {
    match outcome.trim().to_ascii_uppercase().as_str() {
        "YES" | "UP" | "LONG" | "BUY" | "BULL" => Some("YES".to_string()),
        "NO" | "DOWN" | "SHORT" | "SELL" | "BEAR" => Some("NO".to_string()),
        _ => None,
    }
}

fn flip_direction(direction: &str) -> String {
    if matches!(direction.trim().to_ascii_uppercase().as_str(), "YES" | "UP") {
        "NO".to_string()
    } else {
        "YES".to_string()
    }
}

#[derive(Debug, Clone)]
struct PricePoint {
    topic: String,
    symbol: String,
    value: f64,
    timestamp_ms: i64,
    recv_ms: i64,
}

#[derive(Debug, Clone)]
struct TradeEvent {
    wallet: String,
    market_slug: String,
    event_slug: String,
    title: String,
    side: String,
    outcome: String,
    direction: String,
    price: f64,
    size: f64,
    trade_ts_sec: i64,
    tx_hash: String,
}

#[derive(Debug, Clone)]
struct RfqEvent {
    event_type: String,
    request_id: String,
    quote_id: String,
    proxy_address: String,
    market: String,
    condition: String,
    token: String,
    complement: String,
    state: String,
    side: String,
    size_in: Option<f64>,
    size_out: Option<f64>,
    price: Option<f64>,
    expiry_ts_sec: Option<i64>,
    payload_timestamp_ms: Option<i64>,
    ws_timestamp_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct MarketTokenPair {
    up_asset_id: String,
    down_asset_id: String,
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

#[derive(Debug, Clone)]
struct ClobMatchedTopOfBook {
    sample: ClobTopOfBook,
    match_mode: String,
    match_delta_ms: Option<i64>,
}

#[derive(Debug, Default)]
struct ClobStore {
    latest_by_asset: HashMap<String, ClobTopOfBook>,
    history_by_asset: HashMap<String, VecDeque<ClobTopOfBook>>,
}

#[derive(Clone)]
struct ClobFeedShared {
    store: Arc<Mutex<ClobStore>>,
    desired_assets: Arc<Mutex<HashSet<String>>>,
}

impl ClobFeedShared {
    fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(ClobStore::default())),
            desired_assets: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[derive(Clone)]
struct CollectorConfig {
    ws_url: String,
    out_path: PathBuf,
    reconnect_min: f64,
    reconnect_max: f64,
    ping_interval: f64,
    read_timeout: f64,
    ws_debug: bool,
    log_raw: bool,
    include_price_ticks: bool,
    include_rfq: bool,
    max_trades: i64,
    run_seconds: f64,
    wallet_filters: HashSet<String>,
    market_slug_filters: HashSet<String>,
    event_slug_filters: HashSet<String>,
    buy_only: bool,
    min_trade_size: f64,
    price_topics: Vec<String>,
    price_symbols: Vec<String>,
    rfq_types: HashSet<String>,
    clob_join_enabled: bool,
    clob_ws_url: String,
    clob_ws_reconnect_min: f64,
    clob_ws_reconnect_max: f64,
    clob_ws_ping_interval: f64,
    clob_ws_read_timeout: f64,
    clob_match_max_age_ms: i64,
    clob_history_max_records: usize,
    clob_history_max_age_ms: i64,
    gamma_api_url: String,
    gamma_timeout_seconds: f64,
}

impl CollectorConfig {
    fn default_rfq_types() -> HashSet<String> {
        [
            "request_created",
            "request_edited",
            "request_canceled",
            "request_expired",
            "quote_created",
            "quote_edited",
            "quote_canceled",
            "quote_expired",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn from_env() -> Self {
        let ws_url = env_first(&["COPY_COLLECT_WS_URL", "SIGNAL_WS_URL", "RTDS_WS_URL"])
            .unwrap_or_else(|| "wss://ws-live-data.polymarket.com".to_string());
        let out_path = PathBuf::from(
            env_first(&["COPY_COLLECT_OUT_PATH"])
                .unwrap_or_else(|| "state/copy_collect.jsonl".to_string()),
        );
        let mut price_topics = parse_csv_vec(&["COPY_COLLECT_PRICE_TOPICS"]);
        if price_topics.is_empty() {
            price_topics = vec![
                "crypto_prices_chainlink".to_string(),
                "crypto_prices".to_string(),
            ];
        }
        let mut price_symbols = parse_csv_vec(&["COPY_COLLECT_PRICE_SYMBOLS", "RTDS_SYMBOL"])
            .into_iter()
            .map(|s| to_symbol(&s))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if price_symbols.is_empty() {
            if let Some(slug) = env_first(&["MARKET_SLUG", "SIGNAL_COPY_MARKET_SLUG"]) {
                if let Some(sym) = infer_symbol_from_slug(&slug) {
                    price_symbols.push(sym);
                }
            }
        }
        price_symbols.sort();
        price_symbols.dedup();
        let mut rfq_types = parse_csv_set(&["COPY_COLLECT_RFQ_TYPES"]);
        if rfq_types.is_empty() {
            rfq_types = Self::default_rfq_types();
        }
        let default_ws_base = env_first(&["WS_BASE"])
            .unwrap_or_else(|| "wss://ws-subscriptions-clob.polymarket.com".to_string());
        let clob_ws_url = env_first(&["COPY_COLLECT_CLOB_WS_URL"])
            .unwrap_or_else(|| format!("{}/ws/market", default_ws_base.trim_end_matches('/')));
        let gamma_api_url = env_first(&["COPY_COLLECT_GAMMA_API_URL"])
            .unwrap_or_else(|| "https://gamma-api.polymarket.com".to_string());

        Self {
            ws_url,
            out_path,
            reconnect_min: env_f64(&["COPY_COLLECT_RECONNECT_MIN", "SIGNAL_WS_RECONNECT_MIN"], 0.5)
                .max(0.1),
            reconnect_max: env_f64(&["COPY_COLLECT_RECONNECT_MAX", "SIGNAL_WS_RECONNECT_MAX"], 8.0)
                .max(0.5),
            ping_interval: env_f64(&["COPY_COLLECT_PING_INTERVAL", "SIGNAL_WS_PING_INTERVAL"], 5.0)
                .max(0.0),
            read_timeout: env_f64(
                &["COPY_COLLECT_READ_TIMEOUT_SECONDS", "RTDS_WS_READ_TIMEOUT_SECONDS"],
                0.25,
            )
            .max(0.05),
            ws_debug: env_bool(&["COPY_COLLECT_WS_DEBUG", "SIGNAL_WS_DEBUG"], false),
            log_raw: env_bool(&["COPY_COLLECT_LOG_RAW"], false),
            include_price_ticks: env_bool(&["COPY_COLLECT_LOG_PRICE_TICKS"], true),
            include_rfq: env_bool(&["COPY_COLLECT_INCLUDE_RFQ"], true),
            max_trades: env_i64(&["COPY_COLLECT_MAX_TRADES"], 0),
            run_seconds: env_f64(&["COPY_COLLECT_RUN_SECONDS"], 0.0).max(0.0),
            wallet_filters: parse_csv_set(&["SIGNAL_COPY_WALLETS", "COPYTRADE_WALLETS"]),
            market_slug_filters: parse_csv_set(&[
                "SIGNAL_COPY_MARKET_SLUGS",
                "COPYTRADE_MARKET_SLUGS",
            ]),
            event_slug_filters: parse_csv_set(&[
                "SIGNAL_COPY_EVENT_SLUGS",
                "COPYTRADE_EVENT_SLUGS",
            ]),
            buy_only: env_bool(&["SIGNAL_COPY_BUY_ONLY", "COPYTRADE_BUY_ONLY"], false),
            min_trade_size: env_f64(&["SIGNAL_COPY_MIN_SIZE", "COPYTRADE_MIN_SIZE"], 0.0)
                .max(0.0),
            price_topics,
            price_symbols,
            rfq_types,
            clob_join_enabled: env_bool(&["COPY_COLLECT_CLOB_JOIN_ENABLED"], true),
            clob_ws_url,
            clob_ws_reconnect_min: env_f64(&["COPY_COLLECT_CLOB_WS_RECONNECT_MIN"], 0.5).max(0.1),
            clob_ws_reconnect_max: env_f64(&["COPY_COLLECT_CLOB_WS_RECONNECT_MAX"], 8.0).max(0.5),
            clob_ws_ping_interval: env_f64(&["COPY_COLLECT_CLOB_WS_PING_INTERVAL"], 5.0).max(0.0),
            clob_ws_read_timeout: env_f64(&["COPY_COLLECT_CLOB_WS_READ_TIMEOUT_SECONDS"], 0.25)
                .max(0.05),
            clob_match_max_age_ms: env_i64(&["COPY_COLLECT_CLOB_MATCH_MAX_AGE_MS"], 2500).max(100),
            clob_history_max_records: env_i64(&["COPY_COLLECT_CLOB_HISTORY_MAX_RECORDS"], 800)
                .max(32) as usize,
            clob_history_max_age_ms: env_i64(&["COPY_COLLECT_CLOB_HISTORY_MAX_AGE_MS"], 45_000)
                .max(1000),
            gamma_api_url,
            gamma_timeout_seconds: env_f64(&["COPY_COLLECT_GAMMA_TIMEOUT_SECONDS"], 10.0).max(0.5),
        }
    }
}

struct JsonlWriter {
    path: PathBuf,
}

impl JsonlWriter {
    fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self { path })
    }

    fn append(&self, obj: &Value) -> Result<()> {
        let line = serde_json::to_string(obj)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }
}

struct ClobWorkerGuard {
    stop_event: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ClobWorkerGuard {
    fn none() -> Self {
        Self {
            stop_event: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    fn new(stop_event: Arc<AtomicBool>, handle: thread::JoinHandle<()>) -> Self {
        Self {
            stop_event,
            handle: Some(handle),
        }
    }
}

impl Drop for ClobWorkerGuard {
    fn drop(&mut self) {
        self.stop_event.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn configure_socket_timeouts(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>, timeout_s: f64) {
    let timeout = Some(Duration::from_secs_f64(timeout_s.max(0.05)));
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

fn build_subscribe_payload(cfg: &CollectorConfig) -> Value {
    let mut subscriptions = vec![];

    let mut trade_sub = json!({
        "topic": "activity",
        "type": "trades",
    });
    if cfg.market_slug_filters.len() == 1 {
        if let Some(slug) = cfg.market_slug_filters.iter().next() {
            if let Some(obj) = trade_sub.as_object_mut() {
                obj.insert(
                    "filters".to_string(),
                    Value::String(json!({"market_slug": slug}).to_string()),
                );
            }
        }
    } else if cfg.event_slug_filters.len() == 1 {
        if let Some(slug) = cfg.event_slug_filters.iter().next() {
            if let Some(obj) = trade_sub.as_object_mut() {
                obj.insert(
                    "filters".to_string(),
                    Value::String(json!({"event_slug": slug}).to_string()),
                );
            }
        }
    }
    subscriptions.push(trade_sub);

    for topic in &cfg.price_topics {
        if cfg.price_symbols.is_empty() {
            subscriptions.push(json!({
                "topic": topic,
                "type": "update",
            }));
        } else {
            for symbol in &cfg.price_symbols {
                subscriptions.push(json!({
                    "topic": topic,
                    "type": "update",
                    "filters": json!({"symbol": symbol}).to_string(),
                }));
            }
        }
    }

    if cfg.include_rfq {
        let mut rfq_types: Vec<String> = cfg.rfq_types.iter().cloned().collect();
        rfq_types.sort();
        for rfq_type in rfq_types {
            subscriptions.push(json!({
                "topic": "rfq",
                "type": rfq_type,
            }));
        }
    }

    json!({
        "action": "subscribe",
        "subscriptions": subscriptions,
    })
}

fn maybe_json_list(v: &Value) -> Value {
    match v {
        Value::Array(_) => v.clone(),
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| v.clone()),
        _ => v.clone(),
    }
}

fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

fn parse_market_token_pair(market: &Value) -> Option<MarketTokenPair> {
    let clob_ids_raw = market
        .get("clobTokenIds")
        .or_else(|| market.get("clob_token_ids"))
        .or_else(|| market.get("clobTokenIDs"))?;
    let clob_ids_val = maybe_json_list(clob_ids_raw);
    let clob_ids = clob_ids_val.as_array()?;
    if clob_ids.len() < 2 {
        return None;
    }

    let outcomes_raw = market.get("outcomes");
    let outcomes = match outcomes_raw {
        Some(Value::Array(v)) => Some(v.clone()),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .ok()
            .and_then(|v| v.as_array().cloned()),
        _ => None,
    };

    let mut up_i: Option<usize> = None;
    let mut down_i: Option<usize> = None;
    if let Some(outcomes) = outcomes {
        if outcomes.len() == clob_ids.len() {
            for (i, o) in outcomes.iter().enumerate() {
                if let Some(name) = o.as_str() {
                    let n = norm(name);
                    if n == "yes" || n == "up" {
                        up_i = Some(i);
                    }
                    if n == "no" || n == "down" {
                        down_i = Some(i);
                    }
                }
            }
        }
    }

    let ui = up_i.unwrap_or(0);
    let di = down_i.unwrap_or(1);
    let up_asset_id = clob_ids
        .get(ui)
        .map(|v| v.to_string().trim_matches('"').to_string())
        .unwrap_or_default();
    let down_asset_id = clob_ids
        .get(di)
        .map(|v| v.to_string().trim_matches('"').to_string())
        .unwrap_or_default();
    if up_asset_id.trim().is_empty() || down_asset_id.trim().is_empty() {
        return None;
    }
    Some(MarketTokenPair {
        up_asset_id,
        down_asset_id,
    })
}

fn fetch_market_token_pair(cfg: &CollectorConfig, slug: &str) -> Option<MarketTokenPair> {
    if slug.trim().is_empty() {
        return None;
    }
    let client = Client::builder()
        .timeout(Duration::from_secs_f64(cfg.gamma_timeout_seconds.max(0.5)))
        .build()
        .ok()?;
    let url = format!("{}/markets", cfg.gamma_api_url.trim_end_matches('/'));
    let resp = client
        .get(url)
        .query(&[("slug", slug.trim())])
        .send()
        .ok()?;
    let data = resp.json::<Value>().ok()?;
    let arr = data.as_array()?;
    let market = arr.first()?;
    parse_market_token_pair(market)
}

fn get_market_token_pair_cached(
    cfg: &CollectorConfig,
    slug: &str,
    market_pair_cache: &mut HashMap<String, Option<MarketTokenPair>>,
    clob_feed: Option<&ClobFeedShared>,
) -> Option<MarketTokenPair> {
    let key = slug.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    if let Some(v) = market_pair_cache.get(&key) {
        return v.clone();
    }
    let pair = fetch_market_token_pair(cfg, &key);
    if let (Some(feed), Some(p)) = (clob_feed, pair.as_ref()) {
        if let Ok(mut aset) = feed.desired_assets.lock() {
            aset.insert(p.up_asset_id.clone());
            aset.insert(p.down_asset_id.clone());
        }
    }
    market_pair_cache.insert(key, pair.clone());
    pair
}

fn upsert_clob_top_of_book(cfg: &CollectorConfig, feed: &ClobFeedShared, snapshot: ClobTopOfBook) {
    if let Ok(mut store) = feed.store.lock() {
        let queue = store
            .history_by_asset
            .entry(snapshot.asset_id.clone())
            .or_insert_with(VecDeque::new);
        queue.push_back(snapshot.clone());
        while queue.len() > cfg.clob_history_max_records {
            let _ = queue.pop_front();
        }
        while queue
            .front()
            .map(|s| snapshot.recv_ts_ms - s.recv_ts_ms > cfg.clob_history_max_age_ms)
            .unwrap_or(false)
        {
            let _ = queue.pop_front();
        }
        store.latest_by_asset.insert(snapshot.asset_id.clone(), snapshot);
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
                    let replace = best_s
                        .as_ref()
                        .map(|(d, _)| delta_s < *d)
                        .unwrap_or(true);
                    if replace {
                        best_s = Some((delta_s, sample.clone()));
                    }
                } else {
                    let delta_ms = (target_ts_ms - exchange_ts_ms).abs();
                    let replace = best_ms
                        .as_ref()
                        .map(|(d, _)| delta_ms < *d)
                        .unwrap_or(true);
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

fn match_clob_for_trade(
    cfg: &CollectorConfig,
    feed: &ClobFeedShared,
    pair: &MarketTokenPair,
    target_ts_ms: i64,
) -> (Option<ClobMatchedTopOfBook>, Option<ClobMatchedTopOfBook>) {
    let store = match feed.store.lock() {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let up = pick_best_clob_snapshot(
        store.history_by_asset.get(&pair.up_asset_id),
        store.latest_by_asset.get(&pair.up_asset_id),
        target_ts_ms,
        cfg.clob_match_max_age_ms,
    );
    let down = pick_best_clob_snapshot(
        store.history_by_asset.get(&pair.down_asset_id),
        store.latest_by_asset.get(&pair.down_asset_id),
        target_ts_ms,
        cfg.clob_match_max_age_ms,
    );
    (up, down)
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

fn apply_clob_join_to_row(
    cfg: &CollectorConfig,
    row: &mut Value,
    clob_feed: Option<&ClobFeedShared>,
    pair: Option<&MarketTokenPair>,
    target_ts_ms: i64,
) {
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    obj.insert("clob_join_target_ts_ms".to_string(), json!(target_ts_ms));
    if let (Some(feed), Some(pair)) = (clob_feed, pair) {
        let (up, down) = match_clob_for_trade(cfg, feed, pair, target_ts_ms);
        insert_clob_fields(obj, "clob_up", up);
        insert_clob_fields(obj, "clob_down", down);
        return;
    }
    insert_clob_fields(obj, "clob_up", None);
    insert_clob_fields(obj, "clob_down", None);
}

fn process_clob_event(cfg: &CollectorConfig, feed: &ClobFeedShared, msg: &Value) {
    let et = val_str(msg.get("event_type").or_else(|| msg.get("type"))).to_ascii_lowercase();
    if !et.is_empty() && et != "best_bid_ask" {
        return;
    }
    let asset_id = val_str(
        msg.get("asset_id")
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("asset"))
            .or_else(|| msg.get("token")),
    );
    if asset_id.trim().is_empty() {
        return;
    }
    let best_bid_price = as_f64(
        msg.get("best_bid_price")
            .or_else(|| msg.get("best_bid"))
            .or_else(|| msg.get("bid"))
            .or_else(|| msg.get("b")),
    );
    let best_ask_price = as_f64(
        msg.get("best_ask_price")
            .or_else(|| msg.get("best_ask"))
            .or_else(|| msg.get("ask"))
            .or_else(|| msg.get("a")),
    );
    if best_bid_price.is_none() && best_ask_price.is_none() {
        return;
    }
    let (exchange_ts_ms, exchange_ts_precision) = as_i64_ms_with_precision(
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
    upsert_clob_top_of_book(
        cfg,
        feed,
        ClobTopOfBook {
            asset_id,
            best_bid_price,
            best_ask_price,
            mid_price,
            spread,
            exchange_ts_ms,
            exchange_ts_precision: exchange_ts_precision.to_string(),
            recv_ts_ms,
        },
    );
}

fn process_clob_json_value(cfg: &CollectorConfig, feed: &ClobFeedShared, v: &Value) {
    match v {
        Value::Array(items) => {
            for item in items {
                process_clob_json_value(cfg, feed, item);
            }
        }
        Value::Object(map) => {
            process_clob_event(cfg, feed, v);
            if let Some(payload) = map.get("payload") {
                process_clob_json_value(cfg, feed, payload);
            }
            if let Some(data) = map.get("data") {
                process_clob_json_value(cfg, feed, data);
            }
            if let Some(events) = map.get("events") {
                process_clob_json_value(cfg, feed, events);
            }
        }
        _ => {}
    }
}

fn desired_assets_snapshot(feed: &ClobFeedShared) -> Vec<String> {
    let mut out: Vec<String> = feed
        .desired_assets
        .lock()
        .map(|s| s.iter().cloned().collect())
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

fn run_clob_ws_loop(cfg: CollectorConfig, feed: ClobFeedShared, stop_event: Arc<AtomicBool>) {
    let mut backoff = cfg.clob_ws_reconnect_min.max(0.1);

    while !stop_event.load(Ordering::SeqCst) {
        let desired_assets = desired_assets_snapshot(&feed);
        if desired_assets.is_empty() {
            thread::sleep(Duration::from_millis(250));
            continue;
        }

        let conn = connect(&cfg.clob_ws_url);
        let (mut ws, _) = match conn {
            Ok(v) => v,
            Err(e) => {
                let sleep_for = (backoff.min(cfg.clob_ws_reconnect_max))
                    * (0.7 + rand::random::<f64>() * 0.6);
                eprintln!("[copy_collect][clob] connect error: {e}; retry in {sleep_for:.2}s");
                thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                backoff = (backoff * 2.0).min(cfg.clob_ws_reconnect_max);
                continue;
            }
        };
        backoff = cfg.clob_ws_reconnect_min.max(0.1);
        configure_socket_timeouts(&mut ws, cfg.clob_ws_read_timeout);

        let sub = json!({
            "assets_ids": desired_assets.clone(),
            "type": "market",
            "custom_feature_enabled": true
        });
        if let Err(e) = ws.send(Message::Text(sub.to_string().into())) {
            eprintln!("[copy_collect][clob] subscribe error: {e}");
            let _ = ws.close(None);
            thread::sleep(Duration::from_secs_f64(backoff));
            continue;
        }
        let mut sent_assets = desired_assets.clone();
        println!(
            "[copy_collect][clob] connected and subscribed assets={}",
            sent_assets.len()
        );

        let mut last_ping = Instant::now();
        loop {
            if stop_event.load(Ordering::SeqCst) {
                let _ = ws.close(None);
                return;
            }

            let current_assets = desired_assets_snapshot(&feed);
            if current_assets != sent_assets {
                if current_assets.is_empty() {
                    let _ = ws.close(None);
                    break;
                }
                let sub = json!({
                    "assets_ids": current_assets.clone(),
                    "type": "market",
                    "custom_feature_enabled": true
                });
                if let Err(e) = ws.send(Message::Text(sub.to_string().into())) {
                    eprintln!("[copy_collect][clob] resubscribe error: {e}");
                    break;
                }
                sent_assets = current_assets;
            }

            if cfg.clob_ws_ping_interval > 0.0
                && last_ping.elapsed() >= Duration::from_secs_f64(cfg.clob_ws_ping_interval)
            {
                let _ = ws.send(Message::Text("ping".into()));
                last_ping = Instant::now();
            }

            let msg = match ws.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    eprintln!("[copy_collect][clob] ws read error: {e}");
                    break;
                }
            };

            match msg {
                Message::Text(text) => {
                    let s = text.trim();
                    if s.eq_ignore_ascii_case("ping") {
                        let _ = ws.send(Message::Text("pong".into()));
                        continue;
                    }
                    if s.eq_ignore_ascii_case("pong") || s.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(s) {
                        process_clob_json_value(&cfg, &feed, &v);
                    }
                }
                Message::Binary(bin) => {
                    if let Ok(text) = String::from_utf8(bin.to_vec()) {
                        let s = text.trim();
                        if s.is_empty()
                            || s.eq_ignore_ascii_case("ping")
                            || s.eq_ignore_ascii_case("pong")
                        {
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<Value>(s) {
                            process_clob_json_value(&cfg, &feed, &v);
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        let sleep_for = (backoff.min(cfg.clob_ws_reconnect_max)) * (0.7 + rand::random::<f64>() * 0.6);
        eprintln!("[copy_collect][clob] reconnecting in {sleep_for:.2}s");
        thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
        backoff = (backoff * 2.0).min(cfg.clob_ws_reconnect_max);
    }
}

fn extract_price_points(msg: &Value, recv_ms: i64) -> Vec<PricePoint> {
    let topic = val_str(msg.get("topic")).to_ascii_lowercase();
    if !(topic.starts_with("crypto_prices") || topic.starts_with("equity_prices")) {
        return vec![];
    }

    let payload = match msg.get("payload") {
        Some(v) => v,
        None => return vec![],
    };

    let mut out = Vec::<PricePoint>::new();
    if let Some(obj) = payload.as_object() {
        let base_symbol = to_symbol(&val_str(obj.get("symbol")));
        if let Some(v) = as_f64(obj.get("value").or_else(|| obj.get("price"))) {
            let ts = as_i64_ms(obj.get("timestamp").or_else(|| obj.get("ts"))).unwrap_or(recv_ms);
            if !base_symbol.is_empty() {
                out.push(PricePoint {
                    topic: topic.clone(),
                    symbol: base_symbol.clone(),
                    value: v,
                    timestamp_ms: ts,
                    recv_ms,
                });
            }
        }

        if let Some(arr) = obj.get("data").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(m) = item.as_object() {
                    let sym = to_symbol(&val_str(m.get("symbol")));
                    let symbol = if sym.is_empty() {
                        base_symbol.clone()
                    } else {
                        sym
                    };
                    let v = as_f64(m.get("value").or_else(|| m.get("price")));
                    if symbol.is_empty() || v.is_none() {
                        continue;
                    }
                    let ts = as_i64_ms(m.get("timestamp").or_else(|| m.get("ts"))).unwrap_or(recv_ms);
                    out.push(PricePoint {
                        topic: topic.clone(),
                        symbol,
                        value: v.unwrap_or(0.0),
                        timestamp_ms: ts,
                        recv_ms,
                    });
                }
            }
        }
    }
    out
}

fn extract_trade(msg: &Value) -> Option<TradeEvent> {
    let topic = val_str(msg.get("topic")).to_ascii_lowercase();
    let typ = val_str(msg.get("type")).to_ascii_lowercase();
    if topic != "activity" || typ != "trades" {
        return None;
    }

    let payload = msg.get("payload")?;
    if !payload.is_object() {
        return None;
    }
    let wallet = val_str(
        payload
            .get("proxyWallet")
            .or_else(|| payload.get("proxy_wallet"))
            .or_else(|| payload.get("wallet")),
    )
    .to_ascii_lowercase();
    if wallet.is_empty() {
        return None;
    }
    let market_slug = val_str(
        payload
            .get("slug")
            .or_else(|| payload.get("market_slug"))
            .or_else(|| payload.get("marketSlug")),
    );
    let event_slug = val_str(payload.get("eventSlug").or_else(|| payload.get("event_slug")));
    let title = val_str(payload.get("title"));
    let side = val_str(payload.get("side")).to_ascii_uppercase();
    let outcome = val_str(payload.get("outcome"));
    let price = as_f64(payload.get("price")).unwrap_or(0.0);
    let size = as_f64(payload.get("size")).unwrap_or(0.0);
    let trade_ts_sec = as_i64_ms(payload.get("timestamp"))
        .map(|ms| ms / 1000)
        .unwrap_or(0);
    let tx_hash = val_str(
        payload
            .get("transactionHash")
            .or_else(|| payload.get("transaction_hash"))
            .or_else(|| payload.get("txHash")),
    )
    .to_ascii_lowercase();

    if market_slug.is_empty() || side.is_empty() || size <= 0.0 {
        return None;
    }

    let direction = match (normalize_direction(&outcome), side.as_str()) {
        (Some(dir), "SELL") => flip_direction(&dir),
        (Some(dir), _) => dir,
        (None, "BUY") => normalize_direction("BUY").unwrap_or_else(|| "YES".to_string()),
        (None, "SELL") => normalize_direction("SELL").unwrap_or_else(|| "NO".to_string()),
        _ => return None,
    };

    Some(TradeEvent {
        wallet,
        market_slug,
        event_slug,
        title,
        side,
        outcome,
        direction,
        price,
        size,
        trade_ts_sec,
        tx_hash,
    })
}

fn extract_rfq(msg: &Value) -> Option<RfqEvent> {
    let topic = val_str(msg.get("topic")).to_ascii_lowercase();
    if topic != "rfq" {
        return None;
    }
    let event_type = val_str(msg.get("type")).to_ascii_lowercase();
    if event_type.is_empty() {
        return None;
    }
    let payload = msg.get("payload")?;
    if !payload.is_object() {
        return None;
    }

    let request_id = val_str(payload.get("requestId").or_else(|| payload.get("request_id")));
    let quote_id = val_str(payload.get("quoteId").or_else(|| payload.get("quote_id")));
    let proxy_address = val_str(
        payload
            .get("proxyAddress")
            .or_else(|| payload.get("proxy_address")),
    )
    .to_ascii_lowercase();
    let market = val_str(payload.get("market"));
    let condition = val_str(payload.get("condition"));
    let token = val_str(payload.get("token"));
    let complement = val_str(payload.get("complement"));
    let state = val_str(payload.get("state"));
    let side = val_str(payload.get("side")).to_ascii_uppercase();
    let size_in = as_f64(payload.get("sizeIn").or_else(|| payload.get("size_in")));
    let size_out = as_f64(payload.get("sizeOut").or_else(|| payload.get("size_out")));
    let price = as_f64(payload.get("price"));
    let expiry_ts_sec = as_i64_ms(payload.get("expiry")).map(|v| v / 1000);
    let payload_timestamp_ms = as_i64_ms(
        payload
            .get("timestamp")
            .or_else(|| payload.get("createdAt"))
            .or_else(|| payload.get("created_at"))
            .or_else(|| payload.get("updatedAt"))
            .or_else(|| payload.get("updated_at")),
    );
    let ws_timestamp_ms = as_i64_ms(msg.get("timestamp"));

    Some(RfqEvent {
        event_type,
        request_id,
        quote_id,
        proxy_address,
        market,
        condition,
        token,
        complement,
        state,
        side,
        size_in,
        size_out,
        price,
        expiry_ts_sec,
        payload_timestamp_ms,
        ws_timestamp_ms,
    })
}

fn trade_symbol_guess(trade: &TradeEvent) -> String {
    infer_symbol_from_slug(&trade.market_slug)
        .or_else(|| infer_symbol_from_text(&trade.event_slug))
        .or_else(|| infer_symbol_from_text(&trade.title))
        .unwrap_or_default()
}

fn pass_filters(cfg: &CollectorConfig, trade: &TradeEvent) -> bool {
    if !cfg.wallet_filters.is_empty() && !cfg.wallet_filters.contains(&trade.wallet) {
        return false;
    }
    if !cfg.market_slug_filters.is_empty()
        && !cfg
            .market_slug_filters
            .contains(&trade.market_slug.to_ascii_lowercase())
    {
        return false;
    }
    if !cfg.event_slug_filters.is_empty()
        && (trade.event_slug.is_empty()
            || !cfg
                .event_slug_filters
                .contains(&trade.event_slug.to_ascii_lowercase()))
    {
        return false;
    }
    if cfg.buy_only && trade.side != "BUY" {
        return false;
    }
    if cfg.min_trade_size > 0.0 && trade.size + 1e-12 < cfg.min_trade_size {
        return false;
    }
    true
}

fn process_data_message(
    msg_json: Value,
    cfg: &CollectorConfig,
    writer: &JsonlWriter,
    clob_feed: Option<&ClobFeedShared>,
    market_pair_cache: &mut HashMap<String, Option<MarketTokenPair>>,
    latest_price_by_symbol: &mut HashMap<String, PricePoint>,
    trades_logged: &mut i64,
    rfq_logged: &mut i64,
) -> Result<()> {
    let now = now_ms();

    if cfg.log_raw {
        let _ = writer.append(&json!({
            "kind": "raw",
            "ts_ms": now,
            "msg": msg_json,
        }));
    }

    for pp in extract_price_points(&msg_json, now) {
        latest_price_by_symbol.insert(pp.symbol.clone(), pp.clone());
        if cfg.include_price_ticks {
            let _ = writer.append(&json!({
                "kind": "price_tick",
                "collector_ts_ms": now,
                "topic": pp.topic,
                "symbol": pp.symbol,
                "value": pp.value,
                "price_ts_ms": pp.timestamp_ms,
                "price_age_ms": (now - pp.timestamp_ms).max(0),
            }));
        }
    }

    if cfg.include_rfq {
        if let Some(rfq) = extract_rfq(&msg_json) {
            if cfg.rfq_types.contains(&rfq.event_type) {
                let wallet_allowed = cfg.wallet_filters.is_empty()
                    || (!rfq.proxy_address.is_empty()
                        && cfg.wallet_filters.contains(&rfq.proxy_address));
                if wallet_allowed {
                    let row = json!({
                        "kind": "rfq_event",
                        "collector_ts_ms": now,
                        "rfq_event_type": rfq.event_type,
                        "request_id": rfq.request_id,
                        "quote_id": rfq.quote_id,
                        "proxy_address": rfq.proxy_address,
                        "market": rfq.market,
                        "condition": rfq.condition,
                        "token": rfq.token,
                        "complement": rfq.complement,
                        "state": rfq.state,
                        "side": rfq.side,
                        "size_in": rfq.size_in,
                        "size_out": rfq.size_out,
                        "price": rfq.price,
                        "expiry_ts_sec": rfq.expiry_ts_sec,
                        "expiry_ts_ms": rfq.expiry_ts_sec.map(|v| v * 1000),
                        "payload_timestamp_ms": rfq.payload_timestamp_ms,
                        "payload_timestamp_sec": rfq.payload_timestamp_ms.map(|v| v / 1000),
                        "ws_timestamp_ms": rfq.ws_timestamp_ms,
                        "ws_timestamp_sec": rfq.ws_timestamp_ms.map(|v| v / 1000),
                    });
                    writer.append(&row)?;
                    *rfq_logged += 1;
                    println!(
                        "[copy_collect] rfq#{} type={} req={} quote={} proxy={} ws_ts_ms={} expiry_ts_sec={}",
                        *rfq_logged,
                        row.get("rfq_event_type").and_then(|v| v.as_str()).unwrap_or(""),
                        row.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                        row.get("quote_id").and_then(|v| v.as_str()).unwrap_or(""),
                        row.get("proxy_address").and_then(|v| v.as_str()).unwrap_or(""),
                        row.get("ws_timestamp_ms").and_then(|v| v.as_i64()).unwrap_or(0),
                        row.get("expiry_ts_sec").and_then(|v| v.as_i64()).unwrap_or(0),
                    );
                }
            }
        }
    }

    if let Some(trade) = extract_trade(&msg_json) {
        if !pass_filters(cfg, &trade) {
            return Ok(());
        }
        let symbol = trade_symbol_guess(&trade);
        let latest = if symbol.is_empty() {
            None
        } else {
            latest_price_by_symbol.get(&symbol).cloned()
        };
        let rtds_price = latest.as_ref().map(|p| p.value);
        let rtds_ts_ms = latest.as_ref().map(|p| p.timestamp_ms);
        let rtds_age_ms = latest.as_ref().map(|p| (now - p.timestamp_ms).max(0));
        let rtds_recv_age_ms = latest.as_ref().map(|p| (now - p.recv_ms).max(0));
        let trade_ts_ms = if trade.trade_ts_sec > 0 {
            trade.trade_ts_sec * 1000
        } else {
            now
        };
        let market_pair =
            get_market_token_pair_cached(cfg, &trade.market_slug, market_pair_cache, clob_feed);

        let mut row = json!({
            "kind": "copy_trade",
            "collector_ts_ms": now,
            "wallet": trade.wallet,
            "market_slug": trade.market_slug,
            "event_slug": trade.event_slug,
            "title": trade.title,
            "symbol": symbol,
            "direction": trade.direction,
            "side": trade.side,
            "outcome": trade.outcome,
            "trade_price": trade.price,
            "trade_size": trade.size,
            "trade_ts_sec": trade.trade_ts_sec,
            "trade_ts_ms": trade_ts_ms,
            "tx_hash": trade.tx_hash,
            "rtds_price": rtds_price,
            "rtds_price_ts_ms": rtds_ts_ms,
            "rtds_price_age_ms": rtds_age_ms,
            "rtds_recv_age_ms": rtds_recv_age_ms,
        });
        apply_clob_join_to_row(
            cfg,
            &mut row,
            clob_feed,
            market_pair.as_ref(),
            trade_ts_ms,
        );
        writer.append(&row)?;
        *trades_logged += 1;
        println!(
            "[copy_collect] trade#{} wallet={} market={} dir={} trade_px={:.6} size={:.6} rtds_symbol={} rtds_px={}",
            *trades_logged,
            row.get("wallet").and_then(|v| v.as_str()).unwrap_or(""),
            row.get("market_slug").and_then(|v| v.as_str()).unwrap_or(""),
            row.get("direction").and_then(|v| v.as_str()).unwrap_or(""),
            row.get("trade_price").and_then(|v| v.as_f64()).unwrap_or(0.0),
            row.get("trade_size").and_then(|v| v.as_f64()).unwrap_or(0.0),
            row.get("symbol").and_then(|v| v.as_str()).unwrap_or(""),
            row.get("rtds_price")
                .and_then(|v| v.as_f64())
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "null".to_string()),
        );
    }
    Ok(())
}

fn run() -> Result<()> {
    install_rustls_crypto_provider();
    let cfg = CollectorConfig::from_env();
    let writer = JsonlWriter::new(cfg.out_path.clone())?;
    let subscribe = build_subscribe_payload(&cfg);
    let subscribe_text = subscribe.to_string();
    let clob_feed = if cfg.clob_join_enabled {
        Some(ClobFeedShared::new())
    } else {
        None
    };
    let _clob_worker = if let Some(feed) = clob_feed.clone() {
        let stop_event = Arc::new(AtomicBool::new(false));
        let cfg2 = cfg.clone();
        let feed2 = feed.clone();
        let stop2 = stop_event.clone();
        let handle = thread::spawn(move || run_clob_ws_loop(cfg2, feed2, stop2));
        ClobWorkerGuard::new(stop_event, handle)
    } else {
        ClobWorkerGuard::none()
    };

    println!(
        "[copy_collect] ws={} out={} wallets={} market_filters={} event_filters={} include_rfq={} price_topics={:?} price_symbols={:?}",
        cfg.ws_url,
        cfg.out_path.display(),
        cfg.wallet_filters.len(),
        cfg.market_slug_filters.len(),
        cfg.event_slug_filters.len(),
        cfg.include_rfq,
        cfg.price_topics,
        cfg.price_symbols
    );
    println!("[copy_collect] subscribe={subscribe_text}");

    writer.append(&json!({
        "kind": "collector_start",
        "ts_ms": now_ms(),
        "ws_url": cfg.ws_url,
        "clob_ws_url": cfg.clob_ws_url,
        "clob_join_enabled": cfg.clob_join_enabled,
        "out_path": cfg.out_path.to_string_lossy(),
        "subscribe": subscribe,
    }))?;

    let mut latest_price_by_symbol = HashMap::<String, PricePoint>::new();
    let mut market_pair_cache = HashMap::<String, Option<MarketTokenPair>>::new();
    let mut backoff = cfg.reconnect_min.max(0.1);
    let mut trades_logged: i64 = 0;
    let mut rfq_logged: i64 = 0;
    let start_ts = now_s();

    while cfg.max_trades <= 0 || trades_logged < cfg.max_trades {
        if cfg.run_seconds > 0.0 && (now_s() - start_ts) >= cfg.run_seconds {
            println!("[copy_collect] reached COPY_COLLECT_RUN_SECONDS={}", cfg.run_seconds);
            break;
        }

        let conn = connect(&cfg.ws_url);
        let (mut ws, _) = match conn {
            Ok(v) => v,
            Err(e) => {
                let sleep_for = (backoff.min(cfg.reconnect_max)) * (0.7 + rand::random::<f64>() * 0.6);
                eprintln!("[copy_collect] connect error: {e}; retry in {sleep_for:.2}s");
                thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                backoff = (backoff * 2.0).min(cfg.reconnect_max);
                continue;
            }
        };
        backoff = cfg.reconnect_min.max(0.1);

        configure_socket_timeouts(&mut ws, cfg.read_timeout);
        ws.send(Message::Text(subscribe_text.clone().into()))
            .map_err(|e| anyhow!("failed to send subscribe payload: {e}"))?;
        println!("[copy_collect] connected and subscribed");

        let mut last_ping = Instant::now();
        loop {
            if cfg.run_seconds > 0.0 && (now_s() - start_ts) >= cfg.run_seconds {
                println!("[copy_collect] reached COPY_COLLECT_RUN_SECONDS={}", cfg.run_seconds);
                let _ = ws.close(None);
                return Ok(());
            }
            if cfg.max_trades > 0 && trades_logged >= cfg.max_trades {
                println!("[copy_collect] reached COPY_COLLECT_MAX_TRADES={}", cfg.max_trades);
                let _ = ws.close(None);
                return Ok(());
            }

            if cfg.ping_interval > 0.0
                && last_ping.elapsed() >= Duration::from_secs_f64(cfg.ping_interval)
            {
                let _ = ws.send(Message::Text("ping".into()));
                last_ping = Instant::now();
            }

            let msg = match ws.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    eprintln!("[copy_collect] ws read error: {e}");
                    break;
                }
            };

            match msg {
                Message::Text(text) => {
                    let text_owned = text.to_string();
                    let s = text_owned.trim();
                    if s.eq_ignore_ascii_case("ping") {
                        let _ = ws.send(Message::Text("pong".into()));
                        continue;
                    }
                    if s.eq_ignore_ascii_case("pong") || s.is_empty() {
                        continue;
                    }
                    if cfg.ws_debug {
                        println!("[copy_collect] recv {s}");
                    }
                    let msg_json: Value = match serde_json::from_str(s) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("[copy_collect] parse error: {e}");
                            continue;
                        }
                    };
                    process_data_message(
                        msg_json,
                        &cfg,
                        &writer,
                        clob_feed.as_ref(),
                        &mut market_pair_cache,
                        &mut latest_price_by_symbol,
                        &mut trades_logged,
                        &mut rfq_logged,
                    )?;
                }
                Message::Binary(bin) => {
                    if let Ok(text) = String::from_utf8(bin.to_vec()) {
                        let s = text.trim();
                        if s.is_empty() || s.eq_ignore_ascii_case("ping") || s.eq_ignore_ascii_case("pong")
                        {
                            continue;
                        }
                        if cfg.ws_debug {
                            println!("[copy_collect] recv_bin {s}");
                        }
                        let msg_json: Value = match serde_json::from_str(s) {
                            Ok(v) => v,
                            Err(e) => {
                                eprintln!("[copy_collect] parse error (bin): {e}");
                                continue;
                            }
                        };
                        process_data_message(
                            msg_json,
                            &cfg,
                            &writer,
                            clob_feed.as_ref(),
                            &mut market_pair_cache,
                            &mut latest_price_by_symbol,
                            &mut trades_logged,
                            &mut rfq_logged,
                        )?;
                    }
                }
                Message::Close(frame) => {
                    let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(0);
                    let reason = frame
                        .as_ref()
                        .map(|f| f.reason.to_string())
                        .unwrap_or_default();
                    eprintln!("[copy_collect] ws closed code={code} reason={reason}");
                    break;
                }
                _ => {}
            }
        }

        let sleep_for = (backoff.min(cfg.reconnect_max)) * (0.7 + rand::random::<f64>() * 0.6);
        eprintln!("[copy_collect] reconnecting in {sleep_for:.2}s");
        thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
        backoff = (backoff * 2.0).min(cfg.reconnect_max);
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("fatal: {e:#}");
        std::process::exit(1);
    }
}
