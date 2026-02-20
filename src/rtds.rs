use crate::config::BotConfig;
use crate::env_utils::{env_bool, env_float, env_int};
use crate::gamma::fetch_market_by_slug;
use crate::helpers::iso_to_epoch;
use crate::logging::LogLike;
use crate::signal::JsonlFileService;
use anyhow::{anyhow, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
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

fn val_as_f64(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn val_as_i64_ms(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                if i > 1_000_000_000_000 {
                    Some(i)
                } else if i > 1_000_000_000 {
                    Some(i * 1000)
                } else {
                    None
                }
            } else if let Some(f) = n.as_f64() {
                if f > 1_000_000_000_000.0 {
                    Some(f as i64)
                } else if f > 1_000_000_000.0 {
                    Some((f * 1000.0) as i64)
                } else {
                    None
                }
            } else {
                None
            }
        }
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return None;
            }
            if let Ok(i) = t.parse::<i64>() {
                if i > 1_000_000_000_000 {
                    Some(i)
                } else if i > 1_000_000_000 {
                    Some(i * 1000)
                } else {
                    None
                }
            } else if let Ok(f) = t.parse::<f64>() {
                if f > 1_000_000_000_000.0 {
                    Some(f as i64)
                } else if f > 1_000_000_000.0 {
                    Some((f * 1000.0) as i64)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
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

#[derive(Debug, Clone, Default)]
struct RuntimeState {
    latest: Option<PriceTick>,
    before_resolution: Option<PriceTick>,
    first_after_resolution: Option<PriceTick>,
    price_to_beat: Option<f64>,
    finalized: bool,
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

pub struct RtdsService {
    market_slug: String,
    symbol: String,
    asset_id: String,
    resolution_ts_ms: i64,
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
    state_path: PathBuf,
    price_to_beat_state_path: PathBuf,
    latest_path: PathBuf,
    max_records: usize,
    tick_log: Option<Arc<JsonlFileService>>,
    runtime: Arc<Mutex<RuntimeState>>,
    stop_event: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    logger: Arc<dyn LogLike>,
}

impl RtdsService {
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
        let tick_log_enabled = env_bool("RTDS_LOG_TO_FILE", true) && !tick_log_path.is_empty();
        let tick_log = if tick_log_enabled {
            Some(Arc::new(JsonlFileService::new(tick_log_path, true)))
        } else {
            None
        };

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

        let _ = Self::save_price_to_beat_state(
            &price_to_beat_state_path,
            &PriceToBeatStateFile {
                market_slug: market_slug.to_string(),
                price_to_beat: runtime.price_to_beat,
                updated_at_ms: now_ms(),
            },
        );

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
            write_latest_file: env_bool("RTDS_WRITE_LATEST_FILE", true),
            state_path,
            price_to_beat_state_path,
            latest_path,
            max_records: env_int("RTDS_STATE_MAX_RECORDS", 2000).max(100) as usize,
            tick_log,
            runtime: Arc::new(Mutex::new(runtime)),
            stop_event: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
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
    }

    pub fn close(&self) {
        self.stop_event.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.thread.lock() {
            if let Some(handle) = slot.take() {
                let _ = handle.join();
            }
        }
        let _ = self.persist_resolution_snapshot();
        clear_live_snapshot(&self.market_slug);
    }

    fn run_loop(self: Arc<Self>) {
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

    fn configure_socket_timeouts(&self, ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
        let timeout = Some(Duration::from_secs_f64(self.read_timeout.max(0.1)));
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
            if tick.timestamp_ms <= self.resolution_ts_ms {
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
                    .map(|t| tick.timestamp_ms <= t.timestamp_ms)
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
        self.append_tick_log(&json!({
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
        }));
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
        if let Some(fs) = &self.tick_log {
            fs.append(obj);
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
            let chosen = if let Some(t) = rt.before_resolution.clone() {
                (Some(t), "before_resolution".to_string())
            } else if let Some(t) = rt.first_after_resolution.clone() {
                (Some(t), "first_after_resolution".to_string())
            } else if let Some(t) = rt.latest.clone() {
                (Some(t), "latest_seen".to_string())
            } else {
                (None, "none".to_string())
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
        let _ = Self::save_price_to_beat_state(
            &self.price_to_beat_state_path,
            &PriceToBeatStateFile {
                market_slug: self.market_slug.clone(),
                price_to_beat: snapshot.price_to_beat,
                updated_at_ms: now_ms(),
            },
        );
        Ok(Some(snapshot))
    }
}
