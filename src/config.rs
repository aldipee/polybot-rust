use crate::bot::{
    bot_runtime_config_from_reader, bot_runtime_validate_config, BotRuntimeConfigSnapshot,
};
use crate::env_utils::{env_bool, env_float, env_int};
use crate::helpers::{segment, segment_defaults};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;

fn default_market_data_stale_seconds_compat() -> i64 {
    8
}

fn default_market_data_stale_add_block_seconds() -> i64 {
    2
}

fn default_market_data_stale_hard_pause_seconds() -> i64 {
    5
}

fn default_maker_replace_min_interval_seconds() -> f64 {
    1.0
}

fn default_pair_gross_deployed_cost_cap_usd() -> f64 {
    20.0
}

fn default_portfolio_gross_deployed_cost_cap_usd() -> f64 {
    default_pair_gross_deployed_cost_cap_usd() * 4.0
}

fn default_gross_deployed_cost_buffer_usd() -> f64 {
    0.0
}

fn default_gross_cap_include_pending() -> bool {
    true
}

fn default_gross_cap_shared_state_ttl_seconds() -> f64 {
    30.0
}

fn default_bot_order_mode() -> String {
    "shadow".to_string()
}

fn default_bot_live_enabled() -> bool {
    false
}

fn parse_env_bool_like(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn order_mode_from_legacy_dry_run(dry_run: bool) -> String {
    if dry_run {
        "paper".to_string()
    } else {
        default_bot_order_mode()
    }
}

fn parse_bot_order_mode(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "shadow" => Some("shadow".to_string()),
        "paper" => Some("paper".to_string()),
        "live" => Some("live".to_string()),
        _ => None,
    }
}

fn resolve_bot_order_mode_from_env() -> Result<String> {
    let explicit_raw = env::var("BOT_ORDER_MODE").unwrap_or_default();
    let explicit_mode = if explicit_raw.trim().is_empty() {
        None
    } else {
        Some(
            parse_bot_order_mode(&explicit_raw)
                .ok_or_else(|| anyhow!("Invalid BOT_ORDER_MODE={}", explicit_raw.trim()))?,
        )
    };
    let dry_run_raw = env::var("DRY_RUN").ok();
    if let Some(explicit_mode) = explicit_mode {
        if let Some(raw) = dry_run_raw.as_deref() {
            let dry_run = parse_env_bool_like(raw).unwrap_or(false);
            let compat_mode = order_mode_from_legacy_dry_run(dry_run);
            if explicit_mode != compat_mode {
                return Err(anyhow!(
                    "Inconsistent DRY_RUN and BOT_ORDER_MODE; remove DRY_RUN or set BOT_ORDER_MODE to {}",
                    compat_mode
                ));
            }
        }
        return Ok(explicit_mode);
    }
    Ok(order_mode_from_legacy_dry_run(
        dry_run_raw
            .as_deref()
            .and_then(parse_env_bool_like)
            .unwrap_or(false),
    ))
}

fn execution_order_mode_requires_wallet_observation(order_mode: &str) -> bool {
    !order_mode.eq_ignore_ascii_case("paper")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    #[serde(default = "default_maker_replace_min_interval_seconds")]
    pub maker_replace_min_interval_seconds: f64,
    pub entry_edge_ticks: i64,
    pub hedge_buffer_ticks: i64,
    pub max_total_cost: f64,
    #[serde(default = "default_pair_gross_deployed_cost_cap_usd")]
    pub pair_gross_deployed_cost_cap_usd: f64,
    #[serde(default = "default_portfolio_gross_deployed_cost_cap_usd")]
    pub portfolio_gross_deployed_cost_cap_usd: f64,
    #[serde(default = "default_gross_deployed_cost_buffer_usd")]
    pub pair_gross_deployed_cost_buffer_usd: f64,
    #[serde(default = "default_gross_deployed_cost_buffer_usd")]
    pub portfolio_gross_deployed_cost_buffer_usd: f64,
    #[serde(default = "default_gross_cap_include_pending")]
    pub gross_cap_include_pending_maker: bool,
    #[serde(default = "default_gross_cap_include_pending")]
    pub gross_cap_include_pending_taker: bool,
    #[serde(default = "default_gross_cap_shared_state_ttl_seconds")]
    pub gross_cap_shared_state_ttl_seconds: f64,
    pub reserve_usd: f64,
    pub cancel_all_on_start: bool,
    pub dry_run: bool,
    pub log_every: i64,
    #[serde(default = "default_market_data_stale_seconds_compat")]
    pub market_data_stale_seconds: i64,
    #[serde(default = "default_market_data_stale_add_block_seconds")]
    pub market_data_stale_add_block_seconds: i64,
    #[serde(default = "default_market_data_stale_hard_pause_seconds")]
    pub market_data_stale_hard_pause_seconds: i64,
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
            maker_replace_min_interval_seconds: 1.0,
            entry_edge_ticks: 2,
            hedge_buffer_ticks: 1,
            max_total_cost: 20.0,
            pair_gross_deployed_cost_cap_usd: 20.0,
            portfolio_gross_deployed_cost_cap_usd: 80.0,
            pair_gross_deployed_cost_buffer_usd: 0.0,
            portfolio_gross_deployed_cost_buffer_usd: 0.0,
            gross_cap_include_pending_maker: true,
            gross_cap_include_pending_taker: true,
            gross_cap_shared_state_ttl_seconds: 30.0,
            reserve_usd: 2.0,
            cancel_all_on_start: true,
            dry_run: false,
            log_every: 5,
            market_data_stale_seconds: 8,
            market_data_stale_add_block_seconds: 2,
            market_data_stale_hard_pause_seconds: 5,
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
        self.maker_replace_min_interval_seconds = 1.0;
        self.max_total_cost = env::var("MAX_TOTAL_COST")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(15.0);
        self.pair_gross_deployed_cost_cap_usd = self.max_total_cost;
        self.portfolio_gross_deployed_cost_cap_usd = self.pair_gross_deployed_cost_cap_usd * 4.0;
        self.pair_gross_deployed_cost_buffer_usd = 0.0;
        self.portfolio_gross_deployed_cost_buffer_usd = 0.0;
        self.gross_cap_include_pending_maker = true;
        self.gross_cap_include_pending_taker = true;
        self.gross_cap_shared_state_ttl_seconds = 30.0;
        self.reserve_usd = env::var("RESERVE_USD")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(2.0);
        self.market_data_stale_seconds = 8;
        self.market_data_stale_add_block_seconds = 2;
        self.market_data_stale_hard_pause_seconds = 5;
        self.cancel_all_on_start = true;
        self.log_every = 5;
    }
}

pub(crate) fn stale_data_policy_requirement_compliant(cfg: &BotConfig) -> bool {
    cfg.market_data_stale_add_block_seconds == default_market_data_stale_add_block_seconds()
        && cfg.market_data_stale_hard_pause_seconds
            == default_market_data_stale_hard_pause_seconds()
}

pub(crate) fn stale_data_policy_from_legacy_threshold(
    legacy_hard_pause_seconds: i64,
) -> (i64, i64) {
    let hard_pause_seconds = legacy_hard_pause_seconds.max(2);
    let add_block_seconds = (hard_pause_seconds - 3).max(1);
    (add_block_seconds, hard_pause_seconds)
}

fn validate_stale_data_policy(cfg: &BotConfig) -> Result<()> {
    if cfg.market_data_stale_add_block_seconds <= 0 {
        return Err(anyhow!("Invalid MARKET_DATA_STALE_ADD_BLOCK_SECONDS"));
    }
    if cfg.market_data_stale_hard_pause_seconds <= 0 {
        return Err(anyhow!("Invalid MARKET_DATA_STALE_HARD_PAUSE_SECONDS"));
    }
    if cfg.market_data_stale_hard_pause_seconds <= cfg.market_data_stale_add_block_seconds {
        return Err(anyhow!(
            "Invalid stale-data policy: MARKET_DATA_STALE_HARD_PAUSE_SECONDS must be greater than MARKET_DATA_STALE_ADD_BLOCK_SECONDS"
        ));
    }
    Ok(())
}

fn validate_gross_cap_policy(cfg: &BotConfig) -> Result<()> {
    let pair_cap = cfg.pair_gross_deployed_cost_cap_usd;
    let portfolio_cap = cfg.portfolio_gross_deployed_cost_cap_usd;
    let pair_buffer = cfg.pair_gross_deployed_cost_buffer_usd;
    let portfolio_buffer = cfg.portfolio_gross_deployed_cost_buffer_usd;
    let ttl = cfg.gross_cap_shared_state_ttl_seconds;

    if !pair_cap.is_finite() || pair_cap <= 0.0 {
        return Err(anyhow!("Invalid BOT_PAIR_GROSS_DEPLOYED_COST_CAP_USD"));
    }
    if !portfolio_cap.is_finite() || portfolio_cap <= 0.0 {
        return Err(anyhow!("Invalid BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD"));
    }
    if !pair_buffer.is_finite() || pair_buffer < 0.0 || pair_buffer >= pair_cap {
        return Err(anyhow!("Invalid BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD"));
    }
    if !portfolio_buffer.is_finite() || portfolio_buffer < 0.0 || portfolio_buffer >= portfolio_cap
    {
        return Err(anyhow!(
            "Invalid BOT_PORTFOLIO_GROSS_DEPLOYED_COST_BUFFER_USD"
        ));
    }
    if !ttl.is_finite() || ttl <= 0.5 {
        return Err(anyhow!("Invalid BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BotExecutionConfigSnapshot {
    pub wallet_address: String,
    pub min_maker_notional: f64,
    pub min_taker_notional: f64,
    pub reconcile_sell_credit_mult: f64,
    pub first_clip_shares: f64,
    pub first_hedge_full: bool,
    pub warmup_seconds: i64,
    pub max_spread_ticks: i64,
    pub parity_tolerance: f64,
    pub unhedged_timeout_seconds: f64,
    pub hedge_slippage_ticks: i64,
    pub hedge_taker_order_type: String,
    pub taker_order_ttl_seconds: i64,
    pub taker_fill_fallback_from_order_events: bool,
    pub taker_strict_inflight: bool,
    pub taker_hedge_min_interval: f64,
    pub exec_mode: String,
    pub loop_wait_seconds_maker: f64,
    pub loop_wait_seconds_taker: f64,
    pub min_entry_edge_ticks: i64,
    pub exec_latency_log_enabled: bool,
    pub exec_latency_file_log_enabled: bool,
    pub exec_latency_jsonl_enabled: bool,
    pub exec_latency_csv_enabled: bool,
    pub exec_latency_log_dir: String,
    pub exec_latency_jsonl_path: String,
    pub exec_latency_csv_path: String,
    pub clob_gamma_host: String,
    pub clob_order_meta_warmup: bool,
    #[serde(default = "default_bot_order_mode")]
    pub order_mode: String,
    #[serde(default = "default_bot_live_enabled")]
    pub live_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BotRuntimeConfigSnapshotV1 {
    pub phase_controller: String,
    pub prearm_lead_seconds: f64,
    pub open_both_seed_deadline_seconds: f64,
    pub open_both_submit_delta_max_seconds: f64,
    pub open_both_allow_single_late_seed: bool,
    pub seed_budget_min_fraction: f64,
    pub seed_budget_max_fraction: f64,
    pub early_budget_min_fraction: f64,
    pub early_budget_max_fraction: f64,
    pub main_budget_min_fraction: f64,
    pub main_budget_max_fraction: f64,
    pub late_budget_min_fraction: f64,
    pub late_budget_max_fraction: f64,
    pub taper_budget_min_fraction: f64,
    pub taper_budget_max_fraction: f64,
    pub target_both_sides_by_30s: f64,
    pub target_both_sides_by_60s: f64,
    pub late_reduce_start_seconds: f64,
    pub late_balance_only_start_seconds: f64,
    pub late_stop_new_orders_start_seconds: f64,
    pub legacy_late_window_budget_mode: bool,
    pub imbalance_target_fraction: f64,
    pub imbalance_warning_fraction: f64,
    pub imbalance_disable_fraction: f64,
    #[serde(default = "default_imbalance_recovery_fraction")]
    pub imbalance_recovery_fraction: f64,
    pub clip_ladder: [f64; 4],
    pub repair_reserve_buffer_usd: f64,
    pub buy_only_normal_flow: bool,
    pub tail_cap_mid_start_seconds: f64,
    pub tail_cap_late_start_seconds: f64,
    pub tail_cap_early_fraction: f64,
    pub tail_cap_mid_fraction: f64,
    pub tail_cap_late_fraction: f64,
    pub bad_regime_window_seconds: f64,
    pub bad_regime_expensive_fraction: f64,
    #[serde(default = "default_mean_reversion_tilt_fraction")]
    pub mean_reversion_tilt_fraction: f64,
}

fn default_mean_reversion_tilt_fraction() -> f64 {
    0.55
}

fn default_imbalance_recovery_fraction() -> f64 {
    0.12
}

impl From<&BotRuntimeConfigSnapshot> for BotRuntimeConfigSnapshotV1 {
    fn from(value: &BotRuntimeConfigSnapshot) -> Self {
        Self {
            phase_controller: value.phase_controller.to_string(),
            prearm_lead_seconds: value.prearm_lead_seconds,
            open_both_seed_deadline_seconds: value.open_both_seed_deadline_seconds,
            open_both_submit_delta_max_seconds: value.open_both_submit_delta_max_seconds,
            open_both_allow_single_late_seed: value.open_both_allow_single_late_seed,
            seed_budget_min_fraction: value.seed_budget_min_fraction,
            seed_budget_max_fraction: value.seed_budget_max_fraction,
            early_budget_min_fraction: value.early_budget_min_fraction,
            early_budget_max_fraction: value.early_budget_max_fraction,
            main_budget_min_fraction: value.main_budget_min_fraction,
            main_budget_max_fraction: value.main_budget_max_fraction,
            late_budget_min_fraction: value.late_budget_min_fraction,
            late_budget_max_fraction: value.late_budget_max_fraction,
            taper_budget_min_fraction: value.taper_budget_min_fraction,
            taper_budget_max_fraction: value.taper_budget_max_fraction,
            target_both_sides_by_30s: value.target_both_sides_by_30s,
            target_both_sides_by_60s: value.target_both_sides_by_60s,
            late_reduce_start_seconds: value.late_reduce_start_seconds,
            late_balance_only_start_seconds: value.late_balance_only_start_seconds,
            late_stop_new_orders_start_seconds: value.late_stop_new_orders_start_seconds,
            legacy_late_window_budget_mode: value.legacy_late_window_budget_mode,
            imbalance_target_fraction: value.imbalance_target_fraction,
            imbalance_warning_fraction: value.imbalance_warning_fraction,
            imbalance_disable_fraction: value.imbalance_disable_fraction,
            imbalance_recovery_fraction: value.imbalance_recovery_fraction,
            clip_ladder: value.clip_ladder,
            repair_reserve_buffer_usd: value.repair_reserve_buffer_usd,
            buy_only_normal_flow: value.buy_only_normal_flow,
            tail_cap_mid_start_seconds: value.tail_cap_mid_start_seconds,
            tail_cap_late_start_seconds: value.tail_cap_late_start_seconds,
            tail_cap_early_fraction: value.tail_cap_early_fraction,
            tail_cap_mid_fraction: value.tail_cap_mid_fraction,
            tail_cap_late_fraction: value.tail_cap_late_fraction,
            bad_regime_window_seconds: value.bad_regime_window_seconds,
            bad_regime_expensive_fraction: value.bad_regime_expensive_fraction,
            mean_reversion_tilt_fraction: value.mean_reversion_tilt_fraction,
        }
    }
}

impl BotRuntimeConfigSnapshotV1 {
    pub fn to_runtime_config(&self) -> BotRuntimeConfigSnapshot {
        BotRuntimeConfigSnapshot {
            phase_controller: Box::leak(self.phase_controller.clone().into_boxed_str()),
            prearm_lead_seconds: self.prearm_lead_seconds,
            open_both_seed_deadline_seconds: self.open_both_seed_deadline_seconds,
            open_both_submit_delta_max_seconds: self.open_both_submit_delta_max_seconds,
            open_both_allow_single_late_seed: self.open_both_allow_single_late_seed,
            seed_budget_min_fraction: self.seed_budget_min_fraction,
            seed_budget_max_fraction: self.seed_budget_max_fraction,
            early_budget_min_fraction: self.early_budget_min_fraction,
            early_budget_max_fraction: self.early_budget_max_fraction,
            main_budget_min_fraction: self.main_budget_min_fraction,
            main_budget_max_fraction: self.main_budget_max_fraction,
            late_budget_min_fraction: self.late_budget_min_fraction,
            late_budget_max_fraction: self.late_budget_max_fraction,
            taper_budget_min_fraction: self.taper_budget_min_fraction,
            taper_budget_max_fraction: self.taper_budget_max_fraction,
            target_both_sides_by_30s: self.target_both_sides_by_30s,
            target_both_sides_by_60s: self.target_both_sides_by_60s,
            late_reduce_start_seconds: self.late_reduce_start_seconds,
            late_balance_only_start_seconds: self.late_balance_only_start_seconds,
            late_stop_new_orders_start_seconds: self.late_stop_new_orders_start_seconds,
            legacy_late_window_budget_mode: self.legacy_late_window_budget_mode,
            imbalance_target_fraction: self.imbalance_target_fraction,
            imbalance_warning_fraction: self.imbalance_warning_fraction,
            imbalance_disable_fraction: self.imbalance_disable_fraction,
            imbalance_recovery_fraction: self.imbalance_recovery_fraction,
            clip_ladder: self.clip_ladder,
            repair_reserve_buffer_usd: self.repair_reserve_buffer_usd,
            buy_only_normal_flow: self.buy_only_normal_flow,
            tail_cap_mid_start_seconds: self.tail_cap_mid_start_seconds,
            tail_cap_late_start_seconds: self.tail_cap_late_start_seconds,
            tail_cap_early_fraction: self.tail_cap_early_fraction,
            tail_cap_mid_fraction: self.tail_cap_mid_fraction,
            tail_cap_late_fraction: self.tail_cap_late_fraction,
            bad_regime_window_seconds: self.bad_regime_window_seconds,
            bad_regime_expensive_fraction: self.bad_regime_expensive_fraction,
            mean_reversion_tilt_fraction: self.mean_reversion_tilt_fraction,
            post_repair_cooldown_cycles: 2,
            repair_refresh_timeout_seconds: 6.0,
            repair_price_zone_danger: 1.20,
            repair_price_zone_stop_add: 1.10,
            green_price_threshold: 0.99,
            weak_edge_threshold: 0.03,
            one_directional_threshold: 0.70,
            one_directional_min_fills: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VersionedConfigSnapshotV1 {
    pub schema_version: String,
    pub source: String,
    pub loaded_at: String,
    pub config_version: String,
    pub config_hash: String,
    pub bot_config: BotConfig,
    pub runtime_config: BotRuntimeConfigSnapshotV1,
    pub execution_config: BotExecutionConfigSnapshot,
}

#[derive(Debug, Clone)]
pub struct ResolvedVersionedConfigBundle {
    pub snapshot: VersionedConfigSnapshotV1,
    pub effective_bot_config: BotConfig,
    pub runtime_config: BotRuntimeConfigSnapshot,
    pub execution_config: BotExecutionConfigSnapshot,
}

impl ResolvedVersionedConfigBundle {
    pub fn config_version(&self) -> &str {
        self.snapshot.config_version.as_str()
    }

    pub fn config_hash(&self) -> &str {
        self.snapshot.config_hash.as_str()
    }

    pub fn loaded_at(&self) -> &str {
        self.snapshot.loaded_at.as_str()
    }

    pub fn config_text(&self) -> Result<String> {
        serde_json::to_string(&self.snapshot).map_err(|err| anyhow!(err))
    }
}

fn canonicalize_json(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = BTreeMap::new();
            for (k, val) in map {
                out.insert(k, canonicalize_json(val));
            }
            let mut m = serde_json::Map::new();
            for (k, val) in out {
                m.insert(k, val);
            }
            Value::Object(m)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(canonicalize_json).collect()),
        other => other,
    }
}

fn wallet_address_from_env(cfg: &BotConfig) -> String {
    let mut wallet_address = env::var("WALLET_ADDRESS").unwrap_or_default();
    if wallet_address.trim().is_empty() {
        wallet_address = env::var("POLYMARKET_WALLET_ADDRESS").unwrap_or_default();
    }
    if wallet_address.trim().is_empty() {
        wallet_address = env::var("POLYMARKET_FUNDER").unwrap_or_default();
    }
    if wallet_address.trim().is_empty() {
        wallet_address = cfg.funder.clone().unwrap_or_default();
    }
    wallet_address.trim().to_ascii_lowercase()
}

fn exec_latency_jsonl_path_from_env(log_dir: &str) -> String {
    let path = env::var("EXEC_LATENCY_JSONL_PATH").unwrap_or_default();
    if path.trim().is_empty() {
        format!("{log_dir}/exec_latency.jsonl")
    } else {
        path.trim().to_string()
    }
}

fn exec_latency_csv_path_from_env(log_dir: &str) -> String {
    let path = env::var("EXEC_LATENCY_CSV_PATH").unwrap_or_default();
    if path.trim().is_empty() {
        format!("{log_dir}/exec_latency.csv")
    } else {
        path.trim().to_string()
    }
}

pub fn build_effective_bot_config_from_env() -> Result<BotConfig> {
    let mut cfg = BotConfig::from_env();
    let legacy_market_data_stale_seconds =
        env::var("MARKET_DATA_STALE_SECONDS").unwrap_or_default();
    if !legacy_market_data_stale_seconds.trim().is_empty() {
        return Err(anyhow!(
            "MARKET_DATA_STALE_SECONDS is unsupported; use MARKET_DATA_STALE_ADD_BLOCK_SECONDS and MARKET_DATA_STALE_HARD_PAUSE_SECONDS"
        ));
    }

    let seg = segment(&env::var("MARKET_SEGMENT").unwrap_or_else(|_| "15M".to_string()));
    let defaults = segment_defaults(&seg);
    cfg.market_segment = seg;
    cfg.market_duration_seconds = env::var("MARKET_DURATION_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(defaults.duration);
    cfg.market_step_seconds = env::var("MARKET_STEP_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(defaults.step);
    cfg.stop_buffer_seconds = env::var("STOP_BUFFER_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(defaults.stop_buffer);

    let sig = env::var("SIGNATURE_TYPE").unwrap_or_else(|_| "1".to_string());
    let funder = env::var("POLYMARKET_FUNDER").unwrap_or_default();
    if !sig.trim().is_empty() && !funder.trim().is_empty() {
        cfg.signature_type = sig.trim().parse::<i64>().ok();
        cfg.funder = Some(funder.trim().to_string());
    }

    cfg.apply_safe_defaults();
    cfg.min_shares = env_float("MIN_SHARES", cfg.min_shares);
    cfg.clip_shares = env_float("CLIP_SHARES", cfg.clip_shares);
    cfg.max_total_cost = env_float("MAX_TOTAL_COST", cfg.max_total_cost);
    cfg.pair_gross_deployed_cost_cap_usd = env_float(
        "BOT_PAIR_GROSS_DEPLOYED_COST_CAP_USD",
        cfg.pair_gross_deployed_cost_cap_usd,
    );
    cfg.portfolio_gross_deployed_cost_cap_usd = env_float(
        "BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD",
        cfg.portfolio_gross_deployed_cost_cap_usd,
    );
    cfg.pair_gross_deployed_cost_buffer_usd = env_float(
        "BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD",
        cfg.pair_gross_deployed_cost_buffer_usd,
    );
    cfg.portfolio_gross_deployed_cost_buffer_usd = env_float(
        "BOT_PORTFOLIO_GROSS_DEPLOYED_COST_BUFFER_USD",
        cfg.portfolio_gross_deployed_cost_buffer_usd,
    );
    cfg.gross_cap_include_pending_maker = env_bool(
        "BOT_GROSS_CAP_INCLUDE_PENDING_MAKER",
        cfg.gross_cap_include_pending_maker,
    );
    cfg.gross_cap_include_pending_taker = env_bool(
        "BOT_GROSS_CAP_INCLUDE_PENDING_TAKER",
        cfg.gross_cap_include_pending_taker,
    );
    cfg.gross_cap_shared_state_ttl_seconds = env_float(
        "BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS",
        cfg.gross_cap_shared_state_ttl_seconds,
    );
    cfg.reserve_usd = env_float("RESERVE_USD", cfg.reserve_usd);
    cfg.dry_run = env_bool("DRY_RUN", cfg.dry_run);
    cfg.log_every = env_int("LOG_EVERY_SECONDS", cfg.log_every) as i64;
    cfg.market_data_stale_add_block_seconds = env_int(
        "MARKET_DATA_STALE_ADD_BLOCK_SECONDS",
        cfg.market_data_stale_add_block_seconds,
    ) as i64;
    cfg.market_data_stale_hard_pause_seconds = env_int(
        "MARKET_DATA_STALE_HARD_PAUSE_SECONDS",
        cfg.market_data_stale_hard_pause_seconds,
    ) as i64;
    cfg.stop_buffer_seconds = env_int("STOP_BUFFER_SECONDS", cfg.stop_buffer_seconds) as i64;
    cfg.entry_edge_ticks = env_int("ENTRY_EDGE_TICKS", cfg.entry_edge_ticks) as i64;
    cfg.hedge_buffer_ticks = env_int("HEDGE_BUFFER_TICKS", cfg.hedge_buffer_ticks) as i64;
    cfg.maker_buffer_ticks = env_int("MAKER_BUFFER_TICKS", cfg.maker_buffer_ticks) as i64;
    cfg.improve_bid_ticks = env_int("IMPROVE_BID_TICKS", cfg.improve_bid_ticks) as i64;
    cfg.replace_if_price_moves_ticks = env_int(
        "REPLACE_IF_PRICE_MOVES_TICKS",
        cfg.replace_if_price_moves_ticks,
    ) as i64;
    cfg.stale_seconds = env_int("STALE_SECONDS", cfg.stale_seconds) as i64;
    cfg.maker_replace_min_interval_seconds = env_float(
        "MAKER_REPLACE_MIN_INTERVAL_SECONDS",
        cfg.maker_replace_min_interval_seconds,
    )
    .max(0.0);
    let order_mode = resolve_bot_order_mode_from_env()?;
    cfg.dry_run = !order_mode.eq_ignore_ascii_case("live");
    validate_stale_data_policy(&cfg)?;
    validate_gross_cap_policy(&cfg)?;

    Ok(cfg)
}

fn build_execution_config_from_env_with_order_mode(
    cfg: &BotConfig,
    order_mode_override: Option<String>,
) -> Result<BotExecutionConfigSnapshot> {
    let log_dir = env::var("EXEC_LATENCY_LOG_DIR")
        .unwrap_or_else(|_| "./logs".to_string())
        .trim()
        .to_string();
    let warmup_default = segment_defaults(&cfg.market_segment).warmup;
    let exec_mode = env::var("EXEC_MODE")
        .unwrap_or_else(|_| "BOT".to_string())
        .trim()
        .to_ascii_uppercase();
    if exec_mode != "BOT" {
        return Err(anyhow!(
            "Unsupported EXEC_MODE={exec_mode}. Only BOT is supported."
        ));
    }
    let order_mode = match order_mode_override {
        Some(value) => parse_bot_order_mode(value.as_str())
            .ok_or_else(|| anyhow!("Invalid BOT_ORDER_MODE={}", value.trim()))?,
        None => resolve_bot_order_mode_from_env()?,
    };
    let live_enabled = env_bool("BOT_LIVE_ENABLED", default_bot_live_enabled());
    let mut wallet_address = wallet_address_from_env(cfg);
    if wallet_address.trim().is_empty() && order_mode.eq_ignore_ascii_case("paper") {
        wallet_address = "paper".to_string();
    }
    if execution_order_mode_requires_wallet_observation(order_mode.as_str()) {
        if cfg.private_key.trim().is_empty() {
            return Err(anyhow!("Missing POLYMARKET_PRIVATE_KEY"));
        }
        if cfg.funder.clone().unwrap_or_default().trim().is_empty() {
            return Err(anyhow!("Missing POLYMARKET_FUNDER"));
        }
        if wallet_address.trim().is_empty() {
            return Err(anyhow!("Missing WALLET_ADDRESS"));
        }
    }

    Ok(BotExecutionConfigSnapshot {
        wallet_address,
        min_maker_notional: env_float("MIN_MAKER_NOTIONAL", 1.0),
        min_taker_notional: env_float("MIN_TAKER_NOTIONAL", 1.0),
        reconcile_sell_credit_mult: env_float("RECONCILE_SELL_CREDIT_MULT", 1.0).clamp(0.0, 1.0),
        first_clip_shares: env_float("FIRST_CLIP_SHARES", 0.0),
        first_hedge_full: matches!(
            env::var("FIRST_HEDGE_FULL")
                .unwrap_or_else(|_| "false".to_string())
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "y"
        ),
        warmup_seconds: env_int("WARMUP_SECONDS", warmup_default) as i64,
        max_spread_ticks: env_int("MAX_SPREAD_TICKS", 6) as i64,
        parity_tolerance: env_float("PARITY_TOLERANCE", 0.025),
        unhedged_timeout_seconds: env_float("UNHEDGED_TIMEOUT_SECONDS", 2.0),
        hedge_slippage_ticks: env_int("HEDGE_SLIPPAGE_TICKS", 1) as i64,
        hedge_taker_order_type: env::var("HEDGE_TAKER_ORDER_TYPE")
            .unwrap_or_else(|_| "FAK".to_string())
            .trim()
            .to_ascii_uppercase(),
        taker_order_ttl_seconds: env_int("TAKER_ORDER_TTL_SECONDS", 120) as i64,
        taker_fill_fallback_from_order_events: env_bool(
            "TAKER_FILL_FALLBACK_FROM_ORDER_EVENTS",
            true,
        ),
        taker_strict_inflight: env_bool("TAKER_STRICT_INFLIGHT", true),
        taker_hedge_min_interval: env_float("TAKER_HEDGE_MIN_INTERVAL", 1.0),
        exec_mode,
        loop_wait_seconds_maker: env_float("LOOP_WAIT_SECONDS_MAKER", 1.0),
        loop_wait_seconds_taker: env_float("LOOP_WAIT_SECONDS_TAKER", 0.2),
        min_entry_edge_ticks: env_int("MIN_ENTRY_EDGE_TICKS", cfg.entry_edge_ticks).max(0) as i64,
        exec_latency_log_enabled: env_bool("EXEC_LATENCY_LOG_ENABLED", true),
        exec_latency_file_log_enabled: env_bool("EXEC_LATENCY_FILE_LOG_ENABLED", true),
        exec_latency_jsonl_enabled: env_bool("EXEC_LATENCY_JSONL_ENABLED", true),
        exec_latency_csv_enabled: env_bool("EXEC_LATENCY_CSV_ENABLED", true),
        exec_latency_log_dir: log_dir.clone(),
        exec_latency_jsonl_path: exec_latency_jsonl_path_from_env(log_dir.as_str()),
        exec_latency_csv_path: exec_latency_csv_path_from_env(log_dir.as_str()),
        clob_gamma_host: env::var("CLOB_GAMMA_API_URL")
            .or_else(|_| env::var("GAMMA_HOST"))
            .unwrap_or_else(|_| "https://gamma-api.polymarket.com".to_string()),
        clob_order_meta_warmup: env_bool("CLOB_ORDER_META_WARMUP", true),
        order_mode,
        live_enabled,
    })
}

pub fn build_execution_config_from_env(cfg: &BotConfig) -> Result<BotExecutionConfigSnapshot> {
    build_execution_config_from_env_with_order_mode(cfg, None)
}

fn sanitized_bot_config(cfg: &BotConfig) -> BotConfig {
    let mut sanitized = cfg.clone();
    sanitized.private_key.clear();
    sanitized
}

fn config_version_payload(
    bot_config: &BotConfig,
    runtime_config: &BotRuntimeConfigSnapshotV1,
    execution_config: &BotExecutionConfigSnapshot,
) -> Value {
    serde_json::json!({
        "schema_version": "v1",
        "source": "env",
        "bot_config": sanitized_bot_config(bot_config),
        "runtime_config": runtime_config,
        "execution_config": execution_config,
    })
}

fn version_hash_from_payload(payload: &Value) -> Result<String> {
    let canonical = canonicalize_json(payload.clone());
    let serialized = serde_json::to_string(&canonical)?;
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

fn config_version_from_hash(hash: &str) -> String {
    let short = hash.chars().take(12).collect::<String>();
    format!("cfgv1_{short}")
}

fn snapshot_bot_config_max_total_cost_compat(bot_config: &serde_json::Map<String, Value>) -> f64 {
    bot_config
        .get("max_total_cost")
        .and_then(|value| {
            value.as_f64().or_else(|| {
                value
                    .as_str()
                    .and_then(|raw| raw.trim().parse::<f64>().ok())
            })
        })
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or_else(default_pair_gross_deployed_cost_cap_usd)
}

fn backfill_snapshot_bot_config_gross_cap_fields(value: &mut Value) {
    let Some(bot_config) = value.get_mut("bot_config").and_then(Value::as_object_mut) else {
        return;
    };
    let pair_cap = snapshot_bot_config_max_total_cost_compat(bot_config);
    if !bot_config.contains_key("pair_gross_deployed_cost_cap_usd") {
        bot_config.insert(
            "pair_gross_deployed_cost_cap_usd".to_string(),
            Value::from(pair_cap),
        );
    }
    if !bot_config.contains_key("portfolio_gross_deployed_cost_cap_usd") {
        bot_config.insert(
            "portfolio_gross_deployed_cost_cap_usd".to_string(),
            Value::from(pair_cap * 4.0),
        );
    }
    if !bot_config.contains_key("pair_gross_deployed_cost_buffer_usd") {
        bot_config.insert(
            "pair_gross_deployed_cost_buffer_usd".to_string(),
            Value::from(default_gross_deployed_cost_buffer_usd()),
        );
    }
    if !bot_config.contains_key("portfolio_gross_deployed_cost_buffer_usd") {
        bot_config.insert(
            "portfolio_gross_deployed_cost_buffer_usd".to_string(),
            Value::from(default_gross_deployed_cost_buffer_usd()),
        );
    }
    if !bot_config.contains_key("gross_cap_include_pending_maker") {
        bot_config.insert(
            "gross_cap_include_pending_maker".to_string(),
            Value::from(default_gross_cap_include_pending()),
        );
    }
    if !bot_config.contains_key("gross_cap_include_pending_taker") {
        bot_config.insert(
            "gross_cap_include_pending_taker".to_string(),
            Value::from(default_gross_cap_include_pending()),
        );
    }
    if !bot_config.contains_key("gross_cap_shared_state_ttl_seconds") {
        bot_config.insert(
            "gross_cap_shared_state_ttl_seconds".to_string(),
            Value::from(default_gross_cap_shared_state_ttl_seconds()),
        );
    }
}

fn snapshot_bot_config_dry_run_compat(value: &Value) -> bool {
    value
        .get("bot_config")
        .and_then(Value::as_object)
        .and_then(|bot_config| bot_config.get("dry_run"))
        .and_then(|raw| {
            raw.as_bool()
                .or_else(|| raw.as_str().and_then(parse_env_bool_like))
        })
        .unwrap_or(false)
}

fn backfill_snapshot_execution_mode_fields(value: &mut Value) {
    let legacy_dry_run = snapshot_bot_config_dry_run_compat(value);
    let Some(execution_config) = value
        .get_mut("execution_config")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if !execution_config.contains_key("order_mode") {
        execution_config.insert(
            "order_mode".to_string(),
            Value::from(order_mode_from_legacy_dry_run(legacy_dry_run)),
        );
    }
    if !execution_config.contains_key("live_enabled") {
        execution_config.insert(
            "live_enabled".to_string(),
            Value::from(default_bot_live_enabled()),
        );
    }
}

pub(crate) fn snapshot_from_json_text_compat(
    config_text: &str,
) -> Result<VersionedConfigSnapshotV1> {
    let mut value: Value = serde_json::from_str(config_text)?;
    backfill_snapshot_bot_config_gross_cap_fields(&mut value);
    backfill_snapshot_execution_mode_fields(&mut value);
    Ok(serde_json::from_value(value)?)
}

pub(crate) fn snapshot_from_json_value_compat(
    mut value: Value,
) -> Result<VersionedConfigSnapshotV1> {
    backfill_snapshot_bot_config_gross_cap_fields(&mut value);
    backfill_snapshot_execution_mode_fields(&mut value);
    Ok(serde_json::from_value(value)?)
}

pub fn load_versioned_config_bundle_from_env() -> Result<ResolvedVersionedConfigBundle> {
    let effective_bot_config = build_effective_bot_config_from_env()?;
    let runtime_config = bot_runtime_config_from_reader(|key| env::var(key).ok());
    bot_runtime_validate_config(&runtime_config).map_err(|err| anyhow!(err))?;
    let execution_config = build_execution_config_from_env(&effective_bot_config)?;
    let runtime_snapshot = BotRuntimeConfigSnapshotV1::from(&runtime_config);
    let payload =
        config_version_payload(&effective_bot_config, &runtime_snapshot, &execution_config);
    let config_hash = version_hash_from_payload(&payload)?;
    let snapshot = VersionedConfigSnapshotV1 {
        schema_version: "v1".to_string(),
        source: "env".to_string(),
        loaded_at: Utc::now().to_rfc3339(),
        config_version: config_version_from_hash(config_hash.as_str()),
        config_hash,
        bot_config: sanitized_bot_config(&effective_bot_config),
        runtime_config: runtime_snapshot,
        execution_config: execution_config.clone(),
    };
    Ok(ResolvedVersionedConfigBundle {
        snapshot,
        effective_bot_config,
        runtime_config,
        execution_config,
    })
}

fn normalize_execution_snapshot_for_mode(
    effective_bot_config: &mut BotConfig,
    execution_config: &mut BotExecutionConfigSnapshot,
) -> Result<()> {
    if execution_config.order_mode.trim().is_empty() {
        execution_config.order_mode = order_mode_from_legacy_dry_run(effective_bot_config.dry_run);
    } else {
        execution_config.order_mode = parse_bot_order_mode(&execution_config.order_mode)
            .ok_or_else(|| anyhow!("Invalid BOT_ORDER_MODE={}", execution_config.order_mode))?;
    }
    execution_config.wallet_address = execution_config.wallet_address.trim().to_ascii_lowercase();
    if execution_config.wallet_address.is_empty()
        && execution_config.order_mode.eq_ignore_ascii_case("paper")
    {
        execution_config.wallet_address = "paper".to_string();
    }
    if execution_order_mode_requires_wallet_observation(execution_config.order_mode.as_str()) {
        if effective_bot_config.private_key.trim().is_empty() {
            effective_bot_config.private_key = env::var("POLYMARKET_PRIVATE_KEY")
                .unwrap_or_default()
                .trim()
                .to_string();
        }
        if execution_config.wallet_address.trim().is_empty()
            && !effective_bot_config.private_key.trim().is_empty()
        {
            execution_config.wallet_address = wallet_address_from_env(effective_bot_config);
        }
        if effective_bot_config.private_key.trim().is_empty() {
            return Err(anyhow!("Missing POLYMARKET_PRIVATE_KEY"));
        }
        if effective_bot_config
            .funder
            .clone()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(anyhow!("Missing POLYMARKET_FUNDER"));
        }
        if execution_config.wallet_address.trim().is_empty() {
            return Err(anyhow!("Missing WALLET_ADDRESS"));
        }
    }
    effective_bot_config.dry_run = !execution_config.order_mode.eq_ignore_ascii_case("live");
    Ok(())
}

pub fn resolve_versioned_config_bundle_from_snapshot(
    mut snapshot: VersionedConfigSnapshotV1,
) -> Result<ResolvedVersionedConfigBundle> {
    let runtime_config = snapshot.runtime_config.to_runtime_config();
    bot_runtime_validate_config(&runtime_config).map_err(|err| anyhow!(err))?;
    let mut effective_bot_config = snapshot.bot_config.clone();
    validate_stale_data_policy(&effective_bot_config)?;
    validate_gross_cap_policy(&effective_bot_config)?;
    normalize_execution_snapshot_for_mode(
        &mut effective_bot_config,
        &mut snapshot.execution_config,
    )?;
    Ok(ResolvedVersionedConfigBundle {
        execution_config: snapshot.execution_config.clone(),
        snapshot,
        effective_bot_config,
        runtime_config,
    })
}

pub fn build_legacy_versioned_config_bundle(
    mut effective_bot_config: BotConfig,
    config_hash: String,
    config_version: String,
    loaded_at: String,
) -> Result<ResolvedVersionedConfigBundle> {
    validate_stale_data_policy(&effective_bot_config)?;
    validate_gross_cap_policy(&effective_bot_config)?;
    let runtime_config = bot_runtime_config_from_reader(|key| env::var(key).ok());
    bot_runtime_validate_config(&runtime_config).map_err(|err| anyhow!(err))?;
    let legacy_order_mode_override = if env::var("BOT_ORDER_MODE")
        .unwrap_or_default()
        .trim()
        .is_empty()
        && env::var("DRY_RUN").unwrap_or_default().trim().is_empty()
    {
        Some(order_mode_from_legacy_dry_run(effective_bot_config.dry_run))
    } else {
        None
    };
    let mut execution_config = build_execution_config_from_env_with_order_mode(
        &effective_bot_config,
        legacy_order_mode_override,
    )?;
    normalize_execution_snapshot_for_mode(&mut effective_bot_config, &mut execution_config)?;
    let snapshot = VersionedConfigSnapshotV1 {
        schema_version: "legacy_flat_row".to_string(),
        source: "configuration_row".to_string(),
        loaded_at,
        config_version,
        config_hash,
        bot_config: sanitized_bot_config(&effective_bot_config),
        runtime_config: BotRuntimeConfigSnapshotV1::from(&runtime_config),
        execution_config: execution_config.clone(),
    };
    Ok(ResolvedVersionedConfigBundle {
        snapshot,
        effective_bot_config,
        runtime_config,
        execution_config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _guard = crate::test_env_lock().lock().expect("env lock");
        let saved = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), env::var(key).ok()))
            .collect::<Vec<_>>();
        let saved_exec_mode = env::var("EXEC_MODE").ok();
        for (key, value) in vars {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
        if !vars.iter().any(|(key, _)| *key == "EXEC_MODE") {
            env::set_var("EXEC_MODE", "BOT");
        }
        f();
        for (key, value) in saved {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
        match saved_exec_mode {
            Some(v) => env::set_var("EXEC_MODE", v),
            None => env::remove_var("EXEC_MODE"),
        }
    }

    #[test]
    fn versioned_config_bundle_reuses_version_for_identical_effective_config() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("BOT_PREARM_LEAD_SECONDS", Some("20")),
                ("MIN_MAKER_NOTIONAL", Some("1.0")),
            ],
            || {
                let left = load_versioned_config_bundle_from_env().expect("left bundle");
                let right = load_versioned_config_bundle_from_env().expect("right bundle");
                assert_eq!(left.config_hash(), right.config_hash());
                assert_eq!(left.config_version(), right.config_version());
            },
        );
    }

    #[test]
    fn runtime_only_change_rolls_config_version() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("BOT_PREARM_LEAD_SECONDS", Some("20")),
            ],
            || {
                let base = load_versioned_config_bundle_from_env().expect("base bundle");
                env::set_var("BOT_PREARM_LEAD_SECONDS", "21");
                let changed = load_versioned_config_bundle_from_env().expect("changed bundle");
                assert_ne!(base.config_hash(), changed.config_hash());
                assert_ne!(base.config_version(), changed.config_version());
            },
        );
    }

    #[test]
    fn secret_only_change_does_not_roll_config_version_or_persist_secret() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
            ],
            || {
                let left = load_versioned_config_bundle_from_env().expect("left bundle");
                env::set_var("POLYMARKET_PRIVATE_KEY", "secret-b");
                let right = load_versioned_config_bundle_from_env().expect("right bundle");
                assert_eq!(left.config_version(), right.config_version());
                let text = right.config_text().expect("config text");
                assert!(!text.contains("secret-a"));
                assert!(!text.contains("secret-b"));
            },
        );
    }

    #[test]
    fn bot_order_mode_defaults_to_shadow_and_live_disabled_false() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("BOT_ORDER_MODE", None),
                ("BOT_LIVE_ENABLED", None),
                ("DRY_RUN", None),
            ],
            || {
                let bundle = load_versioned_config_bundle_from_env().expect("bundle");
                assert_eq!(bundle.execution_config.order_mode, "shadow");
                assert!(!bundle.execution_config.live_enabled);
                assert!(bundle.effective_bot_config.dry_run);
            },
        );
    }

    #[test]
    fn dry_run_true_backfills_to_paper_without_live_credentials() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", None),
                ("POLYMARKET_FUNDER", None),
                ("WALLET_ADDRESS", None),
                ("BOT_ORDER_MODE", None),
                ("BOT_LIVE_ENABLED", None),
                ("DRY_RUN", Some("true")),
            ],
            || {
                let bundle = load_versioned_config_bundle_from_env().expect("paper bundle");
                assert_eq!(bundle.execution_config.order_mode, "paper");
                assert_eq!(bundle.execution_config.wallet_address, "paper");
                assert!(bundle.effective_bot_config.dry_run);
            },
        );
    }

    #[test]
    fn inconsistent_dry_run_and_bot_order_mode_is_rejected() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("DRY_RUN", Some("true")),
                ("BOT_ORDER_MODE", Some("live")),
            ],
            || {
                let err = load_versioned_config_bundle_from_env()
                    .expect_err("inconsistent mode envs should fail");
                assert!(err
                    .to_string()
                    .contains("Inconsistent DRY_RUN and BOT_ORDER_MODE"));
            },
        );
    }

    #[test]
    fn old_snapshot_without_execution_mode_fields_backfills_legacy_dry_run_mode() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", None),
                ("POLYMARKET_FUNDER", None),
                ("DRY_RUN", Some("true")),
                ("BOT_ORDER_MODE", None),
                ("BOT_LIVE_ENABLED", None),
            ],
            || {
                let bundle = load_versioned_config_bundle_from_env().expect("bundle");
                let mut value: Value =
                    serde_json::from_str(&bundle.config_text().expect("config text"))
                        .expect("snapshot json");
                value
                    .get_mut("execution_config")
                    .and_then(Value::as_object_mut)
                    .expect("execution config object")
                    .remove("order_mode");
                value
                    .get_mut("execution_config")
                    .and_then(Value::as_object_mut)
                    .expect("execution config object")
                    .remove("live_enabled");
                let snapshot =
                    snapshot_from_json_value_compat(value).expect("legacy compat snapshot");
                let resolved =
                    resolve_versioned_config_bundle_from_snapshot(snapshot).expect("resolved");
                assert_eq!(resolved.execution_config.order_mode, "paper");
                assert!(!resolved.execution_config.live_enabled);
            },
        );
    }

    #[test]
    fn legacy_dry_run_flat_row_builds_paper_mode_before_env_validation() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", None),
                ("POLYMARKET_FUNDER", None),
                ("WALLET_ADDRESS", None),
                ("BOT_ORDER_MODE", None),
                ("BOT_LIVE_ENABLED", None),
                ("DRY_RUN", None),
            ],
            || {
                let mut cfg = BotConfig::default();
                cfg.dry_run = true;
                let bundle = build_legacy_versioned_config_bundle(
                    cfg,
                    "legacy_hash".to_string(),
                    "legacy_version".to_string(),
                    "2026-03-22T00:00:00Z".to_string(),
                )
                .expect("legacy paper bundle");
                assert_eq!(bundle.execution_config.order_mode, "paper");
                assert_eq!(bundle.execution_config.wallet_address, "paper");
                assert!(bundle.effective_bot_config.dry_run);
            },
        );
    }

    #[test]
    fn stale_data_policy_defaults_to_requirement_thresholds() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MARKET_DATA_STALE_ADD_BLOCK_SECONDS", None),
                ("MARKET_DATA_STALE_HARD_PAUSE_SECONDS", None),
                ("MARKET_DATA_STALE_SECONDS", None),
            ],
            || {
                let cfg = build_effective_bot_config_from_env().expect("effective config");
                assert_eq!(cfg.market_data_stale_add_block_seconds, 2);
                assert_eq!(cfg.market_data_stale_hard_pause_seconds, 5);
                assert!(stale_data_policy_requirement_compliant(&cfg));
            },
        );
    }

    #[test]
    fn maker_replace_min_interval_defaults_to_one_second() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MAKER_REPLACE_MIN_INTERVAL_SECONDS", None),
            ],
            || {
                let cfg = build_effective_bot_config_from_env().expect("effective config");
                assert!((cfg.maker_replace_min_interval_seconds - 1.0).abs() < 1e-9);
            },
        );
    }

    #[test]
    fn maker_replace_min_interval_respects_operator_override() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MAKER_REPLACE_MIN_INTERVAL_SECONDS", Some("1.5")),
            ],
            || {
                let cfg = build_effective_bot_config_from_env().expect("effective config");
                assert!((cfg.maker_replace_min_interval_seconds - 1.5).abs() < 1e-9);
            },
        );
    }

    #[test]
    fn gross_cap_defaults_follow_max_total_cost() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MAX_TOTAL_COST", Some("37")),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_CAP_USD", None),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD", None),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD", None),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_BUFFER_USD", None),
                ("BOT_GROSS_CAP_INCLUDE_PENDING_MAKER", None),
                ("BOT_GROSS_CAP_INCLUDE_PENDING_TAKER", None),
                ("BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS", None),
            ],
            || {
                let cfg = build_effective_bot_config_from_env().expect("effective config");
                assert!((cfg.pair_gross_deployed_cost_cap_usd - 37.0).abs() < 1e-9);
                assert!((cfg.portfolio_gross_deployed_cost_cap_usd - 148.0).abs() < 1e-9);
                assert_eq!(cfg.pair_gross_deployed_cost_buffer_usd, 0.0);
                assert_eq!(cfg.portfolio_gross_deployed_cost_buffer_usd, 0.0);
                assert!(cfg.gross_cap_include_pending_maker);
                assert!(cfg.gross_cap_include_pending_taker);
                assert!((cfg.gross_cap_shared_state_ttl_seconds - 30.0).abs() < 1e-9);
            },
        );
    }

    #[test]
    fn invalid_gross_cap_policy_rejects_bad_buffers_and_ttl() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_CAP_USD", Some("20")),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD", Some("80")),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD", Some("20")),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_BUFFER_USD", Some("0")),
                ("BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS", Some("30")),
            ],
            || {
                let err = build_effective_bot_config_from_env()
                    .expect_err("pair buffer equal to cap should fail");
                assert!(err
                    .to_string()
                    .contains("BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD"));
            },
        );
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_CAP_USD", Some("20")),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD", Some("80")),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD", Some("0")),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_BUFFER_USD", Some("0")),
                ("BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS", Some("0")),
            ],
            || {
                let err = build_effective_bot_config_from_env().expect_err("zero ttl should fail");
                assert!(err
                    .to_string()
                    .contains("BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS"));
            },
        );
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_CAP_USD", Some("20")),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD", Some("80")),
                ("BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD", Some("0")),
                ("BOT_PORTFOLIO_GROSS_DEPLOYED_COST_BUFFER_USD", Some("0")),
                ("BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS", Some("0.5")),
            ],
            || {
                let err = build_effective_bot_config_from_env().expect_err("0.5s ttl should fail");
                assert!(err
                    .to_string()
                    .contains("BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS"));
            },
        );
    }

    #[test]
    fn stale_data_policy_rejects_legacy_single_threshold_env() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MARKET_DATA_STALE_SECONDS", Some("8")),
            ],
            || {
                let err = build_effective_bot_config_from_env()
                    .expect_err("legacy stale env should fail");
                assert!(err
                    .to_string()
                    .contains("MARKET_DATA_STALE_SECONDS is unsupported"));
            },
        );
    }

    #[test]
    fn relaxed_stale_data_policy_is_allowed_but_noncompliant() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MARKET_DATA_STALE_ADD_BLOCK_SECONDS", Some("3")),
                ("MARKET_DATA_STALE_HARD_PAUSE_SECONDS", Some("6")),
                ("MARKET_DATA_STALE_SECONDS", None),
            ],
            || {
                let cfg = build_effective_bot_config_from_env().expect("effective config");
                assert_eq!(cfg.market_data_stale_add_block_seconds, 3);
                assert_eq!(cfg.market_data_stale_hard_pause_seconds, 6);
                assert!(!stale_data_policy_requirement_compliant(&cfg));
            },
        );
    }

    #[test]
    fn stale_data_policy_rejects_non_ascending_thresholds() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MARKET_DATA_STALE_ADD_BLOCK_SECONDS", Some("6")),
                ("MARKET_DATA_STALE_HARD_PAUSE_SECONDS", Some("6")),
                ("MARKET_DATA_STALE_SECONDS", None),
            ],
            || {
                let err = build_effective_bot_config_from_env()
                    .expect_err("non-ascending stale policy should fail");
                assert!(err
                    .to_string()
                    .contains("MARKET_DATA_STALE_HARD_PAUSE_SECONDS"));
            },
        );
    }

    #[test]
    fn old_snapshot_without_new_stale_fields_uses_requirement_defaults() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MARKET_DATA_STALE_SECONDS", None),
            ],
            || {
                let bundle = load_versioned_config_bundle_from_env().expect("bundle");
                let mut value: Value =
                    serde_json::from_str(&bundle.config_text().expect("config text"))
                        .expect("snapshot json");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("market_data_stale_add_block_seconds");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("market_data_stale_hard_pause_seconds");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("pair_gross_deployed_cost_cap_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("portfolio_gross_deployed_cost_cap_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("pair_gross_deployed_cost_buffer_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("portfolio_gross_deployed_cost_buffer_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("gross_cap_include_pending_maker");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("gross_cap_include_pending_taker");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("gross_cap_shared_state_ttl_seconds");

                let snapshot =
                    snapshot_from_json_value_compat(value).expect("legacy-like snapshot");
                let resolved =
                    resolve_versioned_config_bundle_from_snapshot(snapshot).expect("resolved");
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .market_data_stale_add_block_seconds,
                    2
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .market_data_stale_hard_pause_seconds,
                    5
                );
                assert!(
                    (resolved
                        .effective_bot_config
                        .maker_replace_min_interval_seconds
                        - 1.0)
                        .abs()
                        < 1e-9
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .pair_gross_deployed_cost_cap_usd,
                    resolved.effective_bot_config.max_total_cost
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .portfolio_gross_deployed_cost_cap_usd,
                    resolved.effective_bot_config.max_total_cost * 4.0
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .pair_gross_deployed_cost_buffer_usd,
                    0.0
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .portfolio_gross_deployed_cost_buffer_usd,
                    0.0
                );
                assert!(
                    resolved
                        .effective_bot_config
                        .gross_cap_include_pending_maker
                );
                assert!(
                    resolved
                        .effective_bot_config
                        .gross_cap_include_pending_taker
                );
                assert!(
                    (resolved
                        .effective_bot_config
                        .gross_cap_shared_state_ttl_seconds
                        - 30.0)
                        .abs()
                        < 1e-9
                );
                assert!(stale_data_policy_requirement_compliant(
                    &resolved.effective_bot_config
                ));
            },
        );
    }

    #[test]
    fn old_snapshot_missing_gross_caps_preserves_max_total_cost_derived_policy() {
        with_env(
            &[
                ("POLYMARKET_PRIVATE_KEY", Some("secret-a")),
                ("POLYMARKET_FUNDER", Some("0xfunder")),
                ("MAX_TOTAL_COST", Some("37")),
                ("MARKET_DATA_STALE_SECONDS", None),
                ("EXEC_MODE", Some("BOT")),
            ],
            || {
                let bundle = load_versioned_config_bundle_from_env().expect("bundle");
                let mut value: Value =
                    serde_json::from_str(&bundle.config_text().expect("config text"))
                        .expect("snapshot json");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("pair_gross_deployed_cost_cap_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("portfolio_gross_deployed_cost_cap_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("pair_gross_deployed_cost_buffer_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("portfolio_gross_deployed_cost_buffer_usd");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("gross_cap_include_pending_maker");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("gross_cap_include_pending_taker");
                value
                    .get_mut("bot_config")
                    .and_then(Value::as_object_mut)
                    .expect("bot config object")
                    .remove("gross_cap_shared_state_ttl_seconds");

                let snapshot =
                    snapshot_from_json_value_compat(value).expect("legacy-like snapshot");
                let resolved =
                    resolve_versioned_config_bundle_from_snapshot(snapshot).expect("resolved");

                assert_eq!(resolved.effective_bot_config.max_total_cost, 37.0);
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .pair_gross_deployed_cost_cap_usd,
                    37.0
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .portfolio_gross_deployed_cost_cap_usd,
                    148.0
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .pair_gross_deployed_cost_buffer_usd,
                    0.0
                );
                assert_eq!(
                    resolved
                        .effective_bot_config
                        .portfolio_gross_deployed_cost_buffer_usd,
                    0.0
                );
                assert!(
                    resolved
                        .effective_bot_config
                        .gross_cap_include_pending_maker
                );
                assert!(
                    resolved
                        .effective_bot_config
                        .gross_cap_include_pending_taker
                );
                assert!(
                    (resolved
                        .effective_bot_config
                        .gross_cap_shared_state_ttl_seconds
                        - 30.0)
                        .abs()
                        < 1e-9
                );
            },
        );
    }
}
