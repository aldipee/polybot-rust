use crate::config::{BotConfig, ResolvedVersionedConfigBundle};
use crate::db::BotRepository;
use crate::env_utils::{env_bool, env_float, env_int};
use crate::gamma::{fetch_market_by_slug, parse_tokens_and_condition};
use crate::helpers::{
    canonical_pair_id_from_slug, clamp, cost_per_pair, iso_to_epoch, load_daily_liquidity_state,
    load_state, locked_profit, q_down, round_down, round_up, save_daily_liquidity_state,
    save_state, BotState, DailyLiquidityState, OpenOrderState,
};
use crate::latency_log::LatencyLogService;
use crate::logging::LogLike;
use alloy_signer_local::PrivateKeySigner;
use anyhow::{anyhow, Result};
use chrono::{TimeZone, Utc};
use rand::seq::SliceRandom;
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
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
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

pub fn require_bot_exec_mode() -> Result<String> {
    let exec_mode = std::env::var("EXEC_MODE")
        .unwrap_or_else(|_| "BOT".to_string())
        .trim()
        .to_ascii_uppercase();
    if exec_mode == "BOT" {
        Ok(exec_mode)
    } else {
        Err(anyhow!(
            "Unsupported EXEC_MODE={exec_mode}. Only BOT is supported."
        ))
    }
}

#[derive(Debug, Clone)]
pub struct TradeMetrics {
    pub pair_id: String,
    pub market_slug: String,
    pub condition_id: Option<String>,
    pub yes_asset_id: Option<String>,
    pub no_asset_id: Option<String>,
    pub lp: f64,
    pub total_cost: f64,
    pub q_yes: f64,
    pub q_no: f64,
    pub cpp: f64,
    pub entry_time_iso: Option<String>,
    pub entry_reason: Option<String>,
    pub stop_loss_category: Option<String>,
    pub exit_reason: String,
    pub fill_count: usize,
}

mod shared;
use self::shared::*;

mod runtime;
pub(crate) use self::runtime::*;

mod core;
pub use self::core::MakerHedgeCapBot;

mod audit;
pub(super) use self::audit::AuditWriteTask;
mod execution;
mod maker_exec;
mod maker_orders;
mod public_tail;
mod runtime_ws;
