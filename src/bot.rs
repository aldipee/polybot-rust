use crate::config::BotConfig;
use crate::env_utils::{env_bool, env_float, env_int};
use crate::gamma::{fetch_market_by_slug, parse_tokens_and_condition};
use crate::helpers::{
    clamp, cost_per_pair, iso_to_epoch, load_state, locked_profit, q_down, q_up, round_down,
    round_up, save_state, segment_defaults, BotState, OpenOrderState,
};
use crate::logging::LogLike;
use crate::rtds::get_live_snapshot_for_market;
use crate::signal::{LatencyLogService, SignalHub};
use alloy_signer_local::PrivateKeySigner;
use anyhow::{anyhow, Result};
use chrono::{TimeZone, Utc};
use rand::Rng;
use reqwest::blocking::Client;
use rs_clob_client::headers::create_l2_headers;
use rs_clob_client::{
    ApiKeyCreds, AssetType, BalanceAllowanceParams, Chain, ClobClient as RsClobClient,
    CreateOrderOptions, OpenOrderParams, OrderType as ClobOrderType, Side as ClobSide, TickSize,
    UserLimitOrder,
};
use serde_json::json;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::runtime::{Builder as TokioRuntimeBuilder, Runtime as TokioRuntime};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ts_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct TradeMetrics {
    pub lp: f64,
    pub total_cost: f64,
    pub q_yes: f64,
    pub q_no: f64,
    pub cpp: f64,
    pub exit_reason: String,
}

#[derive(Debug, Clone, Default)]
struct TakerOrderRecord {
    order_id: String,
    asset_id: String,
    size: f64,
    applied: f64,
    px_limit: f64,
    side: String,
    ts: f64,
}

pub struct MakerHedgeCapBot {
    pub cfg: BotConfig,
    pub logger: Arc<dyn LogLike>,
    pub market_slug: String,
    pub signal_hub: Option<Arc<SignalHub>>,
    pub state_file: PathBuf,
    pub state: Arc<Mutex<BotState>>,
    pub start_trade_iso: String,
    pub exit_reason: Arc<Mutex<String>>,
    pub stop_flag: Arc<AtomicBool>,
    pub wallet_address: String,
    pub min_maker_notional: f64,
    pub min_taker_notional: f64,
    pub reconcile_sell_credit_mult: f64,
    pub first_clip_shares: f64,
    pub first_hedge_full: bool,
    pub start_ts: i64,
    pub expiry_ts: i64,
    pub warmup_seconds: i64,
    pub max_spread_ticks: i64,
    pub parity_tolerance: f64,
    pub unhedged_timeout_seconds: f64,
    pub hedge_slippage_ticks: i64,
    pub hedge_taker_order_type: String,
    pub taker_order_ttl_seconds: i64,
    pub taker_fill_fallback_from_order_events: bool,
    pub taker_strict_inflight: bool,
    pub last_taker_hedge_ts: f64,
    pub taker_hedge_min_interval: f64,
    pub exec_mode: String,
    pub loop_wait_seconds_maker: f64,
    pub loop_wait_seconds_taker: f64,
    pub loop_wait_seconds_sniper: f64,
    pub condition_id: Option<String>,
    pub yes_asset: Option<String>,
    pub no_asset: Option<String>,
    pub runtime_flags: HashMap<String, Value>,
    pub market_last_update_ts: Arc<Mutex<f64>>,
    pub best_quotes: Arc<Mutex<HashMap<String, (f64, f64, f64)>>>,
    pub market_connected: Arc<AtomicBool>,
    pub user_connected: Arc<AtomicBool>,
    pub book_cache: Arc<Mutex<HashMap<String, (Value, f64)>>>,
    pub debug_last_ts: Arc<Mutex<HashMap<String, f64>>>,
    pub fsm_state: Arc<Mutex<String>>,
    pub active_signal_context: Arc<Mutex<Option<Value>>>,
    pub order_exec_context: Arc<Mutex<HashMap<String, Value>>>,
    submit_timing_cache: Arc<Mutex<HashMap<String, Value>>>,
    taker_orders: Arc<Mutex<HashMap<String, TakerOrderRecord>>>,
    pub latency_log: Option<Arc<LatencyLogService>>,
    clob_rt: Option<Arc<TokioRuntime>>,
    clob_client: Option<Arc<RsClobClient>>,
    clob_api_creds: Option<ApiKeyCreds>,
    balance_allowance_cache: Arc<Mutex<HashMap<String, (f64, f64, f64)>>>,
    pub exchange_orders_cache: Arc<Mutex<Vec<Value>>>,
}

impl MakerHedgeCapBot {
    pub fn new(
        cfg: BotConfig,
        market_slug: &str,
        bot_logger: Arc<dyn LogLike>,
        signal_hub: Option<Arc<SignalHub>>,
    ) -> Result<Self> {
        let state_file = PathBuf::from(format!("maker_hedgecap_state_{market_slug}.json"));
        let state = load_state(&state_file)?;
        let start_trade_iso = crate::db::now_iso_jakarta();

        let mut wallet_address = std::env::var("WALLET_ADDRESS").unwrap_or_default();
        if wallet_address.trim().is_empty() {
            wallet_address = std::env::var("POLYMARKET_WALLET_ADDRESS").unwrap_or_default();
        }
        if wallet_address.trim().is_empty() {
            wallet_address = std::env::var("POLYMARKET_FUNDER").unwrap_or_default();
        }
        if wallet_address.trim().is_empty() {
            wallet_address = cfg.funder.clone().unwrap_or_default();
        }
        wallet_address = wallet_address.trim().to_ascii_lowercase();

        let mut start_ts = now_ts();
        let mut expiry_ts = start_ts + cfg.market_duration_seconds;
        if let Some(raw_ts) = market_slug
            .split('-')
            .last()
            .and_then(|s| s.parse::<i64>().ok())
        {
            start_ts = raw_ts;
            expiry_ts = raw_ts + cfg.market_duration_seconds;
        }

        let seg_d = segment_defaults(&cfg.market_segment);
        let mut runtime_flags = HashMap::new();
        let exec_latency_log_enabled = env_bool("EXEC_LATENCY_LOG_ENABLED", true);
        let exec_latency_file_log_enabled = env_bool("EXEC_LATENCY_FILE_LOG_ENABLED", true);
        let exec_latency_jsonl_enabled = env_bool("EXEC_LATENCY_JSONL_ENABLED", true);
        let exec_latency_csv_enabled = env_bool("EXEC_LATENCY_CSV_ENABLED", true);
        let exec_latency_log_dir = std::env::var("EXEC_LATENCY_LOG_DIR")
            .unwrap_or_else(|_| "./logs".to_string())
            .trim()
            .to_string();
        let exec_latency_jsonl_path = {
            let p = std::env::var("EXEC_LATENCY_JSONL_PATH").unwrap_or_default();
            if p.trim().is_empty() {
                format!("{exec_latency_log_dir}/exec_latency.jsonl")
            } else {
                p
            }
        };
        let exec_latency_csv_path = {
            let p = std::env::var("EXEC_LATENCY_CSV_PATH").unwrap_or_default();
            if p.trim().is_empty() {
                format!("{exec_latency_log_dir}/exec_latency.csv")
            } else {
                p
            }
        };
        let latency_log = if exec_latency_log_enabled && exec_latency_file_log_enabled {
            Some(Arc::new(LatencyLogService::new(
                exec_latency_jsonl_path,
                exec_latency_csv_path,
                true,
                exec_latency_jsonl_enabled,
                exec_latency_csv_enabled,
                None,
            )))
        } else {
            None
        };
        let clob_gamma_host = std::env::var("CLOB_GAMMA_API_URL")
            .or_else(|_| std::env::var("GAMMA_HOST"))
            .unwrap_or_else(|_| "https://gamma-api.polymarket.com".to_string());
        let (clob_rt, clob_client, clob_api_creds) =
            Self::_init_native_clob_client(&cfg, &bot_logger, &clob_gamma_host)?;

        let mut out = Self {
            cfg,
            logger: bot_logger,
            market_slug: market_slug.to_string(),
            signal_hub,
            state_file,
            state: Arc::new(Mutex::new(state)),
            start_trade_iso,
            exit_reason: Arc::new(Mutex::new("RUNNING".to_string())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            wallet_address,
            min_maker_notional: env_float("MIN_MAKER_NOTIONAL", 1.0),
            min_taker_notional: env_float("MIN_TAKER_NOTIONAL", 1.0),
            reconcile_sell_credit_mult: clamp(
                env_float("RECONCILE_SELL_CREDIT_MULT", 1.0),
                0.0,
                1.0,
            ),
            first_clip_shares: env_float("FIRST_CLIP_SHARES", 0.0),
            first_hedge_full: matches!(
                std::env::var("FIRST_HEDGE_FULL")
                    .unwrap_or_else(|_| "false".to_string())
                    .to_ascii_lowercase()
                    .as_str(),
                "1" | "true" | "yes" | "y"
            ),
            start_ts,
            expiry_ts,
            warmup_seconds: env_int("WARMUP_SECONDS", seg_d.warmup) as i64,
            max_spread_ticks: env_int("MAX_SPREAD_TICKS", 6) as i64,
            parity_tolerance: env_float("PARITY_TOLERANCE", 0.025),
            unhedged_timeout_seconds: env_float("UNHEDGED_TIMEOUT_SECONDS", 2.0),
            hedge_slippage_ticks: env_int("HEDGE_SLIPPAGE_TICKS", 1) as i64,
            hedge_taker_order_type: std::env::var("HEDGE_TAKER_ORDER_TYPE")
                .unwrap_or_else(|_| "FAK".to_string())
                .trim()
                .to_ascii_uppercase(),
            taker_order_ttl_seconds: env_int("TAKER_ORDER_TTL_SECONDS", 120) as i64,
            taker_fill_fallback_from_order_events: env_bool(
                "TAKER_FILL_FALLBACK_FROM_ORDER_EVENTS",
                true,
            ),
            taker_strict_inflight: env_bool("TAKER_STRICT_INFLIGHT", true),
            last_taker_hedge_ts: 0.0,
            taker_hedge_min_interval: env_float("TAKER_HEDGE_MIN_INTERVAL", 1.0),
            exec_mode: std::env::var("EXEC_MODE")
                .unwrap_or_else(|_| "MAKER".to_string())
                .trim()
                .to_ascii_uppercase(),
            loop_wait_seconds_maker: env_float("LOOP_WAIT_SECONDS_MAKER", 1.0),
            loop_wait_seconds_taker: env_float("LOOP_WAIT_SECONDS_TAKER", 0.2),
            loop_wait_seconds_sniper: env_float("LOOP_WAIT_SECONDS_SNIPER", 0.05),
            condition_id: None,
            yes_asset: None,
            no_asset: None,
            runtime_flags: HashMap::new(),
            market_last_update_ts: Arc::new(Mutex::new(0.0)),
            best_quotes: Arc::new(Mutex::new(HashMap::new())),
            market_connected: Arc::new(AtomicBool::new(false)),
            user_connected: Arc::new(AtomicBool::new(false)),
            book_cache: Arc::new(Mutex::new(HashMap::new())),
            debug_last_ts: Arc::new(Mutex::new(HashMap::new())),
            fsm_state: Arc::new(Mutex::new("ACCUMULATE".to_string())),
            active_signal_context: Arc::new(Mutex::new(None)),
            order_exec_context: Arc::new(Mutex::new(HashMap::new())),
            submit_timing_cache: Arc::new(Mutex::new(HashMap::new())),
            taker_orders: Arc::new(Mutex::new(HashMap::new())),
            latency_log,
            clob_rt,
            clob_client,
            clob_api_creds,
            balance_allowance_cache: Arc::new(Mutex::new(HashMap::new())),
            exchange_orders_cache: Arc::new(Mutex::new(Vec::new())),
        };

        runtime_flags.insert(
            "signal_follow_slug".to_string(),
            Value::Bool(env_bool("SIGNAL_FOLLOW_SLUG", false)),
        );
        runtime_flags.insert(
            "signal_provider".to_string(),
            Value::String(
                std::env::var("SIGNAL_PROVIDER")
                    .unwrap_or_else(|_| "WEBSOCKET".to_string())
                    .to_ascii_uppercase(),
            ),
        );
        out.runtime_flags = runtime_flags;

        if let Some(market) = fetch_market_by_slug(&out.market_slug, Some(&out.logger))? {
            if let Ok((yes, no, condition)) = parse_tokens_and_condition(&market) {
                out.condition_id = Some(condition.clone());
                out.yes_asset = Some(yes.clone());
                out.no_asset = Some(no.clone());
                if let Some(st) = market
                    .get("startDate")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_epoch)
                {
                    out.start_ts = st;
                }
                if let Some(et) = market
                    .get("endDate")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_epoch)
                {
                    out.expiry_ts = et;
                }
                out.logger
                    .info(&format!("Market Found: {}", out.market_slug));
                out.logger.info(&format!("Condition ID: {condition}"));
                out.logger.info(&format!("YES asset: {yes}"));
                out.logger.info(&format!("NO  asset: {no}"));
                out.logger.info(&format!(
                    "Start ts: {} | Expiry ts: {}",
                    out.start_ts, out.expiry_ts
                ));
            }
        }
        out._warm_clob_order_meta_cache();

        Ok(out)
    }

    fn _warm_clob_order_meta_cache(&self) {
        if !env_bool("CLOB_ORDER_META_WARMUP", true) {
            return;
        }
        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return,
        };
        let mut assets: Vec<String> = Vec::new();
        if let Some(a) = &self.yes_asset {
            if !a.trim().is_empty() {
                assets.push(a.clone());
            }
        }
        if let Some(a) = &self.no_asset {
            if !a.trim().is_empty() && !assets.iter().any(|v| v == a) {
                assets.push(a.clone());
            }
        }
        for aid in assets {
            let t0 = now_ns();
            let _ = rt.block_on(client.get_tick_size(&aid));
            let _ = rt.block_on(client.get_neg_risk(&aid));
            let _ = rt.block_on(client.get_fee_rate_bps(&aid));
            let ms = ((now_ns() - t0) as f64 / 1_000_000.0).round() as i64;
            let tail: String = aid
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger
                .info(&format!("[CLOB] warm order meta asset={tail} took={ms}ms"));
        }
    }

    fn _init_native_clob_client(
        cfg: &BotConfig,
        logger: &Arc<dyn LogLike>,
        gamma_host: &str,
    ) -> Result<(
        Option<Arc<TokioRuntime>>,
        Option<Arc<RsClobClient>>,
        Option<ApiKeyCreds>,
    )> {
        let key = cfg.private_key.trim();
        if key.is_empty() {
            return Ok((None, None, None));
        }

        let rt = Arc::new(
            TokioRuntimeBuilder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow!("failed to create tokio runtime for CLOB client: {e}"))?,
        );

        let chain = match cfg.chain_id {
            137 => Chain::Polygon,
            80002 => Chain::Amoy,
            other => {
                logger.warning(&format!(
                    "Unsupported CHAIN_ID={other}, defaulting CLOB client to Polygon (137)"
                ));
                Chain::Polygon
            }
        };

        let signature_type = cfg.signature_type.and_then(|v| {
            if (0..=u8::MAX as i64).contains(&v) {
                Some(v as u8)
            } else {
                None
            }
        });
        let funder = cfg
            .funder
            .clone()
            .and_then(|v| (!v.trim().is_empty()).then_some(v));
        let normalized_key = if key.starts_with("0x") || key.starts_with("0X") {
            key.to_string()
        } else {
            format!("0x{key}")
        };
        let wallet = normalized_key
            .parse::<PrivateKeySigner>()
            .map_err(|e| anyhow!("failed to parse POLYMARKET_PRIVATE_KEY: {e}"))?;

        let unauth_client = RsClobClient::new(
            cfg.clob_host.clone(),
            gamma_host.to_string(),
            chain,
            Some(wallet.clone()),
            None,
            signature_type,
            funder.clone(),
            None,
            false,
            None,
            None,
        )
        .map_err(|e| anyhow!("failed to initialize CLOB client: {e}"))?;

        let creds = rt
            .block_on(unauth_client.create_or_derive_api_key(None))
            .map_err(|e| anyhow!("failed to derive CLOB API credentials: {e}"))?;

        let authed_client = RsClobClient::new(
            cfg.clob_host.clone(),
            gamma_host.to_string(),
            chain,
            Some(wallet),
            Some(creds.clone()),
            signature_type,
            funder,
            None,
            false,
            None,
            None,
        )
        .map_err(|e| anyhow!("failed to initialize authenticated CLOB client: {e}"))?;

        Ok((Some(rt), Some(Arc::new(authed_client)), Some(creds)))
    }

    fn _clob_order_type(order_type: &str) -> ClobOrderType {
        match order_type.trim().to_ascii_uppercase().as_str() {
            "FAK" => ClobOrderType::Fak,
            "FOK" => ClobOrderType::Fok,
            "GTD" => ClobOrderType::Gtd,
            _ => ClobOrderType::Gtc,
        }
    }

    fn _clob_side(side: &str) -> Option<ClobSide> {
        match side.trim().to_ascii_uppercase().as_str() {
            "BUY" => Some(ClobSide::Buy),
            "SELL" => Some(ClobSide::Sell),
            _ => None,
        }
    }

    fn _tick_size_from_f64(v: f64) -> TickSize {
        let vv = (v * 10_000.0).round() / 10_000.0;
        if (vv - 0.1).abs() < 1e-9 {
            TickSize::ZeroPointOne
        } else if (vv - 0.01).abs() < 1e-9 {
            TickSize::ZeroPointZeroOne
        } else if (vv - 0.001).abs() < 1e-9 {
            TickSize::ZeroPointZeroZeroOne
        } else {
            TickSize::ZeroPointZeroZeroZeroOne
        }
    }

    fn _value_f64(v: Option<&Value>) -> Option<f64> {
        v.and_then(|x| match x {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse::<f64>().ok(),
            _ => None,
        })
    }

    fn _max_numeric_in_value(v: Option<&Value>) -> Option<f64> {
        fn walk(node: &Value, best: &mut Option<f64>) {
            match node {
                Value::Number(n) => {
                    if let Some(x) = n.as_f64() {
                        *best = Some(best.map_or(x, |b| b.max(x)));
                    }
                }
                Value::String(s) => {
                    if let Ok(x) = s.parse::<f64>() {
                        *best = Some(best.map_or(x, |b| b.max(x)));
                    }
                }
                Value::Array(a) => {
                    for it in a {
                        walk(it, best);
                    }
                }
                Value::Object(m) => {
                    for it in m.values() {
                        walk(it, best);
                    }
                }
                _ => {}
            }
        }

        let mut best = None;
        if let Some(root) = v {
            walk(root, &mut best);
        }
        best
    }

    fn _extract_posted_order_id(resp: &Value) -> Option<String> {
        resp.get("orderID")
            .or_else(|| resp.get("order_id"))
            .or_else(|| resp.get("id"))
            .or_else(|| resp.get("order").and_then(|v| v.get("id")))
            .or_else(|| resp.get("order").and_then(|v| v.get("order_id")))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    fn _build_l2_headers(
        &self,
        method: &str,
        request_path: &str,
        body: Option<&str>,
    ) -> Option<HashMap<String, String>> {
        let creds = self.clob_api_creds.as_ref()?;
        let rt = self.clob_rt.as_ref()?;
        let raw_key = self.cfg.private_key.trim();
        if raw_key.is_empty() {
            return None;
        }
        let normalized_key = if raw_key.starts_with("0x") || raw_key.starts_with("0X") {
            raw_key.to_string()
        } else {
            format!("0x{raw_key}")
        };
        let wallet = normalized_key.parse::<PrivateKeySigner>().ok()?;
        let headers = rt
            .block_on(create_l2_headers(
                &wallet,
                creds,
                method,
                request_path,
                body,
                None,
            ))
            .ok()?;
        Some(headers.to_headers())
    }

    fn _normalize_open_orders_payload(payload: &Value) -> Vec<Value> {
        let items = if let Some(a) = payload.as_array() {
            a.clone()
        } else if let Some(a) = payload.get("data").and_then(|v| v.as_array()) {
            a.clone()
        } else if let Some(a) = payload.get("orders").and_then(|v| v.as_array()) {
            a.clone()
        } else if let Some(a) = payload.get("results").and_then(|v| v.as_array()) {
            a.clone()
        } else {
            Vec::new()
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let oid = item
                .get("id")
                .or_else(|| item.get("order_id"))
                .or_else(|| item.get("orderID"))
                .or_else(|| item.get("orderId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if oid.trim().is_empty() {
                continue;
            }
            let asset_id = item
                .get("asset_id")
                .or_else(|| item.get("token_id"))
                .or_else(|| item.get("assetId"))
                .or_else(|| item.get("tokenId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let price = Self::_value_f64(item.get("price")).unwrap_or(0.0);
            let original_size = Self::_value_f64(
                item.get("original_size")
                    .or_else(|| item.get("originalSize"))
                    .or_else(|| item.get("size")),
            )
            .unwrap_or(0.0);
            let size_matched = Self::_value_f64(
                item.get("size_matched")
                    .or_else(|| item.get("sizeMatched"))
                    .or_else(|| item.get("filled")),
            )
            .unwrap_or(0.0);
            let remaining_size = Self::_value_f64(
                item.get("remaining_size")
                    .or_else(|| item.get("remainingSize"))
                    .or_else(|| item.get("size")),
            )
            .unwrap_or_else(|| (original_size - size_matched).max(0.0));
            out.push(json!({
                "id": oid.clone(),
                "order_id": oid,
                "asset_id": asset_id.clone(),
                "token_id": asset_id,
                "side": item
                    .get("side")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_uppercase(),
                "price": price,
                "size": remaining_size,
                "remaining_size": remaining_size,
                "original_size": original_size,
                "size_matched": size_matched,
                "status": item.get("status").cloned().unwrap_or(Value::Null),
                "market": item.get("market").cloned().unwrap_or(Value::Null),
                "order_type": item.get("order_type").cloned().unwrap_or(Value::Null),
                "created_at": item.get("created_at").cloned().unwrap_or(Value::Null),
            }));
        }
        out
    }

    fn _list_open_orders_exchange_raw(&self) -> Option<Vec<Value>> {
        let endpoint_path = "/data/orders";
        let headers = self._build_l2_headers("GET", endpoint_path, None)?;
        let mut req = Client::new().get(format!(
            "{}{}",
            self.cfg.clob_host.trim_end_matches('/'),
            endpoint_path
        ));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(market) = self
            .condition_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            req = req.query(&[("market", market)]);
        }
        let payload = req.send().ok()?.json::<Value>().ok()?;
        Some(Self::_normalize_open_orders_payload(&payload))
    }

    fn _runtime_ts_get(&self, key: &str) -> f64 {
        self.debug_last_ts
            .lock()
            .ok()
            .and_then(|m| m.get(key).copied())
            .unwrap_or(0.0)
    }

    fn _runtime_ts_set(&self, key: &str, value: f64) {
        if let Ok(mut m) = self.debug_last_ts.lock() {
            m.insert(key.to_string(), value);
        }
    }

    fn _rtds_gate_log(&self, bucket: &str, msg: &str) {
        let every_s = env_float("RTDS_ENTRY_GATE_LOG_EVERY_SECONDS", 0.0);
        if every_s <= 0.0 {
            self.logger.info(msg);
            return;
        }
        let key = format!("__rtds_entry_gate_log_{bucket}");
        let now = now_ts_f64();
        if now >= self._runtime_ts_get(&key) {
            self.logger.info(msg);
            self._runtime_ts_set(&key, now + every_s);
        }
    }

    fn _rtds_gate_load_payload(&self, context: &str) -> Result<Value, String> {
        let use_memory = env_bool("RTDS_ENTRY_GATE_USE_MEMORY", true);
        let fallback_file = env_bool("RTDS_ENTRY_GATE_FALLBACK_FILE", false);
        let latest_path = std::env::var("RTDS_LATEST_PATH")
            .unwrap_or_else(|_| "state/rtds_latest.json".to_string())
            .trim()
            .to_string();
        if use_memory {
            if let Some(s) = get_live_snapshot_for_market(&self.market_slug) {
                return Ok(json!({
                    "market_slug": s.market_slug,
                    "timestamp_ms": s.timestamp_ms,
                    "received_at_ms": s.received_at_ms,
                    "updated_at_ms": s.updated_at_ms,
                    "price": s.price,
                    "price_to_beat": s.price_to_beat,
                    "diff_vs_price_to_beat": s.diff_vs_price_to_beat,
                    "diff_vs_price_to_beat_percentage": s.diff_vs_price_to_beat_percentage,
                }));
            }
        }
        if use_memory && !fallback_file {
            return Err(format!("{context}: missing in-memory RTDS snapshot"));
        }
        if latest_path.is_empty() {
            return Err(format!("{context}: RTDS_LATEST_PATH is empty"));
        }
        let raw = fs::read_to_string(&latest_path).map_err(|e| {
            format!("{context}: cannot read latest file path={latest_path} err={e}")
        })?;
        let payload = serde_json::from_str::<Value>(&raw)
            .map_err(|e| format!("{context}: invalid latest JSON path={latest_path} err={e}"))?;
        Ok(payload)
    }

    fn _rtds_gate_diff_price(payload: &Value) -> Option<f64> {
        let mut diff_price = Self::_value_f64(payload.get("diff_vs_price_to_beat"));
        if diff_price.is_none() {
            let px = Self::_value_f64(payload.get("price"));
            let ptb = Self::_value_f64(payload.get("price_to_beat"));
            if let (Some(px), Some(ptb)) = (px, ptb) {
                diff_price = Some(px - ptb);
            }
        }
        diff_price
    }

    fn _rtds_gate_snapshot(
        &self,
        context: &str,
        max_age_seconds: f64,
        require_market_match: bool,
    ) -> Result<(f64, i64, i64), String> {
        let payload = self._rtds_gate_load_payload(context)?;
        let payload_market = payload
            .get("market_slug")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if require_market_match && !payload_market.is_empty() && payload_market != self.market_slug
        {
            return Err(format!(
                "{context}: market mismatch current={} latest={}",
                self.market_slug, payload_market
            ));
        }

        let ts_ms = Self::_value_f64(
            payload
                .get("updated_at_ms")
                .or_else(|| payload.get("received_at_ms"))
                .or_else(|| payload.get("timestamp_ms"))
                .or_else(|| payload.get("ts_ms")),
        )
        .map(|v| v as i64)
        .unwrap_or(0);
        let now_ms = (now_ts_f64() * 1000.0) as i64;
        let age_ms = (now_ms - ts_ms).max(0);
        let max_age_ms = (max_age_seconds.max(0.05) * 1000.0) as i64;
        if ts_ms <= 0 || age_ms > max_age_ms {
            return Err(format!(
                "{context}: stale/missing RTDS tick ts_ms={} age_ms={} max_age_ms={}",
                ts_ms, age_ms, max_age_ms
            ));
        }
        let diff_price = Self::_rtds_gate_diff_price(&payload)
            .ok_or_else(|| format!("{context}: missing diff_vs_price_to_beat/price_to_beat"))?;
        Ok((diff_price, age_ms, ts_ms))
    }

    fn _rtds_entry_gate_min_diff_price_for_context(&self, side: &str, context: &str) -> f64 {
        let min_common = env_float(
            "RTDS_ENTRY_GATE_MIN_DIFF_PRICE",
            env_float("RTDS_ENTRY_GATE_MIN_DIFF_PCT", 0.0),
        )
        .max(0.0);
        let min_force = std::env::var("RTDS_ENTRY_GATE_MIN_DIFF_PRICE_FORCE")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .or_else(|| {
                std::env::var("RTDS_ENTRY_GATE_MIN_DIFF_PCT_FORCE")
                    .ok()
                    .and_then(|v| v.trim().parse::<f64>().ok())
            })
            .unwrap_or(min_common)
            .max(0.0);
        if context.contains("FORCE") {
            return min_force;
        }
        let min_yes = std::env::var("RTDS_ENTRY_GATE_MIN_DIFF_PRICE_YES")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .or_else(|| {
                std::env::var("RTDS_ENTRY_GATE_MIN_DIFF_PCT_YES")
                    .ok()
                    .and_then(|v| v.trim().parse::<f64>().ok())
            })
            .unwrap_or(min_common)
            .max(0.0);
        let min_no = std::env::var("RTDS_ENTRY_GATE_MIN_DIFF_PRICE_NO")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .or_else(|| {
                std::env::var("RTDS_ENTRY_GATE_MIN_DIFF_PCT_NO")
                    .ok()
                    .and_then(|v| v.trim().parse::<f64>().ok())
            })
            .unwrap_or(min_common)
            .max(0.0);
        if side == "YES" {
            min_yes
        } else {
            min_no
        }
    }

    fn _rtds_entry_gate_allows_side(&self, side: &str, seconds_left: f64, context: &str) -> bool {
        if !env_bool("RTDS_ENTRY_GATE_ENABLED", false) {
            return true;
        }
        let side = side.trim().to_ascii_uppercase();
        if !matches!(side.as_str(), "YES" | "NO") {
            return true;
        }

        let allow_missing = env_bool("RTDS_ENTRY_GATE_ALLOW_MISSING", false);
        let require_market_match = env_bool("RTDS_ENTRY_GATE_REQUIRE_MARKET_MATCH", true);
        let max_age_seconds = env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 2.0);
        let diff_price = match self._rtds_gate_snapshot(
            context,
            max_age_seconds,
            require_market_match,
        ) {
            Ok((d, age_ms, ts_ms)) => {
                self._rtds_gate_log(
                    "snapshot_ok",
                    &format!(
                        "[RTDS_GATE] {} snapshot side={} ts_ms={} age_ms={} diff_price={:+.6} t_left={:.2}s",
                        context, side, ts_ms, age_ms, d, seconds_left
                    ),
                );
                d
            }
            Err(reason) => {
                self._rtds_gate_log(
                    "snapshot_block",
                    &format!("[RTDS_GATE] {} blocked: {}", context, reason),
                );
                return allow_missing;
            }
        };

        let min_req = self._rtds_entry_gate_min_diff_price_for_context(&side, context);
        let pass = if side == "YES" {
            diff_price + 1e-12 >= min_req
        } else {
            diff_price - 1e-12 <= -min_req
        };
        if !pass {
            self._rtds_gate_log(
                "threshold",
                &format!(
                    "[RTDS_GATE] {} blocked: side={} diff_price={:+.6} required={:.6} t_left={:.2}s",
                    context, side, diff_price, min_req, seconds_left
                ),
            );
            return false;
        }

        self._rtds_gate_log(
            "pass",
            &format!(
                "[RTDS_GATE] {} pass: side={} diff_price={:+.6} required={:.6} t_left={:.2}s",
                context, side, diff_price, min_req, seconds_left
            ),
        );
        true
    }

    fn _sniper_endgame_side_from_rtds(&self, seconds_left: f64) -> Option<String> {
        let snap = match get_live_snapshot_for_market(&self.market_slug) {
            Some(s) => s,
            None => {
                self._rtds_gate_log(
                    "endgame_rtds_missing",
                    "[RTDS_ENDGAME] blocked: missing in-memory RTDS snapshot",
                );
                return None;
            }
        };
        let ts_ms = if snap.updated_at_ms > 0 {
            snap.updated_at_ms
        } else if snap.received_at_ms > 0 {
            snap.received_at_ms
        } else {
            snap.timestamp_ms
        };
        let now_ms = (now_ts_f64() * 1000.0) as i64;
        let age_ms = (now_ms - ts_ms).max(0);
        let max_age_s = env_float(
            "SNIPER_ENDGAME_RTDS_MAX_AGE_SECONDS",
            env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 0.15),
        )
        .max(0.01);
        let max_age_ms = (max_age_s * 1000.0) as i64;
        if ts_ms <= 0 || age_ms > max_age_ms {
            self._rtds_gate_log(
                "endgame_rtds_stale",
                &format!(
                    "[RTDS_ENDGAME] blocked: stale/missing tick ts_ms={} age_ms={} max_age_ms={} t_left={:.2}s",
                    ts_ms, age_ms, max_age_ms, seconds_left
                ),
            );
            return None;
        }

        let diff_price = snap
            .diff_vs_price_to_beat
            .or_else(|| snap.price_to_beat.map(|ptb| snap.price - ptb));
        let Some(diff_price) = diff_price.filter(|v| v.is_finite()) else {
            self._rtds_gate_log(
                "endgame_rtds_nodiff",
                &format!(
                    "[RTDS_ENDGAME] blocked: missing diff_vs_price_to_beat/price_to_beat ts_ms={} age_ms={} t_left={:.2}s",
                    ts_ms, age_ms, seconds_left
                ),
            );
            return None;
        };

        let min_abs = env_float("SNIPER_ENDGAME_RTDS_MIN_DIFF_PRICE", 0.0)
            .max(0.0)
            .max(1e-12);
        if diff_price.abs() + 1e-12 < min_abs {
            self._rtds_gate_log(
                "endgame_rtds_min_diff",
                &format!(
                    "[RTDS_ENDGAME] blocked: |diff_price|={:.6} < min_required={:.6} age_ms={} t_left={:.2}s",
                    diff_price.abs(),
                    min_abs,
                    age_ms,
                    seconds_left
                ),
            );
            return None;
        }

        let side = if diff_price > 0.0 { "YES" } else { "NO" };
        self._rtds_gate_log(
            "endgame_rtds_pick",
            &format!(
                "[RTDS_ENDGAME] pick side={} diff_price={:+.6} age_ms={} ts_ms={} t_left={:.2}s",
                side, diff_price, age_ms, ts_ms, seconds_left
            ),
        );
        Some(side.to_string())
    }

    fn _sniper_endgame_resolution_tick_ready(&self, seconds_left: f64) -> bool {
        let snap = match get_live_snapshot_for_market(&self.market_slug) {
            Some(s) => s,
            None => {
                self._rtds_gate_log(
                    "endgame_res_wait_missing",
                    "[RTDS_ENDGAME] waiting: missing in-memory RTDS snapshot for resolution trigger",
                );
                return false;
            }
        };

        let resolution_ts_ms = self.expiry_ts.saturating_mul(1000);
        let source_ts_ms = snap.timestamp_ms;
        if source_ts_ms <= 0 || source_ts_ms + 1 < resolution_ts_ms {
            self._rtds_gate_log(
                "endgame_res_wait_source_ts",
                &format!(
                    "[RTDS_ENDGAME] waiting: source_ts_ms={} < resolution_ts_ms={} t_left={:.2}s",
                    source_ts_ms, resolution_ts_ms, seconds_left
                ),
            );
            return false;
        }

        let tick_ts_ms = if snap.updated_at_ms > 0 {
            snap.updated_at_ms
        } else if snap.received_at_ms > 0 {
            snap.received_at_ms
        } else {
            source_ts_ms
        };
        let now_ms = (now_ts_f64() * 1000.0) as i64;
        let age_ms = (now_ms - tick_ts_ms).max(0);
        let max_age_s = env_float(
            "SNIPER_ENDGAME_RTDS_MAX_AGE_SECONDS",
            env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 0.15),
        )
        .max(0.01);
        let max_age_ms = (max_age_s * 1000.0) as i64;
        if tick_ts_ms <= 0 || age_ms > max_age_ms {
            self._rtds_gate_log(
                "endgame_res_wait_stale",
                &format!(
                    "[RTDS_ENDGAME] waiting: stale tick ts_ms={} age_ms={} max_age_ms={} t_left={:.2}s",
                    tick_ts_ms, age_ms, max_age_ms, seconds_left
                ),
            );
            return false;
        }
        if self._runtime_ts_get("__sniper_endgame_resolution_ready_logged_ts") <= 0.0 {
            let now_ms = (now_ts_f64() * 1000.0) as i64;
            let since_resolution_ms = now_ms.saturating_sub(resolution_ts_ms);
            let source_delta_ms = source_ts_ms.saturating_sub(resolution_ts_ms);
            self.logger.info(&format!(
                "[RTDS_ENDGAME][TIMING] resolution_ready now_ms={} resolution_ts_ms={} since_resolution_ms={} source_ts_ms={} source_delta_ms={} tick_age_ms={} t_left={:.2}s",
                now_ms, resolution_ts_ms, since_resolution_ms, source_ts_ms, source_delta_ms, age_ms, seconds_left
            ));
            self._runtime_ts_set("__sniper_endgame_resolution_ready_logged_ts", now_ts_f64());
        }
        true
    }

    fn _rtds_hold_till_resolution_active(
        &self,
        side: &str,
        seconds_left: f64,
        context: &str,
    ) -> bool {
        if !env_bool("RTDS_HOLD_TILL_RESOLUTION_ENABLED", true) {
            self._runtime_ts_set("__rtds_hold_active", 0.0);
            return false;
        }
        let hold_diff = std::env::var("RTDS_GATE_DIFF_PRICE")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or_else(|| env_float("RTDS_GATE_PCT", 0.0))
            .max(0.0);
        if hold_diff <= 0.0 {
            self._runtime_ts_set("__rtds_hold_active", 0.0);
            return false;
        }
        let side = side.trim().to_ascii_uppercase();
        if !matches!(side.as_str(), "YES" | "NO") {
            self._runtime_ts_set("__rtds_hold_active", 0.0);
            self._rtds_gate_log(
                "hold_invalid_side",
                &format!("[RTDS_HOLD] {} off: invalid side='{}'", context, side),
            );
            return false;
        }

        let max_age_seconds = env_float(
            "RTDS_GATE_MAX_AGE_SECONDS",
            env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 2.0),
        );
        let require_market_match = env_bool("RTDS_ENTRY_GATE_REQUIRE_MARKET_MATCH", true);
        let snap = self._rtds_gate_snapshot(context, max_age_seconds, require_market_match);
        let (diff_price, age_ms, ts_ms) = match snap {
            Ok(v) => v,
            Err(reason) => {
                let was_active = self._runtime_ts_get("__rtds_hold_active") > 0.0;
                self._runtime_ts_set("__rtds_hold_active", 0.0);
                self._runtime_ts_set("__rtds_hold_side_yes", 0.0);
                self._rtds_gate_log(
                    if was_active {
                        "hold_off_snapshot"
                    } else {
                        "hold_skip_snapshot"
                    },
                    &format!("[RTDS_HOLD] {} off: {}", context, reason),
                );
                return false;
            }
        };
        let should_hold = if side == "YES" {
            diff_price + 1e-12 >= hold_diff
        } else {
            diff_price - 1e-12 <= -hold_diff
        };
        let was_active = self._runtime_ts_get("__rtds_hold_active") > 0.0;
        if should_hold {
            self._runtime_ts_set("__rtds_hold_active", 1.0);
            self._runtime_ts_set(
                "__rtds_hold_side_yes",
                if side == "YES" { 1.0 } else { 0.0 },
            );
            self._rtds_gate_log(
                if was_active { "hold_keep" } else { "hold_on" },
                &format!(
                    "[RTDS_HOLD] {} {}: side={} diff_price={:+.6} threshold={:.6} ts_ms={} age_ms={} t_left={:.2}s reason=diff_price_meets_threshold",
                    context,
                    if was_active { "keep" } else { "ON" },
                    side,
                    diff_price,
                    hold_diff,
                    ts_ms,
                    age_ms,
                    seconds_left
                ),
            );
            return true;
        }
        self._runtime_ts_set("__rtds_hold_active", 0.0);
        self._runtime_ts_set("__rtds_hold_side_yes", 0.0);
        self._rtds_gate_log(
            if was_active { "hold_off" } else { "hold_skip" },
            &format!(
                "[RTDS_HOLD] {} {}: side={} diff_price={:+.6} threshold={:.6} ts_ms={} age_ms={} t_left={:.2}s reason=diff_price_below_threshold",
                context,
                if was_active { "OFF" } else { "skip" },
                side,
                diff_price,
                hold_diff,
                ts_ms,
                age_ms,
                seconds_left
            ),
        );
        false
    }

    fn _sniper_entry_pending_key(asset_id: &str) -> String {
        format!("__sniper_entry_pending_{asset_id}")
    }

    fn _sniper_entry_confirmed_key(asset_id: &str) -> String {
        format!("__sniper_entry_confirmed_{asset_id}")
    }

    fn _set_exit_reason(&self, reason: &str) {
        if let Ok(mut r) = self.exit_reason.lock() {
            *r = reason.to_string();
        }
    }

    fn _get_exit_reason(&self) -> String {
        self.exit_reason
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "UNKNOWN".to_string())
    }

    fn _mark_sniper_entry_state(&self, side: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.sniper_last_entry_ts = now_ts_f64();
            s.sniper_last_side = side.to_string();
            s.sniper_trade_count += 1;
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    fn _mark_sniper_exit_state(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.sniper_last_exit_ts = now_ts_f64();
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    fn _clear_local_position_for_asset(&self, asset_id: &str, reason: &str) {
        if asset_id.trim().is_empty() {
            return;
        }
        let mut changed = false;
        if let Ok(mut s) = self.state.lock() {
            if self.yes_asset.as_deref() == Some(asset_id) {
                if s.q_yes > 1e-12 || s.c_yes > 1e-12 {
                    s.q_yes = 0.0;
                    s.c_yes = 0.0;
                    changed = true;
                }
            } else if self.no_asset.as_deref() == Some(asset_id) {
                if s.q_no > 1e-12 || s.c_no > 1e-12 {
                    s.q_no = 0.0;
                    s.c_no = 0.0;
                    changed = true;
                }
            }
            if s.open_orders.remove(asset_id).is_some() {
                changed = true;
            }
            if changed {
                let _ = save_state(&self.state_file, &mut s);
            }
        }
        if changed {
            self._cancel_open_order_local(asset_id, "desync reconcile");
            let assets = vec![asset_id.to_string()];
            self._cancel_exchange_orders_for_assets(&assets, "desync reconcile");
            let tail: String = asset_id
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger.warning(&format!(
                "[SNIPER] local state reconciled to zero for asset={tail} ({reason})"
            ));
        }
    }

    fn _env_first(keys: &[&str]) -> String {
        for key in keys {
            if let Ok(v) = std::env::var(key) {
                let t = v.trim();
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
        String::new()
    }

    fn _user_ws_auth(&self) -> Option<Value> {
        if let Some(creds) = &self.clob_api_creds {
            return Some(json!({
                "apiKey": creds.key,
                "secret": creds.secret,
                "passphrase": creds.passphrase,
            }));
        }

        let api_key = Self::_env_first(&[
            "POLYMARKET_API_KEY",
            "API_KEY",
            "CLOB_API_KEY",
            "POLY_API_KEY",
        ]);
        let api_secret = Self::_env_first(&[
            "POLYMARKET_API_SECRET",
            "API_SECRET",
            "CLOB_API_SECRET",
            "POLY_API_SECRET",
        ]);
        let passphrase = Self::_env_first(&[
            "POLYMARKET_API_PASSPHRASE",
            "API_PASSPHRASE",
            "CLOB_API_PASSPHRASE",
            "POLY_API_PASSPHRASE",
        ]);
        if api_key.is_empty() || api_secret.is_empty() || passphrase.is_empty() {
            return None;
        }
        Some(json!({
            "apiKey": api_key,
            "secret": api_secret,
            "passphrase": passphrase,
        }))
    }

    fn _set_ws_stream_timeouts(
        &self,
        ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        timeout: Duration,
    ) {
        match ws.get_mut() {
            MaybeTlsStream::Plain(sock) => {
                let _ = sock.set_read_timeout(Some(timeout));
                let _ = sock.set_write_timeout(Some(timeout));
            }
            MaybeTlsStream::Rustls(sock) => {
                let tcp = &mut sock.sock;
                let _ = tcp.set_read_timeout(Some(timeout));
                let _ = tcp.set_write_timeout(Some(timeout));
            }
            _ => {}
        }
    }

    pub fn run(&self) -> Result<String> {
        if self.yes_asset.is_none() || self.no_asset.is_none() {
            return Err(anyhow!("NO_MARKET"));
        }
        let reason = thread::scope(|scope| {
            scope.spawn(|| self._ws_runner("market"));
            scope.spawn(|| self._ws_runner("user"));

            if matches!(
                self.exec_mode.as_str(),
                "SIGNAL_SNIPPER" | "SIGNAL_SNIPER" | "SIGNAL_SNIPE" | "SIGNAL"
            ) {
                let out = self._run_signal_sniper_loop();
                self.stop();
                return out;
            }
            if matches!(
                self.exec_mode.as_str(),
                "SNIPER" | "PROB_SNIPER" | "HIGH_PROB" | "HIGH_PROB_SNIPER" | "FIXED_PROFIT"
            ) {
                let out = self._run_sniper_loop();
                self.stop();
                return out;
            }

            let mut in_feed_pause = false;
            let mut unhedged_since: Option<f64> = None;
            let mut last_status_log_ts = 0.0;

            while !self.stop_flag.load(Ordering::SeqCst) {
                let wait_s = if self.exec_mode == "TAKER_PAIR" {
                    self.loop_wait_seconds_taker
                } else {
                    self.loop_wait_seconds_maker
                }
                .max(0.01);
                thread::sleep(Duration::from_secs_f64(wait_s.min(0.5)));

                let now = now_ts_f64();
                let every = (self.cfg.log_every as f64).max(0.5);
                if now - last_status_log_ts >= every {
                    last_status_log_ts = now;
                    self._log_status();
                }

                let (total_cost, qy, qn) = self
                    .state
                    .lock()
                    .map(|s| (s.c_yes + s.c_no, s.q_yes, s.q_no))
                    .unwrap_or((0.0, 0.0, 0.0));

                if total_cost >= self.cfg.max_total_cost {
                    self.logger.info(&format!(
                        "HARD SPEND CAP HIT total_cost={total_cost:.2} >= {:.2} -> CANCEL + STOP",
                        self.cfg.max_total_cost
                    ));
                    self._set_exit_reason("HARD_SPEND_CAP");
                    self.cancel_all_orders_exchange("hard spend cap");
                    break;
                }

                let mut seconds_left = self.expiry_ts as f64 - now;
                seconds_left -= 10.0;
                if seconds_left < self.cfg.stop_buffer_seconds as f64 {
                    let delta = qy - qn;
                    if delta.abs() >= self.cfg.min_shares {
                        self.logger.info(&format!(
                            "Near expiry ({seconds_left:.0}s). Forcing emergency hedge before stopping."
                        ));
                        self._emergency_taker_hedge_step(delta, "near_expiry");
                        thread::sleep(Duration::from_secs(1));
                    }
                    self.logger.info(&format!(
                        "Expiring in {seconds_left:.0}s -> stopping for rollover."
                    ));
                    self._set_exit_reason("ROLLOVER");
                    self.cancel_all_orders_exchange("expiry");
                    break;
                }

                if !self._market_data_fresh() {
                    if !in_feed_pause {
                        self.logger.info("FEED STALE/DOWN -> cancel all + pause.");
                        self.cancel_all_orders_exchange("feed stale");
                        in_feed_pause = true;
                    }
                    continue;
                } else if in_feed_pause {
                    self.logger.info("FEED OK -> resume.");
                    in_feed_pause = false;
                }

                let delta = qy - qn;
                if delta.abs() >= self.cfg.min_shares {
                    if unhedged_since.is_none() {
                        unhedged_since = Some(now);
                    }
                } else {
                    unhedged_since = None;
                }

                let lp = self.state.lock().map(|s| locked_profit(&s)).unwrap_or(0.0);
                if delta.abs() < 0.25 && lp >= self.cfg.lock_profit_target {
                    self.logger.info(&format!(
                        "Target hit. Canceling all orders first. lp={lp:.4}"
                    ));
                    self._set_exit_reason("TARGET_HIT");
                    self.cancel_all_orders_exchange("locked profit target");
                    thread::sleep(Duration::from_secs(2));

                    let (qy2, qn2, lp2) = self
                        .state
                        .lock()
                        .map(|s| (s.q_yes, s.q_no, locked_profit(&s)))
                        .unwrap_or((0.0, 0.0, 0.0));
                    if (qy2 - qn2).abs() < 0.25 && lp2 >= self.cfg.lock_profit_target {
                        self.logger
                            .info(&format!("Still flat after cancel. Stopping. lp={lp2:.4}"));
                        break;
                    }
                }

                if delta.abs() >= self.cfg.min_shares {
                    self._cancel_heavy_side_orders();
                    let unhedged_age = unhedged_since.map(|ts| now - ts).unwrap_or(0.0);
                    if self._maybe_trigger_max_loss(delta, unhedged_age) {
                        continue;
                    }
                    if self.exec_mode == "TAKER_PAIR" {
                        self._emergency_taker_hedge_step(delta, "exposed_taker_pair");
                    } else {
                        self._maker_exposure_step(delta, unhedged_age);
                    }
                    continue;
                }

                let remaining = self.cfg.max_total_cost - total_cost;
                if remaining <= 0.0 {
                    self.logger.info("spend cap hit (balanced) -> stop");
                    self._set_exit_reason("SPEND_CAP");
                    self.cancel_all_orders_exchange("spend cap");
                    break;
                }
                if remaining <= self.cfg.reserve_usd {
                    self.cancel_all_open_orders_local("reserve reached (balanced)");
                    continue;
                }

                if self.exec_mode == "TAKER_PAIR" {
                    let budget = (remaining - self.cfg.reserve_usd).max(0.0);
                    self._taker_pair_arb_step(budget);
                    continue;
                }

                let (ok, why) = self._accumulate_allowed();
                if !ok {
                    self.cancel_all_open_orders_local(&format!("accumulate gate: {why}"));
                    continue;
                }

                let (invalid, inv_reason) = self._quotes_invalidated();
                if invalid {
                    self.cancel_all_open_orders_local(&format!("quote invalidated: {inv_reason}"));
                    if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                        self._cancel_exchange_orders_for_assets(
                            &[y.clone(), n.clone()],
                            "quote invalidated",
                        );
                    }
                    continue;
                }

                let min_entry_edge_ticks =
                    env_int("MIN_ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks) as i64;
                let effective_edge_ticks = self.cfg.entry_edge_ticks.max(min_entry_edge_ticks);
                let entry_edge = effective_edge_ticks as f64 * self.cfg.tick;
                let (yes, no) = match (&self.yes_asset, &self.no_asset) {
                    (Some(y), Some(n)) => (y.as_str(), n.as_str()),
                    _ => break,
                };

                let y_bid = self._maker_bid_cross_ask_safe(yes, no, entry_edge);
                let n_bid = self._maker_bid_cross_ask_safe(no, yes, entry_edge);
                if y_bid.is_none() || n_bid.is_none() {
                    self.cancel_all_open_orders_local("no safe bids");
                    continue;
                }
                let y_bid = y_bid.unwrap_or(0.0);
                let n_bid = n_bid.unwrap_or(0.0);

                let yq = self._best_bid_ask(yes);
                let nq = self._best_bid_ask(no);
                if yq.is_none() || nq.is_none() {
                    self.cancel_all_open_orders_local("missing quotes for paired gate");
                    continue;
                }
                let (_, y_ask) = yq.unwrap_or((0.0, 0.0));
                let (_, n_ask) = nq.unwrap_or((0.0, 0.0));
                let tick = if self.cfg.tick > 0.0 {
                    self.cfg.tick
                } else {
                    0.01
                };
                let buf = env_float("PAIRED_ENTRY_BUFFER_TICKS", 0.0) * tick;
                let tix = |p: f64| -> i64 { (p / tick + 1e-9).round() as i64 };
                let thr_no_ticks = tix(1.0 - y_bid - buf);
                let thr_yes_ticks = tix(1.0 - n_bid - buf);
                if tix(n_ask) > thr_no_ticks {
                    self.cancel_all_open_orders_local("paired gate fail (NO ask)");
                    continue;
                }
                if tix(y_ask) > thr_yes_ticks {
                    self.cancel_all_open_orders_local("paired gate fail (YES ask)");
                    continue;
                }
                if (y_bid + n_bid) > (1.0 - entry_edge) {
                    self.cancel_all_open_orders_local("entry edge fail");
                    continue;
                }

                let mut size = self.cfg.clip_shares.max(self.cfg.min_shares);
                if env_bool("DEPTH_GATE_ENABLED", false) {
                    let (okd, whyd) = self._depth_gate_accumulate(size, y_bid, n_bid, buf);
                    if !okd && !env_bool("DEPTH_GATE_WARN_ONLY", false) {
                        self.cancel_all_open_orders_local(&format!("depth gate: {whyd}"));
                        continue;
                    }
                }

                let est = size * (y_bid + n_bid);
                let avail = remaining - self.cfg.reserve_usd;
                if est > avail {
                    continue;
                }
                size = size.max(self.cfg.min_shares);
                self._maybe_replace(yes, y_bid, size, None);
                self._maybe_replace(no, n_bid, size, None);
            }

            self.stop();
            self._get_exit_reason()
        });

        Ok(reason)
    }

    pub fn _init_clob_client(&self) -> Option<Value> {
        if self.clob_client.is_none() {
            return None;
        }
        Some(json!({
            "host": self.cfg.clob_host,
            "gamma_host": std::env::var("CLOB_GAMMA_API_URL")
                .or_else(|_| std::env::var("GAMMA_HOST"))
                .unwrap_or_else(|_| "https://gamma-api.polymarket.com".to_string()),
            "chain_id": self.cfg.chain_id,
            "signature_type": self.cfg.signature_type,
            "funder": self.cfg.funder,
            "has_api_creds": self.clob_api_creds.is_some(),
        }))
    }

    pub fn _mk_ws(&self, channel: &str) -> Value {
        let base = self.cfg.ws_base.trim_end_matches('/');
        let url = format!("{base}/ws/{channel}");
        let subscribe = if channel.eq_ignore_ascii_case("market") {
            match (&self.yes_asset, &self.no_asset) {
                (Some(yes), Some(no)) => Some(json!({
                    "assets_ids": [yes, no],
                    "type": "market",
                    "custom_feature_enabled": true
                })),
                _ => None,
            }
        } else if channel.eq_ignore_ascii_case("user") {
            match (&self.condition_id, self._user_ws_auth()) {
                (Some(condition_id), Some(auth)) => Some(json!({
                    "markets": [condition_id],
                    "type": "user",
                    "auth": auth
                })),
                _ => None,
            }
        } else {
            None
        };
        json!({
            "channel": channel,
            "url": url,
            "subscribe": subscribe,
            "market_slug": self.market_slug,
        })
    }

    pub fn _on_open(&self, channel: &str) {
        if channel.eq_ignore_ascii_case("market") {
            self.market_connected.store(true, Ordering::SeqCst);
        } else if channel.eq_ignore_ascii_case("user") {
            self.user_connected.store(true, Ordering::SeqCst);
        }
        self.logger.info(&format!("[{channel}] open"));
    }

    pub fn _on_error(&self, channel: &str, err: &str) {
        if channel.eq_ignore_ascii_case("market") {
            self.market_connected.store(false, Ordering::SeqCst);
        } else if channel.eq_ignore_ascii_case("user") {
            self.user_connected.store(false, Ordering::SeqCst);
        }
        self.logger.error(&format!("[{channel}] error: {err}"));
    }

    pub fn _on_close(&self, channel: &str, code: i64, msg: &str) {
        if channel.eq_ignore_ascii_case("market") {
            self.market_connected.store(false, Ordering::SeqCst);
        } else if channel.eq_ignore_ascii_case("user") {
            self.user_connected.store(false, Ordering::SeqCst);
        }
        self.logger
            .warning(&format!("[{channel}] closed: {code} {msg}"));
    }

    pub fn _ping_loop(&self, channel: &str) {
        self._dbg(
            &format!("[{channel}] ping"),
            &format!("ping_{channel}"),
            Some(10.0),
        );
    }

    pub fn _ws_runner(&self, channel: &str) {
        let mut backoff = self.cfg.ws_reconnect_min.max(0.1);
        let ping_interval = env_float("WS_PING_INTERVAL", 10.0).max(1.0);
        let io_timeout = env_float("WS_IO_TIMEOUT_SECONDS", 1.0).max(0.25);

        while !self.stop_flag.load(Ordering::SeqCst) {
            if channel.eq_ignore_ascii_case("market") {
                self.market_connected.store(false, Ordering::SeqCst);
            } else if channel.eq_ignore_ascii_case("user") {
                self.user_connected.store(false, Ordering::SeqCst);
            }

            let ws_meta = self._mk_ws(channel);
            let url = ws_meta
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if url.trim().is_empty() {
                self._on_error(channel, "missing ws url");
                break;
            }

            let (mut ws, _) = match connect(url.as_str()) {
                Ok(v) => v,
                Err(e) => {
                    self._on_error(channel, &format!("connect error: {e}"));
                    let sleep_for = backoff.min(self.cfg.ws_reconnect_max.max(0.1))
                        * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
                    self.logger
                        .info(&format!("[{channel}] reconnecting in {sleep_for:.1}s..."));
                    thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                    backoff = (backoff * 2.0).min(self.cfg.ws_reconnect_max.max(0.1));
                    continue;
                }
            };

            self._set_ws_stream_timeouts(&mut ws, Duration::from_secs_f64(io_timeout));
            self._on_open(channel);
            backoff = self.cfg.ws_reconnect_min.max(0.1);

            if let Some(sub) = ws_meta.get("subscribe").filter(|v| !v.is_null()) {
                let text = sub.to_string();
                if let Err(e) = ws.send(Message::Text(text.into())) {
                    self._on_error(channel, &format!("subscribe error: {e}"));
                    self._on_close(channel, 1006, "subscribe failed");
                    continue;
                }
            } else if channel.eq_ignore_ascii_case("user") {
                self.logger.warning(
                    "[user] missing ws auth or condition id; user feed will be unavailable",
                );
            }

            let mut last_ping = Instant::now();
            let mut close_code: i64 = 1000;
            let mut close_msg = "reconnect".to_string();
            while !self.stop_flag.load(Ordering::SeqCst) {
                if last_ping.elapsed() >= Duration::from_secs_f64(ping_interval) {
                    self._ping_loop(channel);
                    if let Err(e) = ws.send(Message::Ping(Vec::new().into())) {
                        close_code = 1006;
                        close_msg = format!("ping failed: {e}");
                        self._on_error(channel, &close_msg);
                        break;
                    }
                    last_ping = Instant::now();
                }

                match ws.read() {
                    Ok(msg) => match msg {
                        Message::Text(text) => {
                            if channel.eq_ignore_ascii_case("market") {
                                self.on_market_message(text.as_ref());
                            } else if channel.eq_ignore_ascii_case("user") {
                                self.on_user_message(text.as_ref());
                            }
                        }
                        Message::Binary(bin) => {
                            if let Ok(text) = String::from_utf8(bin.to_vec()) {
                                if channel.eq_ignore_ascii_case("market") {
                                    self.on_market_message(&text);
                                } else if channel.eq_ignore_ascii_case("user") {
                                    self.on_user_message(&text);
                                }
                            }
                        }
                        Message::Ping(payload) => {
                            let _ = ws.send(Message::Pong(payload));
                        }
                        Message::Pong(_) => {}
                        Message::Close(frame) => {
                            close_code = frame
                                .as_ref()
                                .map(|f| u16::from(f.code) as i64)
                                .unwrap_or(1000);
                            close_msg = frame
                                .as_ref()
                                .map(|f| f.reason.to_string())
                                .unwrap_or_else(|| "closed".to_string());
                            break;
                        }
                        _ => {}
                    },
                    Err(tungstenite::Error::Io(e))
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                    }
                    Err(tungstenite::Error::ConnectionClosed)
                    | Err(tungstenite::Error::AlreadyClosed) => {
                        close_code = 1000;
                        close_msg = "connection closed".to_string();
                        break;
                    }
                    Err(e) => {
                        close_code = 1006;
                        close_msg = e.to_string();
                        self._on_error(channel, &close_msg);
                        break;
                    }
                }
            }

            let _ = ws.close(None);
            if self.stop_flag.load(Ordering::SeqCst) {
                break;
            }
            self._on_close(channel, close_code, &close_msg);

            let mut rng = rand::thread_rng();
            let sleep_for =
                backoff.min(self.cfg.ws_reconnect_max.max(0.1)) * (0.7 + rng.gen_range(0.0..0.6));
            self.logger
                .info(&format!("[{channel}] reconnecting in {sleep_for:.1}s..."));
            thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
            backoff = (backoff * 2.0).min(self.cfg.ws_reconnect_max.max(0.1));
        }
    }

    pub fn _handle_market_event(&self, msg: &Value) {
        let et = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(
            et.as_str(),
            "tick_size_change" | "ticksizechange" | "tick_size" | "ticksize"
        ) {
            if let Some(v) = self._extract_float_any(
                msg,
                &[
                    "tick_size",
                    "tickSize",
                    "new_tick_size",
                    "newTickSize",
                    "value",
                ],
            ) {
                self.logger
                    .info(&format!("tick_size change signal detected: {v:.6}"));
            }
            self.cancel_all_open_orders_local("tick size change");
            if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                self._cancel_exchange_orders_for_assets(
                    &[y.clone(), n.clone()],
                    "tick size change",
                );
            }
            return;
        }
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
            .to_string();
        if asset_id.is_empty() {
            return;
        }
        let bid = self
            ._extract_float_any(msg, &["best_bid", "bid", "b"])
            .unwrap_or(0.0);
        let ask = self
            ._extract_float_any(msg, &["best_ask", "ask", "a"])
            .unwrap_or(0.0);
        let ts = now_ts_f64();
        if let Ok(mut quotes) = self.best_quotes.lock() {
            quotes.insert(asset_id, (bid, ask, ts));
        }
        if let Ok(mut last) = self.market_last_update_ts.lock() {
            *last = ts;
        }
    }

    pub fn on_market_message(&self, message: &str) {
        if let Ok(v) = serde_json::from_str::<Value>(message) {
            if let Some(items) = v.as_array() {
                for item in items {
                    if item.is_object() {
                        self._handle_market_event(item);
                    }
                }
            } else if v.is_object() {
                self._handle_market_event(&v);
            }
        }
    }

    pub fn _market_data_fresh(&self) -> bool {
        if !self.market_connected.load(Ordering::SeqCst) {
            return false;
        }
        if env_bool("REQUIRE_USER_WS_CONNECTED", true)
            && !self.user_connected.load(Ordering::SeqCst)
        {
            return false;
        }

        let stale_s = self.cfg.market_data_stale_seconds.max(1) as f64;
        let now = now_ts_f64();
        let yes = self.yes_asset.clone();
        let no = self.no_asset.clone();
        let quotes = match self.best_quotes.lock() {
            Ok(m) => m,
            Err(_) => return false,
        };
        for aid in [yes, no].into_iter().flatten() {
            let (_, _, ts) = match quotes.get(&aid).copied() {
                Some(v) => v,
                None => return false,
            };
            if ts <= 0.0 || (now - ts) > stale_s {
                return false;
            }
        }
        true
    }

    pub fn _best_bid_ask(&self, asset_id: &str) -> Option<(f64, f64)> {
        self.best_quotes
            .lock()
            .ok()
            .and_then(|m| m.get(asset_id).cloned().map(|(b, a, _)| (b, a)))
    }

    pub fn _dbg(&self, msg: &str, key: &str, throttle_s: Option<f64>) {
        let throttle = throttle_s.unwrap_or(env_float("DEBUG_THROTTLE_SECONDS", 1.0));
        let now = now_ts_f64();
        if let Ok(mut m) = self.debug_last_ts.lock() {
            let last = m.get(key).copied().unwrap_or(0.0);
            if now - last < throttle {
                return;
            }
            m.insert(key.to_string(), now);
        }
        self.logger.info(msg);
    }

    pub fn _dbg_maker(&self, msg: &str, key: &str, throttle_s: Option<f64>) {
        self._dbg(msg, key, throttle_s);
    }

    pub fn _book_url(&self) -> String {
        format!("{}/book", self.cfg.clob_host.trim_end_matches('/'))
    }

    pub fn _extract_float_any(&self, obj: &Value, keys: &[&str]) -> Option<f64> {
        for k in keys {
            let v = obj.get(*k)?;
            let f = match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.parse::<f64>().ok(),
                _ => None,
            };
            if f.is_some() {
                return f;
            }
        }
        None
    }

    pub fn _fetch_book_summary_http(&self, token_id: &str) -> Option<Value> {
        let url = self._book_url();
        let timeout_s = env_float("ORDERBOOK_HTTP_TIMEOUT", 3.0).max(0.25);
        let client = Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
            .ok()?;
        client
            .get(url)
            .query(&[("token_id", token_id)])
            .send()
            .ok()?
            .json::<Value>()
            .ok()
    }

    pub fn _get_book_cached(
        &self,
        token_id: &str,
        max_age_seconds: Option<f64>,
        force: bool,
    ) -> Option<Value> {
        let max_age = max_age_seconds.unwrap_or(env_float("BOOK_CACHE_TTL_SECONDS", 0.5));
        let now = now_ts_f64();
        if !force {
            if let Ok(cache) = self.book_cache.lock() {
                if let Some((v, ts)) = cache.get(token_id) {
                    if now - *ts <= max_age {
                        return Some(v.clone());
                    }
                }
            }
        }
        let book = self._fetch_book_summary_http(token_id)?;
        if let Ok(mut cache) = self.book_cache.lock() {
            cache.insert(token_id.to_string(), (book.clone(), now));
        }
        Some(book)
    }

    pub fn _iter_book_levels(&self, levels: &Value) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let arr = match levels {
            Value::Array(a) => a,
            _ => return out,
        };
        for lvl in arr {
            if let Value::Object(map) = lvl {
                let p = map
                    .get("price")
                    .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()));
                let s = map
                    .get("size")
                    .or_else(|| map.get("qty"))
                    .or_else(|| map.get("quantity"))
                    .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()));
                if let (Some(px), Some(sz)) = (p, s) {
                    out.push((px, sz));
                }
            }
        }
        out
    }

    pub fn _book_side_levels(&self, book: &Value, side: &str) -> Option<Value> {
        let side_l = side.to_ascii_lowercase();
        if side_l.starts_with('b') {
            return book
                .get("bids")
                .cloned()
                .or_else(|| book.get("bid").cloned());
        }
        if side_l.starts_with('a') {
            return book
                .get("asks")
                .cloned()
                .or_else(|| book.get("ask").cloned());
        }
        None
    }

    pub fn _cum_depth(
        &self,
        token_id: &str,
        side: &str,
        price_limit: f64,
        _max_levels: Option<usize>,
        max_age_seconds: Option<f64>,
    ) -> f64 {
        let book = match self._get_book_cached(token_id, max_age_seconds, false) {
            Some(b) => b,
            None => return 0.0,
        };
        let side_levels = match self._book_side_levels(&book, side) {
            Some(v) => v,
            None => return 0.0,
        };
        let levels = self._iter_book_levels(&side_levels);
        let mut total = 0.0;
        let ask_side = side.to_ascii_lowercase().starts_with('a');
        for (px, sz) in levels {
            let ok = if ask_side {
                px <= price_limit + 1e-12
            } else {
                px >= price_limit - 1e-12
            };
            if ok {
                total += sz.max(0.0);
            }
        }
        total
    }

    pub fn _apply_tick_dependent_params(&mut self) {
        if self.cfg.tick <= 0.0 {
            self.cfg.tick = 0.01;
        }
        self.max_spread_ticks = self.max_spread_ticks.max(1);
        self.hedge_slippage_ticks = self.hedge_slippage_ticks.max(0);
    }

    pub fn _sync_market_params_from_book(&mut self, force: bool) {
        if !force && !env_bool("AUTO_DETECT_MARKET_PARAMS", true) {
            return;
        }
        self._apply_tick_dependent_params();
    }

    pub fn _depth_gate_accumulate(
        &self,
        size: f64,
        y_bid: f64,
        n_bid: f64,
        buf: f64,
    ) -> (bool, String) {
        if size <= 0.0 {
            return (false, "size<=0".to_string());
        }
        if y_bid <= 0.0 || n_bid <= 0.0 {
            return (false, "missing bid".to_string());
        }
        let pair = y_bid + n_bid;
        if pair > 1.0 - buf + 1e-12 {
            return (
                false,
                format!("pair too expensive: y_bid+n_bid={pair:.4} buf={buf:.4}"),
            );
        }
        (true, "ok".to_string())
    }

    pub fn _reconcile_state_from_balances(&self, reason: &str) -> bool {
        self.logger.info(&format!(
            "reconcile state from balances requested: {reason}"
        ));
        false
    }

    pub fn _chunked_unwind_heavy_leg(&self, delta: f64, reason: &str) {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let _ = self._reconcile_state_from_balances(&format!("unwind:{reason}"));
        let (qy, qn) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no))
            .unwrap_or((0.0, 0.0));
        let d = qy - qn;
        if d.abs() < self.cfg.min_shares {
            return;
        }
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let remaining = (d.abs() + 1e-12).floor() as i64;
        if remaining < min_int {
            return;
        }
        let mut chunk = env_float("UNWIND_CHUNK_SHARES", self.cfg.min_shares).floor() as i64;
        if chunk < min_int {
            chunk = min_int;
        }
        let max_passes = env_int("UNWIND_MAX_PASSES", 4).max(1) as usize;
        let wait_s = env_float("UNWIND_WAIT_AFTER_ORDER_SECONDS", 0.6).max(0.05);

        self.cancel_all_open_orders_local(&format!("chunked unwind ({reason})"));
        if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
            self._cancel_exchange_orders_for_assets(
                &[y.clone(), n.clone()],
                &format!("chunked unwind ({reason})"),
            );
        }

        for i in 0..max_passes {
            if self.stop_flag.load(Ordering::SeqCst) {
                return;
            }
            let (qy2, qn2) = self
                .state
                .lock()
                .map(|s| (s.q_yes, s.q_no))
                .unwrap_or((0.0, 0.0));
            let d2 = qy2 - qn2;
            if d2.abs() < self.cfg.min_shares {
                return;
            }
            let heavy_asset = if d2 > 0.0 {
                self.yes_asset.clone()
            } else {
                self.no_asset.clone()
            };
            let Some(heavy_asset) = heavy_asset else {
                return;
            };
            let rem = (d2.abs() + 1e-12).floor() as i64;
            if rem < min_int {
                return;
            }
            let ba = self._best_bid_ask(&heavy_asset);
            let Some((bid, _)) = ba else {
                return;
            };
            if bid <= 0.0 {
                return;
            }
            let slip_ticks =
                env_int("MAKER_EXPOSURE_UNWIND_SLIPPAGE_TICKS", 0).max(0) as i64 + i as i64;
            let mut px = bid - slip_ticks as f64 * tick;
            px = clamp(round_down(px, tick), tick, 0.99);

            let mut sell_int = rem.min(chunk);
            if env_bool("UNWIND_DEPTH_GATE_ENABLED", true) {
                let levels = env_int("DEPTH_GATE_LEVELS", 50).max(1) as usize;
                let age = env_float("DEPTH_GATE_MAX_AGE_SECONDS", 1.5).max(0.05);
                let depth = self._cum_depth(&heavy_asset, "bids", px, Some(levels), Some(age));
                let mut depth_int = (depth + 1e-9).floor() as i64;
                depth_int = if depth_int >= min_int {
                    (depth_int / min_int) * min_int
                } else {
                    0
                };
                if depth_int >= min_int {
                    sell_int = sell_int.min(depth_int);
                } else {
                    continue;
                }
            }
            if sell_int < min_int {
                continue;
            }
            let ot_name = std::env::var("MAKER_EXPOSURE_UNWIND_ORDER_TYPE")
                .unwrap_or_else(|_| self.hedge_taker_order_type.clone());
            self.logger.info(&format!(
                "CHUNKED UNWIND ({reason}) heavy={} rem={rem} sell={sell_int} bid={bid:.3} px={px:.3} pass={}/{} type={}",
                heavy_asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>(),
                i + 1,
                max_passes,
                ot_name.to_ascii_uppercase()
            ));
            self._runtime_ts_set("__taker_inflight_until", now_ts_f64() + wait_s.max(0.75));
            let _ = self._place_taker_ask_fak(
                &heavy_asset,
                px,
                sell_int as f64,
                Some(&ot_name.to_ascii_uppercase()),
            );
            thread::sleep(Duration::from_secs_f64(wait_s));
        }
    }

    pub fn _fsm_set_state(&self, new_state: &str, reason: &str) {
        if let Ok(mut st) = self.fsm_state.lock() {
            let old = st.clone();
            *st = new_state.to_string();
            self.logger
                .info(&format!("FSM {old} -> {new_state} ({reason})"));
        }
    }

    pub fn _apply_cfg_overrides_from_env(&mut self) {
        self.cfg.min_shares = env_float("MIN_SHARES", self.cfg.min_shares);
        self.cfg.clip_shares = env_float("CLIP_SHARES", self.cfg.clip_shares);
        self.cfg.entry_edge_ticks = env_int("ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks) as i64;
        self.cfg.hedge_buffer_ticks =
            env_int("HEDGE_BUFFER_TICKS", self.cfg.hedge_buffer_ticks) as i64;
        self.cfg.maker_buffer_ticks =
            env_int("MAKER_BUFFER_TICKS", self.cfg.maker_buffer_ticks) as i64;
        self.cfg.improve_bid_ticks =
            env_int("IMPROVE_BID_TICKS", self.cfg.improve_bid_ticks) as i64;
        self.cfg.replace_if_price_moves_ticks = env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            self.cfg.replace_if_price_moves_ticks,
        ) as i64;
        self.cfg.stale_seconds = env_int("STALE_SECONDS", self.cfg.stale_seconds) as i64;
    }

    pub fn _accumulate_allowed(&self) -> (bool, String) {
        let now = now_ts_f64();
        if now < (self.start_ts as f64 + self.warmup_seconds as f64) {
            return (false, "warmup".to_string());
        }
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return (false, "missing_assets".to_string()),
        };
        let y = self._best_bid_ask(yes);
        let n = self._best_bid_ask(no);
        if y.is_none() || n.is_none() {
            return (false, "missing_quotes".to_string());
        }
        let (yb, ya) = y.unwrap_or((0.0, 0.0));
        let (nb, na) = n.unwrap_or((0.0, 0.0));
        if yb <= 0.0 || ya <= 0.0 || nb <= 0.0 || na <= 0.0 {
            return (false, "zero_bid_ask".to_string());
        }

        let spr_y_ticks = (ya - yb) / self.cfg.tick.max(0.0001);
        let spr_n_ticks = (na - nb) / self.cfg.tick.max(0.0001);
        if spr_y_ticks > self.max_spread_ticks as f64 || spr_n_ticks > self.max_spread_ticks as f64
        {
            return (
                false,
                format!("wide_spread(y={spr_y_ticks:.1} n={spr_n_ticks:.1})"),
            );
        }

        let mid_y = 0.5 * (yb + ya);
        let mid_n = 0.5 * (nb + na);
        let parity = mid_y + mid_n;
        if (parity - 1.0).abs() > self.parity_tolerance {
            return (false, format!("parity_off({parity:.3})"));
        }

        (true, "ok".to_string())
    }

    pub fn _paired_quotes_active(&self) -> bool {
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y, n),
            _ => return false,
        };
        self.state
            .lock()
            .map(|s| s.open_orders.contains_key(yes) && s.open_orders.contains_key(no))
            .unwrap_or(false)
    }

    pub fn _quotes_invalidated(&self) -> (bool, String) {
        if !env_bool("QUOTE_INVALIDATION_ENABLED", true) {
            return (false, "disabled".to_string());
        }
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return (false, "missing_assets".to_string()),
        };
        let yq = self._best_bid_ask(yes);
        let nq = self._best_bid_ask(no);
        if yq.is_none() || nq.is_none() {
            return (false, "missing_quotes".to_string());
        }
        let (_, y_ask) = yq.unwrap_or((0.0, 0.0));
        let (_, n_ask) = nq.unwrap_or((0.0, 0.0));
        if y_ask <= 0.0 || n_ask <= 0.0 {
            return (false, "zero_ask".to_string());
        }

        let buf = env_float("QUOTE_INVALIDATION_BUFFER_TICKS", 0.0) * self.cfg.tick.max(0.0001);
        let mut reasons: Vec<String> = Vec::new();
        if let Ok(s) = self.state.lock() {
            if let Some(y_o) = s.open_orders.get(yes) {
                let y_p = y_o.price.unwrap_or(0.0);
                if y_p > 0.0 && n_ask > (1.0 - y_p - buf) {
                    reasons.push(format!(
                        "YES bid {y_p:.2} + NO ask {n_ask:.2} > {:.2}",
                        1.0 - buf
                    ));
                }
            }
            if let Some(n_o) = s.open_orders.get(no) {
                let n_p = n_o.price.unwrap_or(0.0);
                if n_p > 0.0 && y_ask > (1.0 - n_p - buf) {
                    reasons.push(format!(
                        "NO bid {n_p:.2} + YES ask {y_ask:.2} > {:.2}",
                        1.0 - buf
                    ));
                }
            }
            if let (Some(y_o), Some(n_o)) = (s.open_orders.get(yes), s.open_orders.get(no)) {
                let y_p = y_o.price.unwrap_or(0.0);
                let n_p = n_o.price.unwrap_or(0.0);
                let min_edge = env_int("MIN_ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks) as i64;
                let edge_ticks = self.cfg.entry_edge_ticks.max(min_edge);
                let entry_edge = edge_ticks as f64 * self.cfg.tick.max(0.0001);
                if (y_p + n_p) > (1.0 - entry_edge) {
                    reasons.push(format!(
                        "edge_lost(sum={:.2} > {:.2})",
                        y_p + n_p,
                        1.0 - entry_edge
                    ));
                }
            }
        }

        if reasons.is_empty() {
            (false, "ok".to_string())
        } else {
            (true, reasons.join("; "))
        }
    }

    pub fn _oco_after_maker_fill(&self, filled_qty_total: f64) -> bool {
        if env_bool("OCO_ON_FILL", true) && filled_qty_total > 0.0 {
            self.cancel_all_open_orders_local("oco_after_fill");
            return true;
        }
        false
    }

    pub fn _apply_fill(
        &self,
        asset_id: &str,
        price: f64,
        filled: f64,
        trade_key: &str,
        side: &str,
    ) -> bool {
        let side_u = side.trim().to_ascii_uppercase();
        if !matches!(side_u.as_str(), "BUY" | "SELL") {
            return false;
        }
        if filled <= 0.0 || price <= 0.0 || trade_key.trim().is_empty() {
            return false;
        }
        let mut guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if guard.seen_trade_keys.iter().any(|k| k == trade_key) {
            return false;
        }
        guard.seen_trade_keys.push(trade_key.to_string());

        let yes_asset = self.yes_asset.as_deref().unwrap_or_default();
        let sign = if side_u == "BUY" { 1.0 } else { -1.0 };
        let qty = sign * filled;
        if asset_id == yes_asset {
            guard.q_yes = (guard.q_yes + qty).max(0.0);
            guard.c_yes = (guard.c_yes + price * qty).max(0.0);
        } else if self.no_asset.as_deref() == Some(asset_id) {
            guard.q_no = (guard.q_no + qty).max(0.0);
            guard.c_no = (guard.c_no + price * qty).max(0.0);
        } else {
            return false;
        }
        let _ = save_state(&self.state_file, &mut guard);
        true
    }

    pub fn _lat_ms(&self, t1: f64, t0: f64) -> Option<i64> {
        if !t1.is_finite() || !t0.is_finite() {
            return None;
        }
        Some(((t1 - t0) * 1000.0).round() as i64)
    }

    pub fn _lat_us(&self, t1: f64, t0: f64) -> Option<i64> {
        if !t1.is_finite() || !t0.is_finite() {
            return None;
        }
        Some(((t1 - t0) * 1_000_000.0).round() as i64)
    }

    pub fn _set_active_signal_context(&self, sig: &Value, purpose: &str) {
        if let Ok(mut ctx) = self.active_signal_context.lock() {
            *ctx = Some(json!({
                "purpose": purpose,
                "signal": sig,
                "set_ts": now_ts_f64(),
            }));
        }
    }

    pub fn _clear_active_signal_context(&self) {
        if let Ok(mut ctx) = self.active_signal_context.lock() {
            *ctx = None;
        }
    }

    pub fn _get_active_signal_context(&self) -> Option<Value> {
        self.active_signal_context
            .lock()
            .ok()
            .and_then(|c| c.clone())
    }

    pub fn _utc_iso(&self, ts: f64) -> String {
        let sec = ts.floor() as i64;
        let nsec = ((ts - sec as f64).max(0.0) * 1_000_000_000.0) as u32;
        Utc.timestamp_opt(sec, nsec)
            .single()
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| Utc::now().to_rfc3339())
    }

    pub fn _should_file_log_submit_event(&self, sig_ts: f64) -> bool {
        if !env_bool("EXEC_LATENCY_FILE_LOG_ENABLED", true) {
            return false;
        }
        if env_bool("EXEC_LATENCY_FILE_LOG_SUBMIT_ALL_EVENTS", false) {
            return true;
        }
        sig_ts > 0.0 && env_bool("EXEC_LATENCY_FILE_LOG_SUBMIT_SIGNAL_EVENTS", true)
    }

    pub fn _latency_file_append(&self, rec: &Value) {
        if let Some(svc) = &self.latency_log {
            svc.append(rec);
        }
    }

    pub fn _prune_order_exec_context_locked(&self, now_ts: f64) {
        let ttl = env_float("EXEC_LATENCY_CONTEXT_TTL_SECONDS", 21600.0).max(1.0);
        let max_records = env_int("EXEC_LATENCY_MAX_CONTEXT_RECORDS", 50000).max(10) as usize;
        if let Ok(mut map) = self.order_exec_context.lock() {
            map.retain(|_, v| {
                let ts = v
                    .get("ts")
                    .and_then(|x| x.as_f64())
                    .or_else(|| v.get("post_start_ts").and_then(|x| x.as_f64()))
                    .unwrap_or(now_ts);
                now_ts - ts <= ttl
            });
            if map.len() > max_records {
                let mut keys: Vec<String> = map.keys().cloned().collect();
                keys.sort();
                let drop_n = map.len() - max_records;
                for k in keys.into_iter().take(drop_n) {
                    map.remove(&k);
                }
            }
        }
    }

    pub fn _track_order_execution_context(&self, order_id: &str, rec: &Value) {
        if order_id.trim().is_empty() {
            return;
        }
        if !env_bool("EXEC_LATENCY_LOG_ENABLED", true) {
            return;
        }

        let now = now_ts_f64();
        let mut rec2 = rec.clone();
        if !rec2.is_object() {
            rec2 = json!({});
        }

        if let Ok(mut timings) = self.submit_timing_cache.lock() {
            if let Some(t) = timings.remove(order_id) {
                if let (Some(dst), Some(src)) = (rec2.as_object_mut(), t.as_object()) {
                    for k in [
                        "sign_start_ns",
                        "sign_end_ns",
                        "sign_start_ts",
                        "sign_end_ts",
                        "prep_start_ns",
                        "prep_end_ns",
                        "prep_start_ts",
                        "prep_end_ts",
                        "post_start_ns",
                        "post_end_ns",
                        "post_start_ts",
                        "post_end_ts",
                        "order_submit_ts",
                        "fee_rate_bps",
                        "tick_size",
                        "neg_risk",
                    ] {
                        if let Some(v) = src.get(k) {
                            dst.insert(k.to_string(), v.clone());
                        }
                    }
                }
            }
        }

        let value_i64 = |v: Option<&Value>| -> Option<i64> {
            v.and_then(|x| match x {
                Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
                Value::String(s) => s.parse::<i64>().ok(),
                _ => None,
            })
        };
        let diff_us_ns = |start_ns: Option<i64>, end_ns: Option<i64>| -> Option<i64> {
            match (start_ns, end_ns) {
                (Some(start), Some(end)) if end >= start => {
                    Some(((end - start) as f64 / 1_000.0).round() as i64)
                }
                _ => None,
            }
        };
        let us_to_ms =
            |us: Option<i64>| -> Option<i64> { us.map(|v| ((v as f64) / 1000.0).round() as i64) };

        let submit_ts = Self::_value_f64(rec2.get("order_submit_ts"))
            .or_else(|| Self::_value_f64(rec2.get("post_end_ts")))
            .unwrap_or(now);
        let send_ts = Self::_value_f64(rec2.get("post_start_ts")).unwrap_or(submit_ts);
        let decide_ts = Self::_value_f64(rec2.get("decision_ts")).unwrap_or(send_ts);
        let decision_ns = value_i64(rec2.get("decision_ns"));
        let prep_start_ns = value_i64(rec2.get("prep_start_ns"));
        let prep_end_ns = value_i64(rec2.get("prep_end_ns"));
        let sign_start_ns = value_i64(rec2.get("sign_start_ns"));
        let sign_end_ns = value_i64(rec2.get("sign_end_ns"));
        let post_start_ns = value_i64(rec2.get("post_start_ns"));
        let post_end_ns = value_i64(rec2.get("post_end_ns"));

        let mut prep_us = diff_us_ns(prep_start_ns, prep_end_ns).or_else(|| {
            rec2.get("prep_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
        });
        if prep_us.is_none() {
            prep_us = rec2
                .get("prep_ms")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .map(|ms| ms * 1000);
        }
        if prep_us.is_none() {
            let prep_start_ts = Self::_value_f64(rec2.get("prep_start_ts")).unwrap_or(0.0);
            let prep_end_ts = Self::_value_f64(rec2.get("prep_end_ts")).unwrap_or(0.0);
            if prep_start_ts > 0.0 && prep_end_ts > 0.0 {
                prep_us = self._lat_us(prep_end_ts, prep_start_ts);
            }
        }
        let prep_ms = us_to_ms(prep_us);

        let sign_us = diff_us_ns(sign_start_ns, sign_end_ns).or_else(|| {
            rec2.get("sign_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("sign_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });
        let sign_ms = us_to_ms(sign_us);
        let sign_total_us: Option<i64> = if let (Some(p), Some(s)) = (prep_us, sign_us) {
            Some(p + s)
        } else {
            rec2.get("sign_total_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("sign_total_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        };
        let sign_total_ms = us_to_ms(sign_total_us);
        let mut decide_to_send_us = diff_us_ns(decision_ns, post_start_ns).or_else(|| {
            rec2.get("decision_to_post_start_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("decision_to_post_start_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });
        let mut send_to_ack_us = diff_us_ns(post_start_ns, post_end_ns).or_else(|| {
            rec2.get("post_start_to_post_end_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("post_start_to_post_end_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });
        let mut decide_to_ack_us = diff_us_ns(decision_ns, post_end_ns).or_else(|| {
            rec2.get("decision_to_post_end_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("decision_to_post_end_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });

        if decide_to_send_us.is_none() {
            decide_to_send_us = self._lat_us(send_ts, decide_ts);
        }
        if send_to_ack_us.is_none() {
            send_to_ack_us = self._lat_us(submit_ts, send_ts);
        }
        if decide_to_ack_us.is_none() {
            decide_to_ack_us = self._lat_us(submit_ts, decide_ts);
        }
        let decide_to_send_ms = us_to_ms(decide_to_send_us);
        let send_to_ack_ms = us_to_ms(send_to_ack_us);
        let decide_to_ack_ms = us_to_ms(decide_to_ack_us);

        let asset_id = rec2
            .get("asset_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let side = rec2
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let origin = rec2
            .get("origin")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let px_limit = Self::_value_f64(rec2.get("px_limit"));
        let size = Self::_value_f64(rec2.get("size"));

        let mut signal_key = String::new();
        let mut signal_direction = String::new();
        let mut signal_provider = String::new();
        let mut signal_market_slug = String::new();
        let mut sig_to_decide_us: Option<i64> = None;
        let mut sig_to_send_us: Option<i64> = None;
        let mut sig_to_ack_us: Option<i64> = None;
        let mut sig_to_submit_us: Option<i64> = None;
        let mut sig_ts = 0.0_f64;

        if let Some(ctx) = self._get_active_signal_context() {
            let sig = ctx.get("signal").unwrap_or(&ctx);
            sig_ts = Self::_value_f64(sig.get("received_ts"))
                .or_else(|| Self::_value_f64(ctx.get("signal_received_ts")))
                .unwrap_or(0.0);
            signal_key = sig
                .get("key")
                .or_else(|| ctx.get("signal_key"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            signal_direction = sig
                .get("direction")
                .or_else(|| ctx.get("signal_direction"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            signal_provider = sig
                .get("provider")
                .or_else(|| ctx.get("signal_provider"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            signal_market_slug = sig
                .get("market_slug")
                .or_else(|| ctx.get("signal_market_slug"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if sig_ts > 0.0 {
                sig_to_decide_us = self._lat_us(decide_ts, sig_ts);
                sig_to_send_us = self._lat_us(send_ts, sig_ts);
                sig_to_ack_us = self._lat_us(submit_ts, sig_ts);
                sig_to_submit_us = sig_to_ack_us;
            }
        }
        let sig_to_decide_ms = us_to_ms(sig_to_decide_us);
        let sig_to_send_ms = us_to_ms(sig_to_send_us);
        let sig_to_ack_ms = us_to_ms(sig_to_ack_us);
        let sig_to_submit_ms = us_to_ms(sig_to_submit_us);

        if let Some(obj) = rec2.as_object_mut() {
            obj.insert("order_id".to_string(), json!(order_id));
            obj.insert("order_submit_ts".to_string(), json!(submit_ts));
            obj.insert("post_end_ts".to_string(), json!(submit_ts));
            obj.insert("post_start_ts".to_string(), json!(send_ts));
            obj.insert("decision_ts".to_string(), json!(decide_ts));
            obj.insert("decision_ns".to_string(), json!(decision_ns));
            obj.insert("prep_start_ns".to_string(), json!(prep_start_ns));
            obj.insert("prep_end_ns".to_string(), json!(prep_end_ns));
            obj.insert("prep_us".to_string(), json!(prep_us));
            obj.insert("prep_ms".to_string(), json!(prep_ms));
            obj.insert("sign_start_ns".to_string(), json!(sign_start_ns));
            obj.insert("sign_end_ns".to_string(), json!(sign_end_ns));
            obj.insert("post_start_ns".to_string(), json!(post_start_ns));
            obj.insert("post_end_ns".to_string(), json!(post_end_ns));
            obj.insert("sign_us".to_string(), json!(sign_us));
            obj.insert("sign_ms".to_string(), json!(sign_ms));
            obj.insert("sign_total_us".to_string(), json!(sign_total_us));
            obj.insert("sign_total_ms".to_string(), json!(sign_total_ms));
            obj.insert(
                "decision_to_post_start_us".to_string(),
                json!(decide_to_send_us),
            );
            obj.insert(
                "decision_to_post_start_ms".to_string(),
                json!(decide_to_send_ms),
            );
            obj.insert(
                "post_start_to_post_end_us".to_string(),
                json!(send_to_ack_us),
            );
            obj.insert(
                "post_start_to_post_end_ms".to_string(),
                json!(send_to_ack_ms),
            );
            obj.insert(
                "decision_to_post_end_us".to_string(),
                json!(decide_to_ack_us),
            );
            obj.insert(
                "decision_to_post_end_ms".to_string(),
                json!(decide_to_ack_ms),
            );
            obj.insert("signal_key".to_string(), json!(signal_key));
            obj.insert("signal_direction".to_string(), json!(signal_direction));
            obj.insert("signal_provider".to_string(), json!(signal_provider));
            obj.insert("signal_market_slug".to_string(), json!(signal_market_slug));
            obj.insert(
                "signal_received_ts".to_string(),
                json!(if sig_ts > 0.0 {
                    Some(sig_ts)
                } else {
                    None::<f64>
                }),
            );
            obj.insert("signal_to_decision_ms".to_string(), json!(sig_to_decide_ms));
            obj.insert("signal_to_decision_us".to_string(), json!(sig_to_decide_us));
            obj.insert("signal_to_post_start_ms".to_string(), json!(sig_to_send_ms));
            obj.insert("signal_to_post_start_us".to_string(), json!(sig_to_send_us));
            obj.insert("signal_to_post_end_ms".to_string(), json!(sig_to_ack_ms));
            obj.insert("signal_to_post_end_us".to_string(), json!(sig_to_ack_us));
            obj.insert("signal_to_submit_ms".to_string(), json!(sig_to_submit_ms));
            obj.insert("signal_to_submit_us".to_string(), json!(sig_to_submit_us));
            if !obj.contains_key("ts") {
                obj.insert("ts".to_string(), json!(now));
            }
        }

        if env_bool("EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE", true) {
            let em = self.exec_mode.trim().to_ascii_uppercase();
            let allow_maker = env_bool("EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE_MAKER", false);
            let allow = !(em == "MAKER"
                && !allow_maker
                && !origin.trim().to_ascii_uppercase().starts_with("TAKER"));
            if allow {
                let aid_tail: String = asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let d2s_us = decide_to_send_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let s2a_us = send_to_ack_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let d2a_us = decide_to_ack_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let pm_us = prep_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let sm_us = sign_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let stm_us = sign_total_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                self.logger.info(&format!(
                    "[LATENCY][SUBMIT] decide->send={d2s_us}us send->ack={s2a_us}us decide->ack={d2a_us}us prep={pm_us}us sign={sm_us}us sign_total={stm_us}us oid={}.. asset={aid_tail} side={side} origin={origin}",
                    order_id.chars().take(10).collect::<String>(),
                ));
            }
        }

        if sig_ts > 0.0 {
            if let Some(us) = sig_to_submit_us {
                let aid_tail: String = asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.info(&format!(
                    "[LATENCY][SIGNAL->SUBMIT] {us}us key={signal_key} dir={signal_direction} oid={}.. asset={aid_tail} side={side} origin={origin}",
                    order_id.chars().take(10).collect::<String>(),
                ));
            }
        }

        if self._should_file_log_submit_event(sig_ts) {
            let row = json!({
                "event": "SUBMIT",
                "ts": submit_ts,
                "ts_utc": self._utc_iso(submit_ts),
                "market_slug": self.market_slug,
                "exec_mode": self.exec_mode,
                "order_id": order_id,
                "asset_id": asset_id,
                "side": side,
                "origin": origin,
                "source": "ORDER_SUBMIT",
                "price": px_limit,
                "qty": size,
                "signal_key": signal_key,
                "signal_direction": signal_direction,
                "signal_provider": signal_provider,
                "signal_market_slug": signal_market_slug,
                "signal_received_ts": if sig_ts > 0.0 { Some(sig_ts) } else { None::<f64> },
                "decision_ts": decide_ts,
                "post_start_ts": send_ts,
                "post_end_ts": submit_ts,
                "order_submit_ts": submit_ts,
                "fill_ts": Value::Null,
                "prep_us": prep_us,
                "prep_ms": prep_ms,
                "sign_us": sign_us,
                "sign_ms": sign_ms,
                "sign_total_us": sign_total_us,
                "sign_total_ms": sign_total_ms,
                "decision_to_post_start_us": decide_to_send_us,
                "decision_to_post_start_ms": decide_to_send_ms,
                "post_start_to_post_end_us": send_to_ack_us,
                "post_start_to_post_end_ms": send_to_ack_ms,
                "decision_to_post_end_us": decide_to_ack_us,
                "decision_to_post_end_ms": decide_to_ack_ms,
                "signal_to_decision_us": sig_to_decide_us,
                "signal_to_decision_ms": sig_to_decide_ms,
                "signal_to_post_start_us": sig_to_send_us,
                "signal_to_post_start_ms": sig_to_send_ms,
                "signal_to_post_end_us": sig_to_ack_us,
                "signal_to_post_end_ms": sig_to_ack_ms,
                "signal_to_submit_us": sig_to_submit_us,
                "signal_to_submit_ms": sig_to_submit_ms,
                "signal_to_fill_ms": Value::Null,
                "post_start_to_fill_ms": Value::Null,
                "decision_to_fill_ms": Value::Null,
                "submit_to_fill_ms": Value::Null,
                "meta_json": rec2,
            });
            self._latency_file_append(&row);
        }

        if let Ok(mut map) = self.order_exec_context.lock() {
            map.insert(order_id.to_string(), rec2);
        }
        self._prune_order_exec_context_locked(now);
    }

    pub fn _get_order_execution_context(&self, order_id: &str) -> Option<Value> {
        self.order_exec_context
            .lock()
            .ok()
            .and_then(|m| m.get(order_id).cloned())
    }

    pub fn _log_execution_latency_on_fill(&self, order_id: &str, fill_ts: f64) {
        if !env_bool("EXEC_LATENCY_LOG_ENABLED", true) {
            return;
        }
        if let Some(ctx) = self._get_order_execution_context(order_id) {
            let mut rec = json!({
                "ts_utc": self._utc_iso(fill_ts),
                "event": "FILL",
                "order_id": order_id,
                "fill_ts": fill_ts,
                "meta_json": ctx,
            });
            let submit_ts = rec
                .get("meta_json")
                .and_then(|m| m.get("order_submit_ts"))
                .and_then(|x| x.as_f64())
                .or_else(|| {
                    rec.get("meta_json")
                        .and_then(|m| m.get("post_end_ts"))
                        .and_then(|x| x.as_f64())
                });
            if let Some(submit_ts) = submit_ts {
                if let Some(ms) = self._lat_ms(fill_ts, submit_ts) {
                    rec["submit_to_fill_ms"] = json!(ms);
                }
            }
            if let Some(ms) = rec.get("submit_to_fill_ms").and_then(|x| x.as_i64()) {
                self.logger.info(&format!(
                    "[LATENCY][FILL] submit->fill={ms}ms oid={}..",
                    order_id.chars().take(10).collect::<String>()
                ));
            } else {
                self.logger.info(&format!(
                    "[LATENCY][FILL] no_timing_ctx oid={}..",
                    order_id.chars().take(10).collect::<String>()
                ));
            }
            self._latency_file_append(&rec);
        }
    }

    pub fn _remember_taker_order(
        &self,
        order_id: &str,
        asset_id: &str,
        size: f64,
        px_limit: f64,
        side: &str,
    ) {
        if order_id.trim().is_empty() {
            return;
        }
        let rec = TakerOrderRecord {
            order_id: order_id.to_string(),
            asset_id: asset_id.to_string(),
            size,
            applied: 0.0,
            px_limit,
            side: side.to_ascii_uppercase(),
            ts: now_ts_f64(),
        };
        if let Ok(mut m) = self.taker_orders.lock() {
            m.insert(order_id.to_string(), rec);
        }
    }

    pub fn _is_recent_taker_order(&self, order_id: &str) -> bool {
        let ttl = self.taker_order_ttl_seconds as f64;
        self.taker_orders
            .lock()
            .ok()
            .and_then(|m| m.get(order_id).cloned())
            .map(|r| now_ts_f64() - r.ts <= ttl)
            .unwrap_or(false)
    }

    pub fn _has_pending_taker_order(&self, side: &str, asset_id: Option<&str>) -> bool {
        let s = side.to_ascii_uppercase();
        self.taker_orders
            .lock()
            .map(|m| {
                m.values().any(|r| {
                    let remaining = (r.size - r.applied).max(0.0);
                    r.side == s
                        && asset_id
                            .map(|aid| aid == r.asset_id.as_str())
                            .unwrap_or(true)
                        && remaining > 1e-9
                        && now_ts_f64() - r.ts <= self.taker_order_ttl_seconds as f64
                })
            })
            .unwrap_or(false)
    }

    pub fn _pending_taker_notional_usd(&self, side: &str, asset_id: Option<&str>) -> f64 {
        let s = side.to_ascii_uppercase();
        self.taker_orders
            .lock()
            .map(|m| {
                m.values()
                    .filter(|r| {
                        let remaining = (r.size - r.applied).max(0.0);
                        r.side == s
                            && asset_id
                                .map(|aid| aid == r.asset_id.as_str())
                                .unwrap_or(true)
                            && remaining > 1e-9
                            && now_ts_f64() - r.ts <= self.taker_order_ttl_seconds as f64
                    })
                    .map(|r| (r.size - r.applied).max(0.0) * r.px_limit)
                    .sum()
            })
            .unwrap_or(0.0)
    }

    pub fn _has_pending_taker_order_recent(
        &self,
        side: &str,
        asset_id: Option<&str>,
        max_age_seconds: f64,
    ) -> bool {
        let s = side.to_ascii_uppercase();
        self.taker_orders
            .lock()
            .map(|m| {
                m.values().any(|r| {
                    let remaining = (r.size - r.applied).max(0.0);
                    r.side == s
                        && asset_id
                            .map(|aid| aid == r.asset_id.as_str())
                            .unwrap_or(true)
                        && remaining > 1e-9
                        && now_ts_f64() - r.ts <= max_age_seconds
                })
            })
            .unwrap_or(false)
    }

    pub fn _get_position_size_data_api(&self, asset_id: &str) -> Option<f64> {
        let aid = asset_id.trim();
        if aid.is_empty() {
            return None;
        }
        let base = std::env::var("POLY_DATA_API_BASE_URL")
            .unwrap_or_else(|_| "https://data-api.polymarket.com".to_string());
        let url = format!("{}/positions", base.trim_end_matches('/'));
        let timeout_s = env_float("SNIPER_POSITIONS_API_TIMEOUT_SECONDS", 3.0).clamp(0.2, 15.0);
        let http = match Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
        {
            Ok(c) => c,
            Err(_) => return None,
        };

        let mut users: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let override_user = std::env::var("SNIPER_POSITIONS_USER").unwrap_or_default();
        for cand in [
            override_user,
            self.cfg.funder.clone().unwrap_or_default(),
            self.wallet_address.clone(),
        ] {
            let t = cand.trim().to_string();
            if t.is_empty() {
                continue;
            }
            let key = t.to_ascii_lowercase();
            if seen.insert(key) {
                users.push(t);
            }
        }
        if users.is_empty() {
            return None;
        }

        let market_filter = env_bool("SNIPER_POSITIONS_FILTER_MARKET", false);
        let market = self
            .condition_id
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let mut best_size = 0.0f64;
        let mut any_ok = false;
        for user in users {
            let mut req = http.get(&url).query(&[
                ("user", user.as_str()),
                ("sizeThreshold", "0"),
                ("limit", "500"),
                ("offset", "0"),
            ]);
            if market_filter {
                if let Some(mkt) = market.as_deref() {
                    req = req.query(&[("market", mkt)]);
                }
            }
            let resp = match req.send() {
                Ok(r) => r,
                Err(_) => continue,
            };
            if !resp.status().is_success() {
                continue;
            }
            let payload: Value = match resp.json() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if env_bool("SNIPER_POSITIONS_DEBUG_ALL", false) {
                let aid_tail: String = aid
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let user_tail: String = user
                    .chars()
                    .rev()
                    .take(8)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.info(&format!(
                    "[SNIPER][DBG_POS_API] asset={aid_tail} user=*{user_tail} resp={payload}"
                ));
            }
            let rows = payload
                .as_array()
                .cloned()
                .or_else(|| payload.get("data").and_then(|v| v.as_array()).cloned())
                .unwrap_or_default();
            let mut sz = 0.0f64;
            for row in &rows {
                let row_asset = row
                    .get("asset")
                    .or_else(|| row.get("asset_id"))
                    .or_else(|| row.get("token_id"))
                    .or_else(|| row.get("tokenId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if row_asset != aid {
                    continue;
                }
                let s = Self::_value_f64(row.get("size")).unwrap_or(0.0);
                if s.is_finite() && s > 0.0 {
                    sz += s;
                }
            }
            any_ok = true;
            if sz > best_size {
                best_size = sz;
            }
        }

        if any_ok {
            Some(best_size.max(0.0))
        } else {
            None
        }
    }

    pub fn _get_balance_allowance_conditional_cached(
        &self,
        token_id: &str,
        max_age_seconds: f64,
    ) -> Option<(f64, f64)> {
        let tid = token_id.trim().to_string();
        if tid.is_empty() {
            return None;
        }
        let now = now_ts_f64();

        if let Ok(cache) = self.balance_allowance_cache.lock() {
            if let Some((ts, bal, allow)) = cache.get(&tid) {
                if now - *ts <= max_age_seconds.max(0.0) {
                    return Some((*bal, *allow));
                }
            }
        }

        let units_per_share = std::env::var("POLY_CONDITIONAL_UNITS_PER_SHARE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(1_000_000.0);
        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return None,
        };
        let ba_update_enabled = env_bool("BALANCE_ALLOWANCE_UPDATE_ENABLED", true);

        // Keep CLOB service-side allowance snapshot fresh (Python parity: best-effort call).
        // Some deployments reject this endpoint or return non-JSON success bodies; in those
        // cases back off and continue with get_balance_allowance only.
        // continue with get_balance_allowance only.
        if ba_update_enabled && now >= self._runtime_ts_get("__ba_update_disabled_until") {
            if let Err(e) = rt.block_on(client.update_balance_allowance(BalanceAllowanceParams {
                asset_type: AssetType::Conditional,
                token_id: Some(tid.clone()),
            })) {
                let err_s = e.to_string();
                let err_l = err_s.to_ascii_lowercase();
                let disable_refresh = err_s.contains("405")
                    || err_s.contains("404")
                    || err_l.contains("method not allowed")
                    || err_l.contains("not found")
                    || err_l.contains("failed to parse json response")
                    || err_l.contains("error decoding response body")
                    || err_l.contains("eof while parsing a value");
                if disable_refresh {
                    self._runtime_ts_set("__ba_update_disabled_until", now + 3600.0);
                    if now >= self._runtime_ts_get("__ba_update_disable_logged_until") {
                        self.logger.warning(&format!(
                            "[BAL] update_balance_allowance unavailable ({err_s}); disabling this refresh call for 1h and continuing with get_balance_allowance."
                        ));
                        self._runtime_ts_set("__ba_update_disable_logged_until", now + 3600.0);
                    }
                }
            }
        }

        let resp = match rt.block_on(client.get_balance_allowance(BalanceAllowanceParams {
            asset_type: AssetType::Conditional,
            token_id: Some(tid.clone()),
        })) {
            Ok(v) => v,
            Err(e) => {
                let tail: String = tid
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.warning(&format!(
                    "[BAL] get_balance_allowance failed token={tail} err={e}"
                ));
                return None;
            }
        };
        if env_bool("BALANCE_ALLOWANCE_DEBUG_ALL", false) {
            let tail: String = tid
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger.info(&format!(
                "[BAL][DBG_ALL] get_balance_allowance token={tail} resp={}",
                resp
            ));
        }

        let bal_raw = Self::_value_f64(resp.get("balance"))
            .or_else(|| Self::_max_numeric_in_value(resp.get("balances")))
            .unwrap_or(0.0);
        let allow_from_scalar = Self::_value_f64(resp.get("allowance"));
        let allow_from_map = Self::_max_numeric_in_value(resp.get("allowances"));
        let allow_raw = allow_from_scalar.or(allow_from_map).unwrap_or(0.0);
        let bal = bal_raw / units_per_share;
        let allow = allow_raw / units_per_share;
        self._runtime_ts_set("__ba_last_fetch_ts", now);
        self._runtime_ts_set("__ba_last_raw_balance", bal_raw);
        self._runtime_ts_set("__ba_last_raw_allowance", allow_raw);
        self._runtime_ts_set("__ba_last_units_per_share", units_per_share);
        self._runtime_ts_set("__ba_last_balance_shares", bal);
        self._runtime_ts_set("__ba_last_allowance_shares", allow);

        if bal_raw <= 0.0 && allow_raw <= 0.0 {
            let next_dbg = self._runtime_ts_get("__ba_zero_payload_log_until");
            if now >= next_dbg {
                let keys = resp
                    .as_object()
                    .map(|m| m.keys().cloned().collect::<Vec<String>>().join(","))
                    .unwrap_or_else(|| "<non-object>".to_string());
                let allowances_snip = resp
                    .get("allowances")
                    .map(|v| {
                        let s = v.to_string();
                        if s.len() > 220 {
                            format!("{}...", &s[..220])
                        } else {
                            s
                        }
                    })
                    .unwrap_or_else(|| "<none>".to_string());
                let tail: String = tid
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.info(&format!(
                    "[BAL][DBG] token={tail} keys=[{keys}] raw_balance={bal_raw:.0} raw_allowance={allow_raw:.0} units_per_share={units_per_share:.0} allowances={allowances_snip}"
                ));
                self._runtime_ts_set("__ba_zero_payload_log_until", now + 5.0);
            }
        }

        if let Ok(mut cache) = self.balance_allowance_cache.lock() {
            cache.insert(tid, (now, bal, allow));
        }
        Some((bal, allow))
    }

    pub fn _taker_order_fallback_on_order_event(&self, msg: &Value) {
        let order_id = msg
            .get("order_id")
            .or_else(|| msg.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if order_id.is_empty() {
            return;
        }
        // Only apply fallback for recent taker orders we submitted.
        let rec0 = self
            .taker_orders
            .lock()
            .ok()
            .and_then(|m| m.get(order_id).cloned());
        let Some(rec0) = rec0 else {
            return;
        };

        let mut matched_total = Self::_value_f64(
            msg.get("size_matched")
                .or_else(|| msg.get("matched_size"))
                .or_else(|| msg.get("filled_size"))
                .or_else(|| msg.get("filled")),
        )
        .unwrap_or(0.0);
        if rec0.size > 0.0 {
            matched_total = matched_total.min(rec0.size.max(0.0));
        }
        let inc = (matched_total - rec0.applied).max(0.0);

        if inc > 1e-9 {
            let price = Self::_value_f64(msg.get("price")).unwrap_or(rec0.px_limit);
            let asset = msg
                .get("asset_id")
                .or_else(|| msg.get("token_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(rec0.asset_id.as_str());
            let side = msg
                .get("side")
                .and_then(|v| v.as_str())
                .filter(|s| matches!(s.trim().to_ascii_uppercase().as_str(), "BUY" | "SELL"))
                .unwrap_or(rec0.side.as_str());
            let key = format!("order_evt:{order_id}:{matched_total:.8}");
            let applied = self._apply_fill(asset, price, inc, &key, side);
            if applied {
                self._log_execution_latency_on_fill(order_id, now_ts_f64());
            }
        }

        let typ = msg
            .get("type")
            .or_else(|| msg.get("event_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let status = msg
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let done_hint = matches!(
            typ.as_str(),
            "CANCELLATION" | "CANCELED" | "CANCELLED" | "REJECTION" | "REJECTED"
        ) || matches!(
            status.as_str(),
            "CANCELED" | "CANCELLED" | "REJECTED" | "FILLED"
        );

        if let Ok(mut m) = self.taker_orders.lock() {
            let mut remove = false;
            if let Some(rec) = m.get_mut(order_id) {
                rec.applied = rec.applied.max(matched_total);
                rec.ts = now_ts_f64();
                if done_hint || (rec.size > 0.0 && rec.applied >= rec.size - 1e-9) {
                    remove = true;
                }
            }
            if remove {
                m.remove(order_id);
            }
        }
    }

    pub fn _handle_user_trade_event(&self, msg: &Value) {
        let event_type = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !event_type.is_empty() && !event_type.contains("trade") {
            return;
        }

        let status = msg
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let token_id = msg
            .get("asset_id")
            .or_else(|| msg.get("assetId"))
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("tokenId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let side_top = msg
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if !status.is_empty() && !token_id.trim().is_empty() {
            let key = format!(
                "__sniper_trade_status_{}_{}",
                token_id,
                status.to_ascii_lowercase()
            );
            self._runtime_ts_set(&key, now_ts_f64());
            if env_bool("SNIPER_ORDER_STATUS_DEBUG", false) {
                let tail: String = token_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.info(&format!(
                    "[SNIPER][TRADE] status={} asset={tail} side={side_top}",
                    status
                ));
            }
        }
        if status == "CONFIRMED" {
            if !token_id.trim().is_empty() && side_top == "BUY" {
                let confirmed_key = Self::_sniper_entry_confirmed_key(&token_id);
                self._runtime_ts_set(&confirmed_key, now_ts_f64());
                if env_bool("SNIPER_ORDER_STATUS_DEBUG", false) {
                    let tail: String = token_id
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    self.logger.info(&format!(
                        "[SNIPER][TRADE] entry status CONFIRMED asset={tail} (top-level BUY)"
                    ));
                }
            }
            // LIMIT/GTC entries can show up as maker legs; confirm using maker leg for our wallet.
            let wallet = self.wallet_address.to_ascii_lowercase();
            let maker_orders = msg
                .get("maker_orders")
                .or_else(|| msg.get("makerOrders"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for mo in maker_orders {
                let mo_addr = mo
                    .get("maker_address")
                    .or_else(|| mo.get("makerAddress"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !wallet.trim().is_empty() && !mo_addr.is_empty() && mo_addr != wallet {
                    continue;
                }
                let mo_side = mo
                    .get("side")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_uppercase();
                if mo_side != "BUY" {
                    continue;
                }
                let mo_asset = mo
                    .get("asset_id")
                    .or_else(|| mo.get("assetId"))
                    .or_else(|| mo.get("token_id"))
                    .or_else(|| mo.get("tokenId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if mo_asset.trim().is_empty() {
                    continue;
                }
                let confirmed_key = Self::_sniper_entry_confirmed_key(&mo_asset);
                self._runtime_ts_set(&confirmed_key, now_ts_f64());
                if env_bool("SNIPER_ORDER_STATUS_DEBUG", false) {
                    let tail: String = mo_asset
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    self.logger.info(&format!(
                        "[SNIPER][TRADE] entry status CONFIRMED asset={tail} (maker BUY leg)"
                    ));
                }
            }
        }
        if !status.is_empty() && !matches!(status.as_str(), "MATCHED" | "MINED" | "CONFIRMED") {
            return;
        }

        let trade_id = msg
            .get("id")
            .or_else(|| msg.get("trade_id"))
            .or_else(|| msg.get("tradeId"))
            .or_else(|| msg.get("tradeID"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let taker_oid = msg
            .get("taker_order_id")
            .or_else(|| msg.get("takerOrderId"))
            .or_else(|| msg.get("taker_orderId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let taker_rec = if taker_oid.trim().is_empty() {
            None
        } else {
            self.taker_orders
                .lock()
                .ok()
                .and_then(|m| m.get(&taker_oid).cloned())
        };
        let taker_ctx = if taker_oid.trim().is_empty() {
            None
        } else {
            self._get_order_execution_context(&taker_oid)
        };

        // CASE A: Taker trade event that matches a recent locally-submitted taker order.
        if taker_rec.is_some() || taker_ctx.is_some() {
            let msg_asset = msg
                .get("asset_id")
                .or_else(|| msg.get("token_id"))
                .or_else(|| msg.get("assetId"))
                .or_else(|| msg.get("tokenId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let msg_side = msg
                .get("side")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_uppercase();
            let ctx_asset = taker_ctx
                .as_ref()
                .and_then(|c| {
                    c.get("asset_id")
                        .or_else(|| c.get("token_id"))
                        .or_else(|| c.get("assetId"))
                        .or_else(|| c.get("tokenId"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("")
                .to_string();
            let ctx_side = taker_ctx
                .as_ref()
                .and_then(|c| c.get("side").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_ascii_uppercase();
            let ctx_px_limit = taker_ctx
                .as_ref()
                .and_then(|c| Self::_value_f64(c.get("px_limit").or_else(|| c.get("price"))))
                .unwrap_or(0.0);
            let ctx_size = taker_ctx
                .as_ref()
                .and_then(|c| Self::_value_f64(c.get("size")))
                .unwrap_or(0.0);

            let mut asset = taker_rec
                .as_ref()
                .map(|r| r.asset_id.clone())
                .unwrap_or_else(|| ctx_asset.clone());
            if asset.trim().is_empty() {
                asset = msg_asset.clone();
            }
            let mut side = taker_rec
                .as_ref()
                .map(|r| r.side.clone())
                .unwrap_or_else(|| ctx_side.clone());
            if !matches!(side.as_str(), "BUY" | "SELL") {
                side = msg_side.clone();
            }
            if (!msg_asset.trim().is_empty() && msg_asset != asset)
                || (matches!(msg_side.as_str(), "BUY" | "SELL") && msg_side != side)
            {
                let msg_tail: String = msg_asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let rec_tail: String = asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let mpx = Self::_value_f64(msg.get("price")).unwrap_or(0.0);
                let msz = Self::_value_f64(
                    msg.get("size")
                        .or_else(|| msg.get("filled"))
                        .or_else(|| msg.get("matched_amount"))
                        .or_else(|| msg.get("matchedAmount"))
                        .or_else(|| msg.get("amount")),
                )
                .unwrap_or(0.0);
                self.logger.info(&format!(
                    "[FILL][DBG_MAP] taker_oid={}.. msg_asset={} rec_asset={} msg_side={} rec_side={} msg_px={mpx:.4} msg_sz={msz:.6}",
                    taker_oid.chars().take(10).collect::<String>(),
                    msg_tail,
                    rec_tail,
                    msg_side,
                    side
                ));
            }

            let mut price = Self::_value_f64(msg.get("price")).unwrap_or_else(|| {
                taker_rec
                    .as_ref()
                    .map(|r| r.px_limit)
                    .unwrap_or(ctx_px_limit)
            });
            if price <= 0.0 {
                price = taker_rec
                    .as_ref()
                    .map(|r| r.px_limit)
                    .unwrap_or(ctx_px_limit);
            }
            let mut size = Self::_value_f64(
                msg.get("size")
                    .or_else(|| msg.get("filled"))
                    .or_else(|| msg.get("matched_amount"))
                    .or_else(|| msg.get("matchedAmount"))
                    .or_else(|| msg.get("amount")),
            )
            .unwrap_or(0.0);
            if size <= 0.0 {
                size = taker_rec
                    .as_ref()
                    .map(|r| (r.size - r.applied).max(0.0))
                    .unwrap_or(ctx_size.max(0.0));
            }
            if let Some(rec) = &taker_rec {
                let remaining = (rec.size - rec.applied).max(0.0);
                if remaining > 0.0 {
                    size = size.min(remaining);
                }
            }
            if size <= 0.0 || price <= 0.0 || asset.trim().is_empty() {
                return;
            }
            if !matches!(side.as_str(), "BUY" | "SELL") {
                return;
            }

            let key = if !trade_id.is_empty() {
                format!("{trade_id}:taker")
            } else {
                format!("trade_fallback:taker:{taker_oid}:{asset}:{side}:{size:.8}:{price:.8}")
            };
            let applied = self._apply_fill(&asset, price, size, &key, &side);
            if applied {
                self._log_execution_latency_on_fill(&taker_oid, now_ts_f64());
                if let Ok(mut m) = self.taker_orders.lock() {
                    let mut remove = false;
                    if let Some(r) = m.get_mut(&taker_oid) {
                        r.applied += size.max(0.0);
                        r.ts = now_ts_f64();
                        if r.size > 0.0 && r.applied >= r.size - 1e-9 {
                            remove = true;
                        }
                    }
                    if remove {
                        m.remove(&taker_oid);
                    }
                }
            }
            return;
        }

        // CASE A2: Direct taker payload with explicit token/side/price/size.
        // This keeps parity with Python behavior and helps after process restarts
        // where recent taker/order context may be empty.
        if !taker_oid.trim().is_empty() {
            let token_id = msg
                .get("asset_id")
                .or_else(|| msg.get("assetId"))
                .or_else(|| msg.get("token_id"))
                .or_else(|| msg.get("tokenId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let side = msg
                .get("side")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_uppercase();
            let qty = Self::_value_f64(
                msg.get("size")
                    .or_else(|| msg.get("matched_amount"))
                    .or_else(|| msg.get("matchedAmount"))
                    .or_else(|| msg.get("amount")),
            )
            .unwrap_or(0.0);
            let px = Self::_value_f64(msg.get("price")).unwrap_or(0.0);
            let yes = self.yes_asset.as_deref().unwrap_or("");
            let no = self.no_asset.as_deref().unwrap_or("");
            if !token_id.trim().is_empty()
                && matches!(side.as_str(), "BUY" | "SELL")
                && qty > 0.0
                && px > 0.0
                && (token_id == yes || token_id == no)
            {
                let key = if !trade_id.is_empty() {
                    format!("{trade_id}:taker")
                } else {
                    format!("trade_fallback:taker:{taker_oid}:{token_id}:{side}:{qty:.8}:{px:.8}")
                };
                let applied = self._apply_fill(&token_id, px, qty, &key, &side);
                if applied {
                    self._log_execution_latency_on_fill(&taker_oid, now_ts_f64());
                }
                return;
            }
        }

        // CASE B: Maker trade event. Apply only if maker leg matches our wallet.
        let wallet = self.wallet_address.to_ascii_lowercase();
        let trader_side = msg
            .get("trader_side")
            .or_else(|| msg.get("traderSide"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let maker_orders = msg
            .get("maker_orders")
            .or_else(|| msg.get("makerOrders"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if !maker_orders.is_empty() {
            let mut maker_leg: Option<Value> = None;
            if !wallet.trim().is_empty() {
                for mo in &maker_orders {
                    let mo_addr = mo
                        .get("maker_address")
                        .or_else(|| mo.get("makerAddress"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if !mo_addr.is_empty() && mo_addr == wallet {
                        maker_leg = Some(mo.clone());
                        break;
                    }
                }
            }
            if maker_leg.is_none() && trader_side == "MAKER" && maker_orders.len() == 1 {
                maker_leg = maker_orders.first().cloned();
            }
            if let Some(mo) = maker_leg {
                let asset = mo
                    .get("asset_id")
                    .or_else(|| mo.get("assetId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let side = mo
                    .get("side")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_uppercase();
                let qty = Self::_value_f64(
                    mo.get("matched_amount")
                        .or_else(|| mo.get("matchedAmount"))
                        .or_else(|| mo.get("size"))
                        .or_else(|| mo.get("filled")),
                )
                .unwrap_or(0.0);
                let px = Self::_value_f64(mo.get("price")).unwrap_or(0.0);
                if !asset.trim().is_empty()
                    && matches!(side.as_str(), "BUY" | "SELL")
                    && qty > 0.0
                    && px > 0.0
                {
                    let maker_oid = mo
                        .get("order_id")
                        .or_else(|| mo.get("orderId"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let key = if !trade_id.is_empty() {
                        format!("{trade_id}:maker")
                    } else {
                        format!("trade_fallback:maker:{maker_oid}:{asset}:{side}:{qty:.8}:{px:.8}")
                    };
                    let applied = self._apply_fill(&asset, px, qty, &key, &side);
                    if applied && !maker_oid.is_empty() {
                        self._log_execution_latency_on_fill(maker_oid, now_ts_f64());
                    }
                    return;
                }
            }
        }

        // Ambiguous trade event: ignore instead of corrupting local state.
        if env_bool("USER_TRADE_DEBUG", false) {
            let has_maker = msg
                .get("maker_orders")
                .or_else(|| msg.get("makerOrders"))
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            self.logger.info(&format!(
                "[FILL][DBG_DROP] drop ambiguous trade event id={} taker_oid={} has_maker_orders={}",
                trade_id,
                taker_oid,
                has_maker
            ));
        }
    }

    pub fn _handle_user_order_event(&self, msg: &Value) {
        let event_type = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !event_type.is_empty() && !event_type.contains("order") {
            return;
        }

        if self.taker_fill_fallback_from_order_events {
            self._taker_order_fallback_on_order_event(msg);
        }

        let asset_id = msg
            .get("asset_id")
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("assetId"))
            .or_else(|| msg.get("tokenId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if asset_id.trim().is_empty() {
            return;
        }
        let is_yn = self.yes_asset.as_deref() == Some(asset_id.as_str())
            || self.no_asset.as_deref() == Some(asset_id.as_str());
        if !is_yn {
            return;
        }

        let side = msg
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if side != "BUY" {
            return;
        }

        let oid = self._extract_order_id(msg).unwrap_or_default();
        if oid.trim().is_empty() {
            return;
        }

        let typ = msg
            .get("type")
            .or_else(|| msg.get("event_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let status = msg
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if !status.is_empty() {
            let st_key = format!(
                "__sniper_entry_status_{}_{}",
                asset_id,
                status.to_ascii_lowercase()
            );
            self._runtime_ts_set(&st_key, now_ts_f64());
            if status == "CONFIRMED" {
                let confirmed_key = Self::_sniper_entry_confirmed_key(&asset_id);
                self._runtime_ts_set(&confirmed_key, now_ts_f64());
                if env_bool("SNIPER_ORDER_STATUS_DEBUG", false) {
                    let tail: String = asset_id
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    self.logger.info(&format!(
                        "[SNIPER][ORDER] entry status CONFIRMED asset={tail} oid={}..",
                        oid.chars().take(10).collect::<String>()
                    ));
                }
            }
        }
        let cancelish = matches!(
            typ.as_str(),
            "CANCELLATION" | "CANCELED" | "CANCELLED" | "REJECTION" | "REJECTED"
        ) || matches!(status.as_str(), "CANCELED" | "CANCELLED" | "REJECTED");

        let price = Self::_value_f64(msg.get("price")).unwrap_or(0.0);
        let original = Self::_value_f64(
            msg.get("original_size")
                .or_else(|| msg.get("originalSize"))
                .or_else(|| msg.get("size")),
        )
        .unwrap_or(0.0);
        let matched = Self::_value_f64(
            msg.get("size_matched")
                .or_else(|| msg.get("matched_size"))
                .or_else(|| msg.get("filled_size"))
                .or_else(|| msg.get("filled")),
        )
        .unwrap_or(0.0);
        let mut remaining = if original > 0.0 {
            (original - matched).max(0.0)
        } else {
            Self::_value_f64(
                msg.get("remaining_size")
                    .or_else(|| msg.get("remainingSize"))
                    .or_else(|| msg.get("size")),
            )
            .unwrap_or(0.0)
            .max(0.0)
        };
        if !remaining.is_finite() {
            remaining = 0.0;
        }

        if cancelish || remaining <= 0.0 {
            if let Ok(mut s) = self.state.lock() {
                let should_remove = s
                    .open_orders
                    .get(&asset_id)
                    .and_then(|oo| oo.order_id.clone())
                    .map(|x| x == oid)
                    .unwrap_or(false);
                if should_remove {
                    s.open_orders.remove(&asset_id);
                    let _ = save_state(&self.state_file, &mut s);
                }
            }
            return;
        }

        if let Ok(mut s) = self.state.lock() {
            s.open_orders.insert(
                asset_id,
                OpenOrderState {
                    order_id: Some(oid),
                    price: Some(price),
                    size: Some(remaining),
                    ts: Some(now_ts_f64()),
                },
            );
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    pub fn _handle_user_event(&self, msg: &Value) {
        let t = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .or_else(|| msg.get("event"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if t.contains("trade") {
            self._handle_user_trade_event(msg);
        } else if t.contains("order") {
            self._handle_user_order_event(msg);
        }
    }

    pub fn on_user_message(&self, message: &str) {
        if let Ok(v) = serde_json::from_str::<Value>(message) {
            if let Some(items) = v.as_array() {
                for item in items {
                    if item.is_object() {
                        self._handle_user_event(item);
                    }
                }
            } else if v.is_object() {
                self._handle_user_event(&v);
            }
        }
    }

    pub fn _cancel(&self, order_id: &str) -> bool {
        if order_id.trim().is_empty() {
            return false;
        }
        if self.cfg.dry_run {
            self.logger.info(&format!("[DRY] cancel {order_id}"));
            return true;
        }

        if let (Some(rt), Some(client)) = (&self.clob_rt, &self.clob_client) {
            match rt.block_on(client.cancel_order(order_id)) {
                Ok(_) => {
                    if let Ok(mut ex) = self.exchange_orders_cache.lock() {
                        ex.retain(|o| self._extract_order_id(o).as_deref() != Some(order_id));
                    }
                    return true;
                }
                Err(e) => {
                    self.logger.error(&format!("Cancel failed: {e}"));
                    return false;
                }
            }
        }

        if let Ok(mut ex) = self.exchange_orders_cache.lock() {
            let before = ex.len();
            ex.retain(|o| self._extract_order_id(o).as_deref() != Some(order_id));
            return ex.len() != before;
        }
        false
    }

    pub fn _cancel_open_order_local(&self, asset_id: &str, reason: &str) {
        let oid = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.open_orders.get(asset_id).and_then(|o| o.order_id.clone()));
        if let Some(order_id) = oid {
            if !reason.trim().is_empty() {
                self.logger.info(&format!(
                    "Cancel {} ({reason})",
                    asset_id
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                ));
            }
            let _ = self._cancel(&order_id);
        }
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.remove(asset_id);
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    pub fn cancel_all_open_orders_local(&self, reason: &str) {
        let oo = self
            .state
            .lock()
            .map(|s| s.open_orders.clone())
            .unwrap_or_default();
        if oo.is_empty() {
            return;
        }
        if !reason.trim().is_empty() {
            self.logger
                .info(&format!("Cancel local open orders: {reason}"));
        }
        for row in oo.values() {
            if let Some(oid) = &row.order_id {
                let _ = self._cancel(oid);
            }
        }
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.clear();
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    pub fn cancel_all_open_orders_local_except(&self, keep_asset_id: &str, reason: &str) {
        let oo = self
            .state
            .lock()
            .map(|s| s.open_orders.clone())
            .unwrap_or_default();
        if oo.is_empty() {
            return;
        }
        let only_keep_exists = oo.len() == 1 && oo.contains_key(keep_asset_id);
        if only_keep_exists {
            return;
        }
        let mut to_cancel: Vec<String> = Vec::new();
        for (aid, row) in &oo {
            if aid == keep_asset_id {
                continue;
            }
            if let Some(oid) = &row.order_id {
                to_cancel.push(oid.clone());
            }
        }
        if to_cancel.is_empty() {
            return;
        }
        if !reason.trim().is_empty() {
            let tail: String = keep_asset_id
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger.info(&format!(
                "Cancel local open orders (except {tail}): {reason}"
            ));
        }
        for oid in to_cancel {
            let _ = self._cancel(&oid);
        }
        if let Ok(mut s) = self.state.lock() {
            let kept = s.open_orders.get(keep_asset_id).cloned();
            s.open_orders.clear();
            if let Some(v) = kept {
                s.open_orders.insert(keep_asset_id.to_string(), v);
            }
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    pub fn _extract_order_id(&self, o: &Value) -> Option<String> {
        o.get("id")
            .or_else(|| o.get("order_id"))
            .or_else(|| o.get("orderID"))
            .or_else(|| o.get("orderId"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    pub fn _extract_order_token_id(&self, o: &Value) -> Option<String> {
        o.get("asset_id")
            .or_else(|| o.get("token_id"))
            .or_else(|| o.get("assetId"))
            .or_else(|| o.get("tokenId"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    pub fn _extract_order_side(&self, o: &Value) -> String {
        o.get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("BUY")
            .to_ascii_uppercase()
    }

    pub fn _extract_order_price(&self, o: &Value) -> f64 {
        o.get("price")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0)
    }

    pub fn _extract_order_remaining_size(&self, o: &Value) -> f64 {
        o.get("size")
            .or_else(|| o.get("remaining_size"))
            .or_else(|| o.get("remainingSize"))
            .or_else(|| o.get("original_size"))
            .or_else(|| o.get("originalSize"))
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0)
    }

    pub fn _list_open_orders_exchange(&self) -> Vec<Value> {
        let fallback = || {
            self.exchange_orders_cache
                .lock()
                .map(|v| v.clone())
                .unwrap_or_default()
        };

        // Prefer raw endpoint parsing first: CLOB /data/orders may return either an array
        // or an object envelope ({data:[...]}), while typed client decoding expects an array.
        if let Some(out) = self._list_open_orders_exchange_raw() {
            if let Ok(mut cache) = self.exchange_orders_cache.lock() {
                *cache = out.clone();
            }
            return out;
        }

        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return fallback(),
        };

        let params = self
            .condition_id
            .clone()
            .and_then(|v| (!v.trim().is_empty()).then_some(v))
            .map(|market| OpenOrderParams {
                market: Some(market),
                ..OpenOrderParams::default()
            });

        match rt.block_on(client.get_open_orders(params)) {
            Ok(orders) => {
                let mut out = Vec::with_capacity(orders.len());
                for o in orders {
                    let order_id = o.id;
                    let asset_id = o.asset_id;
                    let original_size = o.original_size.parse::<f64>().unwrap_or(0.0);
                    let size_matched = o.size_matched.parse::<f64>().unwrap_or(0.0);
                    let remaining_size = (original_size - size_matched).max(0.0);
                    let price = o.price.parse::<f64>().unwrap_or(0.0);
                    out.push(json!({
                        "id": order_id.clone(),
                        "order_id": order_id,
                        "asset_id": asset_id.clone(),
                        "token_id": asset_id,
                        "side": o.side.to_ascii_uppercase(),
                        "price": price,
                        "size": remaining_size,
                        "remaining_size": remaining_size,
                        "original_size": original_size,
                        "size_matched": size_matched,
                        "status": o.status,
                        "market": o.market,
                        "order_type": o.order_type,
                        "created_at": o.created_at,
                    }));
                }
                if let Ok(mut cache) = self.exchange_orders_cache.lock() {
                    *cache = out.clone();
                }
                out
            }
            Err(e) => {
                self.logger
                    .error(&format!("get_orders failed during reconcile: {e}"));
                fallback()
            }
        }
    }

    pub fn _cancel_exchange_orders_for_assets(&self, asset_ids: &[String], reason: &str) {
        if self.cfg.dry_run {
            return;
        }
        let aset: HashSet<String> = asset_ids
            .iter()
            .map(|a| a.to_string())
            .filter(|a| !a.trim().is_empty())
            .collect();
        if aset.is_empty() {
            return;
        }
        let orders = self._list_open_orders_exchange();
        for o in orders {
            let aid = self._extract_order_token_id(&o);
            if aid.is_none() {
                continue;
            }
            let aid = aid.unwrap_or_default();
            if !aset.contains(&aid) {
                continue;
            }
            let oid = self._extract_order_id(&o);
            if let Some(oid) = oid {
                if !reason.trim().is_empty() {
                    self.logger.info(&format!(
                        "Cancel exchange order {}.. for {} ({reason})",
                        oid.chars().take(10).collect::<String>(),
                        aid.chars()
                            .rev()
                            .take(6)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>()
                    ));
                }
                let _ = self._cancel(&oid);
            }
        }
    }

    pub fn _reconcile_exchange_orders_for_asset(
        &self,
        asset_id: &str,
        intended_price: Option<f64>,
        force: bool,
    ) {
        if !env_bool("RECONCILE_EXCHANGE_ORDERS", true) || self.cfg.dry_run {
            return;
        }
        let now = now_ts_f64();
        let key = format!("__reconcile_last_{asset_id}");
        let last = self._runtime_ts_get(&key);
        let interval = env_float("RECONCILE_INTERVAL_SECONDS", 1.0).max(0.0);
        if !force && (now - last) < interval {
            return;
        }
        self._runtime_ts_set(&key, now);

        let orders = self._list_open_orders_exchange();
        let mut mine: Vec<Value> = Vec::new();
        for o in orders {
            let aid = self._extract_order_token_id(&o);
            if aid.as_deref() != Some(asset_id) {
                continue;
            }
            if self._extract_order_side(&o) != "BUY" {
                continue;
            }
            if self._extract_order_id(&o).is_none() {
                continue;
            }
            mine.push(o);
        }
        if mine.is_empty() {
            if let Ok(mut s) = self.state.lock() {
                if s.open_orders.remove(asset_id).is_some() {
                    let _ = save_state(&self.state_file, &mut s);
                }
            }
            return;
        }

        if mine.len() == 1 {
            let o = &mine[0];
            if let Some(oid) = self._extract_order_id(o) {
                let p = self._extract_order_price(o);
                let sz = self._extract_order_remaining_size(o);
                if let Ok(mut s) = self.state.lock() {
                    let local = s.open_orders.get(asset_id).and_then(|x| x.order_id.clone());
                    if local.as_deref() != Some(oid.as_str()) {
                        s.open_orders.insert(
                            asset_id.to_string(),
                            OpenOrderState {
                                order_id: Some(oid),
                                price: Some(p),
                                size: Some(sz),
                                ts: Some(now),
                            },
                        );
                        let _ = save_state(&self.state_file, &mut s);
                    }
                }
            }
            return;
        }

        let local_keep_id = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.open_orders.get(asset_id).and_then(|o| o.order_id.clone()));

        let mut keep_idx: usize = 0;
        if let Some(keep_id) = local_keep_id {
            if let Some((i, _)) = mine.iter().enumerate().find(|(_, o)| {
                self._extract_order_id(o)
                    .map(|id| id == keep_id)
                    .unwrap_or(false)
            }) {
                keep_idx = i;
            }
        } else if let Some(ip) = intended_price.filter(|p| *p > 0.0) {
            let mut best = f64::INFINITY;
            for (i, o) in mine.iter().enumerate() {
                let d = (self._extract_order_price(o) - ip).abs();
                if d < best {
                    best = d;
                    keep_idx = i;
                }
            }
        } else {
            let mut best = -1.0;
            for (i, o) in mine.iter().enumerate() {
                let p = self._extract_order_price(o);
                if p > best {
                    best = p;
                    keep_idx = i;
                }
            }
        }

        let keep = mine.get(keep_idx).cloned();
        let keep_id = keep.as_ref().and_then(|o| self._extract_order_id(o));
        for o in &mine {
            let oid = self._extract_order_id(o);
            if oid.is_none() {
                continue;
            }
            let oid = oid.unwrap_or_default();
            if keep_id.as_deref() == Some(oid.as_str()) {
                continue;
            }
            self.logger.info(&format!(
                "Reconcile: cancel extra order {}.. for {}",
                oid.chars().take(10).collect::<String>(),
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            let _ = self._cancel(&oid);
        }

        if let (Some(keep), Some(keep_id)) = (keep, keep_id) {
            let p = self._extract_order_price(&keep);
            let sz = self._extract_order_remaining_size(&keep);
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    asset_id.to_string(),
                    OpenOrderState {
                        order_id: Some(keep_id),
                        price: Some(p),
                        size: Some(sz),
                        ts: Some(now),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
        }
    }

    pub fn _post_order_compat(
        &self,
        signed_order: &Value,
        order_type: &str,
        post_only: Option<bool>,
    ) -> Option<String> {
        if self.cfg.dry_run {
            return None;
        }

        let asset_id = signed_order
            .get("asset_id")
            .or_else(|| signed_order.get("token_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if asset_id.trim().is_empty() {
            return None;
        }
        let side_u = signed_order
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("BUY")
            .to_ascii_uppercase();
        let side = Self::_clob_side(&side_u)?;
        let price = signed_order
            .get("price")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0);
        let size = signed_order
            .get("size")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0);
        if price <= 0.0 || size <= 0.0 {
            return None;
        }
        if post_only.unwrap_or(false) {
            if let Some((bid, ask)) = self._best_bid_ask(&asset_id) {
                let tick = self.cfg.tick.max(0.0001);
                if matches!(side, ClobSide::Buy) && price >= (ask - tick * 0.5) {
                    return None;
                }
                if matches!(side, ClobSide::Sell) && price <= (bid + tick * 0.5) {
                    return None;
                }
            }
        }

        let clob_order_type = Self::_clob_order_type(order_type);
        let local_fallback = || {
            let oid = crate::db::new_uuid();
            let row = json!({
                "id": oid,
                "order_id": oid,
                "asset_id": asset_id,
                "side": side_u,
                "price": price,
                "size": size,
                "order_type": order_type.to_ascii_uppercase(),
                "post_only": post_only,
                "ts": now_ts_f64(),
            });
            if let Ok(mut ex) = self.exchange_orders_cache.lock() {
                ex.push(row);
            }
            Some(oid)
        };

        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return local_fallback(),
        };
        let prep_start_ns = now_ns();
        let prep_start_ts = now_ts_f64();
        let tick_size = rt
            .block_on(client.get_tick_size(&asset_id))
            .unwrap_or_else(|_| Self::_tick_size_from_f64(self.cfg.tick.max(0.0001)));
        let neg_risk = rt.block_on(client.get_neg_risk(&asset_id)).unwrap_or(false);
        let fee_rate_bps = rt.block_on(client.get_fee_rate_bps(&asset_id)).ok();
        let prep_end_ns = now_ns();
        let prep_end_ts = now_ts_f64();

        let user_order = UserLimitOrder {
            token_id: asset_id.clone(),
            price,
            size,
            side,
            fee_rate_bps,
            nonce: None,
            expiration: None,
            taker: None,
        };
        let create_opts = Some(CreateOrderOptions {
            tick_size,
            neg_risk: Some(neg_risk),
        });
        let sign_start_ns = now_ns();
        let sign_start_ts = now_ts_f64();
        let signed = match rt.block_on(client.create_limit_order(&user_order, create_opts)) {
            Ok(v) => v,
            Err(e) => {
                if post_only.unwrap_or(false) {
                    self.logger
                        .warning(&format!("post-only order rejected: {e}"));
                } else {
                    self.logger.error(&format!("post_order failed: {e}"));
                }
                return None;
            }
        };
        let sign_end_ns = now_ns();
        let sign_end_ts = now_ts_f64();
        let post_start_ns = now_ns();
        let post_start_ts = now_ts_f64();
        let posted = rt.block_on(client.post_order(signed, clob_order_type));
        let post_end_ns = now_ns();
        let post_end_ts = now_ts_f64();

        let resp = match posted {
            Ok(v) => v,
            Err(e) => {
                if post_only.unwrap_or(false) {
                    self.logger
                        .warning(&format!("post-only order rejected: {e}"));
                } else {
                    self.logger.error(&format!("post_order failed: {e}"));
                }
                return None;
            }
        };
        let oid = Self::_extract_posted_order_id(&resp)?;
        let row = json!({
            "id": oid.clone(),
            "order_id": oid.clone(),
            "asset_id": asset_id,
            "side": side_u,
            "price": price,
            "size": size,
            "order_type": order_type.to_ascii_uppercase(),
            "post_only": post_only,
            "ts": now_ts_f64(),
        });
        if let Ok(mut ex) = self.exchange_orders_cache.lock() {
            ex.push(row);
        }
        if let Ok(mut m) = self.submit_timing_cache.lock() {
            m.insert(
                oid.clone(),
                json!({
                    "sign_start_ns": sign_start_ns,
                    "sign_end_ns": sign_end_ns,
                    "sign_start_ts": sign_start_ts,
                    "sign_end_ts": sign_end_ts,
                    "prep_start_ns": prep_start_ns,
                    "prep_end_ns": prep_end_ns,
                    "prep_start_ts": prep_start_ts,
                    "prep_end_ts": prep_end_ts,
                    "post_start_ns": post_start_ns,
                    "post_end_ns": post_end_ns,
                    "post_start_ts": post_start_ts,
                    "post_end_ts": post_end_ts,
                    "order_submit_ts": post_end_ts,
                    "fee_rate_bps": fee_rate_bps,
                    "tick_size": tick_size.as_f64(),
                    "neg_risk": neg_risk,
                }),
            );
        }
        Some(oid)
    }

    pub fn _post_orders_compat(
        &self,
        signed_orders: &[Value],
        order_type: &str,
        post_only: Option<bool>,
    ) -> Vec<Option<String>> {
        if self.cfg.dry_run {
            return signed_orders.iter().map(|_| None).collect();
        }
        signed_orders
            .iter()
            .map(|o| self._post_order_compat(o, order_type, post_only))
            .collect()
    }

    pub fn _place_postonly_bid(&self, asset_id: &str, price: f64, size: f64) -> Option<String> {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut dp = env_int("SIZE_DECIMALS", 6);
        dp = dp.clamp(0, 8);
        let dp = dp as u32;

        let mut size = q_down(size.max(0.0), dp);
        let price = round_down(price.max(0.0), tick);
        if size < self.cfg.min_shares || price <= 0.0 {
            return None;
        }
        if price * size < self.min_maker_notional {
            let need_size = q_up(self.min_maker_notional / price, dp);
            size = size.max(need_size).max(self.cfg.min_shares);
            if price * size < self.min_maker_notional {
                return None;
            }
        }

        let (_bid, ask) = self._best_bid_ask(asset_id)?;
        let maker_max = round_down(
            ask - self.cfg.maker_buffer_ticks as f64 * self.cfg.tick.max(0.0001),
            self.cfg.tick.max(0.0001),
        );
        if price > maker_max {
            return None;
        }

        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] POSTONLY BID asset={} price={price:.2} size={size:.4} notional={:.2}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>(),
                price * size
            ));
            return None;
        }
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": price,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, "GTC", Some(true))?;
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": price,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": "MAKER_POSTONLY_GTC",
            }),
        );
        Some(oid)
    }

    pub fn _place_limit_bid_gtc(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        post_only: Option<bool>,
    ) -> Option<String> {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = clamp(price, tick, 0.99);
        px = round_down(px, tick);
        px = clamp(px, tick, 0.99);

        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let mut sz_int = (size + 1e-12).floor() as i64;
        if sz_int < min_int {
            sz_int = min_int;
        }
        sz_int = (sz_int / min_int) * min_int;
        if sz_int < min_int {
            sz_int = min_int;
        }
        if self.cfg.dry_run {
            let oid = format!("DRY_LIMIT_GTC_{}", (now_ts_f64() * 1000.0) as i64);
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    asset_id.to_string(),
                    OpenOrderState {
                        order_id: Some(oid.clone()),
                        price: Some(px),
                        size: Some(sz_int as f64),
                        ts: Some(now_ts_f64()),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
            self.logger.info(&format!(
                "[DRY] limit bid GTC asset={} px={px:.3} size={} post_only={post_only:?}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>(),
                sz_int
            ));
            return Some(oid);
        }

        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": px,
            "size": sz_int,
        });
        let oid = self._post_order_compat(&signed, "GTC", post_only)?;
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.insert(
                asset_id.to_string(),
                OpenOrderState {
                    order_id: Some(oid.clone()),
                    price: Some(px),
                    size: Some(sz_int as f64),
                    ts: Some(now_ts_f64()),
                },
            );
            let _ = save_state(&self.state_file, &mut s);
        }
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": px,
                "size": sz_int,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": if post_only.unwrap_or(false) { "LIMIT_GTC_POSTONLY" } else { "LIMIT_GTC" },
            }),
        );
        Some(oid)
    }

    pub fn _resolve_order_type(&self, name: &str) -> String {
        let mut n = name.trim().to_ascii_uppercase();
        if matches!(n.as_str(), "LIMIT" | "LIMIT_GTC" | "GTC_LIMIT") {
            n = "GTC".to_string();
        }
        if matches!(n.as_str(), "IOC" | "IOK" | "FILL_AND_KILL" | "FILLANDKILL") {
            n = "FAK".to_string();
        }
        if matches!(n.as_str(), "FILL_OR_KILL" | "FILLORKILL") {
            n = "FOK".to_string();
        }
        match n.as_str() {
            "FAK" | "FOK" | "GTC" => n,
            _ => {
                self.logger
                    .warning(&format!("Unknown OrderType '{n}'. Falling back to GTC."));
                "GTC".to_string()
            }
        }
    }

    pub fn _place_taker_bid_fak(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        order_type_name: Option<&str>,
    ) -> Option<String> {
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = round_up(price, tick);
        px = clamp(px, tick, 0.99);
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let size_int = (size + 1e-12).floor() as i64;
        if size_int < min_int {
            return None;
        }
        let size = size_int as f64;
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER HEDGE BUY asset={} price={px:.2} size={size_int} type={ot}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return None;
        }
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": px,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, &ot, None)?;
        self._remember_taker_order(&oid, asset_id, size, px, "BUY");
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": format!("TAKER_{}_BUY", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent BUY asset={} px={px:.4} sz={size:.0} oid={oid}",
            asset_id
        ));
        Some(oid)
    }

    pub fn _place_taker_ask_fak(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        order_type_name: Option<&str>,
    ) -> Option<String> {
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = round_down(price, tick);
        px = clamp(px, tick, 0.99);
        let mut dp_i = env_int("SIZE_DECIMALS", 6);
        dp_i = dp_i.clamp(0, 8);
        let dp = dp_i as u32;
        let allow_fractional = env_bool("SNIPER_EXIT_ALLOW_FRACTIONAL_SIZE", false);
        let size = if allow_fractional {
            let min_step = 10f64.powi(-(dp as i32));
            let min_size = env_float("SNIPER_EXIT_MIN_ORDER_SIZE", 0.1).max(min_step);
            let q = q_down(size.max(0.0), dp);
            if q + 1e-12 < min_size {
                return None;
            }
            q
        } else {
            let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
            let size_int = (size + 1e-12).floor() as i64;
            if size_int < min_int {
                return None;
            }
            size_int as f64
        };
        let sz_disp = if (size - size.round()).abs() <= 1e-9 {
            format!("{:.0}", size)
        } else {
            format!("{:.4}", size)
        };
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER SELL asset={} price={px:.2} size={sz_disp} type={ot}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return None;
        }
        let signed = json!({
            "asset_id": asset_id,
            "side": "SELL",
            "price": px,
            "size": size,
        });
        let oid = match self._post_order_compat(&signed, &ot, None) {
            Some(v) => v,
            None => {
                self.logger.warning(&format!(
                    "[TAKER {ot}] rejected SELL asset={} px={px:.4} sz={sz_disp} (no oid)",
                    asset_id
                ));
                return None;
            }
        };
        self._remember_taker_order(&oid, asset_id, size, px, "SELL");
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "SELL",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": format!("TAKER_{}_SELL", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent SELL asset={} px={px:.4} sz={sz_disp} oid={oid}",
            asset_id
        ));
        Some(oid)
    }

    pub fn _pair_arb_required_total(&self) -> f64 {
        let min_profit = env_float("PAIR_ARB_MIN_PROFIT_TICKS", 0.0) * self.cfg.tick.max(0.0001);
        let safety = env_float("PAIR_ARB_SAFETY_TICKS", 0.0) * self.cfg.tick.max(0.0001);
        let fees_buf = env_float("PAIR_ARB_FEE_RATE", 0.0);
        clamp(1.0 - min_profit - safety - fees_buf - 1e-9, 0.0, 1.0)
    }

    pub fn _taker_pair_submit(
        &self,
        size_int: i64,
        y_px: f64,
        n_px: f64,
    ) -> (Option<String>, Option<String>) {
        if size_int <= 0 {
            return (None, None);
        }
        let order_type = self._resolve_order_type(
            &std::env::var("PAIR_ARB_ORDER_TYPE").unwrap_or_else(|_| "FOK".to_string()),
        );
        if order_type == "GTC" && !env_bool("PAIR_ARB_ALLOW_GTC", false) {
            self.logger.warning(
                "PAIR_ARB_ORDER_TYPE resolved to GTC, unsafe for atomic pair-arb. Set PAIR_ARB_ALLOW_GTC=true to override.",
            );
            self._set_exit_reason("PAIR_ARB_UNSAFE_GTC");
            self.cancel_all_orders_exchange("pair arb unsafe order type");
            self.stop_flag.store(true, Ordering::SeqCst);
            return (None, None);
        }
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER_PAIR {order_type} size={size_int} y_px={y_px:.2} n_px={n_px:.2}"
            ));
            return (None, None);
        }
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let qty = size_int as f64;
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return (None, None),
        };
        let signed_y = json!({
            "asset_id": yes,
            "side": "BUY",
            "price": y_px,
            "size": qty,
        });
        let signed_n = json!({
            "asset_id": no,
            "side": "BUY",
            "price": n_px,
            "size": qty,
        });
        let resps = self._post_orders_compat(&[signed_y, signed_n], &order_type, None);
        let y_oid = resps.first().and_then(|o| o.clone());
        let n_oid = resps.get(1).and_then(|o| o.clone());
        if let Some(oid) = &y_oid {
            self._remember_taker_order(oid, yes, qty, y_px, "BUY");
            self._track_order_execution_context(
                oid,
                &json!({
                    "order_id": oid,
                    "asset_id": yes,
                    "side": "BUY",
                    "px_limit": y_px,
                    "size": qty,
                    "decision_ts": decide_ts,
                    "decision_ns": decide_ns,
                    "post_start_ts": now_ts_f64(),
                    "post_end_ts": now_ts_f64(),
                    "origin": format!("TAKER_PAIR_{}_YES", order_type),
                }),
            );
        }
        if let Some(oid) = &n_oid {
            self._remember_taker_order(oid, no, qty, n_px, "BUY");
            self._track_order_execution_context(
                oid,
                &json!({
                    "order_id": oid,
                    "asset_id": no,
                    "side": "BUY",
                    "px_limit": n_px,
                    "size": qty,
                    "decision_ts": decide_ts,
                    "decision_ns": decide_ns,
                    "post_start_ts": now_ts_f64(),
                    "post_end_ts": now_ts_f64(),
                    "origin": format!("TAKER_PAIR_{}_NO", order_type),
                }),
            );
        }
        if y_oid.is_none() && n_oid.is_none() {
            let pause_s = env_float("PAIR_ARB_PAUSE_ON_ERROR_SECONDS", 2.0).max(0.0);
            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + pause_s);
        }
        (y_oid, n_oid)
    }

    pub fn _wait_for_pair_fills(
        &self,
        qy0: f64,
        qn0: f64,
        target_size: i64,
        timeout_s: f64,
    ) -> (f64, f64) {
        let deadline = now_ts_f64() + timeout_s.max(0.01);
        while now_ts_f64() < deadline && !self.stop_flag.load(Ordering::SeqCst) {
            let s = self.state.lock().map(|v| v.clone()).unwrap_or_default();
            let fy = (s.q_yes - qy0).max(0.0);
            let fn_ = (s.q_no - qn0).max(0.0);
            if fy >= target_size as f64 && fn_ >= target_size as f64 {
                return (fy, fn_);
            }
            let rem = (deadline - now_ts_f64()).max(0.0);
            if rem <= 0.0 {
                break;
            }
            thread::sleep(Duration::from_secs_f64(rem.min(0.05)));
        }
        let s = self.state.lock().map(|v| v.clone()).unwrap_or_default();
        ((s.q_yes - qy0).max(0.0), (s.q_no - qn0).max(0.0))
    }

    pub fn _handle_exposure_mismatch(&self, filled_yes: f64, filled_no: f64) {
        let mut delta = filled_yes - filled_no;
        if delta.abs() < 1e-9 {
            return;
        }

        let _ = self._reconcile_state_from_balances("exposure_mismatch");
        if delta.abs() < self.cfg.min_shares {
            self.logger.info(&format!(
                "Exposure mismatch below min_shares. filled_yes={filled_yes:.2} filled_no={filled_no:.2} delta={delta:.2} -> STOP"
            ));
            self._set_exit_reason("DUST_EXPOSURE");
            self.cancel_all_orders_exchange("dust exposure");
            self.stop_flag.store(true, Ordering::SeqCst);
            return;
        }

        let policy = self._normalize_exposure_policy(
            &std::env::var("EXPOSURE_POLICY").unwrap_or_else(|_| "UNWIND".to_string()),
        );
        self.logger.info(&format!(
            "EXPOSURE mismatch: filled_yes={filled_yes:.0} filled_no={filled_no:.0} delta={delta:.0} policy={policy}"
        ));
        self.cancel_all_open_orders_local("exposure mismatch cleanup");
        if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
            self._cancel_exchange_orders_for_assets(
                &[y.clone(), n.clone()],
                "exposure mismatch cleanup",
            );
        }

        if policy == "WAIT" {
            return;
        }
        if policy == "HEDGE" {
            self._emergency_taker_hedge_step(delta, "pair_arb_mismatch");
            if env_bool("EXPOSURE_HEDGE_THEN_UNWIND", false) {
                let grace = env_float("EXPOSURE_HEDGE_GRACE_SECONDS", 0.6).max(0.05);
                thread::sleep(Duration::from_secs_f64(grace));
                let (qy2, qn2) = self
                    .state
                    .lock()
                    .map(|s| (s.q_yes, s.q_no))
                    .unwrap_or((0.0, 0.0));
                let delta2 = qy2 - qn2;
                if delta2.abs() >= self.cfg.min_shares {
                    self.logger.info(&format!(
                        "Exposure still present after hedge grace. delta={delta2:.2} -> UNWIND heavy."
                    ));
                    delta = delta2;
                } else {
                    return;
                }
            } else {
                return;
            }
        }
        self._chunked_unwind_heavy_leg(delta, "pair_arb_mismatch");
    }

    pub fn _normalize_exposure_policy(&self, policy: &str) -> String {
        let p = policy.trim().to_ascii_uppercase();
        match p.as_str() {
            "UNWIND" | "HEDGE" | "HEDGE_THEN_UNWIND" | "STOP" | "WAIT" => p,
            _ => "UNWIND".to_string(),
        }
    }

    pub fn _unwind_heavy_leg(&self, delta: f64, reason: &str) {
        if delta.abs() < self.cfg.min_shares {
            return;
        }
        let now = now_ts_f64();
        if now < self._runtime_ts_get("__taker_inflight_until") {
            return;
        }
        if now < self._runtime_ts_get("__taker_fail_pause_until") {
            return;
        }

        let heavy_asset = if delta > 0.0 {
            self.yes_asset.clone()
        } else {
            self.no_asset.clone()
        };
        let Some(heavy_asset) = heavy_asset else {
            return;
        };

        if self.taker_strict_inflight && self._has_pending_taker_order("SELL", Some(&heavy_asset)) {
            return;
        }
        let ba = self._best_bid_ask(&heavy_asset);
        if ba.is_none() || ba.unwrap_or((0.0, 0.0)).0 <= 0.0 {
            self.logger.info(&format!(
                "UNWIND failed: missing best bid for heavy asset={} ({reason})",
                heavy_asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            self._set_exit_reason("UNWIND_NO_BID");
            self.stop_flag.store(true, Ordering::SeqCst);
            return;
        }
        self._chunked_unwind_heavy_leg(delta, reason);
    }

    pub fn _maker_exposure_step(&self, delta: f64, unhedged_age: f64) {
        let policy_raw =
            std::env::var("MAKER_EXPOSURE_POLICY").unwrap_or_else(|_| "HEDGE".to_string());
        let policy = self._normalize_exposure_policy(&policy_raw);
        let max_s = env_float("MAKER_EXPOSURE_MAX_SECONDS", 0.0).max(0.0);
        if max_s > 0.0 && unhedged_age >= max_s {
            self.logger.info(&format!(
                "Exposure age {unhedged_age:.2}s >= max {max_s:.2}s -> UNWIND heavy"
            ));
            self.cancel_all_open_orders_local("maker exposure hard max -> unwind");
            if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                self._cancel_exchange_orders_for_assets(
                    &[y.clone(), n.clone()],
                    "maker exposure hard max -> unwind",
                );
            }
            self._unwind_heavy_leg(delta, "maker_exposure_max_seconds");
            return;
        }

        if policy == "UNWIND" {
            self.cancel_all_open_orders_local("maker exposure policy=UNWIND");
            if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                self._cancel_exchange_orders_for_assets(
                    &[y.clone(), n.clone()],
                    "maker exposure policy=UNWIND",
                );
            }
            self._unwind_heavy_leg(delta, "maker_policy_unwind");
            return;
        }

        let missing_asset = if delta > 0.0 {
            self.no_asset.clone()
        } else {
            self.yes_asset.clone()
        };
        let Some(missing_asset) = missing_asset else {
            return;
        };

        let cap_now = self._hedge_price_cap();
        if cap_now <= 0.0 {
            self.logger.info(&format!(
                "Hedge cap<=0 (cap={cap_now:.2}) delta={delta:.2} policy={policy} -> FLATTEN/STOP"
            ));
            if let Some(info) = self._flatten_now_best(delta) {
                self._force_flatten_and_stop(delta, &info);
            } else {
                self._set_exit_reason("CAP_LOCKED_LOSS");
                self.cancel_all_orders_exchange("cap<=0 locked loss");
                self.stop_flag.store(true, Ordering::SeqCst);
            }
            return;
        }

        let m_ba = self._best_bid_ask(&missing_asset);
        if m_ba.is_none() {
            return;
        }
        let missing_ask = m_ba.unwrap_or((0.0, 0.0)).1;
        if missing_ask <= 0.0 {
            return;
        }

        let cap_blocked = missing_ask > cap_now + 1e-12;
        let grace = env_float(
            "MAKER_EXPOSURE_HEDGE_GRACE_SECONDS",
            env_float("EXPOSURE_HEDGE_GRACE_SECONDS", 0.0),
        )
        .max(0.0);
        let want_then_unwind =
            policy == "HEDGE_THEN_UNWIND" || env_bool("MAKER_EXPOSURE_HEDGE_THEN_UNWIND", false);
        self._dbg_maker(
            &format!(
                "[DBG][MAKER][EXPOSURE] delta={delta:.2} age={unhedged_age:.2}s cap={cap_now:.2} missing_ask={missing_ask:.2} cap_blocked={cap_blocked} policy={policy}"
            ),
            "maker_exposure",
            Some(0.5),
        );

        if cap_blocked {
            if let Some(maker_max) = self._maker_max_price(&missing_asset) {
                let mut target_price = cap_now.min(maker_max);
                target_price = round_down(target_price, self.cfg.tick.max(0.0001));
                let size = delta.abs().min(self.cfg.clip_shares);
                if size >= self.cfg.min_shares && target_price > 0.0 {
                    let hedge_stale = env_int("HEDGE_STALE_SECONDS", self.cfg.stale_seconds) as i64;
                    let _ =
                        self._maybe_replace(&missing_asset, target_price, size, Some(hedge_stale));
                }
            }
            self.cancel_all_open_orders_local_except(
                &missing_asset,
                "cap-blocked (keep maker hedge)",
            );
            if want_then_unwind && unhedged_age >= grace {
                self.logger.info(&format!(
                    "Cap-blocked for {unhedged_age:.2}s (grace={grace:.2}s) -> UNWIND heavy"
                ));
                self.cancel_all_open_orders_local("cap-blocked -> unwind");
                if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                    self._cancel_exchange_orders_for_assets(
                        &[y.clone(), n.clone()],
                        "cap-blocked -> unwind",
                    );
                }
                self._unwind_heavy_leg(delta, "cap_blocked_policy_unwind");
            }
            return;
        }

        if unhedged_age >= self.unhedged_timeout_seconds {
            self._emergency_taker_hedge_step(
                delta,
                &format!("maker_unhedged>{:.2}s", self.unhedged_timeout_seconds),
            );
            return;
        }

        if let Some(maker_max) = self._maker_max_price(&missing_asset) {
            let mut target_price = cap_now.min(maker_max);
            target_price = round_down(target_price, self.cfg.tick.max(0.0001));
            if target_price <= 0.0 {
                return;
            }
            let size = delta.abs().min(self.cfg.clip_shares);
            if size >= self.cfg.min_shares {
                let hedge_stale = env_int("HEDGE_STALE_SECONDS", self.cfg.stale_seconds) as i64;
                let _ = self._maybe_replace(&missing_asset, target_price, size, Some(hedge_stale));
            }
        }
    }

    pub fn _taker_pair_arb_step(&self, remaining_budget: f64) {
        let now = now_ts_f64();
        let dbg = env_bool("PAIR_ARB_DEBUG", false);
        let cooldown_s = env_float("PAIR_ARB_COOLDOWN_SECONDS", 0.25).max(0.0);
        let last_attempt = self._runtime_ts_get("__pair_arb_last_attempt_ts");
        if (now - last_attempt) < cooldown_s {
            if dbg {
                self._dbg(
                    &format!(
                        "[DBG][TAKER_PAIR] skip cooldown dt={:.3}s < {:.3}s",
                        now - last_attempt,
                        cooldown_s
                    ),
                    "pair_cooldown",
                    None,
                );
            }
            return;
        }

        let fail_pause_until = self._runtime_ts_get("__taker_fail_pause_until");
        if now < fail_pause_until {
            if dbg {
                self._dbg(
                    &format!(
                        "[DBG][TAKER_PAIR] skip fail-pause remain={:.3}s",
                        fail_pause_until - now
                    ),
                    "pair_failpause",
                    None,
                );
            }
            return;
        }
        if remaining_budget <= 0.0 {
            if dbg {
                self._dbg(
                    "[DBG][TAKER_PAIR] skip no remaining budget",
                    "pair_budget",
                    None,
                );
            }
            return;
        }
        if env_bool("PAIR_ARB_USE_STABILITY_GATE", true) {
            let (ok, why) = self._accumulate_allowed();
            if !ok {
                if dbg {
                    self._dbg(
                        &format!("[DBG][TAKER_PAIR] skip stability gate: {why}"),
                        &format!("pair_gate_{why}"),
                        None,
                    );
                }
                return;
            }
        }

        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return,
        };
        let yq = self._best_bid_ask(yes);
        let nq = self._best_bid_ask(no);
        if yq.is_none() || nq.is_none() {
            if dbg {
                self._dbg(
                    "[DBG][TAKER_PAIR] skip missing quotes",
                    "pair_missing_quotes",
                    None,
                );
            }
            return;
        }
        let (_yb, y_ask) = yq.unwrap_or((0.0, 0.0));
        let (_nb, n_ask) = nq.unwrap_or((0.0, 0.0));
        if y_ask <= 0.0 || n_ask <= 0.0 {
            if dbg {
                self._dbg(
                    &format!("[DBG][TAKER_PAIR] skip non-positive ask y_ask={y_ask} n_ask={n_ask}"),
                    "pair_zero_ask",
                    None,
                );
            }
            return;
        }
        let max_leg = env_float("PAIR_ARB_MAX_LEG_PRICE", 1.0);
        if y_ask > max_leg || n_ask > max_leg {
            if dbg {
                self._dbg(
                    &format!(
                        "[DBG][TAKER_PAIR] skip max_leg_price y_ask={y_ask:.2} n_ask={n_ask:.2} max={max_leg:.2}"
                    ),
                    "pair_max_leg",
                    None,
                );
            }
            return;
        }

        let max_skew_ticks = env_float("PAIR_ARB_MAX_SKEW_TICKS", 1e9);
        let skew = (y_ask - n_ask).abs();
        if skew > (max_skew_ticks * self.cfg.tick.max(0.0001)) {
            if dbg {
                self._dbg(
                    &format!(
                        "[DBG][TAKER_PAIR] skip skew {skew:.4} > max={:.4}",
                        max_skew_ticks * self.cfg.tick.max(0.0001)
                    ),
                    "pair_skew",
                    None,
                );
            }
            return;
        }

        let slip_ticks = env_float("PAIR_ARB_SLIPPAGE_TICKS", 0.0);
        let mut y_px = y_ask + slip_ticks * self.cfg.tick.max(0.0001);
        let mut n_px = n_ask + slip_ticks * self.cfg.tick.max(0.0001);
        y_px = round_up(
            clamp(y_px, self.cfg.tick.max(0.0001), 0.99),
            self.cfg.tick.max(0.0001),
        );
        n_px = round_up(
            clamp(n_px, self.cfg.tick.max(0.0001), 0.99),
            self.cfg.tick.max(0.0001),
        );

        let total_px = y_px + n_px;
        let req = self._pair_arb_required_total();
        if dbg {
            self._dbg(
                &format!(
                    "[DBG][TAKER_PAIR] quotes y_ask={y_ask:.2} n_ask={n_ask:.2} y_px={y_px:.2} n_px={n_px:.2} sum={total_px:.2} req<={req:.2} budget={remaining_budget:.2}"
                ),
                "pair_summary",
                None,
            );
        }
        if total_px > req {
            if dbg {
                self._dbg(
                    &format!("[DBG][TAKER_PAIR] skip asksum {total_px:.2} > req {req:.2}"),
                    "pair_asksum",
                    None,
                );
            }
            return;
        }

        let min_shares_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let max_shares =
            env_int("PAIR_ARB_MAX_SHARES", self.cfg.clip_shares.floor() as i64).max(min_shares_int);
        let max_affordable = (remaining_budget / total_px + 1e-12).floor() as i64;
        let mut size_int = max_affordable.min(max_shares);
        let min_notional = self.min_taker_notional.max(0.0);
        let need_y = if y_px > 0.0 {
            (min_notional / y_px - 1e-12).ceil() as i64
        } else {
            0
        };
        let need_n = if n_px > 0.0 {
            (min_notional / n_px - 1e-12).ceil() as i64
        } else {
            0
        };
        let min_needed = min_shares_int.max(need_y).max(need_n);
        if size_int < min_needed {
            if min_needed <= max_affordable && min_needed <= max_shares {
                size_int = min_needed;
            } else {
                if dbg {
                    self._dbg(
                        &format!(
                            "[DBG][TAKER_PAIR] skip size too small size_int={size_int} min_needed={min_needed} max_affordable={max_affordable} max_shares={max_shares}"
                        ),
                        "pair_size",
                        None,
                    );
                }
                return;
            }
        }
        if size_int < min_shares_int {
            return;
        }

        self._runtime_ts_set("__pair_arb_last_attempt_ts", now);
        if env_bool("PAIR_ARB_CANCEL_BEFORE_ATTEMPT", true) {
            self.cancel_all_open_orders_local("before pair arb");
            self._cancel_exchange_orders_for_assets(
                &[yes.to_string(), no.to_string()],
                "before pair arb",
            );
        }

        let (qy0, qn0) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no))
            .unwrap_or((0.0, 0.0));
        let pair_order_type =
            std::env::var("PAIR_ARB_ORDER_TYPE").unwrap_or_else(|_| "FOK".to_string());
        self.logger.info(&format!(
            "TAKER_PAIR attempt size={size_int} y_px={y_px:.2} n_px={n_px:.2} total={total_px:.2} req<={req:.2} budget={remaining_budget:.2} type={pair_order_type}"
        ));

        let retries = env_int("PAIR_ARB_MAX_RETRIES", 1).max(1) as usize;
        let timeout_s = env_float("PAIR_ARB_TIMEOUT_SECONDS", 2.0).max(0.01);
        let backoff_min_ms = env_int("PAIR_ARB_RETRY_BACKOFF_MS_MIN", 50).max(0) as u64;
        let backoff_max_ms =
            env_int("PAIR_ARB_RETRY_BACKOFF_MS_MAX", 250).max(backoff_min_ms as i64) as u64;
        for attempt in 0..retries {
            if self.stop_flag.load(Ordering::SeqCst) {
                return;
            }
            let (y_oid, n_oid) = self._taker_pair_submit(size_int, y_px, n_px);
            let fail_pause_until = self._runtime_ts_get("__taker_fail_pause_until");
            if y_oid.is_none() && n_oid.is_none() && now_ts_f64() < fail_pause_until {
                return;
            }

            let (fy, fn_) = self._wait_for_pair_fills(qy0, qn0, size_int, timeout_s);
            if env_bool("PAIR_ARB_RECONCILE_AFTER_TIMEOUT", true) {
                if let Some(oid) = y_oid {
                    let _ = self._cancel(&oid);
                }
                if let Some(oid) = n_oid {
                    let _ = self._cancel(&oid);
                }
                self._cancel_exchange_orders_for_assets(
                    &[yes.to_string(), no.to_string()],
                    "pair arb cleanup",
                );
            }

            if fy <= 0.0 && fn_ <= 0.0 {
                if dbg {
                    self._dbg(
                        &format!(
                            "[DBG][TAKER_PAIR] attempt {} no fills (fy=0 fn=0)",
                            attempt + 1
                        ),
                        "pair_nofill",
                        None,
                    );
                }
                if attempt + 1 < retries {
                    let mut rng = rand::thread_rng();
                    let backoff_ms = if backoff_max_ms > backoff_min_ms {
                        rng.gen_range(backoff_min_ms..=backoff_max_ms)
                    } else {
                        backoff_min_ms
                    };
                    thread::sleep(Duration::from_millis(backoff_ms));
                    continue;
                }
                return;
            }
            if (fy - fn_).abs() < 1e-6 {
                self.logger.info(&format!(
                    "TAKER_PAIR filled YES={fy:.0} NO={fn_:.0} (total_px~{total_px:.2})"
                ));
                return;
            }
            self._handle_exposure_mismatch(fy, fn_);
            return;
        }
    }

    pub fn _desired_maker_bid(&self, asset_id: &str) -> Option<f64> {
        let (bid, _ask) = self._best_bid_ask(asset_id)?;
        Some((bid + (self.cfg.improve_bid_ticks as f64 * self.cfg.tick)).max(0.0))
    }

    pub fn _maker_max_price(&self, _asset_id: &str) -> Option<f64> {
        let edge = (self.cfg.entry_edge_ticks as f64) * self.cfg.tick;
        Some((1.0 - edge).clamp(0.0, 0.999))
    }

    pub fn _maker_bid_cross_ask_safe(
        &self,
        asset_id: &str,
        other_asset_id: &str,
        edge: f64,
    ) -> Option<f64> {
        let (bid, _) = self._best_bid_ask(asset_id)?;
        let (_, other_ask) = self._best_bid_ask(other_asset_id)?;
        let cap = (1.0 - edge - other_ask).max(0.0);
        Some(bid.min(cap))
    }

    pub fn _maybe_replace(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        stale_seconds: Option<i64>,
    ) -> bool {
        let now = now_ts_f64();
        let aid = asset_id.to_string();
        let guard_key = format!("__cancel_pending_until_{aid}");
        if now < self._runtime_ts_get(&guard_key) {
            return false;
        }
        self._reconcile_exchange_orders_for_asset(&aid, Some(price), false);

        let stale = stale_seconds.unwrap_or(self.cfg.stale_seconds).max(1) as f64;
        let oo = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.open_orders.get(&aid).cloned());
        let mut need_new = oo.is_none();
        if let Some(oo) = oo {
            let old_price = oo.price.unwrap_or(0.0);
            let old_size = oo.size.unwrap_or(0.0);
            let age = now - oo.ts.unwrap_or(now);
            let moved_ticks = (price - old_price).abs() / self.cfg.tick.max(0.0001);
            let size_changed = old_size <= 0.0
                || (size - old_size).abs() >= (0.25 * old_size).max(self.cfg.min_shares);
            if age >= stale
                || moved_ticks >= self.cfg.replace_if_price_moves_ticks as f64
                || size_changed
            {
                let reprice_min = env_float("REPRICE_MIN_SECONDS", 0.5).max(0.0);
                if age < reprice_min
                    && !size_changed
                    && age < stale
                    && moved_ticks < (self.cfg.replace_if_price_moves_ticks as f64 * 3.0)
                {
                    return false;
                }
                self.logger.info(&format!(
                    "[REPLACE] {} old={old_price:.2} new={price:.2} moved={moved_ticks:.1} age={age:.1}s",
                    aid.chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                ));
                if let Some(oid_old) = oo.order_id {
                    let _ = self._cancel(&oid_old);
                }
                if let Ok(mut s) = self.state.lock() {
                    s.open_orders.remove(&aid);
                    let _ = save_state(&self.state_file, &mut s);
                }
                let guard_s = env_float("CANCEL_REPLACE_GUARD_SECONDS", 0.2).max(0.0);
                self._runtime_ts_set(&guard_key, now + guard_s);
                return false;
            }
            need_new = false;
        }
        if !need_new {
            return false;
        }

        self._reconcile_exchange_orders_for_asset(&aid, Some(price), true);
        let oid = self._place_postonly_bid(&aid, price, size);
        let Some(oid) = oid else {
            return false;
        };
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.insert(
                aid.to_string(),
                OpenOrderState {
                    order_id: Some(oid),
                    price: Some(price),
                    size: Some(size),
                    ts: Some(now),
                },
            );
            let _ = save_state(&self.state_file, &mut s);
        }
        true
    }

    pub fn _hedge_price_cap(&self) -> f64 {
        let (qy, qn, total_cost) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no, s.c_yes + s.c_no))
            .unwrap_or((0.0, 0.0, 0.0));
        let delta = qy - qn;
        let need = delta.abs();
        if need <= 0.0 {
            return f64::INFINITY;
        }
        let heavy = if delta > 0.0 { qy } else { qn };
        let mut p_max = (heavy - total_cost) / need;
        p_max -= self.cfg.hedge_buffer_ticks as f64 * self.cfg.tick.max(0.0001);
        p_max = round_down(p_max, self.cfg.tick.max(0.0001));
        p_max.max(0.0)
    }

    pub fn _cancel_heavy_side_orders(&self) {
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return,
        };
        let (qy, qn, oo) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no, s.open_orders.clone()))
            .unwrap_or((0.0, 0.0, HashMap::new()));
        let delta = qy - qn;
        if delta.abs() < self.cfg.min_shares {
            return;
        }
        let heavy_asset = if delta > 0.0 { yes } else { no };
        if let Some(oo_row) = oo.get(heavy_asset) {
            if let Some(oid) = &oo_row.order_id {
                self.logger.info(&format!(
                    "Cancel heavy-side order asset={} (delta={delta:.2})",
                    heavy_asset
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                ));
                let _ = self._cancel(oid);
                if let Ok(mut s) = self.state.lock() {
                    s.open_orders.remove(heavy_asset);
                    let _ = save_state(&self.state_file, &mut s);
                }
            }
        }
    }

    pub fn _log_status(&self) {
        let s = self.state.lock().map(|v| v.clone()).unwrap_or_default();
        let lp = locked_profit(&s);
        let cpp = cost_per_pair(&s);
        let total = s.c_yes + s.c_no;
        let mut line = format!(
            "LP={lp:+.4} CPP={cpp:.6} TotalCost={total:.4} qYES={:.2} qNO={:.2} (mode={})",
            s.q_yes, s.q_no, self.exec_mode
        );
        if self.exec_mode == "TAKER_PAIR" || env_bool("DEBUG_MODE", false) {
            if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                let yq = self._best_bid_ask(y);
                let nq = self._best_bid_ask(n);
                if let (Some((yb, ya)), Some((nb, na))) = (yq, nq) {
                    let ask_sum = ya + na;
                    let req = self._pair_arb_required_total();
                    let remaining_budget =
                        (self.cfg.max_total_cost - total - self.cfg.reserve_usd).max(0.0);
                    line.push_str(&format!(
                        " | BBO YES {yb:.2}/{ya:.2} NO {nb:.2}/{na:.2} ask_sum={ask_sum:.2} req<={req:.2} budget~{remaining_budget:.2}"
                    ));
                } else {
                    line.push_str(" | BBO missing");
                }
            }
        }
        self.logger.info(&line);
    }

    pub fn _flatten_now_best(&self, delta: f64) -> Option<Value> {
        let (qy, qn, total_cost) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no, s.c_yes + s.c_no))
            .unwrap_or((0.0, 0.0, 0.0));
        let need = delta.abs();
        if need <= 0.0 {
            return None;
        }
        let (heavy_asset, missing_asset) = if delta > 0.0 {
            (self.yes_asset.clone(), self.no_asset.clone())
        } else {
            (self.no_asset.clone(), self.yes_asset.clone())
        };
        let (Some(heavy_asset), Some(missing_asset)) = (heavy_asset, missing_asset) else {
            return None;
        };
        let heavy_qty = qy.max(qn);
        let light_qty = qy.min(qn);

        let heavy_ba = self._best_bid_ask(&heavy_asset);
        let miss_ba = self._best_bid_ask(&missing_asset);
        let (Some((heavy_bid, _)), Some((_, missing_ask))) = (heavy_ba, miss_ba) else {
            return None;
        };
        if heavy_bid <= 0.0 || missing_ask <= 0.0 {
            return None;
        }
        let cap_now = self._hedge_price_cap();
        let gap = missing_ask - cap_now;
        let lp_buy = heavy_qty - (total_cost + need * missing_ask);
        let lp_sell = light_qty - (total_cost - need * heavy_bid);
        let (best_lp, action) = if lp_sell >= lp_buy {
            (lp_sell, "SELL_HEAVY")
        } else {
            (lp_buy, "BUY_MISSING")
        };
        let loss = (-best_lp).max(0.0);
        Some(json!({
            "action": action,
            "lp": best_lp,
            "loss": loss,
            "cap_now": cap_now,
            "missing_ask": missing_ask,
            "heavy_bid": heavy_bid,
            "gap": gap,
            "need": need,
            "heavy_asset": heavy_asset,
            "missing_asset": missing_asset,
        }))
    }

    pub fn _maybe_trigger_max_loss(&self, delta: f64, unhedged_age: f64) -> bool {
        if !env_bool("MAX_LOSS_ENABLED", true) {
            return false;
        }
        let max_loss = env_float("MAX_LOSS_USD_PER_MARKET", 1.0).max(0.0);
        let s = self.state.lock().map(|v| v.clone()).unwrap_or_default();
        let lp = locked_profit(&s);
        if lp <= -max_loss {
            self.logger.warning(&format!(
                "max-loss triggered lp={lp:.4} delta={delta:.4} unhedged_age={unhedged_age:.2}s"
            ));
            return true;
        }
        false
    }

    pub fn _force_flatten_and_stop(&self, delta: f64, info: &Value) {
        self.logger.warning(&format!(
            "force flatten + stop delta={delta:.4} info={}",
            info
        ));
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Ok(mut r) = self.exit_reason.lock() {
            *r = "FORCED_FLATTEN".to_string();
        }
    }

    pub fn _emergency_taker_hedge_step(&self, delta: f64, reason: &str) {
        if delta.abs() < self.cfg.min_shares {
            return;
        }
        let now = now_ts_f64();
        if now < self._runtime_ts_get("__taker_inflight_until") {
            return;
        }
        let last_taker_hedge_ts = self._runtime_ts_get("__last_taker_hedge_ts");
        if now - last_taker_hedge_ts < self.taker_hedge_min_interval {
            return;
        }
        self._runtime_ts_set("__last_taker_hedge_ts", now);
        let missing_asset = if delta > 0.0 {
            self.no_asset.clone()
        } else {
            self.yes_asset.clone()
        };
        let Some(missing_asset) = missing_asset else {
            return;
        };
        if self.taker_strict_inflight && self._has_pending_taker_order("BUY", Some(&missing_asset))
        {
            return;
        }
        let ba = self._best_bid_ask(&missing_asset);
        let Some((_bid, ask)) = ba else {
            self.logger.info(&format!(
                "Emergency hedge: missing best_bid_ask for {} ({reason})",
                missing_asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return;
        };
        if ask <= 0.0 {
            self.logger.info(&format!(
                "Emergency hedge: missing ask for {} ({reason})",
                missing_asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return;
        }
        let cap = self._hedge_price_cap();
        let mut px_candidate = ask + self.hedge_slippage_ticks as f64 * self.cfg.tick.max(0.0001);
        px_candidate = round_up(px_candidate, self.cfg.tick.max(0.0001));
        px_candidate = clamp(px_candidate, self.cfg.tick.max(0.0001), 0.99);
        let mut px = cap.min(px_candidate);
        px = round_down(px, self.cfg.tick.max(0.0001));

        if cap <= 0.0 {
            self.logger
                .info(&format!("Hedge cap <= 0 (cap={cap:.2}) -> STOP"));
            self._set_exit_reason("CAP_LOCKED_LOSS");
            self.cancel_all_orders_exchange("cap<=0 locked loss");
            self.stop_flag.store(true, Ordering::SeqCst);
            return;
        }
        if ask > cap || px + 1e-9 < ask {
            self.logger.info(&format!(
                "Emergency hedge blocked: ask={ask:.2} cap={cap:.2} (px={px:.2}) ({reason})."
            ));
            let size_try = delta
                .abs()
                .min(self.cfg.clip_shares)
                .max(self.cfg.min_shares);
            let hedge_stale = env_int("HEDGE_STALE_SECONDS", self.cfg.stale_seconds) as i64;
            let _ = self._maybe_replace(&missing_asset, cap, size_try, Some(hedge_stale));
            self.cancel_all_open_orders_local_except(
                &missing_asset,
                "hedge cap blocked (keep hedge)",
            );
            return;
        }

        let total_cost = self.state.lock().map(|s| s.c_yes + s.c_no).unwrap_or(0.0);
        let remaining_usd = self.cfg.max_total_cost - total_cost - self.cfg.reserve_usd;
        if remaining_usd <= 0.0 {
            self.logger.info(&format!(
                "No remaining budget to hedge. total_cost={total_cost:.2} cap={:.2} reserve={:.2} -> STOP",
                self.cfg.max_total_cost, self.cfg.reserve_usd
            ));
            self._set_exit_reason("NO_BUDGET");
            self.cancel_all_orders_exchange("no budget to hedge");
            self.stop_flag.store(true, Ordering::SeqCst);
            return;
        }
        let need_int = (delta.abs() + 1e-12).floor() as i64;
        let max_affordable = (remaining_usd / px + 1e-12).floor() as i64;
        let size_int = need_int.min(max_affordable);
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        if size_int < min_int {
            self.logger.info(&format!(
                "Hedge too expensive for remaining budget. remaining={remaining_usd:.2} px={px:.2} need={need_int} max_affordable={max_affordable} -> STOP"
            ));
            self._set_exit_reason("HEDGE_TOO_EXPENSIVE");
            self.cancel_all_orders_exchange("hedge too expensive");
            self.stop_flag.store(true, Ordering::SeqCst);
            return;
        }
        let partial = size_int < need_int;
        self.logger.info(&format!(
            "EMERGENCY HEDGE ({reason}) delta={delta:.4} need={size_int} buy={} ask={ask:.2} px={px:.2} remaining_usd={remaining_usd:.2} type={}",
            missing_asset
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>(),
            self.hedge_taker_order_type
        ));
        self.cancel_all_open_orders_local("before emergency taker hedge");
        self._runtime_ts_set("__taker_inflight_until", now_ts_f64() + 2.0);
        let oid = self._place_taker_bid_fak(
            &missing_asset,
            px,
            size_int as f64,
            Some(&self.hedge_taker_order_type),
        );
        if partial && (oid.is_some() || self.cfg.dry_run) {
            self.logger.info(&format!(
                "Partial hedge executed ({size_int}/{need_int} shares) due to budget. Stopping."
            ));
            thread::sleep(Duration::from_secs(1));
            self._set_exit_reason("PARTIAL_HEDGE_BUDGET");
            self.cancel_all_orders_exchange("partial hedge stop");
            self.stop_flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn _sniper_best_snapshot(&self) -> (f64, f64, f64, f64) {
        let (yb, ya) = self
            .yes_asset
            .as_deref()
            .and_then(|a| self._best_bid_ask(a))
            .unwrap_or((0.0, 0.0));
        let (nb, na) = self
            .no_asset
            .as_deref()
            .and_then(|a| self._best_bid_ask(a))
            .unwrap_or((0.0, 0.0));
        (yb, ya, nb, na)
    }

    pub fn _sniper_mark_to_market_pnl(&self) -> f64 {
        let s = self.state.lock().map(|v| v.clone()).unwrap_or_default();
        locked_profit(&s)
    }

    pub fn _sniper_position(&self) -> Option<Value> {
        let s = self.state.lock().map(|v| v.clone()).unwrap_or_default();
        let yes = s.q_yes;
        let no = s.q_no;
        let min_sh = self.cfg.min_shares.max(0.0);
        let (side, qty, avg, cost, asset_id) =
            if yes >= (min_sh - 1e-12) && (yes >= no || no < (min_sh - 1e-12)) {
                let qty = yes.max(0.0);
                let avg = if yes > 1e-12 { s.c_yes / yes } else { 0.0 };
                let cost = s.c_yes.max(0.0);
                (
                    "YES",
                    qty,
                    avg,
                    cost,
                    self.yes_asset.clone().unwrap_or_default(),
                )
            } else if no >= (min_sh - 1e-12) {
                let qty = no.max(0.0);
                let avg = if no > 1e-12 { s.c_no / no } else { 0.0 };
                let cost = s.c_no.max(0.0);
                (
                    "NO",
                    qty,
                    avg,
                    cost,
                    self.no_asset.clone().unwrap_or_default(),
                )
            } else {
                return None;
            };
        if qty < (min_sh - 1e-12) {
            return None;
        }
        let (bid, ask) = self._best_bid_ask(&asset_id).unwrap_or((0.0, 0.0));
        Some(json!({
            "side": side,
            "qty": qty,
            "avg": avg,
            "cost": cost,
            "bid": bid,
            "ask": ask,
            "asset_id": asset_id,
        }))
    }

    fn _sniper_has_resting_entry_order(&self) -> bool {
        let yes = self.yes_asset.clone().unwrap_or_default();
        let no = self.no_asset.clone().unwrap_or_default();
        self.state
            .lock()
            .map(|s| {
                [yes, no].iter().any(|aid| {
                    if aid.trim().is_empty() {
                        return false;
                    }
                    s.open_orders
                        .get(aid)
                        .and_then(|oo| oo.order_id.as_ref())
                        .map(|oid| !oid.trim().is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    pub fn _sniper_est_entry_price(&self, ask: f64) -> f64 {
        let tick = self.cfg.tick.max(0.0001);
        let slip_ticks = env_int("SNIPER_ENTRY_SLIPPAGE_TICKS", 1) as f64;
        let hard_max = env_float("SNIPER_HARD_MAX_PRICE", env_float("SNIPER_PRICE_MAX", 0.99));
        let px = round_up(ask + tick * slip_ticks, tick);
        clamp(px, tick, hard_max.max(tick))
    }

    pub fn _sniper_est_exit_price(&self, bid: f64, extra_slip_ticks: f64) -> f64 {
        let tick = self.cfg.tick.max(0.0001);
        clamp(
            round_down(bid - tick * extra_slip_ticks.max(0.0), tick),
            tick,
            0.99,
        )
    }

    pub fn _sniper_maybe_endgame_blind_post(&self, seconds_left: f64, now_ts: f64) -> bool {
        if !env_bool("SNIPER_ENDGAME_BLIND_POST", false) {
            return false;
        }
        let trigger_mode = std::env::var("SNIPER_ENDGAME_TRIGGER_MODE")
            .unwrap_or_else(|_| "WINDOW".to_string())
            .to_ascii_uppercase();
        let win_s = env_float("SNIPER_ENDGAME_BLIND_POST_WINDOW_SECONDS", 0.0);
        let grace = env_float("SNIPER_EXPIRY_GRACE_SECONDS", 0.0).max(0.0);
        if trigger_mode == "RESOLUTION" {
            if seconds_left > 1e-9 || seconds_left < (-grace - 1e-9) {
                return false;
            }
            if self._runtime_ts_get("__sniper_endgame_resolution_watch_start_ts") <= 0.0 {
                let now_ms = (now_ts * 1000.0) as i64;
                let resolution_ts_ms = self.expiry_ts.saturating_mul(1000);
                self.logger.info(&format!(
                    "[RTDS_ENDGAME][TIMING] watch_start now_ms={} resolution_ts_ms={} pre_resolution_ms={} t_left={:.2}s",
                    now_ms,
                    resolution_ts_ms,
                    resolution_ts_ms.saturating_sub(now_ms),
                    seconds_left
                ));
                self._runtime_ts_set("__sniper_endgame_resolution_watch_start_ts", now_ts);
            }
            if self._runtime_ts_get("__sniper_endgame_resolution_attempted_ts") > 0.0 {
                return false;
            }
            if !self._sniper_endgame_resolution_tick_ready(seconds_left) {
                return false;
            }
        } else {
            // WINDOW mode: keep previous behavior.
            // 0 disables. Positive means "final N seconds before expiry".
            // Negative means "start posting N seconds after expiry" (within grace).
            if win_s.abs() <= 1e-9 {
                return false;
            }
            if seconds_left > win_s + 1e-9 || seconds_left < (-grace - 1e-9) {
                return false;
            }
        }
        let (qy, qn, trade_count, open_orders) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no, s.sniper_trade_count, s.open_orders.clone()))
            .unwrap_or((0.0, 0.0, 0, HashMap::new()));
        let min_sh = self.cfg.min_shares;
        if qy >= (min_sh - 1e-9) || qn >= (min_sh - 1e-9) {
            return false;
        }
        if trade_count >= env_int("SNIPER_MAX_TRADES_PER_MARKET", 1) {
            return false;
        }
        let last_attempt = self._runtime_ts_get("__sniper_endgame_post_last_attempt_ts");
        if now_ts - last_attempt < 0.20 {
            return false;
        }
        self._runtime_ts_set("__sniper_endgame_post_last_attempt_ts", now_ts);

        let side_cfg = std::env::var("SNIPER_ENDGAME_SIDE")
            .unwrap_or_else(|_| "AUTO".to_string())
            .to_ascii_uppercase();
        let mut side_src = "AUTO_BBO".to_string();
        let side: Option<String> = if side_cfg == "YES" || side_cfg == "NO" {
            side_src = "FIXED".to_string();
            Some(side_cfg)
        } else if side_cfg == "RTDS" {
            side_src = "RTDS".to_string();
            self._sniper_endgame_side_from_rtds(seconds_left)
        } else {
            let (_yb, ya, _nb, na) = self._sniper_best_snapshot();
            let mut opts: Vec<(&str, f64)> = Vec::new();
            if ya > 0.0 {
                opts.push(("YES", ya));
            }
            if na > 0.0 {
                opts.push(("NO", na));
            }
            if opts.is_empty() {
                return false;
            }
            let price_min = env_float("SNIPER_PRICE_MIN", 0.0);
            let eps = env_float("SNIPER_PRICE_MAX_EPSILON", 0.0);
            let require_min = env_bool("SNIPER_ENDGAME_REQUIRE_PRICE_MIN", true);
            let good: Vec<(&str, f64)> = opts
                .iter()
                .copied()
                .filter(|(_, p)| *p + 1e-12 >= (price_min - eps))
                .collect();
            let side = if good.len() == 1 {
                Some(good[0].0.to_string())
            } else if good.len() > 1 {
                good
                    .iter()
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(s, _)| s.to_string())
            } else if require_min && price_min > 0.0 {
                return false;
            } else {
                opts
                    .iter()
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(s, _)| s.to_string())
            };
            side
        };
        let Some(side) = side else {
            return false;
        };
        if env_bool("RTDS_ENTRY_GATE_APPLY_ENDGAME", true)
            && !self._rtds_entry_gate_allows_side(&side, seconds_left, "SNIPER_ENDGAME")
        {
            return false;
        }
        let asset_id = if side == "YES" {
            self.yes_asset.clone().unwrap_or_default()
        } else {
            self.no_asset.clone().unwrap_or_default()
        };
        if asset_id.trim().is_empty() {
            return false;
        }
        if open_orders
            .get(&asset_id)
            .and_then(|oo| oo.order_id.clone())
            .is_some()
        {
            return true;
        }
        let tick = self.cfg.tick.max(0.0001);
        let mut px = env_float("SNIPER_ENDGAME_BLIND_POST_PRICE", 0.0);
        if px <= 0.0 {
            px = env_float("SNIPER_HARD_MAX_PRICE", env_float("SNIPER_PRICE_MAX", 0.99));
        }
        let mut hard_max = env_float("SNIPER_HARD_MAX_PRICE", px);
        if hard_max <= 0.0 {
            hard_max = px;
        }
        px = round_down(px.min(hard_max), tick);
        px = clamp(px, tick, hard_max.max(tick));
        let mut size_target = self._sniper_calc_entry_size(px);
        if size_target <= 0 {
            return false;
        }
        let size_override = env_int("SNIPER_ENDGAME_BLIND_POST_SIZE_SHARES", 0);
        if size_override > 0 {
            size_target = size_target.min(size_override);
        }
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        if size_target < min_int {
            return false;
        }
        size_target = (size_target / min_int) * min_int;
        if size_target < min_int {
            return false;
        }
        let post_only = if env_bool("SNIPER_ENTRY_POST_ONLY", false) {
            Some(true)
        } else {
            None
        };
        let endgame_ot = self._resolve_order_type(
            &std::env::var("SNIPER_ENDGAME_ORDER_TYPE")
                .unwrap_or_else(|_| "GTC".to_string())
                .to_ascii_uppercase(),
        );
        if trigger_mode == "RESOLUTION" {
            let now_ms = (now_ts * 1000.0) as i64;
            let resolution_ts_ms = self.expiry_ts.saturating_mul(1000);
            let watch_start = self._runtime_ts_get("__sniper_endgame_resolution_watch_start_ts");
            let watch_start_ms = if watch_start > 0.0 {
                (watch_start * 1000.0) as i64
            } else {
                0
            };
            let since_watch_ms = if watch_start_ms > 0 {
                now_ms.saturating_sub(watch_start_ms)
            } else {
                0
            };
            self.logger.info(&format!(
                "[RTDS_ENDGAME][TIMING] fire_order now_ms={} resolution_ts_ms={} since_resolution_ms={} since_watch_ms={} side={} px={:.3} sz={} t_left={:.2}s",
                now_ms,
                resolution_ts_ms,
                now_ms.saturating_sub(resolution_ts_ms),
                since_watch_ms,
                side,
                px,
                size_target,
                seconds_left
            ));
        }
        if trigger_mode == "RESOLUTION" {
            // One-shot mode: mark attempted when we are actually about to submit.
            self._runtime_ts_set("__sniper_endgame_resolution_attempted_ts", now_ts);
        }
        let fresh = if self._market_data_fresh() { "Y" } else { "N" };
        self.logger.info(&format!(
            "[SNIPER] ENDGAME blind-post side={side} src={side_src} trigger={trigger_mode} px={px:.3} sz={size_target} t_left={seconds_left:.2}s fresh={fresh} type={endgame_ot}"
        ));
        let oid = if endgame_ot == "GTC" {
            self._place_limit_bid_gtc(&asset_id, px, size_target as f64, post_only)
        } else {
            let inflight_s = env_float("SNIPER_ENTRY_INFLIGHT_SECONDS", 1.5).max(0.25);
            self._runtime_ts_set("__taker_inflight_until", now_ts_f64() + inflight_s);
            self._place_taker_bid_fak(&asset_id, px, size_target as f64, Some(&endgame_ot))
        };
        if oid.is_none() {
            return false;
        }
        let pending_key = Self::_sniper_entry_pending_key(&asset_id);
        let confirmed_key = Self::_sniper_entry_confirmed_key(&asset_id);
        self._runtime_ts_set(&pending_key, now_ts_f64());
        self._runtime_ts_set(&confirmed_key, 0.0);
        if endgame_ot != "GTC" {
            let inflight_s = env_float("SNIPER_ENTRY_INFLIGHT_SECONDS", 1.5).max(0.25);
            thread::sleep(Duration::from_secs_f64(inflight_s.max(1.0).min(4.0)));
            let filled = self._sniper_position().is_some();
            if !filled {
                let pause_s = env_float("SNIPER_ENTRY_RETRY_PAUSE_SECONDS", 0.0).max(0.0);
                if pause_s > 0.0 {
                    self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + pause_s);
                }
                return false;
            }
            self._mark_sniper_entry_state(&side);
            return true;
        }
        if let Ok(mut s) = self.state.lock() {
            s.sniper_last_entry_ts = now_ts_f64();
            s.sniper_last_side = side;
            let _ = save_state(&self.state_file, &mut s);
        }
        true
    }

    pub fn _sniper_entry_candidate(
        &self,
        seconds_left: f64,
        ignore_roi_gate: bool,
    ) -> Option<Value> {
        let (yb, ya, nb, na) = self._sniper_best_snapshot();
        if yb <= 0.0 || ya <= 0.0 || nb <= 0.0 || na <= 0.0 {
            return None;
        }
        let y_mid = 0.5 * (yb + ya);
        let n_mid = 0.5 * (nb + na);
        let parity = (y_mid + n_mid - 1.0).abs();
        let sniper_parity_tolerance =
            env_float("SNIPER_PARITY_TOLERANCE", self.parity_tolerance.max(0.0));
        if parity > sniper_parity_tolerance {
            return None;
        }
        let tick = self.cfg.tick.max(0.0001);
        let side = if y_mid >= n_mid { "YES" } else { "NO" };
        let bid = if side == "YES" { yb } else { nb };
        let ask = if side == "YES" { ya } else { na };
        let spread_ticks = ((ask - bid) / tick).round() as i64;
        let max_spread_ticks = env_int("SNIPER_MAX_SPREAD_TICKS", self.max_spread_ticks);
        if spread_ticks > max_spread_ticks {
            return None;
        }

        let price_min = env_float("SNIPER_PRICE_MIN", 0.91);
        let price_max = env_float("SNIPER_PRICE_MAX", 0.99);
        let eps = env_float("SNIPER_PRICE_MAX_EPSILON", 0.0);
        let hard_max = env_float("SNIPER_HARD_MAX_PRICE", price_max);
        let entry_type_name = std::env::var("SNIPER_ENTRY_ORDER_TYPE")
            .unwrap_or_else(|_| "FOK".to_string())
            .to_ascii_uppercase();
        let limit_entry = matches!(entry_type_name.as_str(), "GTC" | "LIMIT");
        let entry_px = self._sniper_est_entry_price(ask);
        if entry_px <= 0.0 {
            return None;
        }
        if !limit_entry && entry_px + 1e-12 < ask {
            return None;
        }
        if !limit_entry && ask > hard_max + 1e-9 {
            return None;
        }
        if entry_px < (price_min - eps) || entry_px > (price_max + eps) {
            return None;
        }
        if !ignore_roi_gate {
            let max_exit_price = if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true) {
                0.99
            } else {
                1.0
            };
            let max_roi = if entry_px > 0.0 {
                (max_exit_price - entry_px) / entry_px
            } else {
                0.0
            };
            let fee_allow = if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true) {
                2.0
            } else {
                1.0
            } * env_float("SNIPER_FEE_RATE", 0.0);
            let required_roi = env_float("SNIPER_TAKE_PROFIT_PCT", 0.01)
                + fee_allow
                + env_float("SNIPER_MIN_EDGE_OVER_FEES", 0.0);
            if max_roi + 1e-9 < required_roi {
                return None;
            }
        }
        let asset_id = if side == "YES" {
            self.yes_asset.clone().unwrap_or_default()
        } else {
            self.no_asset.clone().unwrap_or_default()
        };
        Some(json!({
            "side": side,
            "asset_id": asset_id,
            "bid": bid,
            "ask": ask,
            "entry_px": entry_px,
            "spread_ticks": spread_ticks,
            "parity": parity,
            "seconds_left": seconds_left,
        }))
    }

    pub fn _sniper_entry_confirmed(&self, cand: &Value, now_ts: f64) -> bool {
        let confirm_s = env_float("SNIPER_ENTRY_CONFIRM_SECONDS", 0.0);
        if confirm_s <= 0.0 {
            return true;
        }
        let side = cand
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !matches!(side.as_str(), "YES" | "NO") {
            self._runtime_ts_set("__sniper_entry_gate_since", 0.0);
            self._runtime_ts_set("__sniper_entry_gate_side_yes", 0.0);
            self._runtime_ts_set("__sniper_entry_gate_side_no", 0.0);
            return false;
        }
        let since = self._runtime_ts_get("__sniper_entry_gate_since");
        let side_yes = self._runtime_ts_get("__sniper_entry_gate_side_yes") > 0.0;
        let current_side_yes = side == "YES";
        if since <= 0.0 || side_yes != current_side_yes {
            self._runtime_ts_set("__sniper_entry_gate_since", now_ts);
            self._runtime_ts_set(
                "__sniper_entry_gate_side_yes",
                if current_side_yes { 1.0 } else { 0.0 },
            );
            self._runtime_ts_set(
                "__sniper_entry_gate_side_no",
                if current_side_yes { 0.0 } else { 1.0 },
            );
            return false;
        }
        now_ts - since >= confirm_s
    }

    pub fn _sniper_calc_entry_size(&self, entry_price: f64) -> i64 {
        let max_notional = env_float(
            "SNIPER_MAX_NOTIONAL_USD",
            self.cfg.max_total_cost.min(100.0),
        );
        if entry_price <= 0.0 {
            return 0;
        }
        (max_notional / entry_price).floor() as i64
    }

    pub fn _log_status_sniper(&self, seconds_left: f64) {
        let pos = self._sniper_position();
        let pnl = self._sniper_mark_to_market_pnl();
        let tc = self
            .state
            .lock()
            .map(|s| s.sniper_trade_count)
            .unwrap_or_default();
        if pos.is_none() {
            self.logger.info(&format!(
                "[SNIPER] t_left={seconds_left:6.1}s trades={tc} pnl(mtm)={pnl:+.4} (flat)"
            ));
            return;
        }
        let pos = pos.unwrap_or_default();
        let cost = pos.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let qty = pos.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let bid = pos.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let avg = pos.get("avg").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let exit_px = self._sniper_est_exit_price(bid, 0.0);
        let pnl_est = qty * exit_px - cost;
        let pnl_pct = if cost > 1e-12 { pnl_est / cost } else { 0.0 };
        self.logger.info(&format!(
            "[SNIPER] t_left={seconds_left:6.1}s trades={tc} side={} qty={qty:.2} avg={avg:.3} bid={bid:.3} ex={exit_px:.3} pnl={pnl_est:+.4} ({:+.2}%)",
            pos.get("side").and_then(|v| v.as_str()).unwrap_or(""),
            pnl_pct * 100.0
        ));
    }

    pub fn _sniper_try_enter(&self, cand: &Value) -> bool {
        let now = now_ts_f64();
        if now < self._runtime_ts_get("__taker_fail_pause_until") {
            return false;
        }
        if now < self._runtime_ts_get("__taker_inflight_until") {
            return false;
        }
        let last_signal_ts = self._runtime_ts_get("__sniper_last_signal_ts");
        if now - last_signal_ts < 0.25 {
            return false;
        }
        self._runtime_ts_set("__sniper_last_signal_ts", now);

        let ask = cand.get("ask").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if ask <= 0.0 {
            return false;
        }
        let side = cand.get("side").and_then(|v| v.as_str()).unwrap_or("");
        let seconds_left = cand
            .get("seconds_left")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.expiry_ts as f64 - now);
        let entry_mode = cand
            .get("entry_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("NORMAL")
            .to_ascii_uppercase();
        let gate_context = if entry_mode == "FORCE" {
            "SNIPER_ENTRY_FORCE"
        } else {
            "SNIPER_ENTRY"
        };
        if !self._rtds_entry_gate_allows_side(side, seconds_left, gate_context) {
            return false;
        }
        let entry_type_name = std::env::var("SNIPER_ENTRY_ORDER_TYPE")
            .unwrap_or_else(|_| "FOK".to_string())
            .to_ascii_uppercase();
        let limit_entry = matches!(entry_type_name.as_str(), "GTC" | "LIMIT");
        let mut px = cand
            .get("entry_px")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| self._sniper_est_entry_price(ask));
        if px <= 0.0 {
            return false;
        }
        if !limit_entry && px + 1e-12 < ask {
            return false;
        }
        let tick = self.cfg.tick.max(0.0001);
        let hard_max = env_float("SNIPER_HARD_MAX_PRICE", env_float("SNIPER_PRICE_MAX", 0.99));
        px = clamp(round_up(px, tick), tick, hard_max.max(tick));

        let min_sh = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let size_int = self._sniper_calc_entry_size(px);
        if size_int <= 0 {
            if matches!(
                self.exec_mode.as_str(),
                "SIGNAL_SNIPPER" | "SIGNAL_SNIPER" | "SIGNAL_SNIPE" | "SIGNAL"
            ) && env_bool("SIGNAL_DEBUG", false)
            {
                let req_usd = min_sh as f64 * px;
                let cap = env_float("SNIPER_MAX_NOTIONAL_USD", 0.0);
                self.logger.info(&format!(
                    "[SIGNAL] skip entry: cannot meet min_shares with current budget. min_shares={min_sh} px={px:.4} -> min_required~{req_usd:.2} USD but SIGNAL_MAX_NOTIONAL_USD={cap:.2}."
                ));
            }
            return false;
        }

        let inflight_s = env_float("SNIPER_ENTRY_INFLIGHT_SECONDS", 1.5).max(0.25);
        self._runtime_ts_set("__taker_inflight_until", now + inflight_s);

        let side = cand.get("side").and_then(|v| v.as_str()).unwrap_or("");
        let mut asset_id = cand
            .get("asset_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if asset_id.trim().is_empty() {
            asset_id = if side == "YES" {
                self.yes_asset.clone().unwrap_or_default()
            } else if side == "NO" {
                self.no_asset.clone().unwrap_or_default()
            } else {
                String::new()
            };
        }
        if asset_id.trim().is_empty() {
            return false;
        }

        let pending_age_s = env_float("SNIPER_PENDING_ORDER_MAX_AGE_SECONDS", 0.0).max(0.0);
        if !limit_entry
            && pending_age_s > 0.0
            && self._has_pending_taker_order_recent("BUY", Some(&asset_id), pending_age_s)
        {
            return false;
        }

        let target = size_int;
        let chunk_cfg = env_int("SNIPER_ENTRY_CHUNK_SHARES", 0);
        let mut desired_chunk = if chunk_cfg <= 0 {
            target
        } else {
            chunk_cfg.min(target)
        };
        if desired_chunk < min_sh {
            desired_chunk = min_sh;
        }
        desired_chunk = (desired_chunk / min_sh) * min_sh;
        if desired_chunk < min_sh {
            return false;
        }

        if limit_entry {
            let has_open = self
                .state
                .lock()
                .ok()
                .and_then(|s| {
                    s.open_orders
                        .get(&asset_id)
                        .and_then(|o| o.order_id.clone())
                })
                .is_some();
            if has_open {
                return true;
            }
            let post_only = if env_bool("SNIPER_ENTRY_POST_ONLY", false) {
                Some(true)
            } else {
                None
            };
            let oid = self._place_limit_bid_gtc(&asset_id, px, desired_chunk as f64, post_only);
            if oid.is_none() {
                return false;
            }
            let pending_key = Self::_sniper_entry_pending_key(&asset_id);
            let confirmed_key = Self::_sniper_entry_confirmed_key(&asset_id);
            self._runtime_ts_set(&pending_key, now_ts_f64());
            self._runtime_ts_set(&confirmed_key, 0.0);
            // Python parity: for resting LIMIT/GTC entry, record last-entry metadata
            // but do not count it as a completed sniper trade until a fill exists.
            if let Ok(mut s) = self.state.lock() {
                s.sniper_last_entry_ts = now_ts_f64();
                s.sniper_last_side = side.to_string();
                let _ = save_state(&self.state_file, &mut s);
            }
            return true;
        }

        let primary_type = entry_type_name;
        let fallback_type = std::env::var("SNIPER_ENTRY_ORDER_TYPE_FALLBACK")
            .unwrap_or_default()
            .to_ascii_uppercase();
        let max_orders = env_int("SNIPER_ENTRY_MAX_ORDERS", 3).max(1);
        let mut orders_sent = 0i64;
        let mut any_submitted = false;
        let mut sizes_to_try: Vec<i64> = vec![desired_chunk];
        if primary_type == "FOK" {
            let mut s_try = desired_chunk;
            let mut shrink_factor = env_float("SNIPER_ENTRY_SHRINK_FACTOR", 0.5);
            if !(0.05..1.0).contains(&shrink_factor) {
                shrink_factor = 0.5;
            }
            let mut shrink_min = env_int("SNIPER_ENTRY_SHRINK_MIN_CHUNK_SHARES", min_sh);
            if shrink_min < min_sh {
                shrink_min = min_sh;
            }
            shrink_min = (shrink_min / min_sh) * min_sh;
            if shrink_min < min_sh {
                shrink_min = min_sh;
            }
            while s_try > shrink_min {
                let mut s2 = (s_try as f64 * shrink_factor + 1e-12).floor() as i64;
                s2 = s2.max(shrink_min);
                s2 = (s2 / min_sh) * min_sh;
                if s2 < shrink_min {
                    s2 = shrink_min;
                }
                if s2 >= s_try {
                    break;
                }
                sizes_to_try.push(s2);
                s_try = s2;
            }
            if sizes_to_try.last().copied().unwrap_or(min_sh) != shrink_min {
                sizes_to_try.push(shrink_min);
            }
        }
        let mut submitted_primary = false;
        for this_chunk in sizes_to_try {
            if orders_sent >= max_orders {
                break;
            }
            let oid =
                self._place_taker_bid_fak(&asset_id, px, this_chunk as f64, Some(&primary_type));
            orders_sent += 1;
            if oid.is_some() {
                any_submitted = true;
                submitted_primary = true;
                break;
            }
        }
        if !submitted_primary
            && !fallback_type.trim().is_empty()
            && fallback_type != primary_type
            && orders_sent < max_orders
        {
            let fb_chunk = desired_chunk.max(min_sh);
            let oid =
                self._place_taker_bid_fak(&asset_id, px, fb_chunk as f64, Some(&fallback_type));
            orders_sent += 1;
            if oid.is_some() {
                any_submitted = true;
            }
        }
        if !any_submitted {
            return false;
        }

        let pending_key = Self::_sniper_entry_pending_key(&asset_id);
        let confirmed_key = Self::_sniper_entry_confirmed_key(&asset_id);
        self._runtime_ts_set(&pending_key, now_ts_f64());
        self._runtime_ts_set(&confirmed_key, 0.0);

        thread::sleep(Duration::from_secs_f64(inflight_s.max(1.0).min(4.0)));
        let filled = self._sniper_position().is_some();
        if !filled {
            let pause_s = env_float("SNIPER_ENTRY_RETRY_PAUSE_SECONDS", 0.0).max(0.0);
            if pause_s > 0.0 {
                self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + pause_s);
            }
            return false;
        }
        self._mark_sniper_entry_state(side);
        true
    }

    pub fn _sniper_try_exit(&self, pos: &Value, reason: &str) -> bool {
        let now = now_ts_f64();
        if now < self._runtime_ts_get("__taker_fail_pause_until") {
            return false;
        }
        let asset_id = pos
            .get("asset_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if asset_id.trim().is_empty() {
            return false;
        }
        let reason_u = reason.trim().to_ascii_uppercase();
        let mut mode = std::env::var("SNIPER_STOP_LOSS_MODE")
            .unwrap_or_else(|_| "MARKET".to_string())
            .to_ascii_uppercase();
        if matches!(mode.as_str(), "STOP_LIMIT" | "STOPLIMIT") {
            mode = "LIMIT".to_string();
        }
        if matches!(
            mode.as_str(),
            "STOP_MARKET" | "STOPMARKET" | "TAKER" | "AGGRESSIVE"
        ) {
            mode = "MARKET".to_string();
        }
        let mut stop_limit_mode = reason_u == "STOP_LOSS" && mode == "LIMIT";

        if env_bool("SNIPER_EXIT_REQUIRE_CONFIRMED_ENTRY", true) {
            let pending_key = Self::_sniper_entry_pending_key(&asset_id);
            let confirmed_key = Self::_sniper_entry_confirmed_key(&asset_id);
            let pending_ts = self._runtime_ts_get(&pending_key);
            let confirmed_ts = self._runtime_ts_get(&confirmed_key);
            let confirmed_ok =
                confirmed_ts > 0.0 && (pending_ts <= 0.0 || confirmed_ts + 1e-9 >= pending_ts);
            if !confirmed_ok {
                let now = now_ts_f64();
                let log_key = format!("__sniper_exit_wait_confirm_log_until_{asset_id}");
                if now >= self._runtime_ts_get(&log_key) {
                    let aid_tail: String = asset_id
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    let pending_age = if pending_ts > 0.0 {
                        (now - pending_ts).max(0.0)
                    } else {
                        -1.0
                    };
                    self.logger.info(&format!(
                        "[SNIPER] exit gate: waiting entry order status CONFIRMED asset={aid_tail} pending_age={pending_age:.2}s"
                    ));
                    self._runtime_ts_set(&log_key, now + 2.0);
                }
                self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 0.5);
                return false;
            }
        }

        let remaining = pos.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if remaining <= 0.0 {
            self._mark_sniper_exit_state();
            return true;
        }
        let tick = self.cfg.tick.max(0.0001);
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let exit_allow_fractional = env_bool("SNIPER_EXIT_ALLOW_FRACTIONAL_SIZE", false);
        let mut exit_size_dp_i = env_int("SNIPER_EXIT_SIZE_DECIMALS", env_int("SIZE_DECIMALS", 6));
        exit_size_dp_i = exit_size_dp_i.clamp(0, 8);
        let exit_size_dp = exit_size_dp_i as u32;
        let exit_size_step = 10f64.powi(-(exit_size_dp as i32));
        let exit_size_subtract = env_float("SNIPER_EXIT_SIZE_SUBTRACT", 0.0).max(0.0);
        let min_exit_size = if exit_allow_fractional {
            env_float("SNIPER_EXIT_MIN_ORDER_SIZE", 0.1).max(exit_size_step)
        } else {
            min_int as f64
        };
        let remaining_int = (remaining + 1e-12).floor() as i64;
        if (exit_allow_fractional && remaining + 1e-12 < min_exit_size)
            || (!exit_allow_fractional && remaining_int < min_int)
        {
            self._mark_sniper_exit_state();
            return true;
        }

        let mut balance_avail_int = remaining_int;
        let mut allow_int = remaining_int;
        let mut balance_avail_sh = remaining;
        let mut allow_sh = remaining;
        let mut pos_api_size = -1.0f64;
        if let Some((bal, allow)) = self._get_balance_allowance_conditional_cached(&asset_id, 0.0) {
            balance_avail_int = (bal + 1e-12).floor() as i64;
            allow_int = (allow + 1e-12).floor() as i64;
            balance_avail_sh = bal.max(0.0);
            allow_sh = allow.max(0.0);
        }
        if env_bool("SNIPER_EXIT_POSITIONS_FALLBACK", true)
            && ((exit_allow_fractional
                && (balance_avail_sh < min_exit_size || allow_sh < min_exit_size))
                || (!exit_allow_fractional && (balance_avail_int < min_int || allow_int < min_int)))
        {
            if let Some(sz) = self._get_position_size_data_api(&asset_id) {
                pos_api_size = sz;
                let pos_int = (sz + 1e-12).floor() as i64;
                if pos_int > balance_avail_int {
                    balance_avail_int = pos_int;
                }
                if sz > balance_avail_sh {
                    balance_avail_sh = sz;
                }
                // Soft-guard: if we can see shares from positions API, don't hard-block on
                // potentially stale allowance snapshot; let exchange decide on submit.
                if env_bool("SNIPER_EXIT_ALLOWANCE_SOFT_CHECK", true)
                    && ((exit_allow_fractional
                        && allow_sh < min_exit_size
                        && balance_avail_sh >= min_exit_size)
                        || (!exit_allow_fractional
                            && allow_int < min_int
                            && balance_avail_int >= min_int))
                {
                    allow_int = balance_avail_int;
                    allow_sh = balance_avail_sh;
                    if now >= self._runtime_ts_get("__sniper_soft_allow_log_until") {
                        let aid_tail: String = asset_id
                            .chars()
                            .rev()
                            .take(6)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect();
                        self.logger.warning(&format!(
                            "[SNIPER] soft-allowance enabled: positions API shows shares for asset={aid_tail}; trying exit despite allowance snapshot."
                        ));
                        self._runtime_ts_set("__sniper_soft_allow_log_until", now + 10.0);
                    }
                }
            }
        }
        let precheck_allow_low = (exit_allow_fractional && allow_sh + 1e-12 < min_exit_size)
            || (!exit_allow_fractional && allow_int < min_int);
        if env_bool("SNIPER_EXIT_PRECHECK_BALANCE_ALLOWANCE", false) && precheck_allow_low {
            let aid_tail: String = asset_id
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger.warning(
                "[SNIPER] exit failed: allowance too low. Approve conditional tokens for selling, then the bot will retry.",
            );
            self.logger.info(&format!(
                "[SNIPER] allowance snapshot asset={aid_tail} bal={balance_avail_sh:.6} allow={allow_sh:.6} min_required={min_exit_size:.6}"
            ));
            if env_bool("SNIPER_EXIT_POSITIONS_FALLBACK", true) {
                self.logger.info(&format!(
                    "[SNIPER][DBG_POS] asset={aid_tail} pos_api_size={pos_api_size:.6}"
                ));
            }
            let ba_last_ts = self._runtime_ts_get("__ba_last_fetch_ts");
            let ba_age_s = if ba_last_ts > 0.0 {
                (now - ba_last_ts).max(0.0)
            } else {
                -1.0
            };
            let ba_raw_bal = self._runtime_ts_get("__ba_last_raw_balance");
            let ba_raw_allow = self._runtime_ts_get("__ba_last_raw_allowance");
            let ba_units = self._runtime_ts_get("__ba_last_units_per_share");
            let ba_bal_sh = self._runtime_ts_get("__ba_last_balance_shares");
            let ba_allow_sh = self._runtime_ts_get("__ba_last_allowance_shares");
            let side_dbg = pos.get("side").and_then(|v| v.as_str()).unwrap_or("");
            let avg_dbg = pos.get("avg").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let (q_yes, q_no, c_yes, c_no, oo_count) = self
                .state
                .lock()
                .map(|s| (s.q_yes, s.q_no, s.c_yes, s.c_no, s.open_orders.len() as i64))
                .unwrap_or((0.0, 0.0, 0.0, 0.0, 0));
            let wallet_tail: String = self
                .wallet_address
                .chars()
                .rev()
                .take(8)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            let funder_tail: String = self
                .cfg
                .funder
                .as_deref()
                .unwrap_or("")
                .chars()
                .rev()
                .take(8)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            let yes_tail: String = self
                .yes_asset
                .as_deref()
                .unwrap_or("")
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            let no_tail: String = self
                .no_asset
                .as_deref()
                .unwrap_or("")
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger.info(&format!(
                "[SNIPER][DBG_ALLOW] side={side_dbg} qty={remaining:.4} avg={avg_dbg:.4} asset={aid_tail} yes={yes_tail} no={no_tail} ba_age={ba_age_s:.3}s raw_bal={ba_raw_bal:.0} raw_allow={ba_raw_allow:.0} units={ba_units:.0} sh_bal={ba_bal_sh:.6} sh_allow={ba_allow_sh:.6} wallet=*{wallet_tail} funder=*{funder_tail} state(qy={q_yes:.6},qn={q_no:.6},cy={c_yes:.6},cn={c_no:.6},open_orders={oo_count})"
            ));
            if env_bool("SNIPER_DEBUG_BALANCE_BOTH", false) {
                let yes_id = self.yes_asset.clone().unwrap_or_default();
                let no_id = self.no_asset.clone().unwrap_or_default();
                let (yes_bal, yes_allow) = if !yes_id.trim().is_empty() {
                    self._get_balance_allowance_conditional_cached(&yes_id, 0.0)
                        .unwrap_or((0.0, 0.0))
                } else {
                    (0.0, 0.0)
                };
                let (no_bal, no_allow) = if !no_id.trim().is_empty() {
                    self._get_balance_allowance_conditional_cached(&no_id, 0.0)
                        .unwrap_or((0.0, 0.0))
                } else {
                    (0.0, 0.0)
                };
                let yes_tail2: String = yes_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let no_tail2: String = no_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.info(&format!(
                    "[SNIPER][DBG_BOTH] yes={yes_tail2} bal={yes_bal:.6} allow={yes_allow:.6} | no={no_tail2} bal={no_bal:.6} allow={no_allow:.6}"
                ));
            }
            // Safety-first: auto reconcile is opt-in and conservative.
            // Default is disabled to avoid clearing positions due to transient API/cache issues.
            if balance_avail_int <= 0
                && allow_int <= 0
                && remaining_int >= min_int
                && env_bool("SNIPER_ZERO_BALANCE_AUTO_RECONCILE", true)
            {
                let has_local_order_for_asset = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|s| s.open_orders.get(&asset_id).cloned())
                    .and_then(|o| o.order_id)
                    .map(|oid| !oid.trim().is_empty())
                    .unwrap_or(false);
                let has_exchange_order_for_asset =
                    self._list_open_orders_exchange().iter().any(|o| {
                        self._extract_order_token_id(o).as_deref() == Some(asset_id.as_str())
                            && self
                                ._extract_order_id(o)
                                .map(|oid| !oid.trim().is_empty())
                                .unwrap_or(false)
                    });
                if has_exchange_order_for_asset {
                    if now >= self._runtime_ts_get("__sniper_zero_ba_guard_log_until") {
                        self.logger.warning(&format!(
                            "[SNIPER] desync reconcile skipped: exchange open order still exists for this asset (local_open_order={has_local_order_for_asset})."
                        ));
                        self._runtime_ts_set("__sniper_zero_ba_guard_log_until", now + 10.0);
                    }
                } else {
                    let hit_key = format!("__sniper_zero_ba_hits_{asset_id}");
                    let first_key = format!("__sniper_zero_ba_first_ts_{asset_id}");
                    let window_s =
                        env_float("SNIPER_ZERO_BALANCE_RECONCILE_WINDOW_SECONDS", 45.0).max(5.0);
                    let mut hits = self._runtime_ts_get(&hit_key);
                    let first_ts = self._runtime_ts_get(&first_key);
                    if first_ts <= 0.0 || (now - first_ts) > window_s {
                        self._runtime_ts_set(&first_key, now);
                        hits = 1.0;
                    } else {
                        hits += 1.0;
                    }
                    self._runtime_ts_set(&hit_key, hits);
                    let need_hits = env_int("SNIPER_ZERO_BALANCE_RECONCILE_HITS", 3).max(3) as f64;
                    if hits >= need_hits {
                        self.logger.warning(&format!(
                            "[SNIPER] desync suspected (zero balance/allowance for local qty). auto-reconcile hits={hits:.0}/{need_hits:.0}"
                        ));
                        self._runtime_ts_set(&hit_key, 0.0);
                        self._runtime_ts_set(&first_key, 0.0);
                        self._clear_local_position_for_asset(&asset_id, "zero balance+allowance");
                        // Keep run alive; just clear stale local state and continue.
                        self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 1.0);
                        return false;
                    }
                }
            }
            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 60.0);
            return false;
        }

        if stop_limit_mode {
            let stop_pct = env_float("SNIPER_STOP_LOSS_PCT", 0.0);
            let mut ref_px = self._runtime_ts_get("__sniper_entry_ref_price");
            if ref_px <= 0.0 {
                ref_px = pos.get("avg").and_then(|v| v.as_f64()).unwrap_or(0.0);
            }
            if ref_px <= 0.0 || stop_pct <= 0.0 {
                stop_limit_mode = false;
            } else {
                let floor_raw = ref_px * (1.0 - stop_pct);
                let floor_px = clamp(round_up(floor_raw, tick), tick, 0.99);
                let resubmit_s = env_float("SNIPER_STOP_LIMIT_RESUBMIT_SECONDS", 5.0).max(0.0);
                let last_ts = self._runtime_ts_get("__sniper_stop_limit_order_ts");
                let last_px = self._runtime_ts_get("__sniper_stop_limit_order_px");
                if last_ts > 0.0
                    && (last_px - floor_px).abs() <= tick / 2.0
                    && resubmit_s > 0.0
                    && (now - last_ts) < resubmit_s
                {
                    return false;
                }
                self.cancel_all_open_orders_local(&format!("sniper stop-limit {reason_u}"));
                self._cancel_exchange_orders_for_assets(
                    std::slice::from_ref(&asset_id),
                    &format!("sniper stop-limit {reason_u}"),
                );
                thread::sleep(Duration::from_millis(150));

                let mut sell_sz = remaining.min(balance_avail_sh).min(allow_sh);
                if exit_size_subtract > 0.0 {
                    sell_sz = (sell_sz - exit_size_subtract).max(0.0);
                }
                if exit_allow_fractional {
                    sell_sz = q_down(sell_sz, exit_size_dp);
                    if sell_sz + 1e-12 < min_exit_size {
                        return false;
                    }
                } else {
                    let mut sell_int = (sell_sz + 1e-12).floor() as i64;
                    sell_int = (sell_int / min_int) * min_int;
                    if sell_int < min_int {
                        return false;
                    }
                    sell_sz = sell_int as f64;
                }
                let stop_ot = std::env::var("SNIPER_STOP_LIMIT_ORDER_TYPE")
                    .unwrap_or_else(|_| "GTC".to_string())
                    .to_ascii_uppercase();
                let oid = self._place_taker_ask_fak(&asset_id, floor_px, sell_sz, Some(&stop_ot));
                if oid.is_some() {
                    self._runtime_ts_set("__sniper_stop_limit_order_ts", now_ts_f64());
                    self._runtime_ts_set("__sniper_stop_limit_order_px", floor_px);
                    thread::sleep(Duration::from_secs_f64(1.0));
                    if self._sniper_position().is_none() {
                        self._mark_sniper_exit_state();
                        return true;
                    }
                }
                return false;
            }
        }

        if env_bool("SNIPER_CANCEL_EXIT_ORDERS_BEFORE_RETRY", true) {
            self.cancel_all_open_orders_local(&format!("sniper exit {reason_u}"));
            self._cancel_exchange_orders_for_assets(
                std::slice::from_ref(&asset_id),
                &format!("sniper exit {reason_u}"),
            );
            thread::sleep(Duration::from_millis(200));
        }
        let mut chunk = env_float("SNIPER_EXIT_CHUNK_SHARES", min_int as f64);
        if chunk <= 0.0 {
            chunk = min_int as f64;
        }
        chunk = q_down(chunk, exit_size_dp);
        if chunk <= 0.0 {
            chunk = min_exit_size.max(exit_size_step);
        }
        let full_size_first = env_bool("SNIPER_EXIT_FULL_SIZE_FIRST", true);
        let recent_submit_guard_s =
            env_float("SNIPER_EXIT_RECENT_SUBMIT_GUARD_SECONDS", 8.0).max(1.0);
        let exit_slip_ticks = env_int("SNIPER_EXIT_SLIPPAGE_TICKS", 1).max(0);
        let max_passes = 3i64;
        let mut sold_any = false;
        let mut submitted_any = false;
        let mut last_submit_ts = 0.0f64;
        for pass_i in 0..max_passes {
            let cur = self._sniper_position();
            if cur.is_none() {
                self._mark_sniper_exit_state();
                return true;
            }
            let cur = cur.unwrap_or_default();
            let remaining = cur.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let remaining_int = (remaining + 1e-12).floor() as i64;
            if (exit_allow_fractional && remaining + 1e-12 < min_exit_size)
                || (!exit_allow_fractional && remaining_int < min_int)
            {
                return true;
            }
            let bid = cur.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let mut px = bid - (exit_slip_ticks + pass_i) as f64 * tick;
            px = clamp(round_down(px, tick), tick, 0.99);
            let mut sell_sz = if pass_i == 0 && full_size_first {
                remaining
            } else {
                remaining.min(chunk)
            };
            if exit_size_subtract > 0.0 {
                sell_sz = (sell_sz - exit_size_subtract).max(0.0);
            }
            if exit_allow_fractional {
                sell_sz = q_down(sell_sz, exit_size_dp);
                let cap = balance_avail_sh.min(allow_sh);
                if cap > 0.0 {
                    sell_sz = sell_sz.min(cap);
                }
                sell_sz = q_down(sell_sz, exit_size_dp);
                if sell_sz + 1e-12 < min_exit_size {
                    self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 1.0);
                    return false;
                }
            } else {
                let mut sell_int = (sell_sz + 1e-12).floor() as i64;
                sell_int = (sell_int / min_int) * min_int;
                if sell_int < min_int {
                    sell_int = min_int;
                }
                if balance_avail_int >= min_int {
                    sell_int = sell_int.min(balance_avail_int);
                    sell_int = (sell_int / min_int) * min_int;
                }
                if sell_int < min_int {
                    let chunk_i = (chunk + 1e-12).floor() as i64;
                    sell_int = remaining_int.min(chunk_i.max(min_int));
                }
                sell_sz = sell_int as f64;
            }
            self._runtime_ts_set("__taker_inflight_until", now_ts_f64() + 0.75);
            let ot = std::env::var("SNIPER_EXIT_ORDER_TYPE")
                .unwrap_or_else(|_| "FOK".to_string())
                .to_ascii_uppercase();
            let mut oid = self._place_taker_ask_fak(&asset_id, px, sell_sz, Some(&ot));
            if oid.is_none() {
                let fb = std::env::var("SNIPER_EXIT_ORDER_TYPE_FALLBACK")
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                if !fb.trim().is_empty() && fb != ot {
                    oid = self._place_taker_ask_fak(&asset_id, px, sell_sz, Some(&fb));
                }
            }
            if oid.is_none() {
                if let Some((bal, allow)) =
                    self._get_balance_allowance_conditional_cached(&asset_id, 0.0)
                {
                    let bal_int = (bal + 1e-12).floor() as i64;
                    let allow_int_fresh = (allow + 1e-12).floor() as i64;
                    let bal_sh = bal.max(0.0);
                    let allow_sh_fresh = allow.max(0.0);
                    let recent_submit =
                        submitted_any && (now_ts_f64() - last_submit_ts) <= recent_submit_guard_s;
                    let bal_below_min = (exit_allow_fractional && bal_sh + 1e-12 < min_exit_size)
                        || (!exit_allow_fractional && bal_int < min_int);
                    if bal_below_min {
                        if sold_any || recent_submit {
                            let aid_tail: String = asset_id
                                .chars()
                                .rev()
                                .take(6)
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect();
                            self.logger.warning(&format!(
                                "[SNIPER] balance snapshot below min after recent/partial exit for asset={aid_tail}; deferring local-state reconcile and retrying."
                            ));
                            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 2.0);
                            return false;
                        }
                        if env_bool("SNIPER_EXIT_POSITIONS_FALLBACK", true) {
                            if let Some(sz_live) = self._get_position_size_data_api(&asset_id) {
                                let live_int = (sz_live + 1e-12).floor() as i64;
                                let live_ok = (exit_allow_fractional
                                    && sz_live + 1e-12 >= min_exit_size)
                                    || (!exit_allow_fractional && live_int >= min_int);
                                if live_ok {
                                    let aid_tail: String = asset_id
                                        .chars()
                                        .rev()
                                        .take(6)
                                        .collect::<String>()
                                        .chars()
                                        .rev()
                                        .collect();
                                    self.logger.warning(&format!(
                                        "[SNIPER] balance snapshot below min but positions API still shows shares for asset={aid_tail} (pos={live_int}); deferring local-state reconcile."
                                    ));
                                    self._runtime_ts_set(
                                        "__taker_fail_pause_until",
                                        now_ts_f64() + 2.0,
                                    );
                                    return false;
                                }
                            }
                        }
                        let aid_tail: String = asset_id
                            .chars()
                            .rev()
                            .take(6)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect();
                        if env_bool("SNIPER_EXIT_CLEAR_LOCAL_ON_REJECT_ZERO_BALANCE", false) {
                            self.logger.warning(&format!(
                                "[SNIPER] exit rejected (bal<min). Clearing local position state for asset={aid_tail} due to SNIPER_EXIT_CLEAR_LOCAL_ON_REJECT_ZERO_BALANCE=true."
                            ));
                            self._clear_local_position_for_asset(
                                &asset_id,
                                "exit rejected but clob balance below min_shares",
                            );
                        } else {
                            self.logger.warning(&format!(
                                "[SNIPER] exit rejected (bal<min) for asset={aid_tail}; keeping local state and retrying (no immediate market stop)."
                            ));
                        }
                        self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 2.0);
                        return false;
                    }
                    let allow_below_min = (exit_allow_fractional
                        && allow_sh_fresh + 1e-12 < min_exit_size)
                        || (!exit_allow_fractional && allow_int_fresh < min_int);
                    if allow_below_min {
                        if sold_any || recent_submit {
                            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 5.0);
                            return false;
                        }
                        let aid_tail: String = asset_id
                            .chars()
                            .rev()
                            .take(6)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect();
                        self.logger.warning(
                            "[SNIPER] exit failed: allowance too low. Approve conditional tokens for selling, then the bot will retry.",
                        );
                        self.logger.info(&format!(
                            "[SNIPER] allowance snapshot asset={aid_tail} bal={bal_sh:.6} allow={allow_sh_fresh:.6} min_required={min_exit_size:.6}"
                        ));
                        self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 60.0);
                        return false;
                    }
                    if exit_allow_fractional {
                        let mut sellable_sh = bal_sh.min(allow_sh_fresh);
                        sellable_sh = q_down(sellable_sh, exit_size_dp);
                        if sellable_sh + 1e-12 >= min_exit_size && sellable_sh + 1e-9 < remaining {
                            balance_avail_sh = sellable_sh;
                            allow_sh = sellable_sh;
                            balance_avail_int = (sellable_sh + 1e-12).floor() as i64;
                            allow_int = balance_avail_int;
                            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 1.0);
                            continue;
                        }
                    } else {
                        let mut sellable_int = bal_int.min(allow_int_fresh);
                        sellable_int = (sellable_int / min_int) * min_int;
                        if sellable_int >= min_int && sellable_int < remaining_int {
                            balance_avail_int = sellable_int;
                            allow_int = sellable_int;
                            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 1.0);
                            continue;
                        }
                    }
                }
                self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 5.0);
                return false;
            }
            submitted_any = true;
            last_submit_ts = now_ts_f64();
            thread::sleep(Duration::from_secs_f64(1.0));
            let cur2 = self._sniper_position();
            if cur2.is_none() {
                self._mark_sniper_exit_state();
                return true;
            }
            let rem2 = cur2
                .as_ref()
                .and_then(|v| v.get("qty"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if rem2 <= remaining - 1e-9 {
                sold_any = true;
                continue;
            }
            if env_bool("SNIPER_CANCEL_EXIT_ORDERS_BEFORE_RETRY", true) {
                self._cancel_exchange_orders_for_assets(
                    std::slice::from_ref(&asset_id),
                    &format!("sniper exit {reason_u} no-fill cancel"),
                );
            }
            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + 1.0);
            break;
        }
        if sold_any {
            return false;
        }
        false
    }

    pub fn _signal_direction_to_side(&self, direction: &str) -> Option<String> {
        match direction.trim().to_ascii_uppercase().as_str() {
            "YES" | "UP" | "LONG" | "BUY" | "BULL" => Some("YES".to_string()),
            "NO" | "DOWN" | "SHORT" | "SELL" | "BEAR" => Some("NO".to_string()),
            _ => None,
        }
    }

    pub fn _signal_seen(&self, key: &str) -> bool {
        let key = key.trim();
        if key.is_empty() {
            return true;
        }
        self.state
            .lock()
            .map(|s| s.seen_signal_keys.iter().any(|k| k == key))
            .unwrap_or(true)
    }

    pub fn _signal_mark_seen(&self, sig: &Value) {
        let key = sig
            .get("key")
            .or_else(|| sig.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if key.is_empty() {
            return;
        }
        if let Ok(mut s) = self.state.lock() {
            if !s.seen_signal_keys.iter().any(|k| k == &key) {
                s.seen_signal_keys.push(key);
                let _ = save_state(&self.state_file, &mut s);
            }
        }
    }

    pub fn _ensure_signal_hub(&self) {
        if !matches!(
            self.exec_mode.as_str(),
            "SIGNAL_SNIPPER" | "SIGNAL_SNIPER" | "SIGNAL_SNIPE" | "SIGNAL"
        ) {
            return;
        }
        if self.signal_hub.is_some() {
            return;
        }
        let prov = std::env::var("SIGNAL_PROVIDER")
            .unwrap_or_else(|_| "WEBSOCKET".to_string())
            .to_ascii_uppercase();
        if prov == "WEBSOCKET" {
            self.logger
                .warning("signal hub not available for SIGNAL_SNIPPER mode");
        }
    }

    pub fn _signal_entry_candidate_from_signal(
        &self,
        sig: &Value,
        seconds_left: f64,
    ) -> Option<Value> {
        let direction = sig.get("direction").and_then(|v| v.as_str()).unwrap_or("");
        let side = self._signal_direction_to_side(direction)?;

        let (y_bid, y_ask, n_bid, n_ask) = self._sniper_best_snapshot();
        if y_ask <= 0.0 || n_ask <= 0.0 || y_bid <= 0.0 || n_bid <= 0.0 {
            return None;
        }
        let y_mid = 0.5 * (y_bid + y_ask);
        let n_mid = 0.5 * (n_bid + n_ask);
        let parity = (y_mid + n_mid - 1.0).abs();
        let sniper_parity_tolerance =
            env_float("SNIPER_PARITY_TOLERANCE", self.parity_tolerance.max(0.0));
        if parity > sniper_parity_tolerance {
            if env_bool("SIGNAL_DEBUG", false) {
                self.logger.info(&format!(
                    "[SIGNAL] drop: parity {parity:.4} > tol {sniper_parity_tolerance:.4}"
                ));
            }
            return None;
        }

        let (asset_id, bid, ask) = if side == "YES" {
            (self.yes_asset.clone().unwrap_or_default(), y_bid, y_ask)
        } else {
            (self.no_asset.clone().unwrap_or_default(), n_bid, n_ask)
        };

        let tick = self.cfg.tick.max(0.0001);
        let entry_type_name = std::env::var("SNIPER_ENTRY_ORDER_TYPE")
            .unwrap_or_else(|_| "FOK".to_string())
            .to_ascii_uppercase();
        let limit_entry = matches!(entry_type_name.as_str(), "GTC" | "LIMIT");
        let spread_ticks = ((ask - bid) / tick).round() as i64;
        let max_spread = env_int("SNIPER_MAX_SPREAD_TICKS", self.max_spread_ticks);
        if spread_ticks > max_spread {
            if env_bool("SIGNAL_DEBUG", false) {
                self.logger.info(&format!(
                    "[SIGNAL] drop: spread_ticks {spread_ticks} > max {max_spread}"
                ));
            }
            return None;
        }

        let entry_px = self._sniper_est_entry_price(ask);
        if entry_px <= 0.0 {
            return None;
        }
        if !limit_entry && entry_px + 1e-12 < ask {
            return None;
        }
        let price_min = env_float("SNIPER_PRICE_MIN", 0.91);
        let price_max = env_float("SNIPER_PRICE_MAX", 0.99);
        let eps = env_float("SNIPER_PRICE_MAX_EPSILON", 0.0);
        let hard_max = env_float("SNIPER_HARD_MAX_PRICE", price_max);
        if !limit_entry && ask > hard_max + 1e-9 {
            return None;
        }
        if entry_px < (price_min - eps) || entry_px > (price_max + eps) {
            return None;
        }

        let max_drift_ticks = env_float("SIGNAL_PRICE_DRIFT_MAX_TICKS", 0.0);
        let ref_px = sig
            .get("entry_price")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0);
        if max_drift_ticks > 0.0 && ref_px > 0.0 {
            let drift = (entry_px - ref_px).abs();
            if drift > (max_drift_ticks * tick + 1e-9) {
                if env_bool("SIGNAL_DEBUG", false) {
                    self.logger.info(&format!(
                        "[SIGNAL] drop: drift {drift:.4} > {max_drift_ticks} ticks (tick={tick:.4}) | live={entry_px:.4} ref={ref_px:.4}"
                    ));
                }
                return None;
            }
        }

        Some(json!({
            "side": side,
            "asset_id": asset_id,
            "bid": bid,
            "ask": ask,
            "entry_px": entry_px,
            "spread_ticks": spread_ticks,
            "parity": parity,
            "seconds_left": seconds_left,
        }))
    }

    pub fn _log_status_signal(&self, seconds_left: f64) {
        let pos = self._sniper_position();
        let pnl = self._sniper_mark_to_market_pnl();
        let tc = self
            .state
            .lock()
            .map(|s| s.sniper_trade_count)
            .unwrap_or_default();
        if pos.is_none() {
            self.logger.info(&format!(
                "[SIGNAL] t_left={seconds_left:6.1}s trades={tc} pnl(mtm)={pnl:+.4} (flat)"
            ));
            return;
        }
        let pos = pos.unwrap_or_default();
        let cost = pos.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let qty = pos.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let bid = pos.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let avg = pos.get("avg").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let exit_px = self._sniper_est_exit_price(bid, 0.0);
        let pnl_est = qty * exit_px - cost;
        let pnl_pct = if cost > 1e-12 { pnl_est / cost } else { 0.0 };
        self.logger.info(&format!(
            "[SIGNAL] t_left={seconds_left:6.1}s trades={tc} side={} qty={qty:.2} avg={avg:.3} bid={bid:.3} ex={exit_px:.3} pnl={pnl_est:+.4} ({:+.2}%)",
            pos.get("side").and_then(|v| v.as_str()).unwrap_or(""),
            pnl_pct * 100.0
        ));
    }

    pub fn _run_signal_sniper_loop(&self) -> String {
        self._ensure_signal_hub();
        let hub = self.signal_hub.clone();
        if hub.is_none() {
            self._set_exit_reason("SIGNAL_NO_HUB");
            self.stop();
            return self._get_exit_reason();
        }
        let hub = hub.unwrap();
        self.logger.info(&format!(
            "SIGNAL_SNIPPER enabled | provider={} price=[{:.2},{:.2}] hard_max={:.2} TP={:.1}% SL={:.1}% max_trades={} max_notional={:.2} conf_min={:.2} follow_slug={} require_match={} ws_connected={}",
            std::env::var("SIGNAL_PROVIDER").unwrap_or_else(|_| "".to_string()),
            env_float("SNIPER_PRICE_MIN", 0.91),
            env_float("SNIPER_PRICE_MAX", 0.99),
            env_float("SNIPER_HARD_MAX_PRICE", env_float("SNIPER_PRICE_MAX", 0.99)),
            env_float("SNIPER_TAKE_PROFIT_PCT", 0.01) * 100.0,
            env_float("SNIPER_STOP_LOSS_PCT", 0.02) * 100.0,
            env_int("SNIPER_MAX_TRADES_PER_MARKET", 1),
            env_float("SNIPER_MAX_NOTIONAL_USD", self.cfg.max_total_cost),
            env_float("SIGNAL_CONFIDENCE_MIN", 0.0),
            env_bool("SIGNAL_FOLLOW_SLUG", false),
            env_bool("SIGNAL_REQUIRE_SLUG_MATCH", true),
            hub.is_connected(),
        ));

        let mut last_log = 0.0;
        let mut sniper_in_pos = false;
        let mut sniper_pos_open_ts = 0.0;
        let mut sniper_stop_breach_since: Option<f64> = None;

        while !self.stop_flag.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_secs_f64(
                self.loop_wait_seconds_sniper.max(0.01).min(0.5),
            ));
            let now = now_ts_f64();
            let seconds_left = self.expiry_ts as f64 - now;
            if seconds_left <= 0.0 {
                self._set_exit_reason("SIGNAL_MARKET_EXPIRED");
                break;
            }
            if now - last_log >= (self.cfg.log_every as f64).max(0.5) {
                self._log_status_signal(seconds_left);
                last_log = now;
            }
            if !self._market_data_fresh() {
                continue;
            }
            if now < self._runtime_ts_get("__taker_fail_pause_until") {
                continue;
            }

            let pos = self._sniper_position();
            if pos.is_none() {
                self._runtime_ts_set("__rtds_hold_active", 0.0);
                self._runtime_ts_set("__rtds_hold_side_yes", 0.0);
                if sniper_in_pos {
                    sniper_in_pos = false;
                    sniper_pos_open_ts = 0.0;
                    sniper_stop_breach_since = None;
                    self._runtime_ts_set("__sniper_entry_ref_price", 0.0);
                }
            } else if !sniper_in_pos {
                sniper_in_pos = true;
                sniper_pos_open_ts = now;
                sniper_stop_breach_since = None;
                let avg = pos
                    .as_ref()
                    .and_then(|p| p.get("avg"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                self._runtime_ts_set("__sniper_entry_ref_price", avg.max(0.0));
            }

            if pos.is_none() {
                let trade_count = self
                    .state
                    .lock()
                    .map(|s| s.sniper_trade_count)
                    .unwrap_or_default();
                if trade_count >= env_int("SNIPER_MAX_TRADES_PER_MARKET", 1) {
                    if self._sniper_has_resting_entry_order() {
                        continue;
                    }
                    self._set_exit_reason("SIGNAL_MAX_TRADES_REACHED");
                    break;
                }
                if !env_bool("SIGNAL_IGNORE_TIME_WINDOW", true) {
                    if seconds_left > env_float("SNIPER_ENTRY_MAX_SECONDS", f64::INFINITY)
                        || seconds_left < env_float("SNIPER_ENTRY_MIN_SECONDS", 30.0)
                    {
                        continue;
                    }
                }

                let follow_slug = env_bool("SIGNAL_FOLLOW_SLUG", false);
                let mut sig = if follow_slug {
                    hub.inbox.peek(Some(0.2))
                } else {
                    hub.inbox.get_for_slug(&self.market_slug, Some(0.2))
                };
                if sig.is_none() {
                    continue;
                }
                if follow_slug {
                    if let Some(peek_sig) = sig.clone() {
                        if peek_sig.market_slug != self.market_slug {
                            self._set_exit_reason(&format!("SWITCH:{}", peek_sig.market_slug));
                            self.cancel_all_orders_exchange("signal switch");
                            break;
                        }
                    }
                    let consumed = hub.inbox.get(Some(0.0));
                    if consumed.is_some() {
                        sig = consumed;
                    }
                }
                let sig = sig.unwrap();
                if self._signal_seen(&sig.key) {
                    if env_bool("SIGNAL_DEBUG", false) {
                        self.logger
                            .info(&format!("[SIGNAL] skip: already seen key={}", sig.key));
                    }
                    continue;
                }
                let conf = sig.confidence;
                if conf + 1e-12 < env_float("SIGNAL_CONFIDENCE_MIN", 0.0) {
                    if env_bool("SIGNAL_DEBUG", false) {
                        self.logger.info(&format!(
                            "[SIGNAL] drop: conf {conf:.3} < min {:.3}",
                            env_float("SIGNAL_CONFIDENCE_MIN", 0.0)
                        ));
                    }
                    self._signal_mark_seen(&sig.to_dict());
                    continue;
                }

                let sig_v = sig.to_dict();
                let cand = self._signal_entry_candidate_from_signal(&sig_v, seconds_left);
                if cand.is_none() {
                    if env_bool("SIGNAL_USE_ONCE", true) {
                        self._signal_mark_seen(&sig_v);
                    }
                    continue;
                }
                let cand = cand.unwrap_or_default();
                self._set_active_signal_context(&sig_v, "SIGNAL_ENTRY");
                let ok = self._sniper_try_enter(&cand);
                self._clear_active_signal_context();
                if env_bool("SIGNAL_USE_ONCE", true) {
                    self._signal_mark_seen(&sig_v);
                }
                if !ok {
                    if env_bool("SIGNAL_DEBUG", false) {
                        self.logger.info(&format!(
                            "[SIGNAL] entry failed key={} side={} px={}",
                            sig.key,
                            cand.get("side").and_then(|v| v.as_str()).unwrap_or(""),
                            cand.get("entry_px").and_then(|v| v.as_f64()).unwrap_or(0.0)
                        ));
                    }
                    continue;
                }
                let entry_ms = self._lat_ms(now_ts_f64(), sig.received_ts);
                if let Some(entry_ms) = entry_ms {
                    self.logger.info(&format!(
                        "[SIGNAL] ENTERED key={} side={} ask={:.4} px={:.4} conf={conf:.3} latency_ms={entry_ms}",
                        sig.key,
                        cand.get("side").and_then(|v| v.as_str()).unwrap_or(""),
                        cand.get("ask").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        cand.get("entry_px").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    ));
                } else {
                    self.logger.info(&format!(
                        "[SIGNAL] ENTERED key={} side={} ask={:.4} px={:.4} conf={conf:.3}",
                        sig.key,
                        cand.get("side").and_then(|v| v.as_str()).unwrap_or(""),
                        cand.get("ask").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        cand.get("entry_px").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    ));
                }
                continue;
            }

            let pos = pos.unwrap_or_default();
            let cost = pos.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let qty = pos.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let bid = pos.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let exit_px = self._sniper_est_exit_price(bid, 0.0);
            let pnl = qty * exit_px - cost;
            let pnl_pct = if cost > 1e-12 { pnl / cost } else { 0.0 };
            let pos_side = pos
                .get("side")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_uppercase();
            if self._rtds_hold_till_resolution_active(&pos_side, seconds_left, "SIGNAL_HOLD") {
                sniper_stop_breach_since = None;
                continue;
            }

            if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true)
                && seconds_left <= env_float("SNIPER_FORCE_EXIT_SECONDS", 8.0)
            {
                if self._sniper_try_exit(&pos, "FORCE_EXIT") {
                    break;
                }
                continue;
            }
            if cost > 1e-12 && pnl_pct >= env_float("SNIPER_TAKE_PROFIT_PCT", 0.01) {
                if self._sniper_try_exit(&pos, "TAKE_PROFIT") {
                    break;
                }
                continue;
            }
            let stop_pct = env_float("SNIPER_STOP_LOSS_PCT", 0.02).max(0.0);
            if cost > 1e-12 && stop_pct > 0.0 {
                if pnl_pct <= -stop_pct {
                    let held_s = now - sniper_pos_open_ts.max(0.0);
                    if held_s >= env_float("SNIPER_MIN_HOLD_SECONDS", 0.0).max(0.0) {
                        if sniper_stop_breach_since.is_none() {
                            sniper_stop_breach_since = Some(now);
                        }
                        let confirm_s = env_float("SNIPER_STOP_CONFIRM_SECONDS", 0.0).max(0.0);
                        if now - sniper_stop_breach_since.unwrap_or(now) >= confirm_s {
                            if self._sniper_try_exit(&pos, "STOP_LOSS") {
                                break;
                            }
                            continue;
                        }
                    }
                } else {
                    sniper_stop_breach_since = None;
                }
            }
            if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true)
                && seconds_left <= self.cfg.stop_buffer_seconds as f64
            {
                if self._sniper_try_exit(&pos, "STOP_BUFFER_EXIT") {
                    break;
                }
            }
        }
        self.stop();
        self._get_exit_reason()
    }

    pub fn _run_sniper_loop(&self) -> String {
        self.logger.info(&format!(
            "SNIPER mode enabled | price=[{:.2},{:.2}] TP={:.1}% SL={:.1}% entry_window=[{}s..{}s] force_exit={}s exit_before_expiry={} force_entry_min={:.2} force_entry_max_age={}s entry_confirm={:.2}s",
            env_float("SNIPER_PRICE_MIN", 0.91),
            env_float("SNIPER_PRICE_MAX", 0.99),
            env_float("SNIPER_TAKE_PROFIT_PCT", 0.01) * 100.0,
            env_float("SNIPER_STOP_LOSS_PCT", 0.02) * 100.0,
            env_float("SNIPER_ENTRY_MIN_SECONDS", 30.0),
            env_float("SNIPER_ENTRY_MAX_SECONDS", 240.0),
            env_float("SNIPER_FORCE_EXIT_SECONDS", 8.0),
            env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true),
            env_float("SNIPER_FORCE_ENTRY_MIN_PRICE", 0.0),
            env_int("SNIPER_FORCE_ENTRY_MAX_AGE_SECONDS", 0),
            env_float("SNIPER_ENTRY_CONFIRM_SECONDS", 0.0),
        ));
        let repeat_mode = env_bool("SNIPER_REPEAT_MODE", false);
        let repeat_cooldown_s = env_float("SNIPER_REPEAT_COOLDOWN_SECONDS", 0.0).max(0.0);
        let repeat_stop_after_sl = env_bool("SNIPER_REPEAT_STOP_AFTER_STOP_LOSS", true);
        if repeat_mode {
            self.logger.info(&format!(
                "SNIPER repeat enabled | max_trades={} cooldown={repeat_cooldown_s:.2}s stop_after_stop_loss={repeat_stop_after_sl}",
                env_int("SNIPER_MAX_TRADES_PER_MARKET", 1)
            ));
        }
        let mut last_log = 0.0;
        let mut sniper_in_pos = false;
        let mut sniper_pos_open_ts = 0.0;
        let mut sniper_stop_breach_since: Option<f64> = None;

        while !self.stop_flag.load(Ordering::SeqCst) {
            let wait_s = self.loop_wait_seconds_sniper.max(0.01).min(0.5);
            thread::sleep(Duration::from_secs_f64(wait_s));
            let now = now_ts_f64();
            let seconds_left = self.expiry_ts as f64 - now;
            let grace = env_float("SNIPER_EXPIRY_GRACE_SECONDS", 0.0).max(0.0);
            if seconds_left <= -grace {
                self._set_exit_reason("SNIPER_MARKET_EXPIRED");
                break;
            }
            if now - last_log >= (self.cfg.log_every as f64).max(0.5) {
                self._log_status_sniper(seconds_left);
                last_log = now;
            }
            if self._sniper_maybe_endgame_blind_post(seconds_left, now) {
                continue;
            }
            if !self._market_data_fresh() {
                continue;
            }
            if now < self._runtime_ts_get("__taker_fail_pause_until") {
                continue;
            }

            let pos = self._sniper_position();
            if pos.is_none() {
                self._runtime_ts_set("__rtds_hold_active", 0.0);
                self._runtime_ts_set("__rtds_hold_side_yes", 0.0);
                if sniper_in_pos {
                    sniper_in_pos = false;
                    sniper_pos_open_ts = 0.0;
                    sniper_stop_breach_since = None;
                    self._runtime_ts_set("__sniper_entry_ref_price", 0.0);
                    self._runtime_ts_set("__sniper_entry_gate_since", 0.0);
                }
            } else if !sniper_in_pos {
                sniper_in_pos = true;
                sniper_pos_open_ts = now;
                sniper_stop_breach_since = None;
                let avg = pos
                    .as_ref()
                    .and_then(|p| p.get("avg"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                self._runtime_ts_set("__sniper_entry_ref_price", avg.max(0.0));
                self._runtime_ts_set("__sniper_entry_gate_since", 0.0);
            } else if self._runtime_ts_get("__sniper_entry_ref_price") <= 0.0 {
                let avg = pos
                    .as_ref()
                    .and_then(|p| p.get("avg"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                self._runtime_ts_set("__sniper_entry_ref_price", avg.max(0.0));
            }

            if pos.is_none() {
                let trade_count = self
                    .state
                    .lock()
                    .map(|s| s.sniper_trade_count)
                    .unwrap_or_default();
                if trade_count >= env_int("SNIPER_MAX_TRADES_PER_MARKET", 1) {
                    if self._sniper_has_resting_entry_order() {
                        continue;
                    }
                    self._set_exit_reason("SNIPER_MAX_TRADES_REACHED");
                    break;
                }
                if repeat_mode {
                    if seconds_left <= self.cfg.stop_buffer_seconds as f64 {
                        self._set_exit_reason("SNIPER_STOP_BUFFER");
                        break;
                    }
                    if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true)
                        && seconds_left <= env_float("SNIPER_FORCE_EXIT_SECONDS", 8.0) + 1.0
                    {
                        self._set_exit_reason("SNIPER_FORCE_EXIT_WINDOW");
                        break;
                    }
                    if seconds_left < env_float("SNIPER_ENTRY_MIN_SECONDS", 30.0) {
                        self._set_exit_reason("SNIPER_ENTRY_WINDOW_CLOSED");
                        break;
                    }
                    if repeat_cooldown_s > 0.0 {
                        let last_exit_ts = self
                            .state
                            .lock()
                            .map(|s| s.sniper_last_exit_ts)
                            .unwrap_or(0.0);
                        if last_exit_ts > 0.0 && (now - last_exit_ts) < repeat_cooldown_s {
                            continue;
                        }
                    }
                }

                if seconds_left > env_float("SNIPER_ENTRY_MAX_SECONDS", 240.0) {
                    let force_min = env_float("SNIPER_FORCE_ENTRY_MIN_PRICE", 0.0);
                    if force_min > 0.0 {
                        let age_s = now - self.start_ts as f64;
                        let max_age = env_float("SNIPER_FORCE_ENTRY_MAX_AGE_SECONDS", 0.0);
                        if max_age <= 0.0 || age_s <= max_age {
                            let cand = self._sniper_entry_candidate(
                                seconds_left,
                                env_bool("SNIPER_FORCE_ENTRY_IGNORE_ROI_GATE", false),
                            );
                            let ask_ok = cand
                                .as_ref()
                                .and_then(|c| c.get("ask"))
                                .and_then(|v| v.as_f64())
                                .map(|ask| ask + 1e-12 >= force_min)
                                .unwrap_or(false);
                            if !ask_ok {
                                if env_float("SNIPER_ENTRY_CONFIRM_SECONDS", 0.0) > 0.0 {
                                    self._runtime_ts_set("__sniper_entry_gate_since", 0.0);
                                }
                            } else if let Some(mut cand) = cand {
                                if let Value::Object(ref mut o) = cand {
                                    o.insert("entry_mode".to_string(), json!("FORCE"));
                                }
                                if !self._sniper_entry_confirmed(&cand, now) {
                                    continue;
                                }
                                if self._sniper_has_resting_entry_order() {
                                    continue;
                                }
                                self.logger.info(&format!(
                                    "[SNIPER] FORCE-ENTRY triggered side={} ask={:.3} entry_px={:.3}>=min={force_min:.3} age={age_s:.1}s t_left={seconds_left:.1}s spread_ticks={} parity={:.4}",
                                    cand.get("side").and_then(|v| v.as_str()).unwrap_or(""),
                                    cand.get("ask").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                    cand.get("entry_px").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                    cand.get("spread_ticks").and_then(|v| v.as_i64()).unwrap_or(0),
                                    cand.get("parity").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                ));
                                let _ = self._sniper_try_enter(&cand);
                            }
                        }
                    }
                    continue;
                }

                if seconds_left < env_float("SNIPER_ENTRY_MIN_SECONDS", 30.0) {
                    if seconds_left <= self.cfg.stop_buffer_seconds as f64 {
                        self._set_exit_reason("SNIPER_TOO_LATE_TO_ENTER");
                        break;
                    }
                    continue;
                }
                if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true)
                    && seconds_left <= env_float("SNIPER_FORCE_EXIT_SECONDS", 8.0) + 1.0
                {
                    continue;
                }

                let cand = self._sniper_entry_candidate(
                    seconds_left,
                    env_bool("SNIPER_ENTRY_IGNORE_ROI_GATE", false),
                );
                if cand.is_none() {
                    self._runtime_ts_set("__sniper_entry_gate_since", 0.0);
                } else {
                    let cand = cand.unwrap_or_default();
                    if !self._sniper_entry_confirmed(&cand, now) {
                        continue;
                    }
                    let _ = self._sniper_try_enter(&cand);
                }
                continue;
            }

            let pos = pos.unwrap_or_default();
            let cost = pos.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let qty = pos.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let bid = pos.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let exit_px = self._sniper_est_exit_price(bid, 0.0);
            let pnl = qty * exit_px - cost;
            let pnl_pct = if cost > 1e-12 { pnl / cost } else { 0.0 };
            let pos_side = pos
                .get("side")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_uppercase();
            if self._rtds_hold_till_resolution_active(&pos_side, seconds_left, "SNIPER_HOLD") {
                sniper_stop_breach_since = None;
                continue;
            }

            if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true)
                && seconds_left <= env_float("SNIPER_FORCE_EXIT_SECONDS", 8.0)
            {
                if self._sniper_try_exit(&pos, "FORCE_EXIT") {
                    break;
                }
                continue;
            }
            if cost > 1e-12 && pnl_pct >= env_float("SNIPER_TAKE_PROFIT_PCT", 0.01) {
                if self._sniper_try_exit(&pos, "TAKE_PROFIT") {
                    if repeat_mode {
                        continue;
                    }
                    break;
                }
                continue;
            }
            let stop_pct = env_float("SNIPER_STOP_LOSS_PCT", 0.02).max(0.0);
            if cost > 1e-12 && stop_pct > 0.0 {
                if pnl_pct <= -stop_pct {
                    let held_s = now - sniper_pos_open_ts.max(0.0);
                    if held_s >= env_float("SNIPER_MIN_HOLD_SECONDS", 0.0).max(0.0) {
                        if sniper_stop_breach_since.is_none() {
                            sniper_stop_breach_since = Some(now);
                        }
                        if now - sniper_stop_breach_since.unwrap_or(now)
                            >= env_float("SNIPER_STOP_CONFIRM_SECONDS", 0.0).max(0.0)
                        {
                            if self._sniper_try_exit(&pos, "STOP_LOSS") {
                                if repeat_mode && !repeat_stop_after_sl {
                                    continue;
                                }
                                break;
                            }
                            continue;
                        }
                    }
                } else {
                    sniper_stop_breach_since = None;
                }
            }
            if env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true)
                && seconds_left <= self.cfg.stop_buffer_seconds as f64
            {
                if self._sniper_try_exit(&pos, "STOP_BUFFER_EXIT") {
                    break;
                }
            }
        }
        self.stop();
        self._get_exit_reason()
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub fn trade_metrics_snapshot(&self) -> TradeMetrics {
        let state = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        let sniper_mode = matches!(
            self.exec_mode.as_str(),
            "SNIPER"
                | "PROB_SNIPER"
                | "HIGH_PROB"
                | "HIGH_PROB_SNIPER"
                | "FIXED_PROFIT"
                | "SIGNAL_SNIPPER"
                | "SIGNAL_SNIPER"
                | "SIGNAL_SNIPE"
                | "SIGNAL"
        );
        let lp = if sniper_mode {
            let (y_bid, _, n_bid, _) = self._sniper_best_snapshot();
            (state.q_yes * y_bid - state.c_yes) + (state.q_no * n_bid - state.c_no)
        } else {
            locked_profit(&state)
        };
        TradeMetrics {
            lp,
            total_cost: state.c_yes + state.c_no,
            q_yes: state.q_yes,
            q_no: state.q_no,
            cpp: cost_per_pair(&state),
            exit_reason: self._get_exit_reason(),
        }
    }

    pub fn persist_state(&self) {
        if let Ok(mut state) = self.state.lock() {
            let _ = save_state(&self.state_file, &mut state);
        }
    }

    pub fn cancel_all_orders_exchange(&self, reason: &str) {
        if !reason.trim().is_empty() {
            self.logger
                .info(&format!("Cancel-all (exchange): {reason}"));
        }
        let orders = self._list_open_orders_exchange();
        for o in orders {
            if let Some(oid) = self._extract_order_id(&o) {
                let _ = self._cancel(&oid);
            }
        }
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.clear();
            let _ = save_state(&self.state_file, &mut s);
        }
    }
}
