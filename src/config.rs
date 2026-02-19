use crate::helpers::{segment, segment_defaults};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub clob_host: String,
    pub ws_base: String,
    pub chain_id: i64,
    pub private_key: String,
    pub signature_type: Option<i64>,
    pub funder: Option<String>,
    pub market_segment: String,
    pub market_duration_seconds: i64,
    pub market_step_seconds: i64,
    pub tick: f64,
    pub min_shares: f64,
    pub lock_profit_target: f64,
    pub clip_shares: f64,
    pub improve_bid_ticks: i64,
    pub maker_buffer_ticks: i64,
    pub replace_if_price_moves_ticks: i64,
    pub stale_seconds: i64,
    pub entry_edge_ticks: i64,
    pub hedge_buffer_ticks: i64,
    pub max_total_cost: f64,
    pub reserve_usd: f64,
    pub cancel_all_on_start: bool,
    pub dry_run: bool,
    pub log_every: i64,
    pub market_data_stale_seconds: i64,
    pub ws_reconnect_min: f64,
    pub ws_reconnect_max: f64,
    pub stop_buffer_seconds: i64,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            clob_host: "https://clob.polymarket.com".to_string(),
            ws_base: "wss://ws-subscriptions-clob.polymarket.com".to_string(),
            chain_id: 137,
            private_key: String::new(),
            signature_type: None,
            funder: None,
            market_segment: "15M".to_string(),
            market_duration_seconds: 15 * 60,
            market_step_seconds: 15 * 60,
            tick: 0.01,
            min_shares: 5.0,
            lock_profit_target: 0.5,
            clip_shares: 5.0,
            improve_bid_ticks: 0,
            maker_buffer_ticks: 1,
            replace_if_price_moves_ticks: 3,
            stale_seconds: 20,
            entry_edge_ticks: 2,
            hedge_buffer_ticks: 1,
            max_total_cost: 20.0,
            reserve_usd: 2.0,
            cancel_all_on_start: true,
            dry_run: false,
            log_every: 5,
            market_data_stale_seconds: 8,
            ws_reconnect_min: 0.5,
            ws_reconnect_max: 5.0,
            stop_buffer_seconds: 120,
        }
    }
}

impl BotConfig {
    pub fn from_env() -> Self {
        let mut cfg = BotConfig {
            clob_host: env::var("CLOB_HOST")
                .unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
            ws_base: env::var("WS_BASE")
                .unwrap_or_else(|_| "wss://ws-subscriptions-clob.polymarket.com".to_string()),
            chain_id: env::var("CHAIN_ID")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(137),
            private_key: env::var("POLYMARKET_PRIVATE_KEY")
                .unwrap_or_default()
                .trim()
                .to_string(),
            dry_run: env::var("DRY_RUN")
                .unwrap_or_else(|_| "false".to_string())
                .to_ascii_lowercase()
                == "true",
            ..BotConfig::default()
        };

        let seg = segment(&env::var("MARKET_SEGMENT").unwrap_or_else(|_| "15M".to_string()));
        let d = segment_defaults(&seg);
        cfg.market_segment = seg;
        cfg.market_duration_seconds = env::var("MARKET_DURATION_SECONDS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(d.duration);
        cfg.market_step_seconds = env::var("MARKET_STEP_SECONDS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(d.step);
        cfg.stop_buffer_seconds = env::var("STOP_BUFFER_SECONDS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(d.stop_buffer);

        cfg
    }

    pub fn apply_safe_defaults(&mut self) {
        self.min_shares = 5.0;
        self.clip_shares = 5.0;
        self.entry_edge_ticks = 6;
        self.hedge_buffer_ticks = 2;
        self.maker_buffer_ticks = 1;
        self.improve_bid_ticks = 0;
        self.stale_seconds = 5;
        self.replace_if_price_moves_ticks = 3;
        self.max_total_cost = env::var("MAX_TOTAL_COST")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(15.0);
        self.reserve_usd = env::var("RESERVE_USD")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(2.0);
        self.market_data_stale_seconds = 8;
        self.cancel_all_on_start = true;
        self.log_every = 5;
    }
}
