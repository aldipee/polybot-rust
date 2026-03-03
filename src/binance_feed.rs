use crate::env_utils::{env_float, env_int};
use crate::logging::LogLike;
use rand::Rng;
use reqwest::blocking::Client;
use serde_json::Value;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

fn normalize_asset_symbol(raw: &str) -> String {
    let mut s = raw.trim().to_ascii_lowercase();
    if let Some((head, _)) = s.split_once('/') {
        s = head.to_string();
    } else if let Some((head, _)) = s.split_once('-') {
        s = head.to_string();
    }
    if s.ends_with("usdt") && s.len() > 4 {
        s = s[..s.len() - 4].to_string();
    } else if s.ends_with("usd") && s.len() > 3 {
        s = s[..s.len() - 3].to_string();
    }
    match s.as_str() {
        "bitcoin" => "btc".to_string(),
        "ethereum" => "eth".to_string(),
        "solana" => "sol".to_string(),
        "ripple" => "xrp".to_string(),
        "polygon" => "matic".to_string(),
        _ => s,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinanceVenue {
    Global,
    Us,
}

impl BinanceVenue {
    fn from_env() -> Self {
        match std::env::var("SNIPER_BINANCE_VENUE")
            .unwrap_or_else(|_| "GLOBAL".to_string())
            .trim()
            .to_ascii_uppercase()
            .as_str()
        {
            "US" | "BINANCE_US" => Self::Us,
            _ => Self::Global,
        }
    }

    fn default_rest_base_url(&self) -> &'static str {
        match self {
            Self::Global => "https://api.binance.com",
            Self::Us => "https://api.binance.us",
        }
    }

    fn default_ws_base_url(&self) -> &'static str {
        match self {
            Self::Global => "wss://stream.binance.com:9443",
            Self::Us => "wss://stream.binance.us:9443",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BinanceFeedConfig {
    pub venue: BinanceVenue,
    pub rest_base_url: String,
    pub ws_base_url: String,
    pub symbol: String,
    pub symbol_asset: String,
    pub rest_timeout_seconds: f64,
    pub ws_reconnect_min: f64,
    pub ws_reconnect_max: f64,
    pub log_every_seconds: f64,
    pub warmup_limit: usize,
}

impl BinanceFeedConfig {
    pub fn from_env() -> Self {
        let venue = BinanceVenue::from_env();
        let mut rest_base_url = std::env::var("SNIPER_BINANCE_REST_BASE_URL")
            .unwrap_or_else(|_| venue.default_rest_base_url().to_string())
            .trim()
            .to_string();
        if rest_base_url.is_empty() {
            rest_base_url = venue.default_rest_base_url().to_string();
        }
        let mut ws_base_url = std::env::var("SNIPER_BINANCE_WS_BASE_URL")
            .unwrap_or_else(|_| venue.default_ws_base_url().to_string())
            .trim()
            .to_string();
        if ws_base_url.is_empty() {
            ws_base_url = venue.default_ws_base_url().to_string();
        }

        let quote_asset = std::env::var("SNIPER_BINANCE_QUOTE_ASSET")
            .unwrap_or_else(|_| "USDT".to_string())
            .trim()
            .to_ascii_uppercase();
        let symbol_raw = std::env::var("SNIPER_BINANCE_SYMBOL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                std::env::var("MARKET_SYMBOL")
                    .ok()
                    .map(|v| v.trim().to_ascii_uppercase())
                    .filter(|v| !v.is_empty())
                    .map(|asset| format!("{asset}{quote_asset}"))
            })
            .or_else(|| {
                std::env::var("MARKET_SLUG")
                    .ok()
                    .and_then(|slug| {
                        slug.split('-')
                            .next()
                            .map(|s| s.trim().to_ascii_uppercase())
                    })
                    .filter(|v| !v.is_empty())
                    .map(|asset| format!("{asset}{quote_asset}"))
            })
            .unwrap_or_else(|| "BTCUSDT".to_string());
        let symbol = symbol_raw.to_ascii_uppercase();
        let symbol_asset = normalize_asset_symbol(&symbol);

        let ema_slow = env_int("SNIPER_MOMENTUM_EMA_SLOW", 8).max(2) as usize;
        let momentum_window = env_int("SNIPER_MOMENTUM_WINDOW_CANDLES", 4).max(1) as usize;
        let breakout_k = env_int("SNIPER_BREAKOUT_LEVEL_LOOKBACK_CANDLES", 3).max(1) as usize;
        let warmup_limit = std::cmp::max(
            std::cmp::max(ema_slow + 2, breakout_k + 2),
            std::cmp::max(momentum_window + 2, 64),
        );

        Self {
            venue,
            rest_base_url,
            ws_base_url,
            symbol,
            symbol_asset,
            rest_timeout_seconds: env_float("SNIPER_BINANCE_REST_TIMEOUT_SECONDS", 2.0).max(0.2),
            ws_reconnect_min: env_float("SNIPER_BINANCE_WS_RECONNECT_MIN", 0.5).max(0.1),
            ws_reconnect_max: env_float("SNIPER_BINANCE_WS_RECONNECT_MAX", 10.0).max(0.2),
            log_every_seconds: env_float("SNIPER_BINANCE_LOG_EVERY_SECONDS", 1.0).max(0.0),
            warmup_limit,
        }
    }

    pub fn ws_url(&self) -> String {
        format!(
            "{}/ws/{}@trade",
            self.ws_base_url.trim_end_matches('/'),
            self.symbol.to_ascii_lowercase()
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct BinanceTick {
    pub symbol: String,
    pub price: f64,
    pub ts_ms: i64,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct BinanceKline1m {
    pub symbol: String,
    pub open_time_ms: i64,
    pub close_time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Debug, Clone, Default)]
pub struct BinanceFeedSnapshot {
    pub symbol: String,
    pub symbol_asset: String,
    pub connected: bool,
    pub warmed_up: bool,
    pub updated_at_ms: i64,
    pub last_tick: Option<BinanceTick>,
    pub seed_klines: Vec<BinanceKline1m>,
}

#[derive(Debug, Default)]
struct BinanceFeedState {
    connected: bool,
    warmed_up: bool,
    updated_at_ms: i64,
    last_tick: Option<BinanceTick>,
    seed_klines: Vec<BinanceKline1m>,
}

pub struct BinanceFeedService {
    cfg: BinanceFeedConfig,
    logger: Arc<dyn LogLike>,
    stop_flag: Arc<AtomicBool>,
    state: Arc<Mutex<BinanceFeedState>>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl BinanceFeedService {
    pub fn new(
        cfg: BinanceFeedConfig,
        logger: Arc<dyn LogLike>,
        stop_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            cfg,
            logger,
            stop_flag,
            state: Arc::new(Mutex::new(BinanceFeedState::default())),
            worker: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start(&self) {
        if let Ok(mut slot) = self.worker.lock() {
            if slot.is_some() {
                return;
            }
            let cfg = self.cfg.clone();
            let logger = self.logger.clone();
            let stop_flag = self.stop_flag.clone();
            let state = self.state.clone();
            *slot = Some(thread::spawn(move || {
                Self::run_loop(cfg, logger, stop_flag, state);
            }));
        }
    }

    pub fn snapshot(&self) -> BinanceFeedSnapshot {
        match self.state.lock() {
            Ok(st) => BinanceFeedSnapshot {
                symbol: self.cfg.symbol.clone(),
                symbol_asset: self.cfg.symbol_asset.clone(),
                connected: st.connected,
                warmed_up: st.warmed_up,
                updated_at_ms: st.updated_at_ms,
                last_tick: st.last_tick.clone(),
                seed_klines: st.seed_klines.clone(),
            },
            Err(_) => BinanceFeedSnapshot {
                symbol: self.cfg.symbol.clone(),
                symbol_asset: self.cfg.symbol_asset.clone(),
                ..BinanceFeedSnapshot::default()
            },
        }
    }

    fn run_loop(
        cfg: BinanceFeedConfig,
        logger: Arc<dyn LogLike>,
        stop_flag: Arc<AtomicBool>,
        state: Arc<Mutex<BinanceFeedState>>,
    ) {
        let mut backoff = cfg.ws_reconnect_min;
        let mut last_log_ts = 0.0;
        let ping_interval = 10.0;
        let read_timeout = Duration::from_secs_f64(1.0);

        while !stop_flag.load(Ordering::SeqCst) {
            if let Err(e) = Self::warmup_klines(&cfg, &state) {
                let now = now_ms() as f64 / 1000.0;
                if cfg.log_every_seconds <= 0.0 || now >= last_log_ts + cfg.log_every_seconds {
                    logger.warning(&format!("[BINANCE] warmup klines failed: {e}"));
                    last_log_ts = now;
                }
            }

            let ws_url = cfg.ws_url();
            let (mut ws, _) = match connect(&ws_url) {
                Ok(v) => v,
                Err(e) => {
                    Self::set_connected(&state, false);
                    logger.warning(&format!("[BINANCE] ws connect error: {e}"));
                    let sleep_for = backoff.min(cfg.ws_reconnect_max)
                        * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
                    thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                    backoff = (backoff * 2.0).min(cfg.ws_reconnect_max);
                    continue;
                }
            };
            Self::configure_socket_timeouts(&mut ws, read_timeout);
            Self::set_connected(&state, true);
            backoff = cfg.ws_reconnect_min;
            logger.info(&format!(
                "[BINANCE] connected venue={:?} symbol={} ws={}",
                cfg.venue, cfg.symbol, ws_url
            ));

            let mut last_ping = Instant::now();
            while !stop_flag.load(Ordering::SeqCst) {
                if last_ping.elapsed() >= Duration::from_secs_f64(ping_interval) {
                    let _ = ws.send(Message::Ping(Vec::new().into()));
                    last_ping = Instant::now();
                }
                match ws.read() {
                    Ok(Message::Text(text)) => {
                        Self::handle_ws_text(&cfg, &state, text.as_ref());
                    }
                    Ok(Message::Binary(bin)) => {
                        if let Ok(text) = String::from_utf8(bin.to_vec()) {
                            Self::handle_ws_text(&cfg, &state, &text);
                        }
                    }
                    Ok(Message::Ping(payload)) => {
                        let _ = ws.send(Message::Pong(payload));
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Close(frame)) => {
                        let reason = frame
                            .as_ref()
                            .map(|f| f.reason.to_string())
                            .unwrap_or_else(|| "closed".to_string());
                        logger.warning(&format!("[BINANCE] ws closed: {reason}"));
                        break;
                    }
                    Err(tungstenite::Error::Io(e))
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                    }
                    Err(tungstenite::Error::ConnectionClosed)
                    | Err(tungstenite::Error::AlreadyClosed) => {
                        logger.warning("[BINANCE] ws connection closed");
                        break;
                    }
                    Err(e) => {
                        logger.warning(&format!("[BINANCE] ws read error: {e}"));
                        break;
                    }
                    _ => {}
                }
            }

            let _ = ws.close(None);
            Self::set_connected(&state, false);
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }
            let sleep_for =
                backoff.min(cfg.ws_reconnect_max) * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
            logger.info(&format!("[BINANCE] reconnecting in {sleep_for:.1}s"));
            thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
            backoff = (backoff * 2.0).min(cfg.ws_reconnect_max);
        }
    }

    fn set_connected(state: &Arc<Mutex<BinanceFeedState>>, connected: bool) {
        if let Ok(mut st) = state.lock() {
            st.connected = connected;
            st.updated_at_ms = now_ms();
        }
    }

    fn warmup_klines(
        cfg: &BinanceFeedConfig,
        state: &Arc<Mutex<BinanceFeedState>>,
    ) -> Result<(), String> {
        if let Ok(st) = state.lock() {
            if st.warmed_up && !st.seed_klines.is_empty() {
                return Ok(());
            }
        }
        let client = Client::builder()
            .timeout(Duration::from_secs_f64(cfg.rest_timeout_seconds))
            .build()
            .map_err(|e| format!("build http client: {e}"))?;
        let url = format!("{}/api/v3/klines", cfg.rest_base_url.trim_end_matches('/'));
        let payload = client
            .get(&url)
            .query(&[
                ("symbol", cfg.symbol.as_str()),
                ("interval", "1m"),
                ("limit", &cfg.warmup_limit.to_string()),
            ])
            .send()
            .map_err(|e| format!("rest request failed: {e}"))?
            .json::<Value>()
            .map_err(|e| format!("rest json parse failed: {e}"))?;

        let klines = parse_klines_response(&payload, &cfg.symbol, now_ms());
        if klines.is_empty() {
            return Err("warmup returned no completed klines".to_string());
        }
        if let Ok(mut st) = state.lock() {
            st.seed_klines = klines;
            st.warmed_up = true;
            st.updated_at_ms = now_ms();
        }
        Ok(())
    }

    fn handle_ws_text(cfg: &BinanceFeedConfig, state: &Arc<Mutex<BinanceFeedState>>, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        if let Some(tick) = parse_trade_payload(&v, &cfg.symbol) {
            if let Ok(mut st) = state.lock() {
                let prev_ts = st.last_tick.as_ref().map(|t| t.ts_ms).unwrap_or(0);
                if tick.ts_ms <= prev_ts {
                    return;
                }
                st.last_tick = Some(tick);
                st.updated_at_ms = now_ms();
            }
        }
    }

    fn configure_socket_timeouts(
        ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        read_timeout: Duration,
    ) {
        match ws.get_mut() {
            MaybeTlsStream::Plain(stream) => {
                let _ = stream.set_read_timeout(Some(read_timeout));
                let _ = stream.set_write_timeout(Some(read_timeout));
            }
            MaybeTlsStream::Rustls(stream) => {
                let _ = stream.sock.set_read_timeout(Some(read_timeout));
                let _ = stream.sock.set_write_timeout(Some(read_timeout));
            }
            _ => {}
        }
    }
}

pub(crate) fn parse_trade_payload(v: &Value, expected_symbol: &str) -> Option<BinanceTick> {
    let obj = if v.get("data").is_some() {
        v.get("data")?
    } else {
        v
    };
    let symbol = obj
        .get("s")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    if symbol.is_empty() || symbol != expected_symbol.trim().to_ascii_uppercase() {
        return None;
    }
    let price = obj
        .get("p")
        .and_then(|x| x.as_str())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .or_else(|| obj.get("p").and_then(|x| x.as_f64()))
        .unwrap_or(0.0);
    let ts_ms = obj
        .get("T")
        .and_then(|x| x.as_i64())
        .or_else(|| obj.get("E").and_then(|x| x.as_i64()))
        .unwrap_or(0);
    if price <= 0.0 || ts_ms <= 0 {
        return None;
    }
    Some(BinanceTick {
        symbol,
        price,
        ts_ms,
        received_at_ms: now_ms(),
    })
}

pub(crate) fn parse_klines_response(
    payload: &Value,
    symbol: &str,
    now_ms_value: i64,
) -> Vec<BinanceKline1m> {
    let mut out = Vec::new();
    let arr = match payload.as_array() {
        Some(v) => v,
        None => return out,
    };
    for row in arr {
        let Some(cols) = row.as_array() else {
            continue;
        };
        if cols.len() < 5 {
            continue;
        }
        let open_time_ms = cols.first().and_then(|v| v.as_i64()).unwrap_or(0);
        let open = cols
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let high = cols
            .get(2)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let low = cols
            .get(3)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let close = cols
            .get(4)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let close_time_ms = cols.get(6).and_then(|v| v.as_i64()).unwrap_or(0);
        if open_time_ms <= 0 || close_time_ms <= 0 || close_time_ms >= now_ms_value {
            continue;
        }
        if !(open > 0.0 && high > 0.0 && low > 0.0 && close > 0.0) {
            continue;
        }
        out.push(BinanceKline1m {
            symbol: symbol.to_ascii_uppercase(),
            open_time_ms,
            close_time_ms,
            open,
            high,
            low,
            close,
        });
    }
    out.sort_by_key(|k| k.open_time_ms);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_trade_payload_accepts_symbol_and_fields() {
        let v = json!({
            "e": "trade",
            "s": "BTCUSDT",
            "p": "64000.12",
            "T": 1700000000123i64
        });
        let tick = parse_trade_payload(&v, "BTCUSDT").expect("tick");
        assert_eq!(tick.symbol, "BTCUSDT");
        assert_eq!(tick.ts_ms, 1700000000123i64);
        assert!((tick.price - 64000.12).abs() < 1e-9);
    }

    #[test]
    fn parse_trade_payload_rejects_wrong_symbol() {
        let v = json!({
            "e": "trade",
            "s": "ETHUSDT",
            "p": "3200.0",
            "T": 1700000000123i64
        });
        assert!(parse_trade_payload(&v, "BTCUSDT").is_none());
    }

    #[test]
    fn parse_klines_response_excludes_forming_kline() {
        let payload = json!([
            [60000i64, "100", "110", "90", "105", "1", 119999i64],
            [120000i64, "105", "112", "101", "111", "1", 179999i64]
        ]);
        let rows = parse_klines_response(&payload, "BTCUSDT", 150000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].open_time_ms, 60000i64);
        assert!((rows[0].close - 105.0).abs() < 1e-9);
    }

    #[test]
    fn venue_url_selection_defaults() {
        assert_eq!(
            BinanceVenue::Global.default_rest_base_url(),
            "https://api.binance.com"
        );
        assert_eq!(
            BinanceVenue::Us.default_ws_base_url(),
            "wss://stream.binance.us:9443"
        );
    }
}
