use crate::binance_feed::{BinanceFeedConfig, BinanceFeedService};
use crate::config::BotConfig;
use crate::db::TradeDecisionUpsert;
use crate::env_utils::{env_bool, env_float, env_int};
use crate::gamma::{fetch_market_by_slug, parse_tokens_and_condition};
use crate::helpers::{
    clamp, cost_per_pair, iso_to_epoch, load_state, locked_profit, q_down, q_up, round_down,
    round_up, save_state, segment_defaults, BotState, OpenOrderState,
    SniperEntryBreakoutAnchorState,
};
use crate::logging::LogLike;
use crate::rtds::get_live_snapshot_for_market;
use crate::signal::{LatencyLogService, SignalHub};
use crate::sniper_filters::{
    normalize_asset_symbol, BreakoutInvalidationStopDecision,
    FilterDecision as SniperFilterDecision, SniperFilterEngine, SniperFilterPersistedState,
};
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
    pub entry_time_iso: Option<String>,
    pub entry_reason: Option<String>,
    pub stop_loss_category: Option<String>,
    pub exit_reason: String,
    pub fill_count: usize,
}

#[derive(Debug, Clone, Default)]
struct SniperOrderFillAgg {
    qty: f64,
    notional: f64,
}

#[derive(Debug, Clone, Default)]
struct SniperTradeDecisionRuntime {
    order_id: Option<String>,
    data: TradeDecisionUpsert,
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

#[derive(Debug, Clone, Default)]
struct MakerSkewArbState {
    window_start_ts: i64,
    cost_total: f64,
    shares_up: f64,
    shares_down: f64,
    downside: f64,
    upside: f64,
    skew_ratio: f64,
    cpp: f64,
    last_decision_ts: f64,
    unhedged_since: f64,
    stretch_rsi: Option<f64>,
    stretch_diff_vs_start: Option<f64>,
    stretch_default_side: String,
    stretch_biased_side: String,
    stretch_bias_reason: String,
    stretch_eval_ts: f64,
    stretch_chainlink_closes: Vec<f64>,
    stretch_chainlink_last_ts_ms: i64,
}

#[derive(Debug, Clone)]
struct MakerSkewLoopCtx {
    now: f64,
    t_into_s: f64,
    peak_window: bool,
    total_cost: f64,
    budget_usable: f64,
    yes_asset: String,
    no_asset: String,
    y_bid: f64,
    y_ask: f64,
    n_bid: f64,
    n_ask: f64,
    q_yes_eff: f64,
    q_no_eff: f64,
    downside: f64,
    upside: f64,
    skew_ratio: f64,
}

#[derive(Debug, Clone)]
struct MakerSkewRecoveryState {
    mode: bool,
    side: String,
}

#[derive(Debug, Clone)]
struct PairBaseRecoveryState {
    mode: bool,
    gap: f64,
    heavy_side: String,
    light_side: String,
    light_asset_id: String,
}

#[derive(Debug, Clone)]
struct PairBaseFeeNetSnapshot {
    fees_enabled: bool,
    fee_source: String,
    maker_rebate_bps: f64,
    estimated_fees: f64,
    fee_net_pair_cost: f64,
    fee_net_worst_case_pnl: f64,
    fee_net_best_case_pnl: f64,
    pair_coverage: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PairBasePhaseState {
    #[default]
    Flat,
    PairResting,
    MergePending,
    Balanced,
    RiskExitOnly,
}

impl PairBasePhaseState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::PairResting => "PairResting",
            Self::MergePending => "MergePending",
            Self::Balanced => "Balanced",
            Self::RiskExitOnly => "RiskExitOnly",
        }
    }
}

fn pair_base_remaining_gap(actual_gap: f64, light_unsettled: f64) -> f64 {
    (actual_gap.max(0.0) - light_unsettled.max(0.0)).max(0.0)
}

fn pair_base_phase_without_recovery(
    has_inventory: bool,
    actual_gap: f64,
    release: f64,
    pair_orders_live: bool,
) -> Option<PairBasePhaseState> {
    if pair_orders_live {
        return Some(PairBasePhaseState::PairResting);
    }
    if actual_gap <= release + 1e-6 && has_inventory {
        return Some(PairBasePhaseState::Balanced);
    }
    if !has_inventory {
        return Some(PairBasePhaseState::Flat);
    }
    None
}

fn pair_base_should_force_recovery(
    phase: PairBasePhaseState,
    actual_gap: f64,
    release: f64,
    light_leg_trusted: bool,
) -> bool {
    if actual_gap <= release + 1e-6 {
        return false;
    }
    match phase {
        PairBasePhaseState::MergePending => true,
        PairBasePhaseState::PairResting => !light_leg_trusted,
        _ => false,
    }
}

fn pair_base_early_risk_exit_lead_seconds(stop_buffer_s: f64) -> f64 {
    let stop_buffer = stop_buffer_s.max(1.0);
    (stop_buffer * 2.0).max(stop_buffer + 10.0).max(30.0)
}

fn pair_base_should_latch_risk_exit(reason: &str) -> bool {
    matches!(reason.trim(), "near_expiry" | "latched")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PairBaseSubMinGapPolicy {
    Hold,
    TakerImmediate,
}

fn pair_base_sub_min_gap_policy() -> PairBaseSubMinGapPolicy {
    match std::env::var("PAIR_BASE_SUB_MIN_GAP_POLICY")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "taker" | "taker_immediate" | "immediate" => PairBaseSubMinGapPolicy::TakerImmediate,
        _ => PairBaseSubMinGapPolicy::Hold,
    }
}

fn pair_base_near_expiry_taker_override_active(
    reason: &str,
    t_left: f64,
    force_seconds: f64,
    override_max_price: f64,
) -> bool {
    reason.trim() == "pair_base_near_expiry"
        && force_seconds > 0.0
        && override_max_price > 0.0
        && t_left <= force_seconds + 1e-6
}

fn pair_base_effective_taker_cap(base_cap: f64, override_max_price: f64) -> f64 {
    base_cap.max(clamp(override_max_price, 0.0, 0.99))
}

fn pair_base_allows_merge_requote(worst_case_pnl: f64) -> bool {
    worst_case_pnl > 1e-9
}

fn pair_base_recovery_uses_exact_order(origin: &str, size: f64, min_shares: f64) -> bool {
    origin.trim() == "PAIR_BASE_RECOVERY" && size + 1e-6 < min_shares.max(1.0)
}

fn pair_submit_tracks_taker_fallback(resolved_order_type: &str) -> bool {
    resolved_order_type.trim().to_ascii_uppercase() != "GTC"
}

#[derive(Debug, Clone, Default)]
struct PairBaseRuntimeState {
    phase: PairBasePhaseState,
    active_pair_id: Option<String>,
    yes_oid: Option<String>,
    no_oid: Option<String>,
    target_qty: f64,
    filled_yes: f64,
    filled_no: f64,
    state_enter_ts: f64,
    risk_exit_latched: bool,
}

#[derive(Debug, Clone, Default)]
struct LadderOrderState {
    key: String,
    asset_id: String,
    role: String,
    level: i64,
    order_id: String,
    price: f64,
    size: f64,
    ts: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MakerOrderKey {
    asset_id: String,
    side: String,
}

impl MakerOrderKey {
    fn buy(asset_id: &str) -> Self {
        Self {
            asset_id: asset_id.trim().to_string(),
            side: "BUY".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum MakerOrderLifecycle {
    #[default]
    Idle,
    SubmitPending,
    Working,
    CancelPending,
}

#[derive(Debug, Clone, Default)]
struct MakerOrderReplaceTarget {
    price: f64,
    size: f64,
    origin: String,
}

#[derive(Debug, Clone, Default)]
struct MakerOrderSlot {
    state: MakerOrderLifecycle,
    order_id: Option<String>,
    price: f64,
    size: f64,
    remaining: f64,
    last_submit_ts: f64,
    last_cancel_ts: f64,
    last_reject_ts: f64,
    consecutive_rejects: u32,
    origin: String,
    replace_target: Option<MakerOrderReplaceTarget>,
}

#[derive(Debug, Clone, Default)]
struct MakerExecProgress {
    applied_qty: f64,
    last_update_ts: f64,
}

#[derive(Debug, Clone, Default)]
struct MakerExecCandidate {
    order_id: String,
    asset_id: String,
    side: String,
    qty: f64,
    price: f64,
    tx_hash: Option<String>,
    trade_id: Option<String>,
    taker_order_id: Option<String>,
    match_time: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct MakerExecRecord {
    canonical_id: String,
    order_id: String,
    qty: f64,
    price: f64,
    asset_id: String,
    side: String,
    aliases: Vec<String>,
    applied_ts: f64,
}

#[derive(Debug, Clone, Default)]
struct MakerExecLedger {
    alias_to_canonical: HashMap<String, String>,
    records: HashMap<String, MakerExecRecord>,
    per_order_applied: HashMap<String, MakerExecProgress>,
}

#[derive(Debug, Clone)]
enum MakerExecApplyResult {
    Applied { canonical_id: String },
    Duplicate { canonical_id: String },
    Conflict { canonical_id: String, reason: String },
    DroppedWeakId { reason: String },
}

#[derive(Debug, Clone, Default)]
struct ApplyFillMutationMeta {
    opened_position: bool,
    closed_position: bool,
    mark_first_entry_fill: bool,
}

#[derive(Debug, Clone, Default)]
struct PairArbPendingImbalance {
    yes_oid: Option<String>,
    no_oid: Option<String>,
    heavy_side: String,
    light_side: String,
    gap_shares: f64,
    created_ts: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SniperPostHedgePolicy {
    HybridTimed,
    HoldToResolution,
    ImmediateUnwind,
}

impl SniperPostHedgePolicy {
    fn from_env() -> Self {
        match std::env::var("SNIPER_POST_HEDGE_POLICY")
            .unwrap_or_else(|_| "HYBRID_TIMED".to_string())
            .trim()
            .to_ascii_uppercase()
            .as_str()
        {
            "HOLD_TO_RESOLUTION" | "HOLD" => Self::HoldToResolution,
            "IMMEDIATE_UNWIND" | "UNWIND" => Self::ImmediateUnwind,
            _ => Self::HybridTimed,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::HybridTimed => "HYBRID_TIMED",
            Self::HoldToResolution => "HOLD_TO_RESOLUTION",
            Self::ImmediateUnwind => "IMMEDIATE_UNWIND",
        }
    }
}

#[derive(Debug, Clone)]
struct SniperStopCertaintyConfig {
    enabled: bool,
    sell_budget_ms: i64,
    sell_max_submits: i64,
    sell_post_wait_ms: i64,
    no_derisk_eps_shares: f64,
    hedge_budget_ms: i64,
    hedge_max_submits: i64,
    hedge_cap_extra_ticks: i64,
    hedge_slip_base_ticks: i64,
    hedge_slip_step_ticks: i64,
    post_hedge_policy: SniperPostHedgePolicy,
    post_hedge_recover_window_seconds: f64,
    post_hedge_max_unwind_submits: i64,
    stop_loss_stale_cycles_to_hedge: i64,
    stop_loss_open_exposure_max_pause_ms: i64,
    hedged_block_new_entries: bool,
}

impl SniperStopCertaintyConfig {
    fn from_env() -> Self {
        let mut out = Self {
            enabled: env_bool("SNIPER_STOP_CERTAINTY_ENABLED", true),
            sell_budget_ms: env_int("SNIPER_STOP_CERTAINTY_SELL_BUDGET_MS", 700),
            sell_max_submits: env_int("SNIPER_STOP_CERTAINTY_SELL_MAX_SUBMITS", 2),
            sell_post_wait_ms: env_int("SNIPER_STOP_CERTAINTY_SELL_POST_WAIT_MS", 150),
            no_derisk_eps_shares: env_float("SNIPER_STOP_CERTAINTY_NO_DERISK_EPS_SHARES", 0.5),
            hedge_budget_ms: env_int("SNIPER_STOP_CERTAINTY_HEDGE_BUDGET_MS", 1200),
            hedge_max_submits: env_int("SNIPER_STOP_CERTAINTY_HEDGE_MAX_SUBMITS", 3),
            hedge_cap_extra_ticks: env_int("SNIPER_STOP_CERTAINTY_HEDGE_CAP_EXTRA_TICKS", 2),
            hedge_slip_base_ticks: env_int("SNIPER_STOP_CERTAINTY_HEDGE_SLIP_BASE_TICKS", 2),
            hedge_slip_step_ticks: env_int("SNIPER_STOP_CERTAINTY_HEDGE_SLIP_STEP_TICKS", 1),
            post_hedge_policy: SniperPostHedgePolicy::from_env(),
            post_hedge_recover_window_seconds: env_float(
                "SNIPER_POST_HEDGE_RECOVER_WINDOW_SECONDS",
                3.0,
            ),
            post_hedge_max_unwind_submits: env_int("SNIPER_POST_HEDGE_MAX_UNWIND_SUBMITS", 2),
            stop_loss_stale_cycles_to_hedge: env_int("SNIPER_STOP_LOSS_STALE_CYCLES_TO_HEDGE", 2),
            stop_loss_open_exposure_max_pause_ms: env_int(
                "SNIPER_STOP_LOSS_OPEN_EXPOSURE_MAX_PAUSE_MS",
                1500,
            ),
            hedged_block_new_entries: env_bool("SNIPER_HEDGED_BLOCK_NEW_ENTRIES", true),
        };
        out.sell_budget_ms = out.sell_budget_ms.clamp(50, 15_000);
        out.sell_max_submits = out.sell_max_submits.clamp(1, 20);
        out.sell_post_wait_ms = out.sell_post_wait_ms.clamp(20, 2_000);
        out.no_derisk_eps_shares = out.no_derisk_eps_shares.clamp(0.01, 50_000.0);
        out.hedge_budget_ms = out.hedge_budget_ms.clamp(50, 20_000);
        out.hedge_max_submits = out.hedge_max_submits.clamp(1, 30);
        out.hedge_cap_extra_ticks = out.hedge_cap_extra_ticks.clamp(0, 100);
        out.hedge_slip_base_ticks = out.hedge_slip_base_ticks.clamp(0, 200);
        out.hedge_slip_step_ticks = out.hedge_slip_step_ticks.clamp(0, 50);
        out.post_hedge_recover_window_seconds =
            out.post_hedge_recover_window_seconds.clamp(0.0, 120.0);
        out.post_hedge_max_unwind_submits = out.post_hedge_max_unwind_submits.clamp(0, 20);
        out.stop_loss_stale_cycles_to_hedge = out.stop_loss_stale_cycles_to_hedge.clamp(1, 20);
        out.stop_loss_open_exposure_max_pause_ms =
            out.stop_loss_open_exposure_max_pause_ms.clamp(100, 30_000);
        out
    }
}

pub struct MakerHedgeCapBot {
    pub cfg: BotConfig,
    pub logger: Arc<dyn LogLike>,
    pub market_slug: String,
    pub signal_hub: Option<Arc<SignalHub>>,
    pub state_file: PathBuf,
    pub state: Arc<Mutex<BotState>>,
    pub start_trade_iso: String,
    pub first_entry_fill_iso: Arc<Mutex<Option<String>>>,
    pub first_entry_reason: Arc<Mutex<Option<String>>>,
    pub pending_entry_reason: Arc<Mutex<Option<String>>>,
    pub active_entry_reason: Arc<Mutex<Option<String>>>,
    pub stop_loss_category: Arc<Mutex<Option<String>>>,
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
    sniper_stop_certainty: SniperStopCertaintyConfig,
    pub condition_id: Option<String>,
    pub market_fees_enabled: Option<bool>,
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
    /// Suspect tracking for reconciliation: (timestamp, api_balance) per asset.
    reconcile_suspect_yes: Arc<Mutex<Option<(f64, f64)>>>,
    reconcile_suspect_no: Arc<Mutex<Option<(f64, f64)>>>,
    reconcile_last_ts: Arc<Mutex<f64>>,
    pub exchange_orders_cache: Arc<Mutex<Vec<Value>>>,
    pub binance_feed: Option<Arc<BinanceFeedService>>,
    pub sniper_filters: Arc<Mutex<SniperFilterEngine>>,
    sniper_filters_persist_enabled: bool,
    sniper_filters_state_path: Option<PathBuf>,
    sniper_filters_persist_min_interval_ms: i64,
    sniper_trade_decision: Arc<Mutex<Option<SniperTradeDecisionRuntime>>>,
    sniper_order_fill_agg: Arc<Mutex<HashMap<String, SniperOrderFillAgg>>>,
    maker_skew_state: Arc<Mutex<MakerSkewArbState>>,
    maker_ladder_open_orders: Arc<Mutex<HashMap<String, LadderOrderState>>>,
    maker_order_slots: Arc<Mutex<HashMap<MakerOrderKey, MakerOrderSlot>>>,
    maker_order_index: Arc<Mutex<HashMap<String, MakerOrderKey>>>,
    maker_exec_ledger: Arc<Mutex<MakerExecLedger>>,
    pair_arb_pending_imbalance: Arc<Mutex<Option<PairArbPendingImbalance>>>,
    pair_base_state: Arc<Mutex<PairBaseRuntimeState>>,
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
        let slug_window_start_ts = market_slug
            .split('-')
            .last()
            .and_then(|s| s.parse::<i64>().ok());
        if let Some(raw_ts) = slug_window_start_ts {
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
        let market_symbol_hint = std::env::var("MARKET_SYMBOL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                market_slug
                    .split('-')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            });
        let sniper_filters = Arc::new(Mutex::new(SniperFilterEngine::new(&market_symbol_hint)));
        let sniper_filters_persist_enabled = env_bool("SNIPER_FILTERS_PERSIST_STATE", true);
        let sniper_filters_persist_min_interval_ms =
            env_int("SNIPER_FILTERS_STATE_WRITE_MIN_INTERVAL_MS", 250).clamp(0, 60_000);
        let sniper_filters_state_path = if sniper_filters_persist_enabled {
            let p = std::env::var("SNIPER_FILTERS_STATE_PATH")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| {
                    let sym = normalize_asset_symbol(&market_symbol_hint);
                    let bot_id = std::env::var("BOT_ID")
                        .unwrap_or_else(|_| "polybot".to_string())
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                        .collect::<String>();
                    format!("state/sniper_filters_state_{}_{}.json", bot_id, sym)
                });
            let path = PathBuf::from(p);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            Some(path)
        } else {
            None
        };

        let mut out = Self {
            cfg,
            logger: bot_logger,
            market_slug: market_slug.to_string(),
            signal_hub,
            state_file,
            state: Arc::new(Mutex::new(state)),
            start_trade_iso,
            first_entry_fill_iso: Arc::new(Mutex::new(None)),
            first_entry_reason: Arc::new(Mutex::new(None)),
            pending_entry_reason: Arc::new(Mutex::new(None)),
            active_entry_reason: Arc::new(Mutex::new(None)),
            stop_loss_category: Arc::new(Mutex::new(None)),
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
            sniper_stop_certainty: SniperStopCertaintyConfig::from_env(),
            condition_id: None,
            market_fees_enabled: None,
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
            reconcile_suspect_yes: Arc::new(Mutex::new(None)),
            reconcile_suspect_no: Arc::new(Mutex::new(None)),
            reconcile_last_ts: Arc::new(Mutex::new(0.0)),
            exchange_orders_cache: Arc::new(Mutex::new(Vec::new())),
            binance_feed: None,
            sniper_filters,
            sniper_filters_persist_enabled,
            sniper_filters_state_path,
            sniper_filters_persist_min_interval_ms,
            sniper_trade_decision: Arc::new(Mutex::new(None)),
            sniper_order_fill_agg: Arc::new(Mutex::new(HashMap::new())),
            maker_skew_state: Arc::new(Mutex::new(MakerSkewArbState::default())),
            maker_ladder_open_orders: Arc::new(Mutex::new(HashMap::new())),
            maker_order_slots: Arc::new(Mutex::new(HashMap::new())),
            maker_order_index: Arc::new(Mutex::new(HashMap::new())),
            maker_exec_ledger: Arc::new(Mutex::new(MakerExecLedger::default())),
            pair_arb_pending_imbalance: Arc::new(Mutex::new(None)),
            pair_base_state: Arc::new(Mutex::new(PairBaseRuntimeState::default())),
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
        out._apply_cfg_overrides_from_env();
        out.logger.info(&format!(
            "[CFG_EFFECTIVE] dry_run={} max_total_cost={:.2} reserve_usd={:.2} min_shares={:.2} clip_shares={:.2} log_every={} market_data_stale={}s stop_buffer={}s",
            out.cfg.dry_run,
            out.cfg.max_total_cost,
            out.cfg.reserve_usd,
            out.cfg.min_shares,
            out.cfg.clip_shares,
            out.cfg.log_every,
            out.cfg.market_data_stale_seconds,
            out.cfg.stop_buffer_seconds
        ));

        if let Some(market) = fetch_market_by_slug(&out.market_slug, Some(&out.logger))? {
            out.market_fees_enabled = market
                .get("feesEnabled")
                .or_else(|| market.get("fees_enabled"))
                .and_then(|v| v.as_bool());
            if let Ok((yes, no, condition)) = parse_tokens_and_condition(&market) {
                out.condition_id = Some(condition.clone());
                out.yes_asset = Some(yes.clone());
                out.no_asset = Some(no.clone());
                if slug_window_start_ts.is_none() {
                    if let Some(st) = market
                        .get("startDate")
                        .and_then(|v| v.as_str())
                        .and_then(iso_to_epoch)
                    {
                        out.start_ts = st;
                    }
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
        if out._pair_base_mode_enabled() {
            let fee_source = if out.market_fees_enabled.is_some() {
                "market"
            } else {
                "env"
            };
            out.logger.info(&format!(
                "[PAIR_BASE][CFG] pair_budget={:.2} merge_budget={:.2} hard_reserve={:.2} fees_enabled={} fee_source={} fee_model={} maker_rebate_bps={:.2}",
                out._pair_base_window_budget(),
                out._pair_base_merge_budget(),
                out._pair_base_hard_reserve(),
                out.market_fees_enabled
                    .unwrap_or_else(|| env_bool("POLY_FEE_MODEL_ENABLED", true)),
                fee_source,
                env_bool("POLY_FEE_MODEL_ENABLED", true),
                env_float("POLY_MAKER_REBATE_BPS", 0.0).max(0.0)
            ));
        }
        out._warm_clob_order_meta_cache();
        out._sniper_filters_load_state();
        out._init_binance_feed_if_needed();

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

    fn _is_sniper_like_mode(exec_mode: &str) -> bool {
        matches!(
            exec_mode,
            "SNIPER"
                | "PROB_SNIPER"
                | "HIGH_PROB"
                | "HIGH_PROB_SNIPER"
                | "FIXED_PROFIT"
                | "SIGNAL_SNIPPER"
                | "SIGNAL_SNIPER"
                | "SIGNAL_SNIPE"
                | "SIGNAL"
        )
    }

    fn _init_binance_feed_if_needed(&mut self) {
        let sniper_needs_feed = self
            .sniper_filters
            .lock()
            .map(|f| f.uses_binance_feed())
            .unwrap_or(false);
        let maker_stretch_enabled = env_bool("MAKER_STRETCH_BIAS_ENABLED", false);
        let maker_skew_mode = self.exec_mode == "MAKER_SKEW_ARB";
        let needs_feed = sniper_needs_feed || (maker_stretch_enabled && maker_skew_mode);
        if !needs_feed {
            return;
        }
        if !Self::_is_sniper_like_mode(&self.exec_mode) && !maker_skew_mode {
            return;
        }
        let cfg = BinanceFeedConfig::from_env();
        self.logger.info(&format!(
            "[BINANCE] feed init venue={:?} symbol={} rest={} ws={}",
            cfg.venue,
            cfg.symbol,
            cfg.rest_base_url,
            cfg.ws_url()
        ));
        let feed = Arc::new(BinanceFeedService::new(
            cfg,
            self.logger.clone(),
            self.stop_flag.clone(),
        ));
        feed.start();
        self.binance_feed = Some(feed);
    }

    fn _sniper_filters_load_state(&self) {
        if !self.sniper_filters_persist_enabled {
            return;
        }
        let Some(path) = &self.sniper_filters_state_path else {
            return;
        };
        let raw = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(_) => return,
        };
        let parsed = match serde_json::from_str::<SniperFilterPersistedState>(&raw) {
            Ok(v) => v,
            Err(e) => {
                self.logger.warning(&format!(
                    "[SNIPER_FILTERS] state load parse failed path={} err={e}",
                    path.display()
                ));
                return;
            }
        };
        if let Ok(mut f) = self.sniper_filters.lock() {
            if f.import_state(parsed) {
                self.logger.info(&format!(
                    "[SNIPER_FILTERS] state loaded from {}",
                    path.display()
                ));
            }
        }
    }

    fn _sniper_filters_save_state(&self, force: bool) {
        if !self.sniper_filters_persist_enabled {
            return;
        }
        let Some(path) = &self.sniper_filters_state_path else {
            return;
        };
        if !force {
            let min_s = (self.sniper_filters_persist_min_interval_ms as f64 / 1000.0).max(0.0);
            let now = now_ts_f64();
            let key = "__sniper_filters_state_next_write_ts";
            if now + 1e-12 < self._runtime_ts_get(key) {
                return;
            }
            self._runtime_ts_set(key, now + min_s);
        }

        let payload = match self.sniper_filters.lock() {
            Ok(f) => f.export_state(),
            Err(_) => return,
        };
        let raw = match serde_json::to_string_pretty(&payload) {
            Ok(v) => v,
            Err(e) => {
                self.logger.warning(&format!(
                    "[SNIPER_FILTERS] state serialize failed path={} err={e}",
                    path.display()
                ));
                return;
            }
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(path, raw) {
            self.logger.warning(&format!(
                "[SNIPER_FILTERS] state write failed path={} err={e}",
                path.display()
            ));
        }
    }

    fn _sniper_filters_ingest_latest_tick(&self) {
        let Some(feed) = &self.binance_feed else {
            return;
        };
        let snap = feed.snapshot();
        let mut changed = false;
        if let Ok(mut f) = self.sniper_filters.lock() {
            if !snap.seed_klines.is_empty()
                && self._runtime_ts_get("__sniper_filters_seed_applied") < 0.5
            {
                f.seed_completed_klines(&snap.seed_klines);
                self._runtime_ts_set("__sniper_filters_seed_applied", 1.0);
                changed = true;
            }
            if let Some(tick) = snap.last_tick {
                if f.on_tick(&tick) {
                    changed = true;
                }
            }
        }
        if changed {
            self._sniper_filters_save_state(false);
        }
    }

    fn _sniper_filter_log(&self, bucket: &str, every_s: f64, msg: &str) {
        if every_s <= 0.0 {
            self.logger.info(msg);
            return;
        }
        let key = format!("__sniper_filter_log_{bucket}");
        let now = now_ts_f64();
        if now >= self._runtime_ts_get(&key) {
            self.logger.info(msg);
            self._runtime_ts_set(&key, now + every_s);
        }
    }

    fn _sniper_filters_eval_entry(
        &self,
        side: &str,
        context: &str,
        seconds_left: f64,
    ) -> Option<SniperFilterDecision> {
        self._sniper_filters_ingest_latest_tick();
        let now_ms = (now_ts_f64() * 1000.0) as i64;
        let (decision, momentum_log_every, breakout_log_every) = match self.sniper_filters.lock() {
            Ok(f) => (
                f.evaluate_entry(side, now_ms),
                f.momentum_log_every_seconds(),
                f.breakout_log_every_seconds(),
            ),
            Err(_) => return None,
        };

        if decision.breakout.applied {
            let b = &decision.breakout;
            self._sniper_filter_log(
                "breakout",
                breakout_log_every,
                &format!(
                    "[BREAKOUT] {} context={} side={} dir={} trig={} reason={} Hk={} Lk={} buf_up={} buf_dn={} persist_ms={} elapsed_ms={} cooldown_ms={} tick_age_ms={} t_left={:.2}s",
                    if decision.allowed { "PASS" } else { "BLOCK" },
                    context,
                    side,
                    b.direction.as_str(),
                    b.triggered,
                    b.reason,
                    b.hk.map(|v| format!("{v:.6}")).unwrap_or_else(|| "na".to_string()),
                    b.lk.map(|v| format!("{v:.6}")).unwrap_or_else(|| "na".to_string()),
                    b.buffer_up
                        .map(|v| format!("{v:.6}"))
                        .unwrap_or_else(|| "na".to_string()),
                    b.buffer_dn
                        .map(|v| format!("{v:.6}"))
                        .unwrap_or_else(|| "na".to_string()),
                    b.persist_ms,
                    b.elapsed_ms,
                    b.cooldown_remaining_ms,
                    b.tick_age_ms,
                    seconds_left
                ),
            );
        }
        if decision.momentum.applied {
            let m = &decision.momentum;
            self._sniper_filter_log(
                "momentum",
                momentum_log_every,
                &format!(
                    "[MOMENTUM] {} context={} side={} reason={} checks={}/{} trend={} slope={} candles={} fast={} slow={} fast_prev={} body_count={} tick_age_ms={} t_left={:.2}s",
                    if decision.allowed { "PASS" } else { "BLOCK" },
                    context,
                    side,
                    m.reason,
                    m.checks_passed,
                    m.required_checks,
                    m.trend_ok,
                    m.slope_ok,
                    m.candles_ok,
                    m.ema_fast_last
                        .map(|v| format!("{v:.6}"))
                        .unwrap_or_else(|| "na".to_string()),
                    m.ema_slow_last
                        .map(|v| format!("{v:.6}"))
                        .unwrap_or_else(|| "na".to_string()),
                    m.ema_fast_prev
                        .map(|v| format!("{v:.6}"))
                        .unwrap_or_else(|| "na".to_string()),
                    m.bullish_or_bearish_count
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "na".to_string()),
                    m.tick_age_ms,
                    seconds_left
                ),
            );
        }
        Some(decision)
    }

    fn _sniper_filters_allow_entry(&self, side: &str, context: &str, seconds_left: f64) -> bool {
        self._sniper_filters_eval_entry(side, context, seconds_left)
            .map(|d| d.allowed)
            .unwrap_or(true)
    }

    fn _sniper_build_breakout_entry_anchor(
        &self,
        side: &str,
        filter_decision: Option<&SniperFilterDecision>,
        decided_at_ms: i64,
        order_id: Option<String>,
    ) -> Option<SniperEntryBreakoutAnchorState> {
        let d = filter_decision?;
        let b = &d.breakout;
        if !(b.applied && b.passed && b.triggered) {
            return None;
        }
        let (Some(hk), Some(lk), Some(buffer_up), Some(buffer_dn)) =
            (b.hk, b.lk, b.buffer_up, b.buffer_dn)
        else {
            return None;
        };
        if b.direction.as_str() == "NONE" {
            return None;
        }
        Some(SniperEntryBreakoutAnchorState {
            side: side.trim().to_ascii_uppercase(),
            trigger_dir: b.direction.as_str().to_string(),
            entry_hk: hk,
            entry_lk: lk,
            entry_buffer_up: buffer_up,
            entry_buffer_dn: buffer_dn,
            triggered_at_ms: b.triggered_at_ms.unwrap_or(0).max(0),
            decided_at_ms: decided_at_ms.max(0),
            decision_spot_price: b.spot_price.unwrap_or(0.0).max(0.0),
            order_id,
        })
    }

    fn _sniper_set_pending_breakout_entry_anchor(
        &self,
        anchor: Option<SniperEntryBreakoutAnchorState>,
    ) {
        if let Ok(mut s) = self.state.lock() {
            s.sniper_pending_breakout_anchor = anchor;
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    fn _sniper_clear_breakout_entry_anchor_state(&self, clear_pending: bool, clear_active: bool) {
        if !clear_pending && !clear_active {
            return;
        }
        if let Ok(mut s) = self.state.lock() {
            if clear_pending {
                s.sniper_pending_breakout_anchor = None;
            }
            if clear_active {
                s.sniper_active_breakout_anchor = None;
            }
            let _ = save_state(&self.state_file, &mut s);
        }
    }

    fn _sniper_activate_breakout_entry_anchor(
        &self,
        side: &str,
    ) -> Option<SniperEntryBreakoutAnchorState> {
        let mut out: Option<SniperEntryBreakoutAnchorState> = None;
        let pos_side = side.trim().to_ascii_uppercase();
        if let Ok(mut s) = self.state.lock() {
            if let Some(active) = s.sniper_active_breakout_anchor.clone() {
                if active.side.trim().to_ascii_uppercase() == pos_side {
                    out = Some(active);
                } else {
                    s.sniper_active_breakout_anchor = None;
                }
            }
            if out.is_none() {
                if let Some(pending) = s.sniper_pending_breakout_anchor.clone() {
                    if pending.side.trim().to_ascii_uppercase() == pos_side {
                        s.sniper_active_breakout_anchor = Some(pending.clone());
                        s.sniper_pending_breakout_anchor = None;
                        out = Some(pending);
                    } else {
                        s.sniper_pending_breakout_anchor = None;
                    }
                }
            }
            let _ = save_state(&self.state_file, &mut s);
        }
        out
    }

    fn _sniper_filters_arm_breakout_invalidation_stop_from_anchor(
        &self,
        anchor: &SniperEntryBreakoutAnchorState,
        context: &str,
        seconds_left: f64,
    ) -> Option<BreakoutInvalidationStopDecision> {
        self._sniper_filters_ingest_latest_tick();
        let now_ms = (now_ts_f64() * 1000.0) as i64;
        let (decision, every_s) = match self.sniper_filters.lock() {
            Ok(mut f) => (
                f.arm_breakout_invalidation_stop_from_anchor(
                    &anchor.side,
                    anchor.entry_hk,
                    anchor.entry_lk,
                    anchor.entry_buffer_up,
                    anchor.entry_buffer_dn,
                    now_ms,
                ),
                f.breakout_invalidation_stop_log_every_seconds(),
            ),
            Err(_) => return None,
        };
        if decision.armed {
            self._sniper_filters_save_state(false);
        }
        let armed_price = decision
            .spot_price
            .or_else(|| {
                if anchor.decision_spot_price > 0.0 {
                    Some(anchor.decision_spot_price)
                } else {
                    None
                }
            })
            .unwrap_or(0.0);
        let (distance_bps, armed_already_invalidated, entry_buffer_dn, entry_buffer_up) =
            if anchor.side.trim().eq_ignore_ascii_case("NO") && anchor.entry_buffer_dn > 0.0 {
                let d =
                    ((armed_price - anchor.entry_buffer_dn) / anchor.entry_buffer_dn) * 10_000.0;
                (Some(d), Some(d > 0.0), Some(anchor.entry_buffer_dn), None)
            } else if anchor.side.trim().eq_ignore_ascii_case("YES") && anchor.entry_buffer_up > 0.0
            {
                let d =
                    ((anchor.entry_buffer_up - armed_price) / anchor.entry_buffer_up) * 10_000.0;
                (Some(d), Some(d > 0.0), None, Some(anchor.entry_buffer_up))
            } else {
                (None, None, None, None)
            };
        let msg = format!(
            "[STOP_BREAKOUT] {} context={} side={} reason={} Hk={} Lk={} buf_up={} buf_dn={} persist_ms={} elapsed_ms={} tick_age_ms={} armed_price={} entry_buffer_dn={} entry_buffer_up={} distance_bps={} armed_already_invalidated={} trigger_dir={} triggered_at_ms={} decided_at_ms={} t_left={:.2}s",
            if decision.armed { "ARM" } else { "SKIP" },
            context,
            anchor.side,
            decision.reason,
            decision
                .hk
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision
                .lk
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision
                .buffer_up
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision
                .buffer_dn
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision.persist_ms,
            decision.elapsed_ms,
            decision.tick_age_ms,
            if armed_price > 0.0 {
                format!("{armed_price:.6}")
            } else {
                "na".to_string()
            },
            entry_buffer_dn
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            entry_buffer_up
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            distance_bps
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "na".to_string()),
            armed_already_invalidated
                .map(|v| v.to_string())
                .unwrap_or_else(|| "na".to_string()),
            anchor.trigger_dir,
            anchor.triggered_at_ms,
            anchor.decided_at_ms,
            seconds_left
        );
        if decision.armed {
            self.logger.info(&msg);
        } else {
            self._sniper_filter_log("stop_breakout", every_s, &msg);
        }
        Some(decision)
    }

    fn _sniper_filters_clear_breakout_invalidation_stop(&self) {
        if let Ok(mut f) = self.sniper_filters.lock() {
            f.clear_breakout_invalidation_stop();
        }
        self._sniper_filters_save_state(false);
    }

    fn _sniper_arm_breakout_invalidation_stop_for_position(
        &self,
        side: &str,
        context: &str,
        seconds_left: f64,
    ) {
        let Some(anchor) = self._sniper_activate_breakout_entry_anchor(side) else {
            self._sniper_filters_clear_breakout_invalidation_stop();
            let every_s = self
                .sniper_filters
                .lock()
                .map(|f| f.breakout_invalidation_stop_log_every_seconds())
                .unwrap_or(1.0);
            self._sniper_filter_log(
                "stop_breakout",
                every_s,
                &format!(
                    "[STOP_BREAKOUT] SKIP context={} side={} reason=no_entry_anchor t_left={:.2}s",
                    context, side, seconds_left
                ),
            );
            return;
        };
        let _ = self._sniper_filters_arm_breakout_invalidation_stop_from_anchor(
            &anchor,
            context,
            seconds_left,
        );
    }

    fn _sniper_filters_eval_breakout_invalidation_stop(
        &self,
        side: &str,
        context: &str,
        seconds_left: f64,
    ) -> Option<BreakoutInvalidationStopDecision> {
        self._sniper_filters_ingest_latest_tick();
        let now_ms = (now_ts_f64() * 1000.0) as i64;
        let (decision, every_s) = match self.sniper_filters.lock() {
            Ok(mut f) => (
                f.evaluate_breakout_invalidation_stop(side, now_ms),
                f.breakout_invalidation_stop_log_every_seconds(),
            ),
            Err(_) => return None,
        };
        if matches!(
            decision.reason.as_str(),
            "tracking" | "triggered" | "breakout_valid"
        ) {
            self._sniper_filters_save_state(false);
        }

        let msg = format!(
            "[STOP_BREAKOUT] {} context={} side={} reason={} armed={} fired={} Hk={} Lk={} buf_up={} buf_dn={} persist_ms={} elapsed_ms={} tick_age_ms={} t_left={:.2}s",
            if decision.fired {
                "FIRE"
            } else if matches!(
                decision.reason.as_str(),
                "disabled" | "no_anchor" | "stale" | "side_not_directional" | "side_mismatch"
            ) {
                "SKIP"
            } else {
                "TRACK"
            },
            context,
            side,
            decision.reason,
            decision.armed,
            decision.fired,
            decision
                .hk
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision
                .lk
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision
                .buffer_up
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision
                .buffer_dn
                .map(|v| format!("{v:.6}"))
                .unwrap_or_else(|| "na".to_string()),
            decision.persist_ms,
            decision.elapsed_ms,
            decision.tick_age_ms,
            seconds_left
        );
        if decision.fired {
            self.logger.info(&msg);
        } else {
            self._sniper_filter_log("stop_breakout", every_s, &msg);
        }
        Some(decision)
    }

    fn _sniper_submit_order_type_from_origin(origin: &str) -> String {
        let u = origin.trim().to_ascii_uppercase();
        if u.contains("FAK") {
            "FAK".to_string()
        } else if u.contains("FOK") {
            "FOK".to_string()
        } else if u.contains("GTC") || u.contains("LIMIT") {
            "GTC".to_string()
        } else {
            String::new()
        }
    }

    fn _sniper_order_kind_from_origin(origin: &str) -> String {
        let u = origin.trim().to_ascii_uppercase();
        if u.starts_with("TAKER") {
            "taker".to_string()
        } else if u.contains("LIMIT") || u.contains("MAKER") || u.contains("POSTONLY") {
            "maker".to_string()
        } else {
            String::new()
        }
    }

    fn _sniper_apply_fill_stats_to_decision(
        &self,
        data: &mut TradeDecisionUpsert,
        agg: &SniperOrderFillAgg,
    ) {
        if agg.qty <= 1e-12 || agg.notional <= 1e-12 {
            return;
        }
        let avg_fill = agg.notional / agg.qty;
        data.qty_filled = Some(agg.qty);
        data.fill_price_avg = Some(avg_fill);
        if let Some(mid) = data.pm_mid {
            if mid > 1e-12 {
                data.slippage_bps_vs_mid = Some(((avg_fill - mid) / mid) * 10_000.0);
            }
        }
        let fee_rate = env_float("SNIPER_FEE_RATE", 0.0).max(0.0);
        data.fees_paid = Some(agg.notional * fee_rate);
    }

    fn _sniper_trade_decision_record_submit(
        &self,
        order_id: &str,
        side: &str,
        seconds_left: f64,
        asset_id: &str,
        bid: f64,
        ask: f64,
        limit_price: f64,
        qty_requested: f64,
        filter_decision: Option<&SniperFilterDecision>,
    ) {
        if order_id.trim().is_empty() {
            return;
        }
        let mut row = TradeDecisionUpsert {
            t_left_seconds: Some(seconds_left),
            submit_side: Some(side.to_string()),
            pm_best_bid: if bid > 0.0 { Some(bid) } else { None },
            pm_best_ask: if ask > 0.0 { Some(ask) } else { None },
            limit_price_submitted: if limit_price > 0.0 {
                Some(limit_price)
            } else {
                None
            },
            qty_requested: if qty_requested > 0.0 {
                Some(qty_requested)
            } else {
                None
            },
            ..TradeDecisionUpsert::default()
        };
        if bid > 0.0 && ask > 0.0 {
            let mid = 0.5 * (bid + ask);
            let spread_abs = (ask - bid).max(0.0);
            row.pm_mid = Some(mid);
            row.pm_spread_abs = Some(spread_abs);
            row.pm_spread_pct = if mid > 1e-12 {
                Some((spread_abs / mid) * 100.0)
            } else {
                None
            };
        }
        if !asset_id.trim().is_empty() {
            let depth_max_age = env_float("SNIPER_ENTRY_GATE_MAX_AGE_SECONDS", 1.0).max(0.1);
            if bid > 0.0 {
                row.pm_depth_bid_1tick =
                    Some(self._cum_depth(asset_id, "bids", bid, Some(16), Some(depth_max_age)));
            }
            if ask > 0.0 {
                row.pm_depth_ask_1tick =
                    Some(self._cum_depth(asset_id, "asks", ask, Some(16), Some(depth_max_age)));
            }
        }
        if let Some(decision) = filter_decision {
            let mut tick_age: Option<i64> = None;
            if decision.momentum.applied {
                let m = &decision.momentum;
                row.momentum_checks_passed = Some(m.checks_passed as i64);
                row.momentum_checks_required = Some(m.required_checks as i64);
                row.momentum_trend_ok = Some(m.trend_ok);
                row.momentum_slope_ok = Some(m.slope_ok);
                row.momentum_candles_ok = Some(m.candles_ok);
                row.momentum_ema_fast_last = m.ema_fast_last;
                row.momentum_ema_slow_last = m.ema_slow_last;
                row.momentum_ema_fast_prev = m.ema_fast_prev;
                row.momentum_body_count = m.bullish_or_bearish_count.map(|v| v as i64);
                if m.tick_age_ms < i64::MAX / 4 {
                    tick_age = Some(m.tick_age_ms);
                }
            }
            if decision.breakout.applied {
                let b = &decision.breakout;
                row.breakout_dir = Some(b.direction.as_str().to_string());
                row.breakout_triggered = Some(b.triggered);
                row.breakout_reason = Some(b.reason.clone());
                row.breakout_hk = b.hk;
                row.breakout_lk = b.lk;
                row.breakout_buf_up = b.buffer_up;
                row.breakout_buf_dn = b.buffer_dn;
                row.breakout_persist_ms = Some(b.persist_ms);
                row.breakout_elapsed_ms = Some(b.elapsed_ms);
                row.breakout_cooldown_ms = Some(b.cooldown_remaining_ms);
                if b.tick_age_ms < i64::MAX / 4 {
                    tick_age = Some(match tick_age {
                        Some(prev) => prev.min(b.tick_age_ms),
                        None => b.tick_age_ms,
                    });
                }
            }
            row.tick_age_ms = tick_age;
        }

        if let Some(ctx) = self._get_order_execution_context(order_id) {
            let origin = ctx
                .get("origin")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !origin.trim().is_empty() {
                row.submit_origin = Some(origin.clone());
                row.submit_order_type = Some(Self::_sniper_submit_order_type_from_origin(&origin));
                row.order_type = Some(Self::_sniper_order_kind_from_origin(&origin));
            }
            let val_i64 = |key: &str| -> Option<i64> {
                ctx.get(key)
                    .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
            };
            row.decide_to_send_us = val_i64("decision_to_post_start_us");
            row.send_to_ack_us = val_i64("post_start_to_post_end_us");
            row.decide_to_ack_us = val_i64("decision_to_post_end_us");
        }
        if row.order_type.as_deref().unwrap_or("").is_empty() {
            row.order_type = Some("unknown".to_string());
        }
        if row.submit_order_type.as_deref().unwrap_or("").is_empty() {
            row.submit_order_type = Some(String::new());
        }

        if let Ok(fill_map) = self.sniper_order_fill_agg.lock() {
            if let Some(agg) = fill_map.get(order_id) {
                self._sniper_apply_fill_stats_to_decision(&mut row, agg);
            }
        }

        if let Ok(mut holder) = self.sniper_trade_decision.lock() {
            *holder = Some(SniperTradeDecisionRuntime {
                order_id: Some(order_id.to_string()),
                data: row,
            });
        }
    }

    fn _sniper_record_order_fill(&self, order_id: &str, price: f64, qty: f64) {
        if order_id.trim().is_empty() || price <= 0.0 || qty <= 0.0 {
            return;
        }
        let mut agg_row = SniperOrderFillAgg::default();
        if let Ok(mut m) = self.sniper_order_fill_agg.lock() {
            let e = m.entry(order_id.to_string()).or_default();
            e.qty += qty.max(0.0);
            e.notional += qty.max(0.0) * price.max(0.0);
            agg_row = e.clone();
        }
        if let Ok(mut snap) = self.sniper_trade_decision.lock() {
            if let Some(cur) = snap.as_mut() {
                if cur.order_id.as_deref() == Some(order_id) {
                    self._sniper_apply_fill_stats_to_decision(&mut cur.data, &agg_row);
                }
            }
        }
    }

    fn _sniper_hedge_oid_key(order_id: &str) -> String {
        format!("__sniper_hedge_oid_{order_id}")
    }

    fn _sniper_hedge_last_remaining_key(order_id: &str) -> String {
        format!("__sniper_hedge_last_remaining_{order_id}")
    }

    fn _sniper_is_hedge_order(&self, order_id: &str) -> bool {
        if order_id.trim().is_empty() {
            return false;
        }
        self._runtime_ts_get(&Self::_sniper_hedge_oid_key(order_id)) > 0.0
    }

    fn _sniper_mark_hedge_order(&self, order_id: &str) {
        if order_id.trim().is_empty() {
            return;
        }
        self._runtime_ts_set(&Self::_sniper_hedge_oid_key(order_id), now_ts_f64());
        self._runtime_ts_set(&Self::_sniper_hedge_last_remaining_key(order_id), -1.0);
    }

    fn _sniper_clear_hedge_order(&self, order_id: &str) {
        if order_id.trim().is_empty() {
            return;
        }
        self._runtime_ts_set(&Self::_sniper_hedge_oid_key(order_id), 0.0);
        self._runtime_ts_set(&Self::_sniper_hedge_last_remaining_key(order_id), 0.0);
    }

    fn _sniper_log_hedge_order_progress(
        &self,
        order_id: &str,
        asset_id: &str,
        side: &str,
        filled: f64,
        remaining: f64,
        total: f64,
        source: &str,
        status: &str,
    ) {
        if !self._sniper_is_hedge_order(order_id) {
            return;
        }
        let rem = remaining.max(0.0);
        let key = Self::_sniper_hedge_last_remaining_key(order_id);
        let last_rem = self._runtime_ts_get(&key);
        if last_rem >= 0.0 && (last_rem - rem).abs() <= 1e-9 {
            return;
        }
        self._runtime_ts_set(&key, rem);
        let aid_tail: String = asset_id
            .chars()
            .rev()
            .take(6)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let fill_pct = if total > 1e-9 {
            (filled.max(0.0) / total.max(1e-9)) * 100.0
        } else {
            0.0
        };
        self.logger.info(&format!(
            "[SNIPER][HEDGE][{source}] oid={}.. asset={aid_tail} side={} filled={:.6} remaining={:.6} total={:.6} fill_pct={:.2}% status={}",
            order_id.chars().take(10).collect::<String>(),
            side,
            filled.max(0.0),
            rem,
            total.max(0.0),
            fill_pct,
            status
        ));
    }

    fn _sniper_stop_loss_fail_key(asset_id: &str) -> String {
        format!("__sniper_stop_loss_sell_failures_{asset_id}")
    }

    fn _sniper_normalize_stop_loss_mode(mode: &str) -> String {
        let mut m = mode.trim().to_ascii_uppercase();
        if matches!(m.as_str(), "STOP_LIMIT" | "STOPLIMIT") {
            m = "LIMIT".to_string();
        }
        if matches!(m.as_str(), "STOP_HEDGE" | "HEDGE_STOP") {
            m = "HEDGE".to_string();
        }
        if matches!(
            m.as_str(),
            "STOP_MARKET" | "STOPMARKET" | "TAKER" | "AGGRESSIVE"
        ) {
            m = "MARKET".to_string();
        }
        m
    }

    fn _sniper_stop_loss_mode(&self) -> String {
        let raw = std::env::var("SNIPER_STOP_LOSS_MODE")
            .ok()
            .or_else(|| std::env::var("SNIPER_STOP_LESS_MODE").ok())
            .unwrap_or_else(|| "MARKET".to_string());
        Self::_sniper_normalize_stop_loss_mode(&raw)
    }

    fn _sniper_stop_loss_fallback_mode(&self) -> String {
        let mut raw = std::env::var("SNIPER_STOP_LOSS_MODE_FALLBACK")
            .unwrap_or_default()
            .to_ascii_uppercase();
        if raw.trim().is_empty() && env_bool("SNIPER_STOP_LOSS_MODE_FALLBACK_HEDGE", false) {
            raw = "HEDGE".to_string();
        }
        Self::_sniper_normalize_stop_loss_mode(&raw)
    }

    fn _sniper_stop_loss_fallback_fails(&self) -> f64 {
        env_int("SNIPER_STOP_LOSS_MODE_FALLBACK_FAILS", 5).max(1) as f64
    }

    fn _sniper_stop_loss_reset_failures(&self, asset_id: &str) {
        if asset_id.trim().is_empty() {
            return;
        }
        let key = Self::_sniper_stop_loss_fail_key(asset_id);
        self._runtime_ts_set(&key, 0.0);
    }

    fn _sniper_stop_loss_record_sell_failure(
        &self,
        pos: &Value,
        asset_id: &str,
        current_mode: &str,
        reason_u: &str,
        trigger: &str,
    ) {
        if reason_u != "STOP_LOSS" || asset_id.trim().is_empty() {
            return;
        }
        let mode = Self::_sniper_normalize_stop_loss_mode(current_mode);
        if mode == "HEDGE" {
            return;
        }
        let threshold = self._sniper_stop_loss_fallback_fails();
        let fallback_mode = self._sniper_stop_loss_fallback_mode();
        let fail_key = Self::_sniper_stop_loss_fail_key(asset_id);
        let prev_fails = self._runtime_ts_get(&fail_key).max(0.0);
        if !fallback_mode.trim().is_empty() && prev_fails + 1e-9 >= threshold {
            // Once fallback is active, stop counting "sell failures" to avoid inflating the counter.
            return;
        }
        let fails = prev_fails + 1.0;
        self._runtime_ts_set(&fail_key, fails);
        let now = now_ts_f64();
        let log_key = format!("__sniper_stop_loss_fallback_log_until_{asset_id}");
        if now >= self._runtime_ts_get(&log_key) {
            self.logger.warning(&format!(
                "[SNIPER][STOP_LOSS] sell_failed trigger={trigger} mode={mode} fails={:.0}/{:.0} fallback_mode={}",
                fails,
                threshold,
                if fallback_mode.is_empty() {
                    "<none>"
                } else {
                    fallback_mode.as_str()
                }
            ));
            self._runtime_ts_set(&log_key, now + 2.0);
        }
        if !fallback_mode.is_empty() && fails + 1e-9 >= threshold && fallback_mode == "HEDGE" {
            self._sniper_maybe_exit_hedge(pos, reason_u, "stop_loss_fallback_threshold");
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

    fn _rtds_entry_gate_eval_side(
        &self,
        side: &str,
        seconds_left: f64,
        context: &str,
    ) -> (bool, bool) {
        if !env_bool("RTDS_ENTRY_GATE_ENABLED", false) {
            return (true, false);
        }
        let side = side.trim().to_ascii_uppercase();
        if !matches!(side.as_str(), "YES" | "NO") {
            return (true, false);
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
                return (allow_missing, false);
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
            return (false, true);
        }

        self._rtds_gate_log(
            "pass",
            &format!(
                "[RTDS_GATE] {} pass: side={} diff_price={:+.6} required={:.6} t_left={:.2}s",
                context, side, diff_price, min_req, seconds_left
            ),
        );
        (true, false)
    }

    fn _rtds_entry_gate_allows_side(&self, side: &str, seconds_left: f64, context: &str) -> bool {
        self._rtds_entry_gate_eval_side(side, seconds_left, context)
            .0
    }

    fn _sniper_force_entry_diff_signal(&self, seconds_left: f64) -> Option<(String, f64)> {
        let min_abs = env_float("SNIPER_FORCE_ENTRY_MIN_DIFF_PRICE", 0.0).max(0.0);
        if min_abs <= 0.0 {
            return None;
        }

        let require_market_match = env_bool("RTDS_ENTRY_GATE_REQUIRE_MARKET_MATCH", true);
        let max_age_seconds = env_float(
            "SNIPER_FORCE_ENTRY_MIN_DIFF_PRICE_MAX_AGE_SECONDS",
            env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 2.0),
        )
        .max(0.05);
        let context = "SNIPER_FORCE_DIFF_ENTRY";
        let (diff_price, age_ms, ts_ms) =
            match self._rtds_gate_snapshot(context, max_age_seconds, require_market_match) {
                Ok(v) => v,
                Err(reason) => {
                    self._rtds_gate_log(
                        "force_diff_snapshot_block",
                        &format!("[RTDS_GATE] {context} blocked: {reason}"),
                    );
                    return None;
                }
            };

        if diff_price.abs() + 1e-12 < min_abs {
            self._rtds_gate_log(
                "force_diff_threshold",
                &format!(
                    "[RTDS_GATE] {context} blocked: |diff_price|={:.6} < required={:.6} t_left={:.2}s",
                    diff_price.abs(),
                    min_abs,
                    seconds_left
                ),
            );
            return None;
        }

        let side = if diff_price >= 0.0 { "YES" } else { "NO" };
        self._rtds_gate_log(
            "force_diff_pass",
            &format!(
                "[RTDS_GATE] {context} trigger: side={} diff_price={:+.6} required={:.6} ts_ms={} age_ms={} t_left={:.2}s",
                side, diff_price, min_abs, ts_ms, age_ms, seconds_left
            ),
        );
        Some((side.to_string(), diff_price))
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

        let side = if diff_price >= 0.0 { "YES" } else { "NO" };
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

    fn _default_entry_reason(&self) -> String {
        match self.exec_mode.as_str() {
            "SIGNAL_SNIPPER" | "SIGNAL_SNIPER" | "SIGNAL_SNIPE" | "SIGNAL" => {
                "SIGNAL_ENTRY".to_string()
            }
            "SNIPER" | "PROB_SNIPER" | "HIGH_PROB" | "HIGH_PROB_SNIPER" | "FIXED_PROFIT" => {
                "SNIPER_ENTRY".to_string()
            }
            _ => "MAKER_ENTRY".to_string(),
        }
    }

    fn _active_entry_reason_or_default(&self) -> String {
        self.active_entry_reason
            .lock()
            .ok()
            .and_then(|v| v.clone())
            .or_else(|| self.first_entry_reason.lock().ok().and_then(|v| v.clone()))
            .unwrap_or_else(|| self._default_entry_reason())
    }

    fn _env_positive_float_if_set(name: &str) -> Option<f64> {
        let val = env_float(name, f64::NAN);
        if val.is_finite() && val > 0.0 {
            Some(val)
        } else {
            None
        }
    }

    fn _sniper_tp_sl_overrides_for_entry_reason(entry_reason: &str) -> (Option<f64>, Option<f64>) {
        let reason = entry_reason.trim().to_ascii_uppercase();
        match reason.as_str() {
            "SNIPER_FORCE_DIFF_ENTRY" | "RTDS_DIFF_TIME_OVERRIDE" => (
                Self::_env_positive_float_if_set("SNIPER_FORCE_DIFF_ENTRY_TAKE_PROFIT_PCT"),
                Self::_env_positive_float_if_set("SNIPER_FORCE_DIFF_ENTRY_STOP_LOSS_PCT"),
            ),
            "SNIPER_ENTRY_FORCE" | "SNIPER_FORCE_ENTRY" => (
                Self::_env_positive_float_if_set("SNIPER_FORCE_ENTRY_TAKE_PROFIT_PCT").or_else(
                    || Self::_env_positive_float_if_set("SNIPER_ENTRY_FORCE_TAKE_PROFIT_PCT"),
                ),
                Self::_env_positive_float_if_set("SNIPER_FORCE_ENTRY_STOP_LOSS_PCT").or_else(
                    || Self::_env_positive_float_if_set("SNIPER_ENTRY_FORCE_STOP_LOSS_PCT"),
                ),
            ),
            _ => (None, None),
        }
    }

    fn _sniper_tp_sl_for_entry_reason(&self, entry_reason: &str) -> (f64, f64) {
        let base_tp = env_float("SNIPER_TAKE_PROFIT_PCT", 0.01).max(0.0);
        let base_sl = env_float("SNIPER_STOP_LOSS_PCT", 0.02).max(0.0);
        let (tp_override, sl_override) =
            Self::_sniper_tp_sl_overrides_for_entry_reason(entry_reason);
        (
            tp_override.unwrap_or(base_tp),
            sl_override.unwrap_or(base_sl),
        )
    }

    fn _force_diff_entry_reason(reason: &str) -> bool {
        matches!(
            reason.trim().to_ascii_uppercase().as_str(),
            "SNIPER_FORCE_DIFF_ENTRY" | "RTDS_DIFF_TIME_OVERRIDE"
        )
    }

    fn _should_bypass_rtds_hold_for_take_profit(
        &self,
        entry_reason: &str,
        cost: f64,
        pnl_pct: f64,
        take_profit_pct: f64,
    ) -> bool {
        if cost <= 1e-12 || !Self::_force_diff_entry_reason(entry_reason) {
            return false;
        }
        let (tp_override, _) = Self::_sniper_tp_sl_overrides_for_entry_reason(entry_reason);
        tp_override.is_some() && pnl_pct + 1e-12 >= take_profit_pct
    }

    fn _entry_reason_from_candidate(&self, cand: &Value) -> String {
        let entry_reason = cand
            .get("entry_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        if !entry_reason.is_empty() {
            return entry_reason;
        }
        let entry_mode = cand
            .get("entry_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        if entry_mode == "FORCE" {
            return "SNIPER_ENTRY_FORCE".to_string();
        }
        if entry_mode == "SIGNAL" {
            return "SIGNAL_ENTRY".to_string();
        }
        self._default_entry_reason()
    }

    fn _set_pending_entry_reason(&self, reason: &str) {
        if let Ok(mut pending) = self.pending_entry_reason.lock() {
            *pending = Some(reason.to_string());
        }
    }

    fn _take_pending_entry_reason(&self) -> Option<String> {
        self.pending_entry_reason
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
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
        let mut now_flat = false;
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
            now_flat = (s.q_yes + s.q_no) <= 1e-12;
        }
        if changed {
            if now_flat {
                if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                    *active_reason = None;
                }
            }
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
                        if self.exec_mode == "MAKER_SKEW_ARB" && self._pair_base_mode_enabled() {
                            self._maker_pair_base_risk_exit_step("near_expiry", total_cost, true);
                        } else {
                            self.logger.info(&format!(
                                "Near expiry ({seconds_left:.0}s). Forcing emergency hedge before stopping."
                            ));
                            self._emergency_taker_hedge_step(delta, "near_expiry");
                        }
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
                    if self.exec_mode == "MAKER_SKEW_ARB" && self._pair_base_mode_enabled() {
                        let stale_key = "__pair_base_feed_stale_since";
                        let mut stale_since = self._runtime_ts_get(stale_key);
                        if stale_since <= 0.0 {
                            stale_since = now;
                            self._runtime_ts_set(stale_key, stale_since);
                        }
                        let delta = qy - qn;
                        let stale_for = (now - stale_since).max(0.0);
                        let trigger_after = 2.0 * self.cfg.market_data_stale_seconds.max(1) as f64;
                        if delta.abs() >= self.cfg.min_shares && stale_for >= trigger_after {
                            self._maker_pair_base_risk_exit_step(
                                &format!("feed_stale({stale_for:.1}s)"),
                                total_cost,
                                true,
                            );
                        }
                    }
                    continue;
                } else if in_feed_pause {
                    self.logger.info("FEED OK -> resume.");
                    in_feed_pause = false;
                }
                self._runtime_ts_set("__pair_base_feed_stale_since", 0.0);

                if self.exec_mode == "MAKER_SKEW_ARB" {
                    self._maker_skew_arb_step(now, qy, qn, total_cost);
                    continue;
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

            if self.stop_flag.load(Ordering::SeqCst) {
                drop(ws);
                break;
            }
            let _ = ws.close(None);
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

    fn _maker_dbg_idle(&self, msg: &str, key: &str) {
        self._dbg_maker(msg, key, Some(5.0));
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

    pub fn _reconcile_state_from_positions(&self, reason: &str) -> bool {
        // Primary source is Data API positions. Legacy balance-based mode can be
        // explicitly enabled, but mixed per-leg fallback is intentionally disabled.
        let use_data_api = env_bool("RECONCILE_USE_DATA_API", true);
        let use_legacy_balance = env_bool("MISMATCH_RECONCILE_FROM_BALANCE", false);
        if !use_data_api && !use_legacy_balance {
            return false;
        }

        let now = now_ts_f64();
        let min_interval = env_float("RECONCILE_MIN_INTERVAL_SECONDS", 5.0).max(0.1);
        if let Ok(last) = self.reconcile_last_ts.lock() {
            if now - *last < min_interval {
                return false;
            }
        }

        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return false,
        };
        let (yes_bal, no_bal) = if use_data_api {
            let yes_pos = self._get_position_size_data_api(yes);
            let no_pos = self._get_position_size_data_api(no);
            match (yes_pos, no_pos) {
                (Some(y), Some(n)) => (y, n),
                _ => return false,
            }
        } else {
            // Legacy mode only.
            let by = self._get_balance_allowance_conditional_cached(yes, 0.0);
            let bn = self._get_balance_allowance_conditional_cached(no, 0.0);
            match (by, bn) {
                (Some((yb, _)), Some((nb, _))) => (yb, nb),
                _ => return false,
            }
        };

        // Skip if either source returned invalid data.
        if yes_bal < -0.5 || no_bal < -0.5 {
            return false;
        }

        if let Ok(mut last) = self.reconcile_last_ts.lock() {
            *last = now;
        }

        let mut changed = false;
        let y_ba = self._best_bid_ask(yes);
        let n_ba = self._best_bid_ask(no);
        let tick = self.cfg.tick.max(0.0001);
        let mut y_ask = y_ba.map(|(_, a)| a).unwrap_or(0.0);
        let mut n_ask = n_ba.map(|(_, a)| a).unwrap_or(0.0);
        let mut y_bid = y_ba.map(|(b, _)| b).unwrap_or(0.0);
        let mut n_bid = n_ba.map(|(b, _)| b).unwrap_or(0.0);
        y_ask = clamp(if y_ask > 0.0 { y_ask } else { 0.99 }, tick, 0.99);
        n_ask = clamp(if n_ask > 0.0 { n_ask } else { 0.99 }, tick, 0.99);
        y_bid = clamp(if y_bid > 0.0 { y_bid } else { tick }, tick, 0.99);
        n_bid = clamp(if n_bid > 0.0 { n_bid } else { tick }, tick, 0.99);
        let sell_credit_mult = self.reconcile_sell_credit_mult.max(0.0);

        let mut new_q_yes;
        let mut new_q_no;
        let mut new_c_yes;
        let mut new_c_no;
        if let Ok(s) = self.state.lock() {
            new_q_yes = s.q_yes;
            new_q_no = s.q_no;
            new_c_yes = s.c_yes;
            new_c_no = s.c_no;
        } else {
            return false;
        }

        let confirm_delay = env_float("RECONCILE_CONFIRM_DELAY_SECONDS", 3.0).max(0.5);
        let never_zero = env_bool("RECONCILE_NEVER_ZERO_WITHOUT_CONFIRM", true);
        let delta_threshold = self.cfg.min_shares.max(1e-6);

        // --- YES reconciliation ---
        if yes_bal > new_q_yes + delta_threshold {
            // Data API shows MORE than we track — missed fills → trust immediately.
            let dq = yes_bal - new_q_yes;
            new_c_yes += dq * y_ask;
            new_q_yes = yes_bal;
            changed = true;
            // Clear suspect since we're adjusting upward
            if let Ok(mut s) = self.reconcile_suspect_yes.lock() {
                *s = None;
            }
        } else if yes_bal + delta_threshold < new_q_yes {
            // Data API shows LESS than we track — possible stale data or real sell.
            // Require dual-confirmation: discrepancy must persist across two checks.
            let dq = new_q_yes - yes_bal;

            // Safety: never zero out a large position from a single API check.
            if never_zero && yes_bal < 1e-6 && new_q_yes >= self.cfg.min_shares {
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_yes.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - yes_bal).abs() < 1e-6 => {
                            // Same zero reading twice — check delay
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            // First time seeing this discrepancy — record and wait
                            *suspect = Some((now, yes_bal));
                            self.logger.warning(&format!(
                                "[RECONCILE] YES suspect: internal={new_q_yes:.2} api={yes_bal:.2} — waiting {confirm_delay:.1}s to confirm ({reason})"
                            ));
                        }
                    }
                }
                if !confirmed {
                    // Don't apply yet — wait for confirmation
                } else {
                    new_c_yes -= dq * y_bid * sell_credit_mult;
                    new_q_yes = yes_bal;
                    changed = true;
                    self.logger.warning(&format!(
                        "[RECONCILE] YES confirmed zero after delay: internal→{yes_bal:.2} ({reason})"
                    ));
                }
            } else {
                // Non-zero downward adjustment — apply with standard dual-confirm
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_yes.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - yes_bal).abs() < delta_threshold => {
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            *suspect = Some((now, yes_bal));
                        }
                    }
                }
                if confirmed {
                    new_c_yes -= dq * y_bid * sell_credit_mult;
                    new_q_yes = yes_bal;
                    changed = true;
                }
            }
        } else {
            // Consistent — clear suspect
            if let Ok(mut s) = self.reconcile_suspect_yes.lock() {
                *s = None;
            }
        }

        // --- NO reconciliation (same logic) ---
        if no_bal > new_q_no + delta_threshold {
            let dq = no_bal - new_q_no;
            new_c_no += dq * n_ask;
            new_q_no = no_bal;
            changed = true;
            if let Ok(mut s) = self.reconcile_suspect_no.lock() {
                *s = None;
            }
        } else if no_bal + delta_threshold < new_q_no {
            let dq = new_q_no - no_bal;

            if never_zero && no_bal < 1e-6 && new_q_no >= self.cfg.min_shares {
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_no.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - no_bal).abs() < 1e-6 => {
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            *suspect = Some((now, no_bal));
                            self.logger.warning(&format!(
                                "[RECONCILE] NO suspect: internal={new_q_no:.2} api={no_bal:.2} — waiting {confirm_delay:.1}s to confirm ({reason})"
                            ));
                        }
                    }
                }
                if confirmed {
                    new_c_no -= dq * n_bid * sell_credit_mult;
                    new_q_no = no_bal;
                    changed = true;
                    self.logger.warning(&format!(
                        "[RECONCILE] NO confirmed zero after delay: internal→{no_bal:.2} ({reason})"
                    ));
                }
            } else {
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_no.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - no_bal).abs() < delta_threshold => {
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            *suspect = Some((now, no_bal));
                        }
                    }
                }
                if confirmed {
                    new_c_no -= dq * n_bid * sell_credit_mult;
                    new_q_no = no_bal;
                    changed = true;
                }
            }
        } else {
            if let Ok(mut s) = self.reconcile_suspect_no.lock() {
                *s = None;
            }
        }

        if !changed {
            return false;
        }
        new_c_yes = new_c_yes.max(0.0);
        new_c_no = new_c_no.max(0.0);
        if let Ok(mut s) = self.state.lock() {
            s.q_yes = new_q_yes;
            s.q_no = new_q_no;
            s.c_yes = new_c_yes;
            s.c_no = new_c_no;
            let _ = save_state(&self.state_file, &mut s);
        }
        let tag = if reason.trim().is_empty() {
            String::new()
        } else {
            format!(" ({reason})")
        };
        self.logger.warning(&format!(
            "Reconciled state from positions{} qYES={new_q_yes:.6} qNO={new_q_no:.6} total_cost={:.4}",
            tag,
            new_c_yes + new_c_no
        ));
        true
    }

    pub fn _chunked_unwind_heavy_leg(&self, delta: f64, reason: &str) {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let _ = self._reconcile_state_from_positions(&format!("unwind:{reason}"));
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
        self.cfg.max_total_cost = env_float("MAX_TOTAL_COST", self.cfg.max_total_cost);
        self.cfg.reserve_usd = env_float("RESERVE_USD", self.cfg.reserve_usd);
        self.cfg.dry_run = env_bool("DRY_RUN", self.cfg.dry_run);
        self.cfg.log_every = env_int("LOG_EVERY_SECONDS", self.cfg.log_every) as i64;
        self.cfg.market_data_stale_seconds = env_int(
            "MARKET_DATA_STALE_SECONDS",
            self.cfg.market_data_stale_seconds,
        ) as i64;
        self.cfg.stop_buffer_seconds =
            env_int("STOP_BUFFER_SECONDS", self.cfg.stop_buffer_seconds) as i64;
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

    fn _parse_clip_set_from_env(&self, key: &str, default_values: &[i64]) -> Vec<i64> {
        let raw = std::env::var(key).unwrap_or_default();
        let mut out: Vec<i64> = raw
            .split(',')
            .filter_map(|v| v.trim().parse::<i64>().ok())
            .filter(|v| *v > 0)
            .collect();
        if out.is_empty() {
            out = default_values
                .iter()
                .copied()
                .filter(|v| *v > 0)
                .collect::<Vec<i64>>();
        }
        out.sort();
        out.dedup();
        out
    }

    fn _maker_price_bucket(price: f64) -> String {
        if price <= 0.0 {
            "NA".to_string()
        } else if price <= 0.20 {
            "LE_020".to_string()
        } else if price <= 0.35 {
            "020_035".to_string()
        } else if price <= 0.65 {
            "035_065".to_string()
        } else {
            "GT_065".to_string()
        }
    }

    fn _maker_clip_bucket(clip: f64) -> String {
        if clip <= 0.0 {
            "NA".to_string()
        } else if clip <= 12.0 {
            "SMALL".to_string()
        } else if clip <= 36.0 {
            "MID".to_string()
        } else {
            "LARGE".to_string()
        }
    }

    fn _maker_pick_clip_size_for_price(&self, price: f64, peak_window: bool) -> f64 {
        let small =
            self._parse_clip_set_from_env("MAKER_CLIP_SET_SMALL", &[2, 3, 5, 7, 8, 9, 10, 11, 12]);
        let mid = self._parse_clip_set_from_env("MAKER_CLIP_SET_MID", &[16, 21, 30, 35, 36]);
        let large =
            self._parse_clip_set_from_env("MAKER_CLIP_SET_LARGE", &[40, 42, 45, 48, 54, 56]);

        let mut rng = rand::thread_rng();
        let mut pick = |pool: &[i64], fallback: i64| -> i64 {
            pool.choose(&mut rng).copied().unwrap_or(fallback.max(1))
        };
        let mut clip = if price <= 0.20 {
            pick(&large, 40)
        } else if price <= 0.35 {
            pick(
                &[mid.clone(), large.clone()].concat(),
                mid.first().copied().unwrap_or(16),
            )
        } else if price <= 0.65 {
            pick(
                &[small.clone(), mid.clone()].concat(),
                mid.first().copied().unwrap_or(16),
            )
        } else {
            pick(
                &[small.clone(), mid.clone()].concat(),
                small.first().copied().unwrap_or(8),
            )
        } as f64;

        if peak_window {
            clip *= env_float("MAKER_SKEW_PEAK_CLIP_MULT", 1.25).clamp(1.0, 3.0);
        }
        clip.max(self.cfg.min_shares.max(1.0))
    }

    fn _maker_skew_update_state(&self, now: f64, q_yes: f64, q_no: f64, total_cost: f64) {
        if let Ok(mut st) = self.maker_skew_state.lock() {
            if st.window_start_ts <= 0 {
                if let Ok(s) = self.state.lock() {
                    st.window_start_ts = s.maker_skew_window_start_ts;
                    st.last_decision_ts = s.maker_skew_last_decision_ts;
                    st.unhedged_since = s.maker_skew_unhedged_since;
                }
            }
            if st.window_start_ts <= 0 {
                st.window_start_ts = self.start_ts;
            }
            if st.window_start_ts != self.start_ts {
                *st = MakerSkewArbState {
                    window_start_ts: self.start_ts,
                    ..MakerSkewArbState::default()
                };
            }
            st.cost_total = total_cost.max(0.0);
            st.shares_up = q_yes.max(0.0);
            st.shares_down = q_no.max(0.0);
            let (downside, upside, skew_ratio) =
                Self::_maker_payoff_envelope(st.shares_up, st.shares_down, st.cost_total);
            st.downside = downside;
            st.upside = upside;
            st.skew_ratio = skew_ratio;
            if (st.shares_up - st.shares_down).abs() >= self.cfg.min_shares.max(1.0) {
                if st.unhedged_since <= 0.0 {
                    st.unhedged_since = now;
                }
            } else {
                st.unhedged_since = 0.0;
            }
        }
        let persist_every = env_float("MAKER_SKEW_STATE_PERSIST_SECONDS", 2.0).max(0.2);
        let key = "__maker_skew_state_persist_at";
        if now >= self._runtime_ts_get(key) {
            let rec = self
                .maker_skew_state
                .lock()
                .map(|st| (st.window_start_ts, st.last_decision_ts, st.unhedged_since))
                .ok();
            if let Some((wts, lts, uts)) = rec {
                if let Ok(mut s) = self.state.lock() {
                    s.maker_skew_window_start_ts = wts;
                    s.maker_skew_last_decision_ts = lts;
                    s.maker_skew_unhedged_since = uts;
                    let _ = save_state(&self.state_file, &mut s);
                }
            }
            self._runtime_ts_set(key, now + persist_every);
        }
    }

    fn _maker_poly_fee_estimate(
        &self,
        qty: f64,
        price: f64,
        is_maker: bool,
        model_enabled: bool,
    ) -> f64 {
        let fee_rate = env_float("POLY_FEE_RATE", 0.25).max(0.0);
        let exponent = env_float("POLY_FEE_EXPONENT", 2.0).clamp(0.0, 8.0);
        let maker_rebate_bps = env_float("POLY_MAKER_REBATE_BPS", 0.0).max(0.0);
        Self::_maker_poly_fee_formula(
            qty,
            price,
            fee_rate,
            exponent,
            maker_rebate_bps,
            is_maker,
            model_enabled,
        )
    }

    fn _maker_pair_edge_after_fees(&self, qty: f64, p_yes: f64, p_no: f64, is_maker: bool) -> f64 {
        let model_enabled = env_bool("POLY_FEE_MODEL_ENABLED", true);
        let gross = qty * (1.0 - p_yes - p_no);
        let fee_yes = self._maker_poly_fee_estimate(qty, p_yes, is_maker, model_enabled);
        let fee_no = self._maker_poly_fee_estimate(qty, p_no, is_maker, model_enabled);
        gross - fee_yes - fee_no
    }

    fn _maker_single_inflight_enabled(&self) -> bool {
        env_bool("MAKER_SINGLE_INFLIGHT_PER_SIDE", true)
    }

    fn _maker_submit_pending_ttl_seconds(&self) -> f64 {
        env_float("MAKER_SUBMIT_PENDING_TTL_SECONDS", 6.0).max(0.5)
    }

    fn _maker_cancel_pending_ttl_seconds(&self) -> f64 {
        env_float("MAKER_CANCEL_PENDING_TTL_SECONDS", 3.0).max(0.5)
    }

    fn _maker_working_missing_ttl_seconds(&self) -> f64 {
        env_float("MAKER_WORKING_MISSING_TTL_SECONDS", 12.0).max(1.0)
    }

    fn _maker_replace_min_interval_seconds(&self) -> f64 {
        env_float("MAKER_REPLACE_MIN_INTERVAL_SECONDS", 0.5).max(0.0)
    }

    fn _maker_submit_reject_cooldown_seconds(&self) -> f64 {
        env_float("MAKER_SUBMIT_REJECT_COOLDOWN_SECONDS", 5.0).max(0.0)
    }

    fn _pair_arb_imbalance_enter_shares(&self) -> f64 {
        env_float(
            "PAIR_ARB_IMBALANCE_ENTER_SHARES",
            self.cfg.min_shares.max(1.0),
        )
        .max(0.0)
    }

    fn _pair_arb_imbalance_release_shares(&self) -> f64 {
        env_float("PAIR_ARB_IMBALANCE_RELEASE_SHARES", 1.0)
            .max(0.0)
            .min(self._pair_arb_imbalance_enter_shares())
    }

    fn _maker_actual_inventory(&self) -> (f64, f64) {
        self.state
            .lock()
            .map(|s| (s.q_yes.max(0.0), s.q_no.max(0.0)))
            .unwrap_or((0.0, 0.0))
    }

    fn _maker_projected_gap_from_inventory(
        q_yes: f64,
        q_no: f64,
        unsettled_yes: f64,
        unsettled_no: f64,
        buy_side: &str,
        add_size: f64,
    ) -> f64 {
        let side = buy_side.trim().to_ascii_uppercase();
        let add = add_size.max(0.0);
        let proj_yes = q_yes.max(0.0) + unsettled_yes.max(0.0) + if side == "YES" { add } else { 0.0 };
        let proj_no = q_no.max(0.0) + unsettled_no.max(0.0) + if side == "NO" { add } else { 0.0 };
        (proj_yes - proj_no).abs()
    }

    fn _maker_projected_gap_after_buy(
        &self,
        asset_id: &str,
        add_size: f64,
    ) -> Option<(f64, f64, f64, f64, f64, f64)> {
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return None,
        };
        let side = if asset_id == yes {
            "YES"
        } else if asset_id == no {
            "NO"
        } else {
            return None;
        };
        let (q_yes, q_no) = self._maker_actual_inventory();
        let (unsettled_yes, unsettled_no) = self._maker_recovery_unsettled_buy_risks();
        let current_gap = (q_yes - q_no).abs();
        let projected_gap = Self::_maker_projected_gap_from_inventory(
            q_yes,
            q_no,
            unsettled_yes,
            unsettled_no,
            side,
            add_size,
        );
        Some((
            current_gap,
            projected_gap,
            q_yes,
            q_no,
            unsettled_yes,
            unsettled_no,
        ))
    }

    fn _maker_effective_inventory(&self) -> (f64, f64) {
        let (q_yes, q_no) = self._maker_actual_inventory();
        if !env_bool("MAKER_EFFECTIVE_Q_INCLUDE_OPEN_BUYS", true) {
            return (q_yes, q_no);
        }
        let yes_open = self
            .yes_asset
            .as_deref()
            .map(|aid| self._maker_order_open_buy_remaining(aid))
            .unwrap_or(0.0);
        let no_open = self
            .no_asset
            .as_deref()
            .map(|aid| self._maker_order_open_buy_remaining(aid))
            .unwrap_or(0.0);
        (q_yes + yes_open, q_no + no_open)
    }

    fn _maker_trade_exec_candidate(
        &self,
        msg: &Value,
        maker_leg: &Value,
    ) -> Option<MakerExecCandidate> {
        let order_id = maker_leg
            .get("order_id")
            .or_else(|| maker_leg.get("orderId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let asset_id = maker_leg
            .get("asset_id")
            .or_else(|| maker_leg.get("assetId"))
            .or_else(|| maker_leg.get("token_id"))
            .or_else(|| maker_leg.get("tokenId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let side = maker_leg
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        let qty = Self::_value_f64(
            maker_leg
                .get("matched_amount")
                .or_else(|| maker_leg.get("matchedAmount"))
                .or_else(|| maker_leg.get("size"))
                .or_else(|| maker_leg.get("filled")),
        )
        .unwrap_or(0.0);
        let price = Self::_value_f64(maker_leg.get("price")).unwrap_or(0.0);
        if order_id.is_empty()
            || asset_id.is_empty()
            || !matches!(side.as_str(), "BUY" | "SELL")
            || qty <= 0.0
            || price <= 0.0
        {
            return None;
        }
        let tx_hash = msg
            .get("transaction_hash")
            .or_else(|| msg.get("transactionHash"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let trade_id = msg
            .get("id")
            .or_else(|| msg.get("trade_id"))
            .or_else(|| msg.get("tradeId"))
            .or_else(|| msg.get("tradeID"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let taker_order_id = msg
            .get("taker_order_id")
            .or_else(|| msg.get("takerOrderId"))
            .or_else(|| msg.get("taker_orderId"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let match_time = msg
            .get("match_time")
            .or_else(|| msg.get("matchTime"))
            .or_else(|| msg.get("timestamp"))
            .or_else(|| msg.get("ts"))
            .and_then(|v| match v {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty());
        Some(MakerExecCandidate {
            order_id,
            asset_id,
            side,
            qty,
            price,
            tx_hash,
            trade_id,
            taker_order_id,
            match_time,
        })
    }

    fn _maker_trade_exec_aliases(candidate: &MakerExecCandidate) -> Vec<String> {
        let mut aliases: Vec<String> = Vec::new();
        if let Some(tx_hash) = candidate.tx_hash.as_deref() {
            aliases.push(format!(
                "maker_tx:{}:{}:{:.8}:{:.8}",
                candidate.order_id, tx_hash, candidate.qty, candidate.price
            ));
        }
        if let Some(trade_id) = candidate.trade_id.as_deref() {
            aliases.push(format!("maker_trade:{}:{}", candidate.order_id, trade_id));
        }
        if let (Some(taker_oid), Some(match_time)) = (
            candidate.taker_order_id.as_deref(),
            candidate.match_time.as_deref(),
        ) {
            aliases.push(format!(
                "maker_match:{}:{}:{}:{:.8}:{:.8}",
                candidate.order_id, taker_oid, match_time, candidate.qty, candidate.price
            ));
        }
        aliases
    }

    fn _maker_exec_alias_kind(exec_id: &str) -> &'static str {
        if exec_id.starts_with("maker_tx:") {
            "tx"
        } else if exec_id.starts_with("maker_trade:") {
            "trade"
        } else if exec_id.starts_with("maker_match:") {
            "match"
        } else {
            "unknown"
        }
    }

    fn _maker_exec_record_matches(record: &MakerExecRecord, candidate: &MakerExecCandidate) -> bool {
        const EPS: f64 = 1e-9;
        record.order_id == candidate.order_id
            && record.asset_id == candidate.asset_id
            && record.side == candidate.side
            && (record.qty - candidate.qty).abs() <= EPS
            && (record.price - candidate.price).abs() <= EPS
    }

    fn _maker_exec_order_sum(ledger: &MakerExecLedger, order_id: &str) -> f64 {
        if order_id.trim().is_empty() {
            return 0.0;
        }
        ledger
            .records
            .values()
            .filter(|rec| rec.order_id == order_id)
            .map(|rec| rec.qty.max(0.0))
            .sum::<f64>()
    }

    fn _maker_exec_attach_aliases(
        ledger: &mut MakerExecLedger,
        canonical_id: &str,
        aliases: &[String],
    ) {
        let mut clean_aliases: Vec<String> = aliases
            .iter()
            .filter(|alias| !alias.trim().is_empty())
            .cloned()
            .collect();
        clean_aliases.dedup();
        for alias in &clean_aliases {
            ledger
                .alias_to_canonical
                .insert(alias.clone(), canonical_id.to_string());
        }
        if let Some(record) = ledger.records.get_mut(canonical_id) {
            for alias in clean_aliases {
                if !record.aliases.iter().any(|v| v == &alias) {
                    record.aliases.push(alias);
                }
            }
        }
    }

    fn _maker_exec_applied_qty(&self, order_id: &str) -> f64 {
        if order_id.trim().is_empty() {
            return 0.0;
        }
        self.maker_exec_ledger
            .lock()
            .ok()
            .and_then(|ledger| ledger.per_order_applied.get(order_id).cloned())
            .map(|rec| rec.applied_qty.max(0.0))
            .unwrap_or(0.0)
    }

    fn _maker_commit_exec_fill(&self, candidate: MakerExecCandidate) -> MakerExecApplyResult {
        const EPS: f64 = 1e-9;
        let aliases = Self::_maker_trade_exec_aliases(&candidate);
        if aliases.is_empty() {
            return MakerExecApplyResult::DroppedWeakId {
                reason: "no_strong_alias".to_string(),
            };
        }

        let now = now_ts_f64();
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return MakerExecApplyResult::Conflict {
                    canonical_id: aliases[0].clone(),
                    reason: "state_lock_failed".to_string(),
                }
            }
        };
        let mut ledger = match self.maker_exec_ledger.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return MakerExecApplyResult::Conflict {
                    canonical_id: aliases[0].clone(),
                    reason: "maker_exec_ledger_lock_failed".to_string(),
                }
            }
        };

        let mut canonical_id = aliases
            .iter()
            .find_map(|alias| ledger.alias_to_canonical.get(alias).cloned());
        if canonical_id.is_none() {
            canonical_id = aliases
                .iter()
                .find(|alias| state.seen_trade_keys.iter().any(|seen| seen == *alias))
                .cloned();
        }
        let canonical_id = canonical_id.unwrap_or_else(|| aliases[0].clone());

        if let Some(existing) = ledger.records.get(&canonical_id).cloned() {
            if !Self::_maker_exec_record_matches(&existing, &candidate) {
                return MakerExecApplyResult::Conflict {
                    canonical_id,
                    reason: format!(
                        "alias_resolved_to_existing_record_mismatch order_id={} qty={:.8} price={:.8} asset={} side={}",
                        existing.order_id, existing.qty, existing.price, existing.asset_id, existing.side
                    ),
                };
            }
            Self::_maker_exec_attach_aliases(&mut ledger, &existing.canonical_id, &aliases);
            return MakerExecApplyResult::Duplicate {
                canonical_id: existing.canonical_id,
            };
        }

        if state.seen_trade_keys.iter().any(|seen| seen == &canonical_id)
            || aliases
                .iter()
                .any(|alias| state.seen_trade_keys.iter().any(|seen| seen == alias))
        {
            return MakerExecApplyResult::Duplicate { canonical_id };
        }

        let order_sum_before = Self::_maker_exec_order_sum(&ledger, &candidate.order_id);
        let applied_before = ledger
            .per_order_applied
            .get(&candidate.order_id)
            .map(|rec| rec.applied_qty.max(0.0))
            .unwrap_or(0.0);
        if (order_sum_before - applied_before).abs() > EPS {
            self.logger.warning(&format!(
                "[FILL][MAKER_INVARIANT] oid={}.. applied={applied_before:.8} expected={order_sum_before:.8} stage=pre_apply",
                candidate.order_id.chars().take(10).collect::<String>()
            ));
            return MakerExecApplyResult::Conflict {
                canonical_id,
                reason: format!(
                    "pre_apply_invariant_mismatch applied={applied_before:.8} expected={order_sum_before:.8}"
                ),
            };
        }

        let Some(meta) = self._apply_fill_locked_nodedupe(
            &mut state,
            &candidate.asset_id,
            candidate.price,
            candidate.qty,
            &candidate.side,
        ) else {
            return MakerExecApplyResult::Conflict {
                canonical_id,
                reason: "apply_fill_locked_nodedupe_failed".to_string(),
            };
        };

        state.seen_trade_keys.push(canonical_id.clone());
        let _ = save_state(&self.state_file, &mut state);
        drop(state);

        let record = MakerExecRecord {
            canonical_id: canonical_id.clone(),
            order_id: candidate.order_id.clone(),
            qty: candidate.qty,
            price: candidate.price,
            asset_id: candidate.asset_id.clone(),
            side: candidate.side.clone(),
            aliases: Vec::new(),
            applied_ts: now,
        };
        ledger.records.insert(canonical_id.clone(), record);
        Self::_maker_exec_attach_aliases(&mut ledger, &canonical_id, &aliases);
        let entry = ledger
            .per_order_applied
            .entry(candidate.order_id.clone())
            .or_default();
        entry.applied_qty += candidate.qty.max(0.0);
        entry.last_update_ts = now;

        let order_sum_after = Self::_maker_exec_order_sum(&ledger, &candidate.order_id);
        let applied_after = ledger
            .per_order_applied
            .get(&candidate.order_id)
            .map(|rec| rec.applied_qty.max(0.0))
            .unwrap_or(0.0);
        if (order_sum_after - applied_after).abs() > EPS {
            self.logger.warning(&format!(
                "[FILL][MAKER_INVARIANT] oid={}.. applied={applied_after:.8} expected={order_sum_after:.8} stage=post_apply",
                candidate.order_id.chars().take(10).collect::<String>()
            ));
            drop(ledger);
            self._apply_fill_finalize(meta);
            return MakerExecApplyResult::Conflict {
                canonical_id,
                reason: format!(
                    "post_apply_invariant_mismatch applied={applied_after:.8} expected={order_sum_after:.8}"
                ),
            };
        }
        drop(ledger);

        self._apply_fill_finalize(meta);
        MakerExecApplyResult::Applied { canonical_id }
    }

    fn _pair_arb_set_pending_imbalance(
        &self,
        yes_oid: Option<&str>,
        no_oid: Option<&str>,
        heavy_side: &str,
        light_side: &str,
        gap_shares: f64,
    ) {
        let gap = gap_shares.max(0.0);
        if gap <= 1e-9 {
            return;
        }
        let now = now_ts_f64();
        if let Ok(mut holder) = self.pair_arb_pending_imbalance.lock() {
            *holder = Some(PairArbPendingImbalance {
                yes_oid: yes_oid.map(|s| s.to_string()).filter(|s| !s.trim().is_empty()),
                no_oid: no_oid.map(|s| s.to_string()).filter(|s| !s.trim().is_empty()),
                heavy_side: heavy_side.trim().to_ascii_uppercase(),
                light_side: light_side.trim().to_ascii_uppercase(),
                gap_shares: gap,
                created_ts: now,
            });
        }
        self.logger.info(&format!(
            "[MAKER_SKEW][ARB] pending imbalance set heavy={} light={} gap={gap:.2}",
            heavy_side.trim().to_ascii_uppercase(),
            light_side.trim().to_ascii_uppercase()
        ));
    }

    fn _pair_arb_clear_pending_if_resolved(&self) {
        let release = self._pair_arb_imbalance_release_shares();
        let now = now_ts_f64();
        let (q_yes, q_no) = self
            .state
            .lock()
            .map(|s| (s.q_yes.max(0.0), s.q_no.max(0.0)))
            .unwrap_or((0.0, 0.0));
        let gap = (q_yes - q_no).abs();
        let mut cleared = false;
        let mut heavy = "YES".to_string();
        let mut light = "NO".to_string();
        let mut pending_age_s = 0.0;
        let mut yes_oid = "?".to_string();
        let mut no_oid = "?".to_string();
        if q_no > q_yes {
            heavy = "NO".to_string();
            light = "YES".to_string();
        }
        if let Ok(mut holder) = self.pair_arb_pending_imbalance.lock() {
            if let Some(pending) = holder.as_mut() {
                pending.gap_shares = gap;
                pending.heavy_side = heavy.clone();
                pending.light_side = light.clone();
                pending_age_s = (now - pending.created_ts).max(0.0);
                yes_oid = pending
                    .yes_oid
                    .as_deref()
                    .map(|s| s.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "?".to_string());
                no_oid = pending
                    .no_oid
                    .as_deref()
                    .map(|s| s.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "?".to_string());
                if gap <= release + 1e-6 {
                    *holder = None;
                    cleared = true;
                }
            }
        }
        if cleared {
            self.logger.info(&format!(
                "[MAKER_SKEW][ARB] pending imbalance cleared gap={gap:.2} release={release:.2} age={pending_age_s:.1}s yes_oid={} no_oid={}",
                yes_oid,
                no_oid
            ));
        }
    }

    fn _pair_arb_pending_active(&self) -> bool {
        self._pair_arb_clear_pending_if_resolved();
        self.pair_arb_pending_imbalance
            .lock()
            .map(|p| p.is_some())
            .unwrap_or(false)
    }

    fn _maker_recovery_unsettled_buy_risk(&self, asset_id: &str) -> f64 {
        if !self._maker_single_inflight_enabled() || asset_id.trim().is_empty() {
            return 0.0;
        }
        let key = MakerOrderKey::buy(asset_id);
        let slot = self._maker_order_slot_get(&key);
        let now = now_ts_f64();
        let settle_grace = self
            ._maker_working_missing_ttl_seconds()
            .max(self._maker_cancel_pending_ttl_seconds())
            .max(self._maker_submit_pending_ttl_seconds())
            .max(1.0);

        let mut risk = if matches!(
            slot.state,
            MakerOrderLifecycle::Working
                | MakerOrderLifecycle::SubmitPending
                | MakerOrderLifecycle::CancelPending
        ) {
            if slot.remaining > 0.0 {
                slot.remaining.max(0.0)
            } else {
                slot.size.max(0.0)
            }
        } else {
            0.0
        };

        // Late fills can still arrive shortly after local cancel/idle transitions. Preserve
        // the last known order size as unsettled risk for a short grace window.
        if slot.order_id.is_none()
            && slot.state == MakerOrderLifecycle::Idle
            && slot.last_cancel_ts > 0.0
            && now - slot.last_cancel_ts < settle_grace
        {
            risk = risk.max(slot.remaining.max(slot.size).max(0.0));
        }

        let state_open_risk = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.open_orders.get(asset_id).cloned())
            .filter(|oo| {
                oo.order_id.is_some()
                    && oo.ts.map(|ts| now - ts < settle_grace).unwrap_or(false)
            })
            .and_then(|oo| oo.size)
            .unwrap_or(0.0)
            .max(0.0);
        risk.max(state_open_risk)
    }

    fn _maker_recovery_unsettled_buy_risks(&self) -> (f64, f64) {
        let unsettled_yes = self
            .yes_asset
            .as_deref()
            .map(|aid| self._maker_recovery_unsettled_buy_risk(aid))
            .unwrap_or(0.0);
        let unsettled_no = self
            .no_asset
            .as_deref()
            .map(|aid| self._maker_recovery_unsettled_buy_risk(aid))
            .unwrap_or(0.0);
        (unsettled_yes, unsettled_no)
    }

    fn _maker_recovery_light_requote_ready(&self, asset_id: &str) -> bool {
        if asset_id.trim().is_empty() || !self._maker_single_inflight_enabled() {
            return false;
        }
        let key = MakerOrderKey::buy(asset_id);
        let slot = self._maker_order_slot_get(&key);
        if slot.order_id.is_some() || slot.state != MakerOrderLifecycle::Idle {
            return false;
        }
        if slot.last_cancel_ts <= 0.0 {
            return false;
        }
        let now = now_ts_f64();
        let cooldown = self
            ._maker_cancel_pending_ttl_seconds()
            .max(self._maker_replace_min_interval_seconds())
            .max(0.5);
        now - slot.last_cancel_ts >= cooldown
    }

    fn _maker_recovery_light_refresh_reason(&self, asset_id: &str) -> Option<String> {
        if asset_id.trim().is_empty() || !self._maker_single_inflight_enabled() {
            return None;
        }
        let key = MakerOrderKey::buy(asset_id);
        let slot = self._maker_order_slot_get(&key);
        if slot.state != MakerOrderLifecycle::Working || slot.order_id.is_none() {
            return None;
        }
        let now = now_ts_f64();
        let stale = env_int("STALE_SECONDS", self.cfg.stale_seconds).max(1) as f64;
        let age = (now - slot.last_submit_ts).max(0.0);
        if age + 1e-6 < stale {
            return None;
        }
        let (invalid, inv_reason) = self._quotes_invalidated();
        if invalid {
            return Some(format!("recovery quote invalidated: {inv_reason}"));
        }
        let (bid, _ask) = self._best_bid_ask(asset_id).unwrap_or((0.0, 0.0));
        if bid > 0.0 && slot.price > 0.0 {
            let moved_ticks = (bid - slot.price).abs() / self.cfg.tick.max(0.0001);
            let replace_ticks = env_int(
                "REPLACE_IF_PRICE_MOVES_TICKS",
                self.cfg.replace_if_price_moves_ticks,
            )
            .max(1) as f64;
            if moved_ticks >= replace_ticks {
                return Some(format!("recovery refresh bid_move={moved_ticks:.1}t"));
            }
        }
        let max_age = (stale * 3.0).max(5.0);
        if age >= max_age {
            return Some(format!("recovery stale age={age:.1}s"));
        }
        None
    }

    fn _maker_recovery_mode_snapshot(
        &self,
    ) -> (bool, f64, String, String, Option<String>, f64) {
        self._pair_arb_clear_pending_if_resolved();
        let pending_active = self
            .pair_arb_pending_imbalance
            .lock()
            .map(|p| p.is_some())
            .unwrap_or(false);
        let (q_yes, q_no) = self._maker_actual_inventory();
        let actual_gap = (q_yes - q_no).abs();
        let (unsettled_yes, unsettled_no) = self._maker_recovery_unsettled_buy_risks();
        let was_active = self._runtime_ts_get("__maker_recovery_mode_active") > 0.0;
        let enter = self._pair_arb_imbalance_enter_shares();
        let release = self._pair_arb_imbalance_release_shares();
        // Determine heavy/light from actual fills; fall back to persisted direction when balanced
        let (heavy_side, light_side) = if actual_gap > 1e-6 {
            if q_yes > q_no {
                ("YES", "NO")
            } else {
                ("NO", "YES")
            }
        } else if was_active {
            // Fills balanced but recovery still active — use persisted direction
            if self._runtime_ts_get("__maker_recovery_heavy_yes") > 0.5 {
                ("YES", "NO")
            } else {
                ("NO", "YES")
            }
        } else {
            if q_yes >= q_no {
                ("YES", "NO")
            } else {
                ("NO", "YES")
            }
        };
        // Only heavy-side unsettled risk can keep recovery alive
        let unsettled_heavy = if heavy_side == "YES" {
            unsettled_yes
        } else {
            unsettled_no
        };
        let active = if pending_active {
            true
        } else if was_active {
            // Stay: actual gap above release OR heavy-side unsettled risk still exists
            actual_gap > release + 1e-6 || unsettled_heavy > 1e-6
        } else {
            // Enter: actual filled gap only — unsettled risk alone cannot trigger entry
            actual_gap + 1e-6 >= enter
        };
        let light_asset = if light_side == "YES" {
            self.yes_asset.clone()
        } else {
            self.no_asset.clone()
        };
        (
            active,
            actual_gap,
            heavy_side.to_string(),
            light_side.to_string(),
            light_asset,
            unsettled_heavy,
        )
    }

    fn _maker_order_slot_get(&self, key: &MakerOrderKey) -> MakerOrderSlot {
        self.maker_order_slots
            .lock()
            .ok()
            .and_then(|m| m.get(key).cloned())
            .unwrap_or_default()
    }

    fn _maker_order_is_live(
        &self,
        asset_id: &str,
        expected_oid: Option<&str>,
        max_age_s: f64,
    ) -> bool {
        if !self._maker_single_inflight_enabled() {
            return false;
        }
        let key = MakerOrderKey::buy(asset_id);
        let slot = self._maker_order_slot_get(&key);
        if !matches!(
            slot.state,
            MakerOrderLifecycle::Working | MakerOrderLifecycle::SubmitPending
        ) {
            return false;
        }
        let Some(slot_oid) = &slot.order_id else {
            return false;
        };
        if let Some(expected) = expected_oid {
            if slot_oid != expected {
                return false;
            }
        }
        let age = now_ts_f64() - slot.last_submit_ts;
        if age > max_age_s || age < 0.0 {
            return false;
        }
        true
    }

    fn _maker_order_clear_index_for_key(&self, key: &MakerOrderKey) {
        if let Ok(mut idx) = self.maker_order_index.lock() {
            idx.retain(|_, v| v != key);
        }
    }

    fn _maker_order_open_buy_remaining(&self, asset_id: &str) -> f64 {
        if !self._maker_single_inflight_enabled() {
            return 0.0;
        }
        let key = MakerOrderKey::buy(asset_id);
        let slot = self._maker_order_slot_get(&key);
        if matches!(
            slot.state,
            MakerOrderLifecycle::Working
                | MakerOrderLifecycle::SubmitPending
                | MakerOrderLifecycle::CancelPending
        ) {
            if slot.remaining > 0.0 {
                slot.remaining.max(0.0)
            } else {
                slot.size.max(0.0)
            }
        } else {
            0.0
        }
    }

    fn _maker_order_on_cancel_ack_by_order_id(&self, order_id: &str) {
        if !self._maker_single_inflight_enabled() || order_id.trim().is_empty() {
            return;
        }
        let key = self
            .maker_order_index
            .lock()
            .ok()
            .and_then(|idx| idx.get(order_id).cloned());
        let Some(key) = key else {
            return;
        };
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            if slot.order_id.as_deref() == Some(order_id) {
                slot.state = MakerOrderLifecycle::Idle;
                slot.order_id = None;
                slot.last_cancel_ts = now_ts_f64();
                slot.replace_target = None;
            }
        }
        if let Ok(mut idx) = self.maker_order_index.lock() {
            idx.remove(order_id);
        }
    }

    fn _maker_order_on_submit_ack(
        &self,
        order_id: &str,
        key: &MakerOrderKey,
        price: f64,
        size: f64,
        origin: &str,
    ) {
        if !self._maker_single_inflight_enabled() || order_id.trim().is_empty() {
            return;
        }
        let now = now_ts_f64();
        let mut prev_oid: Option<String> = None;
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            prev_oid = slot.order_id.clone();
            slot.state = MakerOrderLifecycle::Working;
            slot.order_id = Some(order_id.to_string());
            slot.price = price;
            slot.size = size.max(0.0);
            slot.remaining = size.max(0.0);
            slot.last_submit_ts = now;
            slot.origin = origin.to_string();
            slot.replace_target = None;
            slot.consecutive_rejects = 0;
        }
        if let Ok(mut idx) = self.maker_order_index.lock() {
            if let Some(prev) = prev_oid {
                if prev != order_id {
                    idx.remove(&prev);
                }
            }
            idx.insert(order_id.to_string(), key.clone());
        }
        if key.side == "BUY" && !key.asset_id.trim().is_empty() {
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    key.asset_id.clone(),
                    OpenOrderState {
                        order_id: Some(order_id.to_string()),
                        price: Some(price),
                        size: Some(size.max(0.0)),
                        ts: Some(now),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
        }
    }

    fn _maker_order_on_submit_reject(&self, key: &MakerOrderKey, reason: &str) {
        if !self._maker_single_inflight_enabled() {
            return;
        }
        let now = now_ts_f64();
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            slot.last_reject_ts = now;
            slot.consecutive_rejects = slot.consecutive_rejects.saturating_add(1);
            if slot.order_id.is_some() {
                slot.state = MakerOrderLifecycle::Working;
            } else {
                slot.state = MakerOrderLifecycle::Idle;
                slot.replace_target = None;
            }
        }
        if !reason.trim().is_empty() {
            self.logger.warning(&format!(
                "[MAKER_ORD] submit reject asset={} side={} reason={reason}",
                key.asset_id, key.side
            ));
        }
    }

    fn _maker_order_request_cancel(&self, key: &MakerOrderKey, reason: &str) -> bool {
        if !self._maker_single_inflight_enabled() {
            return false;
        }
        let now = now_ts_f64();
        let slot = self._maker_order_slot_get(key);
        let Some(oid) = slot.order_id.clone() else {
            return false;
        };
        if slot.state == MakerOrderLifecycle::CancelPending
            && now - slot.last_cancel_ts < self._maker_cancel_pending_ttl_seconds()
        {
            return false;
        }
        if now - slot.last_cancel_ts < self._maker_replace_min_interval_seconds() {
            return false;
        }
        if !self._cancel(&oid) {
            return false;
        }
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            if let Some(s) = slots.get_mut(key) {
                if s.order_id.as_deref() == Some(oid.as_str()) {
                    s.state = MakerOrderLifecycle::CancelPending;
                    s.last_cancel_ts = now;
                }
            }
        }
        if !reason.trim().is_empty() {
            self.logger.info(&format!(
                "[MAKER_ORD] cancel requested asset={} side={} oid={}.. ({reason})",
                key.asset_id,
                key.side,
                oid.chars().take(10).collect::<String>()
            ));
        }
        true
    }

    fn _maker_order_cancel_all_except_asset(&self, keep_asset_id: Option<&str>, reason: &str) {
        if !self._maker_single_inflight_enabled() {
            return;
        }
        let keep = keep_asset_id.unwrap_or("").trim().to_string();
        let keys: Vec<MakerOrderKey> = self
            .maker_order_slots
            .lock()
            .map(|m| {
                m.keys()
                    .filter_map(|k| {
                        if k.side != "BUY" {
                            return None;
                        }
                        if !keep.is_empty() && k.asset_id == keep {
                            return None;
                        }
                        Some(k.clone())
                    })
                    .collect::<Vec<MakerOrderKey>>()
            })
            .unwrap_or_default();
        for key in keys {
            let _ = self._maker_order_request_cancel(&key, reason);
        }
    }

    fn _maker_cancel_strategy_orders(&self, keep_asset_id: Option<&str>, reason: &str) {
        if self._maker_single_inflight_enabled() {
            self._maker_order_cancel_all_except_asset(keep_asset_id, reason);
            return;
        }
        if let Some(keep) = keep_asset_id {
            self.cancel_all_open_orders_local_except(keep, reason);
        } else {
            self.cancel_all_open_orders_local(reason);
        }
    }

    fn _maker_order_reconcile_asset(&self, asset_id: &str, intended_price: Option<f64>) {
        if !self._maker_single_inflight_enabled() || self.cfg.dry_run {
            return;
        }
        let aid = asset_id.trim().to_string();
        if aid.is_empty() {
            return;
        }
        let key = MakerOrderKey::buy(&aid);
        let max_active = env_int("MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET", 1).max(1) as usize;
        let pick_keep_oid = |orders: &[Value], tracked_oid: Option<String>| -> Option<String> {
            if orders.is_empty() {
                return None;
            }
            if let Some(t) = tracked_oid {
                let has_tracked = orders
                    .iter()
                    .any(|o| self._extract_order_id(o).map(|id| id == t).unwrap_or(false));
                if has_tracked {
                    return Some(t);
                }
            }
            if let Some(ip) = intended_price.filter(|p| *p > 0.0) {
                return orders
                    .iter()
                    .filter_map(|o| {
                        let oid = self._extract_order_id(o)?;
                        let d = (self._extract_order_price(o) - ip).abs();
                        Some((oid, d))
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|x| x.0);
            }
            orders
                .iter()
                .filter_map(|o| {
                    self._extract_order_id(o)
                        .map(|oid| (oid, self._extract_order_price(o)))
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|x| x.0)
        };

        let mut buy_orders: Vec<Value> = self
            ._list_open_orders_exchange()
            .into_iter()
            .filter(|o| {
                self._extract_order_token_id(o).as_deref() == Some(aid.as_str())
                    && self._extract_order_side(o) == "BUY"
                    && self._extract_order_id(o).is_some()
                    && self._extract_order_remaining_size(o) > 1e-9
            })
            .collect();
        if buy_orders.len() > max_active {
            let tracked_oid = self._maker_order_slot_get(&key).order_id;
            let keep_oid = pick_keep_oid(&buy_orders, tracked_oid);
            for o in &buy_orders {
                let Some(oid) = self._extract_order_id(o) else {
                    continue;
                };
                if keep_oid.as_deref() == Some(oid.as_str()) {
                    continue;
                }
                let _ = self._cancel(&oid);
            }
            buy_orders = self
                ._list_open_orders_exchange()
                .into_iter()
                .filter(|o| {
                    self._extract_order_token_id(o).as_deref() == Some(aid.as_str())
                        && self._extract_order_side(o) == "BUY"
                        && self._extract_order_id(o).is_some()
                        && self._extract_order_remaining_size(o) > 1e-9
                })
                .collect();
        }

        let tracked_oid = self._maker_order_slot_get(&key).order_id;
        let keep_oid = pick_keep_oid(&buy_orders, tracked_oid);
        let keep_order = keep_oid.as_ref().and_then(|oid| {
            buy_orders.iter().find_map(|o| {
                self._extract_order_id(o)
                    .filter(|x| x == oid)
                    .map(|_| o.clone())
            })
        });

        if let Some(order) = keep_order {
            let oid = self._extract_order_id(&order).unwrap_or_default();
            let price = self._extract_order_price(&order);
            let remaining = self._extract_order_remaining_size(&order).max(0.0);
            let size = remaining.max(0.0);
            self._maker_order_on_submit_ack(&oid, &key, price, size, "RECONCILE");
            if max_active == 1 {
                for o in buy_orders {
                    let Some(oid2) = self._extract_order_id(&o) else {
                        continue;
                    };
                    if oid2 == oid {
                        continue;
                    }
                    let _ = self._cancel(&oid2);
                }
            }
            return;
        }

        let now = now_ts_f64();
        let submit_ttl = self._maker_submit_pending_ttl_seconds();
        let cancel_ttl = self._maker_cancel_pending_ttl_seconds();
        let working_missing_ttl = self._maker_working_missing_ttl_seconds();
        let cur_slot = self._maker_order_slot_get(&key);
        if buy_orders.is_empty()
            && cur_slot.state == MakerOrderLifecycle::Working
            && cur_slot.order_id.is_some()
            && (now - cur_slot.last_submit_ts) < working_missing_ttl
        {
            // Exchange list can be transiently stale right after submit/cancel churn.
            // Keep local working slot conservative for a short grace period to avoid
            // duplicate same-side submits.
            return;
        }
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            let keep_pending = match slot.state {
                MakerOrderLifecycle::SubmitPending => now - slot.last_submit_ts < submit_ttl,
                MakerOrderLifecycle::CancelPending => now - slot.last_cancel_ts < cancel_ttl,
                _ => false,
            };
            if !keep_pending {
                slot.state = MakerOrderLifecycle::Idle;
                slot.order_id = None;
                slot.remaining = 0.0;
                slot.replace_target = None;
            }
        }
        self._maker_order_clear_index_for_key(&key);
        if let Ok(mut s) = self.state.lock() {
            let should_remove = s
                .open_orders
                .get(&aid)
                .and_then(|oo| oo.order_id.clone())
                .is_some();
            if should_remove {
                s.open_orders.remove(&aid);
                let _ = save_state(&self.state_file, &mut s);
            }
        }
    }

    fn _maker_order_on_user_event(&self, msg: &Value) {
        if !self._maker_single_inflight_enabled() {
            return;
        }
        let oid = self._extract_order_id(msg).unwrap_or_default();
        if oid.trim().is_empty() {
            return;
        }
        let side = msg
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        let key_from_index = self
            .maker_order_index
            .lock()
            .ok()
            .and_then(|idx| idx.get(&oid).cloned());
        let msg_asset_id = msg
            .get("asset_id")
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("assetId"))
            .or_else(|| msg.get("tokenId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let key = if let Some(k) = key_from_index {
            k
        } else {
            if side != "BUY" || msg_asset_id.is_empty() {
                return;
            }
            MakerOrderKey::buy(&msg_asset_id)
        };
        if key.side != "BUY" || key.asset_id.trim().is_empty() {
            return;
        }
        let asset_id = key.asset_id.clone();
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
        if cancelish || remaining <= 1e-9 {
            if let Ok(mut slots) = self.maker_order_slots.lock() {
                let slot = slots.entry(key.clone()).or_default();
                if slot.order_id.as_deref() == Some(oid.as_str())
                    || slot.order_id.is_none()
                    || slot.state == MakerOrderLifecycle::CancelPending
                {
                    slot.state = MakerOrderLifecycle::Idle;
                    slot.order_id = None;
                    slot.remaining = 0.0;
                    slot.replace_target = None;
                }
            }
            if let Ok(mut idx) = self.maker_order_index.lock() {
                idx.remove(&oid);
            }
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

        let max_active = env_int("MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET", 1).max(1);
        let mut duplicate_oid: Option<String> = None;
        let mut should_adopt = true;
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            if let Some(cur_oid) = slot.order_id.clone() {
                if cur_oid != oid && slot.state == MakerOrderLifecycle::Working && max_active <= 1 {
                    duplicate_oid = Some(oid.clone());
                    should_adopt = false;
                }
            }
            if should_adopt {
                slot.state = MakerOrderLifecycle::Working;
                slot.order_id = Some(oid.clone());
                slot.price = if price > 0.0 { price } else { slot.price };
                slot.size = original.max(remaining).max(slot.size);
                slot.remaining = remaining;
                slot.origin = if slot.origin.trim().is_empty() {
                    "ORDER_EVENT".to_string()
                } else {
                    slot.origin.clone()
                };
                slot.replace_target = None;
            }
        }
        if let Some(dup) = duplicate_oid {
            self.logger.warning(&format!(
                "[MAKER_ORD] duplicate BUY order for asset={} tracked differs; canceling {}..",
                asset_id,
                dup.chars().take(10).collect::<String>()
            ));
            let _ = self._cancel(&dup);
            return;
        }
        if should_adopt {
            if let Ok(mut idx) = self.maker_order_index.lock() {
                idx.retain(|_, v| v != &key);
                idx.insert(oid.clone(), key);
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
    }

    fn _maker_order_upsert_gtc(
        &self,
        key: &MakerOrderKey,
        price: f64,
        size: f64,
        origin: &str,
    ) -> Option<String> {
        if key.asset_id.trim().is_empty() || key.side != "BUY" {
            return None;
        }
        let min_shares = self.cfg.min_shares.max(1.0);
        let exact_recovery = pair_base_recovery_uses_exact_order(origin, size, min_shares);
        if !self._maker_single_inflight_enabled() {
            return if exact_recovery {
                self._place_limit_bid_gtc_exact_with_origin(
                    &key.asset_id,
                    price,
                    size,
                    Some(true),
                    origin,
                )
            } else {
                self._place_limit_bid_gtc_with_origin(&key.asset_id, price, size, Some(true), origin)
            };
        }
        let (
            recovery_active,
            recovery_gap,
            recovery_heavy_side,
            recovery_light_side,
            recovery_asset,
            _recovery_unsettled_heavy,
        ) = self._maker_recovery_mode_snapshot();
        if recovery_active {
            if let Some(light_asset_id) = recovery_asset.as_deref() {
                if key.asset_id != light_asset_id {
                    // Always block heavy-side BUY during recovery
                    self.logger.info(&format!(
                        "[MAKER_ORD] skip heavy-side BUY during recovery asset={} heavy={} light={} gap={recovery_gap:.2} origin={}",
                        key.asset_id,
                        recovery_heavy_side,
                        recovery_light_side,
                        origin
                    ));
                    return None;
                }
                // Light-side: one-flight — block stacking if a recovery order is already unsettled
                let light_unsettled = self._maker_recovery_unsettled_buy_risk(light_asset_id);
                if light_unsettled > 1e-6 && !self._maker_recovery_light_requote_ready(light_asset_id)
                {
                    let current_oid = self._maker_order_slot_get(key).order_id;
                    self.logger.info(&format!(
                        "[MAKER_ORD] skip light-side BUY stacking during recovery asset={} light_unsettled={light_unsettled:.2} gap={recovery_gap:.2} origin={}",
                        key.asset_id,
                        origin
                    ));
                    return current_oid;
                }
            }
        }

        let now = now_ts_f64();
        let submit_ttl = self._maker_submit_pending_ttl_seconds();
        let cancel_ttl = self._maker_cancel_pending_ttl_seconds();
        let reject_cooldown = self._maker_submit_reject_cooldown_seconds();
        let replace_min = self._maker_replace_min_interval_seconds();
        let stale = env_int("STALE_SECONDS", self.cfg.stale_seconds).max(1) as f64;
        let replace_ticks = env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            self.cfg.replace_if_price_moves_ticks,
        ) as f64;

        self._maker_order_reconcile_asset(&key.asset_id, Some(price));
        let mut slot = self._maker_order_slot_get(key);
        let mut target_price = price;
        let mut target_size = size;
        let mut target_origin = origin.to_string();
        if slot.state == MakerOrderLifecycle::CancelPending {
            if let Some(tgt) = slot.replace_target.clone() {
                if tgt.price > 0.0 && tgt.size > 0.0 {
                    target_price = tgt.price;
                    target_size = tgt.size;
                    if !tgt.origin.trim().is_empty() {
                        target_origin = tgt.origin;
                    }
                }
            }
        }
        if slot.state == MakerOrderLifecycle::SubmitPending
            && now - slot.last_submit_ts < submit_ttl
        {
            return slot.order_id.clone();
        }
        if slot.state == MakerOrderLifecycle::CancelPending
            && now - slot.last_cancel_ts < cancel_ttl
        {
            return None;
        }
        if slot.order_id.is_none()
            && slot.state == MakerOrderLifecycle::Idle
            && reject_cooldown > 0.0
            && slot.last_reject_ts > 0.0
        {
            // Backoff: base cooldown * 2^(consecutive_rejects-1), capped at max
            let max_reject_cooldown =
                env_float("MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS", 60.0).max(reject_cooldown);
            let effective_cooldown = if slot.consecutive_rejects <= 1 {
                reject_cooldown
            } else {
                (reject_cooldown
                    * 2.0_f64.powi((slot.consecutive_rejects - 1).min(6) as i32))
                .min(max_reject_cooldown)
            };
            if now - slot.last_reject_ts < effective_cooldown {
                return None;
            }
        }
        if slot.state != MakerOrderLifecycle::Working
            && slot.state != MakerOrderLifecycle::Idle
            && now - slot.last_submit_ts >= submit_ttl
            && now - slot.last_cancel_ts >= cancel_ttl
        {
            if let Ok(mut slots) = self.maker_order_slots.lock() {
                let s = slots.entry(key.clone()).or_default();
                s.state = MakerOrderLifecycle::Idle;
                s.order_id = None;
                s.remaining = 0.0;
                s.replace_target = None;
            }
            slot = self._maker_order_slot_get(key);
        }

        if slot.state == MakerOrderLifecycle::Working {
            if let Some(oid) = slot.order_id.clone() {
                let old_price = slot.price.max(0.0);
                let old_size = slot.remaining.max(slot.size).max(0.0);
                let age = (now - slot.last_submit_ts).max(0.0);
                let moved_ticks = (target_price - old_price).abs() / self.cfg.tick.max(0.0001);
                let exact_recovery_sizing =
                    pair_base_recovery_uses_exact_order(&target_origin, target_size, min_shares)
                        || pair_base_recovery_uses_exact_order(&slot.origin, old_size, min_shares);
                let size_changed = if exact_recovery_sizing {
                    old_size <= 0.0 || (target_size - old_size).abs() >= 0.01
                } else {
                    old_size <= 0.0
                        || (target_size - old_size).abs() >= (0.25 * old_size).max(self.cfg.min_shares)
                };
                if age < stale && moved_ticks < replace_ticks && !size_changed {
                    return Some(oid);
                }
                if now - slot.last_cancel_ts < replace_min {
                    return None;
                }
                if self._maker_order_request_cancel(key, "maker_order_replace") {
                    if let Ok(mut slots) = self.maker_order_slots.lock() {
                        if let Some(s) = slots.get_mut(key) {
                            s.replace_target = Some(MakerOrderReplaceTarget {
                                price: target_price,
                                size: target_size,
                                origin: target_origin.clone(),
                            });
                        }
                    }
                }
                return None;
            }
        }

        if slot.state == MakerOrderLifecycle::CancelPending
            && now - slot.last_cancel_ts < cancel_ttl
        {
            return None;
        }

        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let s = slots.entry(key.clone()).or_default();
            s.state = MakerOrderLifecycle::SubmitPending;
            s.last_submit_ts = now;
            s.price = target_price;
            s.size = target_size.max(0.0);
            s.remaining = target_size.max(0.0);
            s.origin = target_origin.clone();
        }
        let exact_recovery_target =
            pair_base_recovery_uses_exact_order(&target_origin, target_size, min_shares);
        let oid = if exact_recovery_target {
            self._place_limit_bid_gtc_exact_with_origin(
                &key.asset_id,
                target_price,
                target_size,
                Some(true),
                &target_origin,
            )
        } else {
            self._place_limit_bid_gtc_with_origin(
                &key.asset_id,
                target_price,
                target_size,
                Some(true),
                &target_origin,
            )
        };
        if let Some(oid) = oid {
            self._maker_order_on_submit_ack(&oid, key, target_price, target_size, &target_origin);
            return Some(oid);
        }
        self._maker_order_on_submit_reject(key, "post_order returned no oid");
        self._maker_order_reconcile_asset(&key.asset_id, Some(price));
        None
    }

    fn _maker_payoff_envelope(
        shares_up: f64,
        shares_down: f64,
        cost_total: f64,
    ) -> (f64, f64, f64) {
        let up = shares_up.max(0.0);
        let down = shares_down.max(0.0);
        let cost = cost_total.max(0.0);
        let downside = up.min(down) - cost;
        let upside = up.max(down) - cost;
        let mn = up.min(down);
        let mx = up.max(down);
        let skew_ratio = if mn > 1e-12 { mx / mn } else { f64::INFINITY };
        (downside, upside, skew_ratio)
    }

    fn _maker_poly_fee_formula(
        qty: f64,
        price: f64,
        fee_rate: f64,
        exponent: f64,
        maker_rebate_bps: f64,
        is_maker: bool,
        model_enabled: bool,
    ) -> f64 {
        if qty <= 0.0 || price <= 0.0 || !model_enabled {
            return 0.0;
        }
        if is_maker {
            return 0.0;
        }
        let p = clamp(price, 1e-6, 0.999_999);
        let notional = qty * p;
        let taker_fee =
            notional * fee_rate.max(0.0) * (p * (1.0 - p)).powf(exponent.clamp(0.0, 8.0));
        let _ = maker_rebate_bps;
        taker_fee.max(0.0)
    }

    fn _maker_ladder_cancel_all(&self, reason: &str) {
        let orders = self
            .maker_ladder_open_orders
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        if orders.is_empty() {
            return;
        }
        for rec in orders.values() {
            if !rec.order_id.trim().is_empty() {
                let _ = self._cancel(&rec.order_id);
            }
        }
        if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
            m.clear();
        }
        if !reason.trim().is_empty() {
            self.logger
                .info(&format!("[MAKER_SKEW] ladder cleared: {reason}"));
        }
    }

    fn _maker_ladder_reserved_notional(&self) -> f64 {
        self.maker_ladder_open_orders
            .lock()
            .map(|m| {
                m.values()
                    .map(|o| o.price.max(0.0) * o.size.max(0.0))
                    .sum::<f64>()
            })
            .unwrap_or(0.0)
    }

    fn _maker_ladder_place_or_replace(
        &self,
        key: &str,
        asset_id: &str,
        role: &str,
        level: i64,
        target_price: f64,
        target_size: f64,
    ) {
        if key.trim().is_empty() || asset_id.trim().is_empty() || target_price <= 0.0 {
            return;
        }
        let now = now_ts_f64();
        let stale = env_int("STALE_SECONDS", self.cfg.stale_seconds).max(1) as f64;
        let replace_ticks = env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            self.cfg.replace_if_price_moves_ticks,
        ) as f64;

        let existing = self
            .maker_ladder_open_orders
            .lock()
            .ok()
            .and_then(|m| m.get(key).cloned());
        if let Some(prev) = existing {
            let age = (now - prev.ts).max(0.0);
            let moved_ticks = (target_price - prev.price).abs() / self.cfg.tick.max(0.0001);
            let size_changed =
                (target_size - prev.size).abs() >= (0.25 * prev.size).max(self.cfg.min_shares);
            if age < stale && moved_ticks < replace_ticks && !size_changed {
                return;
            }
            if !prev.order_id.trim().is_empty() {
                let _ = self._cancel(&prev.order_id);
            }
            if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
                m.remove(key);
            }
        }
        let oid = self._place_postonly_bid(asset_id, target_price, target_size);
        let Some(oid) = oid else {
            return;
        };
        if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
            m.insert(
                key.to_string(),
                LadderOrderState {
                    key: key.to_string(),
                    asset_id: asset_id.to_string(),
                    role: role.to_string(),
                    level,
                    order_id: oid,
                    price: target_price,
                    size: target_size,
                    ts: now,
                },
            );
        }
    }

    fn _maker_ladder_sync_role(
        &self,
        role: &str,
        asset_id: &str,
        base_bid: f64,
        clip_size: f64,
        levels: i64,
        tick_step: i64,
    ) {
        if levels <= 0 || base_bid <= 0.0 || clip_size <= 0.0 {
            return;
        }
        let tick = self.cfg.tick.max(0.0001);
        let lv = levels.max(1);
        let step = tick_step.max(1) as f64;
        let mut target_prices: Vec<f64> = Vec::new();

        if role.eq_ignore_ascii_case("underdog") {
            let floor = env_float("MAKER_UNDERDOG_FLOOR_PRICE", 0.20).clamp(tick, 0.99);
            for i in 0..lv {
                let mut px = base_bid - (i as f64) * step * tick;
                px = round_down(clamp(px, floor, 0.99), tick);
                if target_prices
                    .last()
                    .map(|p| (p - px).abs() > tick * 0.5)
                    .unwrap_or(true)
                {
                    target_prices.push(px);
                }
                if px <= floor + tick * 0.5 {
                    break;
                }
            }
            if target_prices.is_empty() {
                target_prices.push(round_down(clamp(floor, tick, 0.99), tick));
            }
        } else if role.eq_ignore_ascii_case("hedge") {
            let floor = env_float("MAKER_HEDGE_FLOOR_PRICE", 0.55).clamp(tick, 0.99);
            let span = ((lv - 1) as f64) * step * tick;
            let start = (base_bid.max(floor + span)).clamp(tick, 0.99);
            for i in 0..lv {
                let mut px = start - (i as f64) * step * tick;
                px = round_down(clamp(px, floor, 0.99), tick);
                if target_prices
                    .last()
                    .map(|p| (p - px).abs() > tick * 0.5)
                    .unwrap_or(true)
                {
                    target_prices.push(px);
                }
                if px <= floor + tick * 0.5 {
                    break;
                }
            }
            if target_prices.is_empty() {
                target_prices.push(round_down(clamp(floor, tick, 0.99), tick));
            }
        } else {
            for i in 0..lv {
                let mut px = base_bid - (i as f64) * step * tick;
                px = round_down(clamp(px, tick, 0.99), tick);
                if target_prices
                    .last()
                    .map(|p| (p - px).abs() > tick * 0.5)
                    .unwrap_or(true)
                {
                    target_prices.push(px);
                }
            }
        }

        if self._maker_single_inflight_enabled() {
            self._maker_ladder_cancel_all("single_inflight_guard");
            if let Some(px) = target_prices.first().copied() {
                let key = MakerOrderKey::buy(asset_id);
                let _ = self._maker_order_upsert_gtc(&key, px, clip_size, "MAKER_POSTONLY_GTC");
            }
            return;
        }

        let min_per_order = self.cfg.min_shares.max(1.0);
        let max_levels_by_clip = ((clip_size + 1e-12) / min_per_order).floor().max(1.0) as usize;
        if target_prices.len() > max_levels_by_clip {
            target_prices.truncate(max_levels_by_clip);
        }
        let level_count = target_prices.len().max(1) as f64;
        let per_level = (clip_size / level_count).max(min_per_order);
        let mut desired: HashSet<String> = HashSet::new();
        for (idx, px) in target_prices.into_iter().enumerate() {
            if px <= 0.0 {
                continue;
            }
            let i = idx as i64;
            let key = format!("{role}:{asset_id}:{i}");
            desired.insert(key.clone());
            self._maker_ladder_place_or_replace(&key, asset_id, role, i, px, per_level);
        }

        let stale_keys: Vec<String> = self
            .maker_ladder_open_orders
            .lock()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| {
                        if v.role == role && v.asset_id == asset_id && !desired.contains(k) {
                            Some(k.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        if stale_keys.is_empty() {
            return;
        }
        for key in stale_keys {
            let mut rec = None;
            if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
                rec = m.remove(&key);
            }
            if let Some(r) = rec {
                let _ = self._cancel(&r.order_id);
            }
        }
    }

    fn _maker_ladder_cancel_except_role_asset(&self, keep_role: &str, keep_asset_id: &str) {
        let stale_keys: Vec<String> = self
            .maker_ladder_open_orders
            .lock()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| {
                        if v.role == keep_role && v.asset_id == keep_asset_id {
                            None
                        } else {
                            Some(k.clone())
                        }
                    })
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        for key in stale_keys {
            let mut rec = None;
            if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
                rec = m.remove(&key);
            }
            if let Some(r) = rec {
                let _ = self._cancel(&r.order_id);
            }
        }
    }

    fn _maker_compute_rsi(closes: &[f64], period: usize) -> Option<f64> {
        if closes.len() <= period || period < 2 {
            return None;
        }
        let mut gain = 0.0;
        let mut loss = 0.0;
        for i in (closes.len() - period)..closes.len() {
            if i == 0 {
                continue;
            }
            let d = closes[i] - closes[i - 1];
            if d > 0.0 {
                gain += d;
            } else {
                loss += -d;
            }
        }
        let avg_gain = gain / period as f64;
        let avg_loss = loss / period as f64;
        if avg_loss <= 1e-12 {
            Some(100.0)
        } else {
            let rs = avg_gain / avg_loss;
            Some(100.0 - (100.0 / (1.0 + rs)))
        }
    }

    fn _maker_stretch_bias_side(&self, default_side: &str) -> String {
        let now = now_ts_f64();
        let default_norm = default_side.trim().to_ascii_uppercase();
        let record_stretch =
            |rsi: Option<f64>, diff_vs_start: Option<f64>, biased_side: &str, reason: &str| {
                if let Ok(mut st) = self.maker_skew_state.lock() {
                    st.stretch_rsi = rsi;
                    st.stretch_diff_vs_start = diff_vs_start;
                    st.stretch_default_side = default_norm.clone();
                    st.stretch_biased_side = biased_side.to_ascii_uppercase();
                    st.stretch_bias_reason = reason.to_string();
                    st.stretch_eval_ts = now;
                }
            };
        if !env_bool("MAKER_STRETCH_BIAS_ENABLED", false) {
            record_stretch(None, None, &default_norm, "disabled");
            return default_norm;
        }
        let delta_threshold = env_float("MAKER_STRETCH_DELTA_THRESHOLD", 0.0).abs();
        let diff_price_opt =
            get_live_snapshot_for_market(&self.market_slug).and_then(|s| s.diff_vs_price_to_beat);
        let diff_price = diff_price_opt.unwrap_or(0.0);
        let rsi_period = env_int("MAKER_STRETCH_RSI_PERIOD", 14).clamp(2, 100) as usize;
        let rsi_oversold = env_float("MAKER_STRETCH_RSI_OVERSOLD", 40.0).clamp(1.0, 99.0);
        let rsi_overbought = (100.0 - rsi_oversold).clamp(1.0, 99.0);
        let rsi_source = std::env::var("MAKER_STRETCH_RSI_SOURCE")
            .unwrap_or_else(|_| "BINANCE".to_string())
            .trim()
            .to_ascii_uppercase();

        let mut rsi_value = None;
        let mut has_feed = false;
        if rsi_source == "CHAINLINK" || rsi_source == "RTDS" || rsi_source == "POLYMARKET" {
            let live = get_live_snapshot_for_market(&self.market_slug);
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let max_age_ms = (self.cfg.market_data_stale_seconds.max(1) as i64) * 1000;
            if let Some(snap) = live {
                let age_ms = now_ms.saturating_sub(snap.updated_at_ms.max(snap.timestamp_ms));
                if age_ms <= max_age_ms {
                    has_feed = true;
                    if let Ok(mut st) = self.maker_skew_state.lock() {
                        if snap.timestamp_ms > st.stretch_chainlink_last_ts_ms {
                            st.stretch_chainlink_last_ts_ms = snap.timestamp_ms;
                            st.stretch_chainlink_closes.push(snap.price.max(0.0));
                            let keep = (rsi_period * 6).max(rsi_period + 2);
                            if st.stretch_chainlink_closes.len() > keep {
                                let drop_n = st.stretch_chainlink_closes.len() - keep;
                                st.stretch_chainlink_closes.drain(0..drop_n);
                            }
                        }
                        rsi_value =
                            Self::_maker_compute_rsi(&st.stretch_chainlink_closes, rsi_period);
                    }
                }
            }
        } else if let Some(feed) = &self.binance_feed {
            has_feed = true;
            let snap = feed.snapshot();
            let mut closes: Vec<f64> = snap.seed_klines.iter().map(|k| k.close).collect();
            if let Some(last) = snap.last_tick {
                closes.push(last.price.max(0.0));
            }
            rsi_value = Self::_maker_compute_rsi(&closes, rsi_period);
        }

        let mut biased_side = default_norm.clone();
        let mut reason = "default_side".to_string();
        if diff_price <= -delta_threshold {
            if let Some(rsi) = rsi_value {
                if rsi <= rsi_oversold {
                    biased_side = "YES".to_string();
                    reason = "oversold_yes".to_string();
                }
            } else {
                reason = if has_feed {
                    "no_rsi".to_string()
                } else {
                    "no_feed".to_string()
                };
            }
        } else if diff_price >= delta_threshold {
            if let Some(rsi) = rsi_value {
                if rsi >= rsi_overbought {
                    biased_side = "NO".to_string();
                    reason = "overbought_no".to_string();
                }
            } else {
                reason = if has_feed {
                    "no_rsi".to_string()
                } else {
                    "no_feed".to_string()
                };
            }
        } else {
            reason = "delta_below_threshold".to_string();
        }
        record_stretch(rsi_value, diff_price_opt, &biased_side, &reason);
        biased_side
    }

    fn _maker_submit_pair_orders(
        &self,
        size_int: i64,
        y_px: f64,
        n_px: f64,
        order_type: &str,
        post_only: Option<bool>,
        origin: &str,
    ) -> (Option<String>, Option<String>) {
        if size_int <= 0 {
            return (None, None);
        }
        let qty = size_int as f64;
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return (None, None),
        };
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let resolved = self._resolve_order_type(order_type);
        let track_taker_fallback = pair_submit_tracks_taker_fallback(&resolved);
        let (y_oid, n_oid) = if resolved == "GTC" && self._maker_single_inflight_enabled() {
            let y_key = MakerOrderKey::buy(yes);
            let n_key = MakerOrderKey::buy(no);
            (
                self._maker_order_upsert_gtc(&y_key, y_px, qty, &format!("{origin}_YES")),
                self._maker_order_upsert_gtc(&n_key, n_px, qty, &format!("{origin}_NO")),
            )
        } else {
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
            let resps = self._post_orders_compat(&[signed_y, signed_n], &resolved, post_only);
            (
                resps.first().and_then(|o| o.clone()),
                resps.get(1).and_then(|o| o.clone()),
            )
        };
        if let Some(oid) = &y_oid {
            if track_taker_fallback {
                self._remember_taker_order(oid, yes, qty, y_px, "BUY");
            } else {
                self._forget_taker_order(oid);
            }
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
                    "post_start_ts": decide_ts,
                    "post_end_ts": now_ts_f64(),
                    "origin": format!("{origin}_YES"),
                }),
            );
        }
        if let Some(oid) = &n_oid {
            if track_taker_fallback {
                self._remember_taker_order(oid, no, qty, n_px, "BUY");
            } else {
                self._forget_taker_order(oid);
            }
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
                    "post_start_ts": decide_ts,
                    "post_end_ts": now_ts_f64(),
                    "origin": format!("{origin}_NO"),
                }),
            );
        }
        (y_oid, n_oid)
    }

    fn _pair_base_mode_enabled(&self) -> bool {
        env_bool("PAIR_BASE_ENABLED", false)
            && !env_bool("MAKER_SKEW_ENABLED", true)
            && !env_bool("MAKER_ARB_ENABLED", true)
            && !env_bool("MAKER_STRETCH_BIAS_ENABLED", false)
    }

    fn _pair_recovery_enabled(&self) -> bool {
        env_bool("PAIR_RECOVERY_ENABLED", true)
    }

    fn _pair_base_window_budget(&self) -> f64 {
        env_float("PAIR_BASE_WINDOW_BUDGET_USDC", self.cfg.max_total_cost)
            .max(1.0)
            .min(self.cfg.max_total_cost.max(1.0))
    }

    fn _pair_base_merge_budget(&self) -> f64 {
        env_float("PAIR_BASE_MERGE_BUDGET_USDC", self.cfg.max_total_cost)
            .max(1.0)
            .min(self.cfg.max_total_cost.max(1.0))
    }

    fn _pair_base_hard_reserve(&self) -> f64 {
        env_float("PAIR_BASE_HARD_RESERVE_USDC", self.cfg.reserve_usd)
            .max(0.0)
            .min(self._pair_base_window_budget())
    }

    fn _pair_base_fee_net_snapshot(
        &self,
        q_yes: f64,
        q_no: f64,
        total_cost: f64,
        add_yes: f64,
        p_yes: f64,
        add_no: f64,
        p_no: f64,
    ) -> PairBaseFeeNetSnapshot {
        let fee_model_enabled = env_bool("POLY_FEE_MODEL_ENABLED", true);
        let fees_enabled = self.market_fees_enabled.unwrap_or(fee_model_enabled);
        let fee_source = if self.market_fees_enabled.is_some() {
            "market".to_string()
        } else {
            "env".to_string()
        };
        let maker_rebate_bps = env_float("POLY_MAKER_REBATE_BPS", 0.0).max(0.0);
        let fee_yes = if add_yes > 0.0 && p_yes > 0.0 {
            self._maker_poly_fee_estimate(add_yes, p_yes, true, fee_model_enabled && fees_enabled)
        } else {
            0.0
        };
        let fee_no = if add_no > 0.0 && p_no > 0.0 {
            self._maker_poly_fee_estimate(add_no, p_no, true, fee_model_enabled && fees_enabled)
        } else {
            0.0
        };
        let estimated_fees = fee_yes + fee_no;
        let q_yes_after = q_yes.max(0.0) + add_yes.max(0.0);
        let q_no_after = q_no.max(0.0) + add_no.max(0.0);
        let cost_after =
            total_cost.max(0.0) + add_yes.max(0.0) * p_yes.max(0.0) + add_no.max(0.0) * p_no.max(0.0);
        let fee_net_pair_cost = cost_after + estimated_fees;
        let mn = q_yes_after.min(q_no_after);
        let mx = q_yes_after.max(q_no_after);
        let pair_coverage = if mx > 1e-9 { mn / mx } else { 1.0 };
        PairBaseFeeNetSnapshot {
            fees_enabled,
            fee_source,
            maker_rebate_bps,
            estimated_fees,
            fee_net_pair_cost,
            fee_net_worst_case_pnl: mn - fee_net_pair_cost,
            fee_net_best_case_pnl: mx - fee_net_pair_cost,
            pair_coverage,
        }
    }

    fn _pair_base_log_fee_net(
        &self,
        label: &str,
        pair_id: &str,
        q_yes: f64,
        q_no: f64,
        total_cost: f64,
        add_yes: f64,
        p_yes: f64,
        add_no: f64,
        p_no: f64,
    ) -> PairBaseFeeNetSnapshot {
        let snap =
            self._pair_base_fee_net_snapshot(q_yes, q_no, total_cost, add_yes, p_yes, add_no, p_no);
        self.logger.info(&format!(
            "[PAIR_BASE][FEE] label={} pair_id={} fees_enabled={} fee_source={} maker_rebate_bps={:.2} est_fees={:.4} fee_net_pair_cost={:.4} fee_net_worst_case_pnl={:+.4} fee_net_best_case_pnl={:+.4} pair_coverage={:.3}",
            label,
            pair_id,
            snap.fees_enabled,
            snap.fee_source,
            snap.maker_rebate_bps,
            snap.estimated_fees,
            snap.fee_net_pair_cost,
            snap.fee_net_worst_case_pnl,
            snap.fee_net_best_case_pnl,
            snap.pair_coverage
        ));
        snap
    }

    fn _pair_base_live_order_id(&self, asset_id: &str) -> Option<String> {
        if asset_id.trim().is_empty() {
            return None;
        }
        let slot = self._maker_order_slot_get(&MakerOrderKey::buy(asset_id));
        if !matches!(
            slot.state,
            MakerOrderLifecycle::Working
                | MakerOrderLifecycle::SubmitPending
                | MakerOrderLifecycle::CancelPending
        ) {
            return None;
        }
        if !(slot.origin.starts_with("PAIR_BASE_") || slot.origin == "PAIR_BASE_RECOVERY") {
            return None;
        }
        slot.order_id
    }

    fn _pair_base_cancel_orders(&self, reason: &str) {
        let (Some(yes), Some(no)) = (&self.yes_asset, &self.no_asset) else {
            return;
        };
        let _ = self._maker_order_request_cancel(&MakerOrderKey::buy(yes), reason);
        let _ = self._maker_order_request_cancel(&MakerOrderKey::buy(no), reason);
    }

    fn _pair_base_set_phase(
        &self,
        phase: PairBasePhaseState,
        pair_id: Option<String>,
        yes_oid: Option<String>,
        no_oid: Option<String>,
        target_qty: f64,
        filled_yes: f64,
        filled_no: f64,
    ) {
        let short_oid = |oid: &Option<String>| -> String {
            oid.as_deref()
                .map(|s| s.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "-".to_string())
        };
        if let Ok(mut st) = self.pair_base_state.lock() {
            let changed = st.phase != phase
                || st.active_pair_id != pair_id
                || st.yes_oid != yes_oid
                || st.no_oid != no_oid;
            if changed {
                let pair_label = pair_id.as_deref().unwrap_or("-");
                self.logger.info(&format!(
                    "[PAIR_BASE] phase {} -> {} pair_id={} y_oid={} n_oid={} target={target_qty:.2} fy={filled_yes:.2} fn={filled_no:.2}",
                    st.phase.as_str(),
                    phase.as_str(),
                    pair_label,
                    short_oid(&yes_oid),
                    short_oid(&no_oid)
                ));
                st.state_enter_ts = now_ts_f64();
            }
            st.phase = phase;
            st.active_pair_id = pair_id;
            st.yes_oid = yes_oid;
            st.no_oid = no_oid;
            st.target_qty = target_qty;
            st.filled_yes = filled_yes;
            st.filled_no = filled_no;
        }
    }

    fn _maker_pair_base_recovery_phase(
        &self,
        ctx: &MakerSkewLoopCtx,
    ) -> Option<PairBaseRecoveryState> {
        let (
            recovery_mode,
            recovery_gap,
            recovery_heavy_side,
            recovery_side,
            recovery_asset,
            recovery_unsettled_heavy,
        ) = self._maker_recovery_mode_snapshot();
        let (current_phase, _active_pair_id, _current_target_qty, state_enter_ts) = self
            .pair_base_state
            .lock()
            .map(|st| {
                (
                    st.phase,
                    st.active_pair_id.clone(),
                    st.target_qty,
                    st.state_enter_ts,
                )
            })
            .unwrap_or((PairBasePhaseState::Flat, None, 0.0, 0.0));
        let release = self._pair_arb_imbalance_release_shares();
        let recovery_asset_id = recovery_asset.as_deref().unwrap_or(if recovery_side == "YES" {
            ctx.yes_asset.as_str()
        } else {
            ctx.no_asset.as_str()
        });
        let trusted_age_s = (env_float("PAIR_BASE_REFRESH_SECONDS", 2.0).max(0.2) * 3.0).max(6.0);
        let light_oid = self._pair_base_live_order_id(recovery_asset_id);
        let light_unsettled = self._maker_recovery_unsettled_buy_risk(recovery_asset_id);
        let live_light_fresh = light_oid
            .as_deref()
            .map(|oid| self._maker_order_is_live(recovery_asset_id, Some(oid), trusted_age_s))
            .unwrap_or(false);
        let live_light_refresh_reason = self._maker_recovery_light_refresh_reason(recovery_asset_id);
        let light_leg_trusted = live_light_fresh
            && live_light_refresh_reason.is_none()
            && light_unsettled > 1e-6;
        let forced_pair_recovery_reason = if pair_base_should_force_recovery(
            current_phase,
            recovery_gap,
            release,
            light_leg_trusted,
        ) {
            if current_phase == PairBasePhaseState::MergePending {
                Some("merge_pending_latched".to_string())
            } else if recovery_asset_id.trim().is_empty() {
                Some("missing_light_asset".to_string())
            } else if let Some(reason) = live_light_refresh_reason.clone() {
                Some(reason)
            } else if light_oid.is_none() || light_unsettled <= 1e-6 {
                Some("missing_light_order".to_string())
            } else {
                let phase_age_s = (ctx.now - state_enter_ts).max(0.0);
                Some(format!(
                    "pair_resting_stale_live_leg age={phase_age_s:.1}s max_age={trusted_age_s:.1}s"
                ))
            }
        } else {
            None
        };
        let effective_recovery_mode = recovery_mode || forced_pair_recovery_reason.is_some();
        let recovery_active_key = "__pair_base_recovery_mode_active";
        let recovery_heavy_key = "__pair_base_recovery_heavy_yes";
        let recovery_log_key = "__pair_base_recovery_mode_log_until";
        let recovery_log_every = 5.0;
        if effective_recovery_mode {
            self._maker_ladder_cancel_all("pair_base recovery");
            self._maker_cancel_strategy_orders(Some(recovery_asset_id), "pair_base recovery");
            if self._runtime_ts_get(recovery_active_key) <= 0.0 {
                if let Some(reason) = forced_pair_recovery_reason.as_deref() {
                    self.logger.info(&format!(
                        "[PAIR_BASE] recovery enter gap={recovery_gap:.2} heavy={recovery_heavy_side} light={recovery_side} reason={reason}"
                    ));
                } else {
                    self.logger.info(&format!(
                        "[PAIR_BASE] recovery enter gap={recovery_gap:.2} heavy={recovery_heavy_side} light={recovery_side}"
                    ));
                }
                self._runtime_ts_set(recovery_active_key, 1.0);
                self._runtime_ts_set(
                    recovery_heavy_key,
                    if recovery_heavy_side == "YES" { 1.0 } else { 0.0 },
                );
                self._runtime_ts_set(recovery_log_key, ctx.now + recovery_log_every);
            } else if ctx.now >= self._runtime_ts_get(recovery_log_key) {
                self.logger.info(&format!(
                    "[PAIR_BASE] recovery remain gap={recovery_gap:.2} heavy={recovery_heavy_side} light={recovery_side} unsettled_heavy={recovery_unsettled_heavy:.2}"
                ));
                self._runtime_ts_set(recovery_log_key, ctx.now + recovery_log_every);
            }
            if light_unsettled > 1e-6 {
                let recovery_key = MakerOrderKey::buy(recovery_asset_id);
                if let Some(reason) = forced_pair_recovery_reason
                    .clone()
                    .filter(|reason| reason != "merge_pending_latched")
                    .or_else(|| self._maker_recovery_light_refresh_reason(recovery_asset_id))
                {
                    let _ = self._maker_order_request_cancel(&recovery_key, &reason);
                    self._maker_dbg_idle(
                        &format!(
                            "[PAIR_BASE] merge: requoting_light_leg reason={} gap={recovery_gap:.2} light_risk={light_unsettled:.2}",
                            reason
                        ),
                        "pair_base_recovery_refresh",
                    );
                    return None;
                }
                if !self._maker_recovery_light_requote_ready(recovery_asset_id) {
                    self._maker_cancel_strategy_orders(
                        Some(recovery_asset_id),
                        "pair_base recovery unsettled",
                    );
                    self._maker_dbg_idle(
                        &format!(
                            "[PAIR_BASE] merge: waiting_light_leg reason=unsettled gap={recovery_gap:.2} light_risk={light_unsettled:.2}"
                        ),
                        "pair_base_recovery_wait_unsettled",
                    );
                    return None;
                }
            }
        } else if self._runtime_ts_get(recovery_active_key) > 0.0 {
            self._maker_ladder_cancel_all("pair_base recovery settled");
            self._maker_cancel_strategy_orders(None, "pair_base recovery settled");
            self.logger.info(&format!(
                "[PAIR_BASE] recovery exit gap={recovery_gap:.2} threshold={:.2}",
                self._pair_arb_imbalance_release_shares()
            ));
            self._runtime_ts_set(recovery_active_key, 0.0);
            self._runtime_ts_set(recovery_heavy_key, 0.0);
            self._runtime_ts_set(recovery_log_key, 0.0);
        }
        Some(PairBaseRecoveryState {
            mode: effective_recovery_mode,
            gap: recovery_gap,
            heavy_side: recovery_heavy_side,
            light_side: recovery_side,
            light_asset_id: recovery_asset_id.to_string(),
        })
    }

    fn _maker_pair_base_risk_exit_step(&self, reason: &str, total_cost: f64, allow_taker: bool) {
        let (q_yes, q_no) = self._maker_actual_inventory();
        let gap = (q_yes - q_no).abs();
        let delta = q_yes - q_no;
        if let Ok(mut st) = self.pair_base_state.lock() {
            if pair_base_should_latch_risk_exit(reason) {
                st.risk_exit_latched = true;
            }
        }
        let pair_id = self
            .pair_base_state
            .lock()
            .ok()
            .and_then(|st| st.active_pair_id.clone())
            .unwrap_or_else(|| format!("pb-{}", now_ns()));
        self._pair_base_set_phase(
            PairBasePhaseState::RiskExitOnly,
            Some(pair_id.clone()),
            self._pair_base_live_order_id(self.yes_asset.as_deref().unwrap_or("")),
            self._pair_base_live_order_id(self.no_asset.as_deref().unwrap_or("")),
            gap,
            q_yes,
            q_no,
        );
        let t_left = (self.expiry_ts as f64 - now_ts_f64()).max(0.0);
        let risk_key = reason
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        self._maker_dbg_idle(
            &format!(
                "[PAIR_BASE] risk_exit_only reason={} gap={gap:.2} qYES={q_yes:.2} qNO={q_no:.2} total_cost={total_cost:.2} t_left={t_left:.1}s",
                reason
            ),
            &format!("pair_base_risk_exit_{risk_key}"),
        );
        if !allow_taker || gap < 0.01 {
            return;
        }
        let live_yes = self
            .yes_asset
            .as_deref()
            .and_then(|aid| self._pair_base_live_order_id(aid));
        let live_no = self
            .no_asset
            .as_deref()
            .and_then(|aid| self._pair_base_live_order_id(aid));
        if live_yes.is_some() || live_no.is_some() {
            self.cancel_all_open_orders_local("pair_base risk_exit preempt");
            if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                self._cancel_exchange_orders_for_assets(
                    &[y.clone(), n.clone()],
                    "pair_base risk_exit preempt",
                );
            }
            self._maker_dbg_idle(
                "[PAIR_BASE] risk_exit_only waiting_cancels",
                "pair_base_risk_exit_waiting_cancels",
            );
            return;
        }
        if gap + 1e-6 < self.cfg.min_shares {
            let heavy_asset = if delta > 0.0 {
                self.yes_asset.clone()
            } else {
                self.no_asset.clone()
            };
            if let Some(heavy_asset) = heavy_asset {
                let heavy_bid = self._best_bid_ask(&heavy_asset).map(|v| v.0).unwrap_or(0.0);
                let intended_notional_sell = if heavy_bid > 0.0 { gap * heavy_bid } else { 0.0 };
                self.logger.warning(&format!(
                    "[PAIR_BASE] risk_exit_action trigger={} action=taker_sell heavy_asset={} gap={gap:.2} intended_notional={intended_notional_sell:.4} qYES={q_yes:.2} qNO={q_no:.2}",
                    reason,
                    heavy_asset
                ));
            }
            let hedge_reason = format!("pair_base_{reason}");
            self._pair_base_exact_taker_hedge_step(delta, &hedge_reason);
        } else {
            let hedge_reason = format!("pair_base_{reason}");
            let missing_asset = if delta > 0.0 {
                self.no_asset.clone()
            } else {
                self.yes_asset.clone()
            };
            let Some(missing_asset) = missing_asset else {
                return;
            };
            let (_bid, ask) = self._best_bid_ask(&missing_asset).unwrap_or((0.0, 0.0));
            let intended_notional = if ask > 0.0 { gap * ask } else { 0.0 };
            self.logger.warning(&format!(
                "[PAIR_BASE] risk_exit_action trigger={} action=taker_buy missing_asset={} gap={gap:.2} intended_notional={intended_notional:.4} qYES={q_yes:.2} qNO={q_no:.2}",
                reason,
                missing_asset
            ));
            self._emergency_taker_hedge_step(delta, &hedge_reason);
        }
    }

    fn _maker_pair_base_recovery_step(
        &self,
        ctx: &MakerSkewLoopCtx,
        total_cost: f64,
        recovery: &PairBaseRecoveryState,
    ) {
        if !recovery.mode {
            return;
        }
        if recovery.light_asset_id.trim().is_empty() {
            self._maker_dbg_idle(
                "[PAIR_BASE] merge: waiting_light_leg reason=missing_light_asset",
                "pair_base_recovery_missing_asset",
            );
            return;
        }
        let gap = recovery.gap;
        let heavy = recovery.heavy_side.as_str();
        let light = recovery.light_side.as_str();
        let light_asset_id = recovery.light_asset_id.as_str();
        let pair_id = self
            .pair_base_state
            .lock()
            .ok()
            .and_then(|st| st.active_pair_id.clone())
            .unwrap_or_else(|| format!("pb-{}", now_ns()));
        let light_unsettled = self._maker_recovery_unsettled_buy_risk(light_asset_id);
        let remaining_gap = pair_base_remaining_gap(gap, light_unsettled);
        if remaining_gap < 0.01 {
            self._maker_dbg_idle(
                &format!(
                    "[PAIR_BASE] merge: waiting_light_leg reason=covered_by_live_order gap={gap:.2} light_risk={light_unsettled:.2}"
                ),
                "pair_base_recovery_covered",
            );
            return;
        }
        let min_shares = self.cfg.min_shares.max(1.0);
        if remaining_gap + 1e-6 < min_shares {
            let (q_yes_actual, q_no_actual) = self._maker_actual_inventory();
            match pair_base_sub_min_gap_policy() {
                PairBaseSubMinGapPolicy::Hold => {
                    self._pair_base_set_phase(
                        PairBasePhaseState::MergePending,
                        Some(pair_id),
                        self._pair_base_live_order_id(&ctx.yes_asset),
                        self._pair_base_live_order_id(&ctx.no_asset),
                        remaining_gap,
                        q_yes_actual,
                        q_no_actual,
                    );
                    self._maker_dbg_idle(
                        &format!(
                            "[PAIR_BASE] merge: sub_min_gap policy=hold gap={gap:.2} remaining_gap={remaining_gap:.2} heavy={heavy} light={light}"
                        ),
                        "pair_base_recovery_sub_min_hold",
                    );
                }
                PairBaseSubMinGapPolicy::TakerImmediate => {
                    self._maker_dbg_idle(
                        &format!(
                            "[PAIR_BASE] merge: sub_min_gap policy=taker_immediate gap={gap:.2} remaining_gap={remaining_gap:.2} heavy={heavy} light={light}"
                        ),
                        "pair_base_recovery_sub_min_taker",
                    );
                    self._maker_pair_base_risk_exit_step("sub_min_immediate", total_cost, true);
                }
            }
            return;
        }
        let other_asset = if light == "YES" {
            ctx.no_asset.as_str()
        } else {
            ctx.yes_asset.as_str()
        };
        let edge_ticks = env_int("MIN_ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks).max(0) as f64;
        let entry_edge = edge_ticks * self.cfg.tick.max(0.0001);
        let Some(bid) =
            self._maker_bid_cross_ask_safe(&light_asset_id, other_asset, entry_edge)
        else {
            self._maker_dbg_idle(
                &format!(
                    "[PAIR_BASE] merge: waiting_light_leg reason=no_safe_bid gap={gap:.2} heavy={heavy} light={light}"
                ),
                "pair_base_recovery_no_safe_bid",
            );
            return;
        };
        let ask = if light == "YES" { ctx.y_ask } else { ctx.n_ask };
        if ask <= 0.0 {
            self._maker_dbg_idle(
                &format!(
                    "[PAIR_BASE] merge: waiting_light_leg reason=missing_ask gap={gap:.2} light={light}"
                ),
                "pair_base_recovery_missing_ask",
            );
            return;
        }
        let merge_room = (self._pair_base_merge_budget() - total_cost).max(0.0);
        let max_affordable = round_down(merge_room / ask.max(1e-9), 0.01).max(0.0);
        let target_gap = round_down(remaining_gap, 0.01).max(0.0);
        let size = target_gap.min(max_affordable);
        if size < 0.01 {
            self._maker_pair_base_risk_exit_step("merge_budget_too_small", total_cost, false);
            return;
        }
        let (q_yes_actual, q_no_actual) = self._maker_actual_inventory();
        let fee_snap = self._pair_base_log_fee_net(
            "merge_requote",
            &pair_id,
            q_yes_actual,
            q_no_actual,
            total_cost,
            if light == "YES" { size } else { 0.0 },
            if light == "YES" { bid } else { 0.0 },
            if light == "NO" { size } else { 0.0 },
            if light == "NO" { bid } else { 0.0 },
        );
        if !pair_base_allows_merge_requote(fee_snap.fee_net_worst_case_pnl) {
            self._maker_dbg_idle(
                &format!(
                    "[PAIR_BASE] merge: stop negative_economics gap={gap:.2} remaining_gap={target_gap:.2} worst_case={:+.4} best_case={:+.4}",
                    fee_snap.fee_net_worst_case_pnl, fee_snap.fee_net_best_case_pnl
                ),
                "pair_base_recovery_negative_economics",
            );
            self._pair_base_set_phase(
                PairBasePhaseState::MergePending,
                Some(pair_id),
                self._pair_base_live_order_id(&ctx.yes_asset),
                self._pair_base_live_order_id(&ctx.no_asset),
                target_gap,
                q_yes_actual,
                q_no_actual,
            );
            return;
        }
        let key = MakerOrderKey::buy(&light_asset_id);
        let _ = self._maker_order_upsert_gtc(&key, bid, size, "PAIR_BASE_RECOVERY");
        self._pair_base_set_phase(
            PairBasePhaseState::MergePending,
            Some(pair_id),
            self._pair_base_live_order_id(&ctx.yes_asset),
            self._pair_base_live_order_id(&ctx.no_asset),
            target_gap,
            q_yes_actual,
            q_no_actual,
        );
        self._maker_record_trade_decision(
            ctx.t_into_s,
            bid,
            size,
            ctx.downside,
            ctx.upside,
            ctx.skew_ratio,
            false,
            None,
            &light,
            "GTC",
            "PAIR_BASE_RECOVERY",
        );
    }

    fn _maker_pair_base_step(&self, now: f64, q_yes: f64, q_no: f64, total_cost: f64) {
        let refresh_s = env_float("PAIR_BASE_REFRESH_SECONDS", 2.0).max(0.2);
        let start_after_s = env_float("MAKER_SKEW_START_AFTER_SECONDS", 15.0).max(0.0);
        let stop_new_after_s = env_float("MAKER_SKEW_STOP_NEW_AFTER_SECONDS", 290.0).max(1.0);
        let t_into_s = (now - self.start_ts as f64).max(0.0);
        let (downside, upside, skew_ratio) = Self::_maker_payoff_envelope(q_yes, q_no, total_cost);
        let last_decision_ts = self
            .maker_skew_state
            .lock()
            .map(|s| s.last_decision_ts)
            .unwrap_or(0.0);
        if now - last_decision_ts < refresh_s {
            return;
        }
        if let Ok(mut st) = self.maker_skew_state.lock() {
            st.last_decision_ts = now;
            st.downside = downside;
            st.upside = upside;
            st.skew_ratio = skew_ratio;
        }

        self._maker_ladder_cancel_all("pair_base");

        if t_into_s < start_after_s {
            self._pair_base_set_phase(PairBasePhaseState::Flat, None, None, None, 0.0, q_yes, q_no);
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "BOTH",
                "GTC",
                "PAIR_BASE_WARMUP",
            );
            return;
        }
        if t_into_s >= stop_new_after_s {
            self._pair_base_cancel_orders("PAIR_BASE stop new orders");
            self._pair_base_set_phase(PairBasePhaseState::Flat, None, None, None, 0.0, q_yes, q_no);
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "BOTH",
                "GTC",
                "PAIR_BASE_STOP_NEW",
            );
            return;
        }

        let (Some(yes), Some(no)) = (&self.yes_asset, &self.no_asset) else {
            return;
        };
        let yq = self._best_bid_ask(yes).unwrap_or((0.0, 0.0));
        let nq = self._best_bid_ask(no).unwrap_or((0.0, 0.0));
        let include_open_buys = env_bool("MAKER_EFFECTIVE_Q_INCLUDE_OPEN_BUYS", true);
        let q_yes_eff = if include_open_buys {
            q_yes + self._maker_order_open_buy_remaining(yes)
        } else {
            q_yes
        };
        let q_no_eff = if include_open_buys {
            q_no + self._maker_order_open_buy_remaining(no)
        } else {
            q_no
        };
        let ctx = MakerSkewLoopCtx {
            now,
            t_into_s,
            peak_window: false,
            total_cost,
            budget_usable: 0.0,
            yes_asset: yes.clone(),
            no_asset: no.clone(),
            y_bid: yq.0,
            y_ask: yq.1,
            n_bid: nq.0,
            n_ask: nq.1,
            q_yes_eff,
            q_no_eff,
            downside,
            upside,
            skew_ratio,
        };

        let actual_gap = (q_yes - q_no).abs();
        let has_inventory = q_yes > 1e-6 || q_no > 1e-6;
        let release = self._pair_arb_imbalance_release_shares();
        let (_current_phase, active_pair_id, current_target_qty, risk_exit_latched) = self
            .pair_base_state
            .lock()
            .map(|st| (st.phase, st.active_pair_id.clone(), st.target_qty, st.risk_exit_latched))
            .unwrap_or((PairBasePhaseState::Flat, None, 0.0, false));
        let live_yes_oid = self._pair_base_live_order_id(yes);
        let live_no_oid = self._pair_base_live_order_id(no);
        let pair_orders_live = live_yes_oid.is_some() || live_no_oid.is_some();
        if risk_exit_latched {
            if actual_gap > release + 1e-6 {
                self._maker_pair_base_risk_exit_step("latched", total_cost, true);
            } else {
                self._pair_base_cancel_orders("PAIR_BASE risk exit latched");
                self._pair_base_set_phase(
                    PairBasePhaseState::Balanced,
                    None,
                    None,
                    None,
                    0.0,
                    q_yes,
                    q_no,
                );
                self._maker_dbg_idle(
                    "[PAIR_BASE] idle: risk_exit_latched",
                    "pair_base_idle_risk_exit_latched",
                );
            }
            return;
        }
        if has_inventory {
            let t_left = (self.expiry_ts as f64 - now).max(0.0);
            let risk_exit_lead_s =
                pair_base_early_risk_exit_lead_seconds(self.cfg.stop_buffer_seconds as f64);
            if actual_gap > release + 1e-6 && t_left <= risk_exit_lead_s {
                self._maker_pair_base_risk_exit_step("near_expiry", total_cost, true);
                return;
            }
            let max_loss_limit =
                env_float("PAIR_BASE_MAX_WORST_CASE_LOSS_USDC", self._pair_base_window_budget() * 0.5)
                    .max(0.0);
            let fee_snap =
                self._pair_base_fee_net_snapshot(q_yes, q_no, total_cost, 0.0, 0.0, 0.0, 0.0);
            if actual_gap > release + 1e-6 && fee_snap.fee_net_worst_case_pnl <= -max_loss_limit {
                self._maker_pair_base_risk_exit_step(
                    &format!(
                        "max_loss(worst={:.2},limit={:.2})",
                        fee_snap.fee_net_worst_case_pnl, max_loss_limit
                    ),
                    total_cost,
                    true,
                );
                return;
            }
        }

        if self._pair_recovery_enabled() {
            let Some(recovery) = self._maker_pair_base_recovery_phase(&ctx) else {
                let pair_id = self
                    .pair_base_state
                    .lock()
                    .ok()
                    .and_then(|st| st.active_pair_id.clone())
                    .unwrap_or_else(|| format!("pb-{}", now_ns()));
                self._pair_base_set_phase(
                    PairBasePhaseState::MergePending,
                    Some(pair_id),
                    self._pair_base_live_order_id(yes),
                    self._pair_base_live_order_id(no),
                    (q_yes - q_no).abs(),
                    q_yes,
                    q_no,
                );
                return;
            };
            if recovery.mode {
                self._maker_pair_base_recovery_step(&ctx, total_cost, &recovery);
                return;
            }
        }

        if let Some(phase) =
            pair_base_phase_without_recovery(has_inventory, actual_gap, release, pair_orders_live)
        {
            match phase {
                PairBasePhaseState::PairResting => {
                    self._pair_base_set_phase(
                        PairBasePhaseState::PairResting,
                        Some(active_pair_id.unwrap_or_else(|| format!("pb-{}", now_ns()))),
                        live_yes_oid,
                        live_no_oid,
                        current_target_qty,
                        q_yes,
                        q_no,
                    );
                    return;
                }
                PairBasePhaseState::Balanced => {
                    self._pair_base_set_phase(
                        PairBasePhaseState::Balanced,
                        None,
                        None,
                        None,
                        0.0,
                        q_yes,
                        q_no,
                    );
                }
                PairBasePhaseState::Flat => {
                    self._pair_base_set_phase(
                        PairBasePhaseState::Flat,
                        None,
                        None,
                        None,
                        0.0,
                        q_yes,
                        q_no,
                    );
                }
                PairBasePhaseState::MergePending | PairBasePhaseState::RiskExitOnly => {}
            }
        }

        let (ok, why) = self._maker_quote_only_allowed(yes, no);
        if !ok {
            self._maker_dbg_idle(
                &format!("[PAIR_BASE] idle: {why}"),
                &format!("pair_base_idle_{}", why.split('(').next().unwrap_or("unknown")),
            );
            self._pair_base_cancel_orders(&format!("PAIR_BASE gate: {why}"));
            return;
        }

        let min_entry_edge_ticks = env_int("MIN_ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks).max(0);
        let entry_edge = min_entry_edge_ticks as f64 * self.cfg.tick.max(0.0001);
        let y_bid = self._maker_bid_cross_ask_safe(yes, no, entry_edge);
        let n_bid = self._maker_bid_cross_ask_safe(no, yes, entry_edge);
        if y_bid.is_none() || n_bid.is_none() {
            self._maker_dbg_idle(
                "[PAIR_BASE] idle: no_pair_edge reason=no_safe_bids",
                "pair_base_idle_no_safe_bids",
            );
            self._pair_base_cancel_orders("PAIR_BASE no safe bids");
            return;
        }
        let y_bid = y_bid.unwrap_or(0.0);
        let n_bid = n_bid.unwrap_or(0.0);
        let (_, y_ask) = yq;
        let (_, n_ask) = nq;
        let tick = self.cfg.tick.max(0.0001);
        let buf = env_float("PAIRED_ENTRY_BUFFER_TICKS", 0.0) * tick;
        let tix = |p: f64| -> i64 { (p / tick + 1e-9).round() as i64 };
        let thr_no_ticks = tix(1.0 - y_bid - buf);
        let thr_yes_ticks = tix(1.0 - n_bid - buf);
        if tix(n_ask) > thr_no_ticks
            || tix(y_ask) > thr_yes_ticks
            || (y_bid + n_bid) > (1.0 - entry_edge)
        {
            self._maker_dbg_idle(
                &format!(
                    "[PAIR_BASE] idle: no_pair_edge reason=paired_gate_fail sum={:.3} y_bid={y_bid:.3} n_bid={n_bid:.3} y_ask={y_ask:.3} n_ask={n_ask:.3}",
                    y_bid + n_bid
                ),
                "pair_base_idle_paired_gate",
            );
            self._pair_base_cancel_orders("PAIR_BASE paired gate fail");
            return;
        }

        let pair_budget = self._pair_base_window_budget();
        let hard_reserve = self._pair_base_hard_reserve();
        let remaining = pair_budget - total_cost;
        let budget_usable = (remaining - hard_reserve).max(0.0);
        let pair_sum = (y_bid + n_bid).max(1e-9);
        let min_shares = self.cfg.min_shares.max(1.0);
        let max_affordable = (budget_usable / pair_sum).floor();
        if max_affordable + 1e-9 < min_shares {
            self._maker_dbg_idle(
                &format!(
                    "[PAIR_BASE] idle: budget_too_small pair_sum={pair_sum:.3} pair_budget={pair_budget:.2} usable={budget_usable:.2} reserve={hard_reserve:.2}"
                ),
                "pair_base_idle_budget",
            );
            self._pair_base_cancel_orders("PAIR_BASE budget too small");
            return;
        }
        let size = self.cfg.clip_shares.max(min_shares).min(max_affordable).floor() as i64;
        if size < min_shares.ceil() as i64 {
            self._maker_dbg_idle(
                &format!(
                    "[PAIR_BASE] idle: clip_below_min size={size} min={min_shares:.2} pair_sum={pair_sum:.3} usable={budget_usable:.2}"
                ),
                "pair_base_idle_clip_below_min",
            );
            return;
        }

        let pair_id = self
            .pair_base_state
            .lock()
            .ok()
            .and_then(|st| {
                if st.phase == PairBasePhaseState::PairResting {
                    st.active_pair_id.clone()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| format!("pb-{}", now_ns()));

        self._pair_base_log_fee_net(
            "pair_entry",
            &pair_id,
            q_yes,
            q_no,
            total_cost,
            size as f64,
            y_bid,
            size as f64,
            n_bid,
        );
        let (y_oid, n_oid) =
            self._maker_submit_pair_orders(size, y_bid, n_bid, "GTC", Some(true), "PAIR_BASE_GTC");
        if y_oid.is_some() ^ n_oid.is_some() {
            self._pair_base_cancel_orders("PAIR_BASE asymmetric submit");
            self._pair_base_set_phase(PairBasePhaseState::Flat, None, None, None, 0.0, q_yes, q_no);
            self._maker_dbg_idle(
                "[PAIR_BASE] idle: no_pair_edge reason=asymmetric_submit",
                "pair_base_idle_asymmetric_submit",
            );
            return;
        }

        let yes_oid_live = self._pair_base_live_order_id(yes).or(y_oid);
        let no_oid_live = self._pair_base_live_order_id(no).or(n_oid);
        if yes_oid_live.is_none() && no_oid_live.is_none() {
            self._maker_dbg_idle(
                "[PAIR_BASE] idle: no_pair_edge reason=no_pair_orders_live",
                "pair_base_idle_no_live_orders",
            );
            return;
        }

        self._pair_base_set_phase(
            PairBasePhaseState::PairResting,
            Some(pair_id),
            yes_oid_live,
            no_oid_live,
            size as f64,
            q_yes,
            q_no,
        );
        self._maker_record_trade_decision(
            t_into_s,
            0.5 * (y_bid + n_bid),
            size as f64,
            downside,
            upside,
            skew_ratio,
            false,
            None,
            "BOTH",
            "GTC",
            "PAIR_BASE_GTC",
        );
    }

    fn _maker_record_trade_decision(
        &self,
        t_into_s: f64,
        reference_price: f64,
        clip: f64,
        downside: f64,
        upside: f64,
        skew_ratio: f64,
        arb_triggered: bool,
        arb_edge_after_fees: Option<f64>,
        side: &str,
        order_type: &str,
        submit_origin: &str,
    ) {
        let t_left = (self.expiry_ts as f64 - now_ts_f64()).max(0.0);
        let row = TradeDecisionUpsert {
            t_left_seconds: Some(t_left),
            submit_origin: Some(submit_origin.to_string()),
            submit_side: Some(side.to_string()),
            submit_order_type: Some(order_type.to_string()),
            order_type: Some(order_type.to_string()),
            qty_requested: Some(clip.max(0.0)),
            limit_price_submitted: if reference_price > 0.0 {
                Some(reference_price)
            } else {
                None
            },
            maker_downside: Some(downside),
            maker_upside: Some(upside),
            maker_skew_ratio: Some(skew_ratio),
            maker_arb_triggered: Some(arb_triggered),
            maker_arb_edge_after_fees: arb_edge_after_fees,
            maker_t_into_s: Some(t_into_s.max(0.0)),
            maker_price_bucket: Some(Self::_maker_price_bucket(reference_price)),
            maker_clip_bucket: Some(Self::_maker_clip_bucket(clip)),
            ..TradeDecisionUpsert::default()
        };
        if let Ok(mut holder) = self.sniper_trade_decision.lock() {
            *holder = Some(SniperTradeDecisionRuntime {
                order_id: None,
                data: row,
            });
        }
    }

    fn _maker_skew_try_arb(
        &self,
        budget_usable: f64,
        y_bid: f64,
        y_ask: f64,
        n_bid: f64,
        n_ask: f64,
        t_into_s: f64,
        downside: f64,
        upside: f64,
        skew_ratio: f64,
    ) -> bool {
        if !env_bool("MAKER_ARB_ENABLED", true) || budget_usable <= 0.0 {
            return false;
        }
        let order_type = self._resolve_order_type(
            &std::env::var("MAKER_ARB_ORDER_TYPE").unwrap_or_else(|_| "FAK".to_string()),
        );
        let is_maker = order_type == "GTC";
        let strict_passive = env_bool("MAKER_ARB_STRICT_PASSIVE", true);
        let threshold = if is_maker {
            env_float("MAKER_ARB_MAKER_THRESHOLD", 0.995)
        } else {
            env_float("MAKER_ARB_TAKER_THRESHOLD", 0.985)
        }
        .clamp(0.0, 1.0);
        let p_yes = if is_maker { y_bid } else { y_ask };
        let p_no = if is_maker { n_bid } else { n_ask };
        if p_yes <= 0.0 || p_no <= 0.0 {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_SKEW][ARB] idle: missing_prices type={} p_yes={p_yes:.3} p_no={p_no:.3}",
                    order_type
                ),
                "maker_skew_arb_idle_missing_prices",
            );
            return false;
        }
        let sum_px = p_yes + p_no;
        if sum_px > threshold + 1e-12 {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_SKEW][ARB] idle: sum_above_threshold type={} sum={sum_px:.3} thr={threshold:.3} p_yes={p_yes:.3} p_no={p_no:.3}",
                    order_type
                ),
                "maker_skew_arb_idle_sum_above_threshold",
            );
            return false;
        }

        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let max_tick = env_int(
            "MAKER_ARB_MAX_SHARES_PER_TICK",
            self.cfg.clip_shares.max(self.cfg.min_shares).floor() as i64,
        )
        .max(min_int);
        let max_affordable = (budget_usable / sum_px + 1e-12).floor() as i64;
        let size_int = max_affordable.min(max_tick);
        if size_int < min_int {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_SKEW][ARB] idle: size_below_min type={} size={} min={} budget={budget_usable:.2} sum={sum_px:.3}",
                    order_type, size_int, min_int
                ),
                "maker_skew_arb_idle_size_below_min",
            );
            return false;
        }
        let edge_after_fees =
            self._maker_pair_edge_after_fees(size_int as f64, p_yes, p_no, is_maker);
        if edge_after_fees <= 0.0 {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_SKEW][ARB] idle: non_positive_edge type={} size={} edge={edge_after_fees:+.6} sum={sum_px:.3}",
                    order_type, size_int
                ),
                "maker_skew_arb_idle_non_positive_edge",
            );
            return false;
        }

        self.logger.info(&format!(
            "[MAKER_SKEW][ARB] trigger type={} size={} p_yes={:.3} p_no={:.3} sum={:.3} thr={:.3} edge_after_fees={:+.6}",
            order_type,
            size_int,
            p_yes,
            p_no,
            sum_px,
            threshold,
            edge_after_fees
        ));
        let (qy0, qn0) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no))
            .unwrap_or((0.0, 0.0));
        let (q_yes_actual, q_no_actual) = self._maker_actual_inventory();
        let current_gap = (q_yes_actual - q_no_actual).abs();
        let max_imbalance = env_float(
            "PAIR_ARB_MAX_IMBALANCE_SHARES",
            self.cfg.clip_shares.max(self.cfg.min_shares),
        )
        .max(1.0);
        if self._pair_arb_pending_active() {
            self.logger.info(&format!(
                "[MAKER_SKEW][ARB] suppressed: pending imbalance active (qYES={q_yes_actual:.2} qNO={q_no_actual:.2} gap={current_gap:.2})"
            ));
            return false;
        }
        let projected_gap = current_gap + size_int as f64;
        if projected_gap > max_imbalance + 1e-6 {
            self.logger.info(&format!(
                "[MAKER_SKEW][ARB] suppressed: projected_gap={projected_gap:.1} > max={max_imbalance:.1} (qYES={q_yes_actual:.1} qNO={q_no_actual:.1} size={size_int})"
            ));
            return false;
        }

        let (y_oid, n_oid) = self._maker_submit_pair_orders(
            size_int,
            p_yes,
            p_no,
            &order_type,
            if is_maker && strict_passive {
                Some(true)
            } else {
                None
            },
            &format!("MAKER_SKEW_ARB_{}", order_type),
        );
        if is_maker && strict_passive && (y_oid.is_some() ^ n_oid.is_some()) {
            if let Some(oid) = &y_oid {
                let _ = self._cancel(oid);
            }
            if let Some(oid) = &n_oid {
                let _ = self._cancel(oid);
            }
            return false;
        }
        if y_oid.is_none() && n_oid.is_none() {
            self._maker_record_trade_decision(
                t_into_s,
                p_yes.min(p_no),
                size_int as f64,
                downside,
                upside,
                skew_ratio,
                false,
                Some(edge_after_fees),
                "BOTH",
                &order_type,
                "MAKER_SKEW_ARB_SKIP",
            );
            return false;
        }

        let timeout_s = env_float("PAIR_ARB_TIMEOUT_SECONDS", 2.0).max(0.2);
        let y0 = 0.0;
        let n0 = 0.0;
        let (fy, fn_) = if is_maker {
            self._wait_for_pair_order_fills(
                y_oid.as_deref(),
                n_oid.as_deref(),
                y0,
                n0,
                size_int,
                timeout_s,
            )
        } else {
            self._wait_for_pair_fills(qy0, qn0, size_int, timeout_s)
        };
        self.logger.info(&format!(
            "[MAKER_SKEW][ARB] fill wait y_oid={} n_oid={} fy={fy:.2} fn={fn_:.2}",
            y_oid
                .as_deref()
                .map(|s| s.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "?".to_string()),
            n_oid
                .as_deref()
                .map(|s| s.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "?".to_string())
        ));
        let mismatch = (fy - fn_).abs() > 1e-6;
        let target = size_int as f64;
        let y_filled = fy >= target - 1e-6;
        let n_filled = fn_ >= target - 1e-6;
        let mut y_live = false;
        let mut n_live = false;
        if mismatch && is_maker && self._maker_single_inflight_enabled() {
            let max_live_age = env_float(
                "PAIR_ARB_LIVE_DEFER_MAX_SECONDS",
                (timeout_s * 3.0).max(0.5),
            )
            .max(0.2);
            let yes_asset = self.yes_asset.as_deref().unwrap_or("");
            let no_asset = self.no_asset.as_deref().unwrap_or("");
            if !yes_asset.is_empty() {
                self._maker_order_reconcile_asset(yes_asset, Some(p_yes));
            }
            if !no_asset.is_empty() {
                self._maker_order_reconcile_asset(no_asset, Some(p_no));
            }
            y_live = !y_filled
                && y_oid
                    .as_deref()
                    .map(|oid| self._maker_order_is_live(yes_asset, Some(oid), max_live_age))
                    .unwrap_or(false);
            n_live = !n_filled
                && n_oid
                    .as_deref()
                    .map(|oid| self._maker_order_is_live(no_asset, Some(oid), max_live_age))
                    .unwrap_or(false);
        }

        if mismatch && (y_live || n_live) {
            // Unfilled leg still live on exchange - don't panic-hedge.
            // Cancel only the filled leg's residual when reconcile-after-timeout is enabled.
            if env_bool("PAIR_ARB_RECONCILE_AFTER_TIMEOUT", true) {
                if !y_live {
                    if let Some(oid) = &y_oid {
                        let _ = self._cancel(oid);
                    }
                }
                if !n_live {
                    if let Some(oid) = &n_oid {
                        let _ = self._cancel(oid);
                    }
                }
            }
            let heavy_side = if fy >= fn_ { "YES" } else { "NO" };
            let light_side = if heavy_side == "YES" { "NO" } else { "YES" };
            self._pair_arb_set_pending_imbalance(
                y_oid.as_deref(),
                n_oid.as_deref(),
                heavy_side,
                light_side,
                (fy - fn_).abs(),
            );
            self.logger.info(&format!(
                "Pair arb partial: fy={fy:.0} fn={fn_:.0} - unfilled leg still live (y_live={y_live} n_live={n_live}), deferring hedge"
            ));
        } else {
            // Both sides resolved or dead - normal cleanup.
            if env_bool("PAIR_ARB_RECONCILE_AFTER_TIMEOUT", true) {
                if let Some(oid) = &y_oid {
                    let _ = self._cancel(oid);
                }
                if let Some(oid) = &n_oid {
                    let _ = self._cancel(oid);
                }
            }
            if mismatch {
                self._handle_exposure_mismatch(fy, fn_);
            } else {
                self._pair_arb_clear_pending_if_resolved();
            }
        }
        self._maker_record_trade_decision(
            t_into_s,
            p_yes.min(p_no),
            size_int as f64,
            downside,
            upside,
            skew_ratio,
            true,
            Some(edge_after_fees),
            "BOTH",
            &order_type,
            "MAKER_SKEW_ARB",
        );
        true
    }

    fn _maker_quote_only_step(&self, now: f64, q_yes: f64, q_no: f64, total_cost: f64) {
        let refresh_s = env_float("MAKER_SKEW_REFRESH_SECONDS", 2.0).max(0.2);
        let start_after_s = env_float("MAKER_SKEW_START_AFTER_SECONDS", 15.0).max(0.0);
        let stop_new_after_s = env_float("MAKER_SKEW_STOP_NEW_AFTER_SECONDS", 290.0).max(1.0);
        let t_into_s = (now - self.start_ts as f64).max(0.0);
        let (downside, upside, skew_ratio) = Self::_maker_payoff_envelope(q_yes, q_no, total_cost);
        let last_decision_ts = self
            .maker_skew_state
            .lock()
            .map(|s| s.last_decision_ts)
            .unwrap_or(0.0);
        if now - last_decision_ts < refresh_s {
            return;
        }
        if let Ok(mut st) = self.maker_skew_state.lock() {
            st.last_decision_ts = now;
            st.downside = downside;
            st.upside = upside;
            st.skew_ratio = skew_ratio;
        }
        if t_into_s < start_after_s {
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "BOTH",
                "GTC",
                "MAKER_QUOTE_ONLY_WARMUP",
            );
            return;
        }
        if t_into_s >= stop_new_after_s {
            self._maker_ladder_cancel_all("quote-only stop_new_after");
            self._maker_cancel_strategy_orders(None, "MAKER_QUOTE_ONLY stop new orders");
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "BOTH",
                "GTC",
                "MAKER_QUOTE_ONLY_STOP_NEW",
            );
            return;
        }

        let remaining = self.cfg.max_total_cost - total_cost;
        if remaining <= self.cfg.reserve_usd {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_QUOTE_ONLY] idle: budget_too_small remaining={remaining:.2} reserve={:.2}",
                    self.cfg.reserve_usd
                ),
                "maker_quote_only_idle_budget",
            );
            self._maker_ladder_cancel_all("quote-only reserve reached");
            self._maker_cancel_strategy_orders(None, "MAKER_QUOTE_ONLY reserve reached");
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "BOTH",
                "GTC",
                "MAKER_QUOTE_ONLY_BUDGET_EXHAUSTED",
            );
            return;
        }
        let budget_usable = (remaining - self.cfg.reserve_usd).max(0.0);

        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return,
        };
        let (ok, why) = self._maker_quote_only_allowed(yes, no);
        if !ok {
            let why_key = why.split('(').next().unwrap_or("unknown");
            self._maker_dbg_idle(
                &format!("[MAKER_QUOTE_ONLY] idle: {why}"),
                &format!("maker_quote_only_idle_gate_{why_key}"),
            );
            self._maker_ladder_cancel_all(&format!("quote-only gate: {why}"));
            self._maker_cancel_strategy_orders(None, &format!("quote-only gate: {why}"));
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "BOTH",
                "GTC",
                &format!("MAKER_QUOTE_ONLY_GATE_{why}"),
            );
            return;
        }

        let min_entry_edge_ticks = env_int("MIN_ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks) as i64;
        let effective_edge_ticks = self.cfg.entry_edge_ticks.max(min_entry_edge_ticks);
        let entry_edge = effective_edge_ticks as f64 * self.cfg.tick;
        let y_bid = self._maker_bid_cross_ask_safe(yes, no, entry_edge);
        let n_bid = self._maker_bid_cross_ask_safe(no, yes, entry_edge);
        if y_bid.is_none() || n_bid.is_none() {
            self._maker_dbg_idle(
                "[MAKER_QUOTE_ONLY] idle: no_pair_edge reason=no_safe_bids",
                "maker_quote_only_idle_no_safe_bids",
            );
            self._maker_cancel_strategy_orders(None, "quote-only no safe bids");
            return;
        }
        let y_bid = y_bid.unwrap_or(0.0);
        let n_bid = n_bid.unwrap_or(0.0);

        let yq = self._best_bid_ask(yes);
        let nq = self._best_bid_ask(no);
        if yq.is_none() || nq.is_none() {
            self._maker_dbg_idle(
                "[MAKER_QUOTE_ONLY] idle: no_pair_edge reason=missing_quotes_for_pair_gate",
                "maker_quote_only_idle_missing_quotes",
            );
            self._maker_cancel_strategy_orders(None, "quote-only missing quotes");
            return;
        }
        let (_, y_ask) = yq.unwrap_or((0.0, 0.0));
        let (_, n_ask) = nq.unwrap_or((0.0, 0.0));
        let tick = if self.cfg.tick > 0.0 { self.cfg.tick } else { 0.01 };
        let buf = env_float("PAIRED_ENTRY_BUFFER_TICKS", 0.0) * tick;
        let tix = |p: f64| -> i64 { (p / tick + 1e-9).round() as i64 };
        let thr_no_ticks = tix(1.0 - y_bid - buf);
        let thr_yes_ticks = tix(1.0 - n_bid - buf);
        if tix(n_ask) > thr_no_ticks || tix(y_ask) > thr_yes_ticks || (y_bid + n_bid) > (1.0 - entry_edge) {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_QUOTE_ONLY] idle: no_pair_edge reason=paired_gate_fail sum={:.3} y_bid={y_bid:.3} n_bid={n_bid:.3} y_ask={y_ask:.3} n_ask={n_ask:.3}",
                    y_bid + n_bid
                ),
                "maker_quote_only_idle_paired_gate",
            );
            self._maker_cancel_strategy_orders(None, "quote-only paired gate fail");
            return;
        }

        let mut size = self.cfg.clip_shares.max(self.cfg.min_shares).max(1.0);
        if env_bool("DEPTH_GATE_ENABLED", false) {
            let (okd, whyd) = self._depth_gate_accumulate(size, y_bid, n_bid, buf);
            if !okd && !env_bool("DEPTH_GATE_WARN_ONLY", false) {
                self._maker_dbg_idle(
                    &format!("[MAKER_QUOTE_ONLY] idle: no_pair_edge reason=depth_gate({whyd})"),
                    "maker_quote_only_idle_depth_gate",
                );
                self._maker_cancel_strategy_orders(None, &format!("quote-only depth gate: {whyd}"));
                return;
            }
        }

        let pair_sum = (y_bid + n_bid).max(1e-9);
        let max_affordable = (budget_usable / pair_sum).floor();
        if max_affordable < self.cfg.min_shares.max(1.0) {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_QUOTE_ONLY] idle: budget_too_small pair_sum={pair_sum:.3} usable={budget_usable:.2}"
                ),
                "maker_quote_only_idle_pair_budget",
            );
            self._maker_cancel_strategy_orders(None, "quote-only budget too small");
            self._maker_record_trade_decision(
                t_into_s,
                pair_sum,
                max_affordable.max(0.0),
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "BOTH",
                "GTC",
                "MAKER_QUOTE_ONLY_BUDGET_TOO_SMALL",
            );
            return;
        }
        size = size.min(max_affordable);
        let (recovery_active, recovery_gap, _heavy_side, _light_side, _light_asset, _unsettled_heavy) =
            self._maker_recovery_mode_snapshot();
        if recovery_active && recovery_gap > 0.0 {
            size = size.min(recovery_gap);
        }
        if size < self.cfg.min_shares.max(1.0) {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_QUOTE_ONLY] idle: clip_below_min clip={size:.2} min={:.2}",
                    self.cfg.min_shares.max(1.0)
                ),
                "maker_quote_only_idle_clip_below_min",
            );
            return;
        }

        self._maker_ladder_cancel_all("quote-only");
        let placed_yes = self._maybe_replace(yes, y_bid, size, None);
        let placed_no = self._maybe_replace(no, n_bid, size, None);
        self._maker_record_trade_decision(
            t_into_s,
            0.5 * (y_bid + n_bid),
            size,
            downside,
            upside,
            skew_ratio,
            false,
            None,
            "BOTH",
            "GTC",
            if placed_yes || placed_no {
                "MAKER_QUOTE_ONLY"
            } else {
                "MAKER_QUOTE_ONLY_WAIT"
            },
        );
    }

    fn _maker_skew_arb_step(&self, now: f64, q_yes: f64, q_no: f64, total_cost: f64) {
        if self._pair_base_mode_enabled() {
            self._maker_pair_base_step(now, q_yes, q_no, total_cost);
            return;
        }
        if !env_bool("MAKER_SKEW_ENABLED", true) {
            if !env_bool("MAKER_ARB_ENABLED", true)
                && !env_bool("MAKER_STRETCH_BIAS_ENABLED", false)
            {
                self._maker_quote_only_step(now, q_yes, q_no, total_cost);
                return;
            }
            self._maker_ladder_cancel_all("disabled");
            self._maker_cancel_strategy_orders(None, "MAKER_SKEW disabled");
            return;
        }
        let refresh_base_s = env_float("MAKER_SKEW_REFRESH_SECONDS", 2.0).max(0.2);
        let start_after_s = env_float("MAKER_SKEW_START_AFTER_SECONDS", 15.0).max(0.0);
        let stop_new_after_s = env_float("MAKER_SKEW_STOP_NEW_AFTER_SECONDS", 290.0).max(1.0);
        let peak_start_s = env_float("MAKER_SKEW_PEAK_START_SECONDS", 60.0).max(0.0);
        let peak_end_s = env_float("MAKER_SKEW_PEAK_END_SECONDS", 180.0).max(peak_start_s);
        let t_into_s = (now - self.start_ts as f64).max(0.0);
        let peak_window = t_into_s >= peak_start_s && t_into_s <= peak_end_s;
        let refresh_peak_s = env_float(
            "MAKER_SKEW_REFRESH_SECONDS_PEAK",
            (refresh_base_s * 0.5).max(0.2),
        )
        .max(0.05);
        let refresh_offpeak_s = env_float(
            "MAKER_SKEW_REFRESH_SECONDS_OFFPEAK",
            (refresh_base_s * 1.5).max(refresh_base_s),
        )
        .max(refresh_peak_s);
        let refresh_s = if peak_window {
            refresh_peak_s
        } else {
            refresh_offpeak_s
        };

        self._maker_skew_update_state(now, q_yes, q_no, total_cost);
        let (downside, upside, skew_ratio, last_decision_ts, unhedged_age) = self
            .maker_skew_state
            .lock()
            .map(|s| {
                let age = if s.unhedged_since > 0.0 {
                    (now - s.unhedged_since).max(0.0)
                } else {
                    0.0
                };
                (s.downside, s.upside, s.skew_ratio, s.last_decision_ts, age)
            })
            .unwrap_or((0.0, 0.0, 1.0, 0.0, 0.0));

        if self._maybe_trigger_max_loss(q_yes - q_no, unhedged_age) {
            return;
        }

        if now - last_decision_ts < refresh_s {
            return;
        }
        if let Ok(mut st) = self.maker_skew_state.lock() {
            st.last_decision_ts = now;
        }
        if t_into_s < start_after_s {
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "NONE",
                "NONE",
                "MAKER_SKEW_WARMUP",
            );
            return;
        }
        if t_into_s >= stop_new_after_s {
            self._maker_ladder_cancel_all("stop_new_after");
            self._maker_cancel_strategy_orders(None, "MAKER_SKEW stop new orders");
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "NONE",
                "NONE",
                "MAKER_SKEW_STOP_NEW",
            );
            return;
        }

        let window_budget = env_float("MAKER_SKEW_WINDOW_BUDGET_USDC", 1000.0).max(1.0);
        let hard_cap = self.cfg.max_total_cost.min(window_budget);
        let ladder_reserved = self._maker_ladder_reserved_notional();
        let remaining = hard_cap - total_cost - ladder_reserved;
        if remaining <= self.cfg.reserve_usd {
            self._maker_ladder_cancel_all("reserve reached");
            self._maker_cancel_strategy_orders(None, "MAKER_SKEW reserve reached");
            self._maker_record_trade_decision(
                t_into_s,
                0.0,
                0.0,
                downside,
                upside,
                skew_ratio,
                false,
                None,
                "NONE",
                "NONE",
                "MAKER_SKEW_RESERVE_REACHED",
            );
            return;
        }
        let budget_usable = (remaining - self.cfg.reserve_usd).max(0.0);

        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return,
        };
        let yq = self._best_bid_ask(yes);
        let nq = self._best_bid_ask(no);
        let (Some((y_bid, y_ask)), Some((n_bid, n_ask))) = (yq, nq) else {
            return;
        };
        if y_bid <= 0.0 || y_ask <= 0.0 || n_bid <= 0.0 || n_ask <= 0.0 {
            return;
        }

        let include_open_buys = env_bool("MAKER_EFFECTIVE_Q_INCLUDE_OPEN_BUYS", true);
        let q_yes_eff = if include_open_buys {
            q_yes + self._maker_order_open_buy_remaining(yes)
        } else {
            q_yes
        };
        let q_no_eff = if include_open_buys {
            q_no + self._maker_order_open_buy_remaining(no)
        } else {
            q_no
        };
        let ctx = MakerSkewLoopCtx {
            now,
            t_into_s,
            peak_window,
            total_cost,
            budget_usable,
            yes_asset: yes.to_string(),
            no_asset: no.to_string(),
            y_bid,
            y_ask,
            n_bid,
            n_ask,
            q_yes_eff,
            q_no_eff,
            downside,
            upside,
            skew_ratio,
        };

        if self._maker_skew_handle_base_seed_phase(&ctx) {
            return;
        }
        if self._maker_skew_handle_shared_gate_phase(&ctx) {
            return;
        }
        let Some(recovery) = self._maker_skew_handle_recovery_phase(&ctx) else {
            return;
        };

        if self._maker_skew_try_arb(
            ctx.budget_usable,
            ctx.y_bid,
            ctx.y_ask,
            ctx.n_bid,
            ctx.n_ask,
            ctx.t_into_s,
            ctx.downside,
            ctx.upside,
            ctx.skew_ratio,
        ) {
            return;
        }

        self._maker_skew_handle_directional_phase(&ctx, &recovery);
    }

    fn _maker_skew_handle_base_seed_phase(&self, ctx: &MakerSkewLoopCtx) -> bool {
        let base_min = env_float(
            "MAKER_SKEW_BASE_MIN_SHARES",
            2.0 * self.cfg.min_shares.max(1.0),
        )
        .max(self.cfg.min_shares.max(1.0));
        let base_needs_seed =
            ctx.q_yes_eff + 1e-9 < base_min || ctx.q_no_eff + 1e-9 < base_min;
        let seed_since_key = "__maker_skew_seed_pending_since";
        let seed_inflight_key = "__maker_skew_seed_inflight_until";
        let seed_wait_s = env_float("MAKER_SKEW_BASE_SEED_MAX_WAIT_SECONDS", 45.0).max(1.0);
        if !base_needs_seed {
            self._runtime_ts_set(seed_since_key, 0.0);
            return false;
        }

        let inflight_until = self._runtime_ts_get(seed_inflight_key);
        if inflight_until > 0.0 && ctx.now < inflight_until {
            self._maker_record_trade_decision(
                ctx.t_into_s,
                0.0,
                0.0,
                ctx.downside,
                ctx.upside,
                ctx.skew_ratio,
                false,
                None,
                "NONE",
                "NONE",
                "MAKER_SKEW_BASE_SEED_INFLIGHT",
            );
            return true;
        }
        self._runtime_ts_set(seed_inflight_key, 0.0);

        if self._runtime_ts_get(seed_since_key) <= 0.0 {
            self._runtime_ts_set(seed_since_key, ctx.now);
        }
        let (seed_side, seed_asset, seed_bid, seed_ask, q_side) = if ctx.q_yes_eff <= ctx.q_no_eff
        {
            (
                "YES",
                ctx.yes_asset.clone(),
                ctx.y_bid,
                ctx.y_ask,
                ctx.q_yes_eff.max(0.0),
            )
        } else {
            (
                "NO",
                ctx.no_asset.clone(),
                ctx.n_bid,
                ctx.n_ask,
                ctx.q_no_eff.max(0.0),
            )
        };
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1) as f64;
        let need = (base_min - q_side).max(min_int);
        let max_affordable = (ctx.budget_usable / seed_ask.max(1e-9)).floor();
        if seed_ask <= 0.0 || max_affordable + 1e-9 < min_int {
            let pending_for = ctx.now - self._runtime_ts_get(seed_since_key);
            if pending_for >= seed_wait_s {
                self.logger.warning(&format!(
                    "[MAKER_SKEW] base seed timeout side={} need={:.2} ask={:.3} budget={:.2} pending_for={:.2}s -> STOP",
                    seed_side, need, seed_ask, ctx.budget_usable, pending_for
                ));
                self._set_exit_reason("MAKER_SKEW_BASE_SEED_TIMEOUT");
                self.cancel_all_orders_exchange("maker_skew base seed timeout");
                self.stop_flag.store(true, Ordering::SeqCst);
            }
            self._maker_record_trade_decision(
                ctx.t_into_s,
                seed_ask,
                need,
                ctx.downside,
                ctx.upside,
                ctx.skew_ratio,
                false,
                None,
                seed_side,
                "NONE",
                "MAKER_SKEW_BASE_SEED_PENDING",
            );
            return true;
        }
        let seed_order_type = self._resolve_order_type(
            &std::env::var("MAKER_SKEW_BASE_SEED_ORDER_TYPE")
                .unwrap_or_else(|_| "GTC".to_string()),
        );
        let size = need.min(max_affordable).max(min_int);
        let submitted = if seed_order_type == "GTC" {
            let seed_key = MakerOrderKey::buy(&seed_asset);
            self._maker_order_upsert_gtc(&seed_key, seed_bid, size, "LIMIT_GTC_POSTONLY")
        } else {
            let seed_slip_ticks = env_int("MAKER_SKEW_BASE_SEED_SLIPPAGE_TICKS", 1).max(0);
            let mut px = seed_ask + seed_slip_ticks as f64 * self.cfg.tick.max(0.0001);
            px = round_up(
                clamp(px, self.cfg.tick.max(0.0001), 0.99),
                self.cfg.tick.max(0.0001),
            );
            self._place_taker_bid_fak(&seed_asset, px, size, Some(&seed_order_type))
        };
        if seed_order_type != "GTC" && (submitted.is_some() || self.cfg.dry_run) {
            let cooldown_s = env_float("MAKER_SKEW_BASE_SEED_INFLIGHT_SECONDS", 6.0).max(1.0);
            self._runtime_ts_set(seed_inflight_key, ctx.now + cooldown_s);
        }
        self._maker_record_trade_decision(
            ctx.t_into_s,
            seed_ask,
            size,
            ctx.downside,
            ctx.upside,
            ctx.skew_ratio,
            false,
            None,
            seed_side,
            &seed_order_type,
            if submitted.is_some() || self.cfg.dry_run {
                "MAKER_SKEW_BASE_SEED"
            } else {
                "MAKER_SKEW_BASE_SEED_FAIL"
            },
        );
        true
    }

    fn _maker_skew_handle_shared_gate_phase(&self, ctx: &MakerSkewLoopCtx) -> bool {
        let (ok, why) = self._accumulate_allowed();
        if !ok {
            let why_key = why.split('(').next().unwrap_or("unknown");
            self._maker_dbg_idle(
                &format!("[MAKER_SKEW] idle: accumulate gate blocked reason={why}"),
                &format!("maker_skew_idle_gate_{why_key}"),
            );
            self._maker_ladder_cancel_all(&format!("accumulate gate: {why}"));
            self._maker_cancel_strategy_orders(None, &format!("accumulate gate: {why}"));
            self._maker_record_trade_decision(
                ctx.t_into_s,
                0.0,
                0.0,
                ctx.downside,
                ctx.upside,
                ctx.skew_ratio,
                false,
                None,
                "NONE",
                "NONE",
                &format!("MAKER_SKEW_GATE_{why}"),
            );
            return true;
        }
        let (invalid, inv_reason) = self._quotes_invalidated();
        if invalid {
            self._maker_dbg_idle(
                &format!("[MAKER_SKEW] idle: quote invalidated reason={inv_reason}"),
                "maker_skew_idle_quote_invalid",
            );
            self._maker_ladder_cancel_all(&format!("quote invalidated: {inv_reason}"));
            self._maker_cancel_strategy_orders(None, &format!("quote invalidated: {inv_reason}"));
            self._maker_record_trade_decision(
                ctx.t_into_s,
                0.0,
                0.0,
                ctx.downside,
                ctx.upside,
                ctx.skew_ratio,
                false,
                None,
                "NONE",
                "NONE",
                "MAKER_SKEW_QUOTE_INVALID",
            );
            return true;
        }
        false
    }

    fn _maker_skew_handle_recovery_phase(
        &self,
        ctx: &MakerSkewLoopCtx,
    ) -> Option<MakerSkewRecoveryState> {
        let (
            recovery_mode,
            recovery_gap,
            recovery_heavy_side,
            recovery_side,
            recovery_asset,
            recovery_unsettled_heavy,
        ) = self._maker_recovery_mode_snapshot();
        let recovery_asset_id = recovery_asset.as_deref().unwrap_or(if recovery_side == "YES" {
            ctx.yes_asset.as_str()
        } else {
            ctx.no_asset.as_str()
        });
        let recovery_active_key = "__maker_recovery_mode_active";
        let recovery_heavy_key = "__maker_recovery_heavy_yes";
        let recovery_log_key = "__maker_recovery_mode_log_until";
        let recovery_log_every = 5.0;
        if recovery_mode {
            self._maker_ladder_cancel_all("imbalance recovery");
            self._maker_cancel_strategy_orders(Some(recovery_asset_id), "imbalance recovery");
            if self._runtime_ts_get(recovery_active_key) <= 0.0 {
                self.logger.info(&format!(
                    "[MAKER_SKEW] recovery mode enter gap={recovery_gap:.2} heavy={recovery_heavy_side} light={recovery_side}"
                ));
                self._runtime_ts_set(recovery_active_key, 1.0);
                self._runtime_ts_set(
                    recovery_heavy_key,
                    if recovery_heavy_side == "YES" { 1.0 } else { 0.0 },
                );
                self._runtime_ts_set(recovery_log_key, ctx.now + recovery_log_every);
            } else if ctx.now >= self._runtime_ts_get(recovery_log_key) {
                self.logger.info(&format!(
                    "[MAKER_SKEW] recovery mode remain gap={recovery_gap:.2} heavy={recovery_heavy_side} light={recovery_side} unsettled_heavy={recovery_unsettled_heavy:.2}"
                ));
                self._runtime_ts_set(recovery_log_key, ctx.now + recovery_log_every);
            }
            let light_unsettled = self._maker_recovery_unsettled_buy_risk(recovery_asset_id);
            if light_unsettled > 1e-6 {
                let recovery_key = MakerOrderKey::buy(recovery_asset_id);
                if let Some(reason) = self._maker_recovery_light_refresh_reason(recovery_asset_id) {
                    let _ = self._maker_order_request_cancel(&recovery_key, &reason);
                    self._maker_record_trade_decision(
                        ctx.t_into_s,
                        0.0,
                        0.0,
                        ctx.downside,
                        ctx.upside,
                        ctx.skew_ratio,
                        false,
                        None,
                        "NONE",
                        "NONE",
                        &format!(
                            "MAKER_SKEW_RECOVERY_REFRESH(gap={recovery_gap:.2},light_risk={light_unsettled:.2})"
                        ),
                    );
                    return None;
                }
                if !self._maker_recovery_light_requote_ready(recovery_asset_id) {
                    self._maker_cancel_strategy_orders(
                        Some(recovery_asset_id),
                        "imbalance recovery unsettled",
                    );
                    self._maker_record_trade_decision(
                        ctx.t_into_s,
                        0.0,
                        0.0,
                        ctx.downside,
                        ctx.upside,
                        ctx.skew_ratio,
                        false,
                        None,
                        "NONE",
                        "NONE",
                        &format!(
                            "MAKER_SKEW_RECOVERY_WAIT_UNSETTLED(gap={recovery_gap:.2},light_risk={light_unsettled:.2})"
                        ),
                    );
                    return None;
                }
            }
        } else if self._runtime_ts_get(recovery_active_key) > 0.0 {
            self._maker_ladder_cancel_all("recovery settled");
            self._maker_cancel_strategy_orders(None, "recovery settled");
            self.logger.info(&format!(
                "[MAKER_SKEW] recovery mode exit gap={recovery_gap:.2} threshold={:.2}",
                self._pair_arb_imbalance_release_shares()
            ));
            self._runtime_ts_set(recovery_active_key, 0.0);
            self._runtime_ts_set(recovery_heavy_key, 0.0);
            self._runtime_ts_set(recovery_log_key, 0.0);
        }
        Some(MakerSkewRecoveryState {
            mode: recovery_mode,
            side: recovery_side,
        })
    }

    fn _maker_skew_handle_directional_phase(
        &self,
        ctx: &MakerSkewLoopCtx,
        recovery: &MakerSkewRecoveryState,
    ) {
        let yes = ctx.yes_asset.as_str();
        let no = ctx.no_asset.as_str();
        let target_ratio = env_float("MAKER_SKEW_TARGET_RATIO", 1.5).max(1.0);
        let ratio_max = env_float("MAKER_SKEW_MAX_RATIO", 3.3).max(target_ratio);
        let max_loss = env_float("MAKER_SKEW_MAX_WORST_CASE_LOSS_USDC", 350.0).max(1.0);
        let default_underdog = if ctx.y_bid <= ctx.n_bid { "YES" } else { "NO" };
        let underdog = self._maker_stretch_bias_side(default_underdog);
        let hedge = if underdog == "YES" { "NO" } else { "YES" };
        let min_base = self.cfg.min_shares.max(1.0);

        let min_shares_held = ctx.q_yes_eff.min(ctx.q_no_eff).max(0.0);
        let cpp = if min_shares_held > 1e-9 {
            ctx.total_cost / min_shares_held
        } else {
            f64::INFINITY
        };
        let cpp_soft = env_float("MAKER_SKEW_CPP_SOFT_CAP", 1.33).max(1.0);
        let cpp_hard = env_float("MAKER_SKEW_CPP_HARD_CAP", 1.98).max(cpp_soft);

        if let Ok(mut st) = self.maker_skew_state.lock() {
            st.cpp = cpp;
        }

        if cpp.is_finite() && cpp >= cpp_hard {
            self._maker_ladder_cancel_all("cpp_hard_cap");
            self._maker_cancel_strategy_orders(None, "MAKER_SKEW cpp hard cap");
            self._maker_record_trade_decision(
                ctx.t_into_s,
                0.0,
                0.0,
                ctx.downside,
                ctx.upside,
                ctx.skew_ratio,
                false,
                None,
                "NONE",
                "NONE",
                &format!("MAKER_SKEW_CPP_HARD_CAP(cpp={cpp:.3})"),
            );
            return;
        }

        let mut side = if recovery.mode {
            recovery.side.clone()
        } else {
            underdog.to_string()
        };
        let mut role = if recovery.mode {
            "imbalance_rebalance".to_string()
        } else {
            "underdog".to_string()
        };
        let side_yes = ctx.q_yes_eff.max(0.0);
        let side_no = ctx.q_no_eff.max(0.0);
        if recovery.mode {
            side = recovery.side.clone();
            role = "imbalance_rebalance".to_string();
        } else if side_yes < min_base || side_no < min_base {
            side = if side_yes <= side_no {
                "YES".to_string()
            } else {
                "NO".to_string()
            };
            role = "base".to_string();
        } else if ctx.downside < -max_loss {
            side = if side_yes <= side_no {
                "YES".to_string()
            } else {
                "NO".to_string()
            };
            role = "hedge".to_string();
        } else if cpp.is_finite() && cpp >= cpp_soft {
            side = if side_yes <= side_no {
                "YES".to_string()
            } else {
                "NO".to_string()
            };
            role = "cpp_hedge".to_string();
            self.logger.info(&format!(
                "[MAKER_SKEW] CPP soft cap hit cpp={cpp:.3} >= {cpp_soft:.3} -> hedge smaller side={side}"
            ));
        } else if ctx.skew_ratio > ratio_max {
            side = hedge.to_string();
            role = "hedge".to_string();
        } else if ctx.skew_ratio < target_ratio {
            side = underdog.to_string();
            role = "underdog".to_string();
        }

        let (asset_id, bid, ask) = if side == "YES" {
            (yes.to_string(), ctx.y_bid, ctx.y_ask)
        } else {
            (no.to_string(), ctx.n_bid, ctx.n_ask)
        };
        let mut clip = self._maker_pick_clip_size_for_price(ask.min(bid), ctx.peak_window);
        let max_shares_budget = (ctx.budget_usable / ask.max(1e-9)).floor();
        clip = clip.min(max_shares_budget).max(0.0);
        if clip < self.cfg.min_shares.max(1.0) {
            self._maker_dbg_idle(
                &format!(
                    "[MAKER_SKEW] idle: clip_below_min side={side} role={role} clip={clip:.2} min={:.2} bid={bid:.3} ask={ask:.3} budget={:.2}",
                    self.cfg.min_shares.max(1.0),
                    ctx.budget_usable
                ),
                "maker_skew_idle_clip_below_min",
            );
            return;
        }
        let ladder_enabled = env_bool("MAKER_LADDER_ENABLED", true);
        let order_type = if role == "hedge" && ctx.downside < -max_loss {
            self._resolve_order_type(
                &std::env::var("MAKER_EXPOSURE_UNWIND_ORDER_TYPE")
                    .unwrap_or_else(|_| self.hedge_taker_order_type.clone()),
            )
        } else if role == "cpp_hedge" {
            self._resolve_order_type(
                &std::env::var("MAKER_SKEW_CPP_HEDGE_ORDER_TYPE")
                    .unwrap_or_else(|_| "GTC".to_string()),
            )
        } else {
            "GTC".to_string()
        };

        if !recovery.mode && order_type == "GTC" {
            if let Some((
                current_gap,
                projected_gap,
                q_yes_actual,
                q_no_actual,
                unsettled_yes,
                unsettled_no,
            )) = self._maker_projected_gap_after_buy(&asset_id, clip)
            {
                let enter = self._pair_arb_imbalance_enter_shares();
                if projected_gap > enter + 1e-6 && projected_gap > current_gap + 1e-6 {
                    self.logger.info(&format!(
                        "[MAKER_SKEW] normal BUY suppressed side={side} role={role} projected_gap={projected_gap:.2} > enter={enter:.2} current_gap={current_gap:.2} clip={clip:.2} qYES={q_yes_actual:.2} qNO={q_no_actual:.2} unsettled_yes={unsettled_yes:.2} unsettled_no={unsettled_no:.2}"
                    ));
                    let keep_asset_id = if asset_id == yes { no } else { yes };
                    self._maker_cancel_strategy_orders(
                        Some(keep_asset_id),
                        "maker_skew projected gap suppress",
                    );
                    self._maker_record_trade_decision(
                        ctx.t_into_s,
                        bid,
                        clip,
                        ctx.downside,
                        ctx.upside,
                        ctx.skew_ratio,
                        false,
                        None,
                        &side,
                        &order_type,
                        &format!(
                            "MAKER_SKEW_PROJECTED_GAP_SUPPRESS(gap={projected_gap:.2},enter={enter:.2},role={role})"
                        ),
                    );
                    return;
                }
            }
        }

        if (role == "hedge" && ctx.downside < -max_loss)
            || (role == "cpp_hedge" && order_type != "GTC")
        {
            let slip_ticks = if role == "cpp_hedge" {
                env_int("MAKER_SKEW_CPP_HEDGE_SLIPPAGE_TICKS", 1).max(0)
            } else {
                self.hedge_slippage_ticks as i64
            };
            let mut px = ask + slip_ticks as f64 * self.cfg.tick.max(0.0001);
            px = round_up(
                clamp(px, self.cfg.tick.max(0.0001), 0.99),
                self.cfg.tick.max(0.0001),
            );
            let _ = self._place_taker_bid_fak(&asset_id, px, clip, Some(&order_type));
            self._maker_ladder_cancel_all(&format!("{role} taker"));
        } else if ladder_enabled {
            let levels = if role == "hedge" {
                env_int("MAKER_HEDGE_LADDER_LEVELS", 2)
            } else {
                env_int("MAKER_UNDERDOG_LADDER_LEVELS", 4)
            }
            .max(1);
            let step_ticks = env_int("MAKER_LADDER_TICKS_STEP", 1).max(1);
            self._maker_ladder_cancel_except_role_asset(&role, &asset_id);
            self._maker_ladder_sync_role(&role, &asset_id, bid, clip, levels, step_ticks);
            self._maker_order_cancel_all_except_asset(Some(&asset_id), "maker_skew ladder mode");
        } else {
            let _ = self._maybe_replace(&asset_id, bid, clip, None);
        }

        self._maker_record_trade_decision(
            ctx.t_into_s,
            bid,
            clip,
            ctx.downside,
            ctx.upside,
            ctx.skew_ratio,
            false,
            None,
            &side,
            &order_type,
            &format!("MAKER_SKEW_{role}"),
        );
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

    fn _maker_quote_only_allowed(&self, yes: &str, no: &str) -> (bool, String) {
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

        let tick = self.cfg.tick.max(0.0001);
        let spr_y_ticks = (ya - yb) / tick;
        let spr_n_ticks = (na - nb) / tick;
        if spr_y_ticks > self.max_spread_ticks as f64 || spr_n_ticks > self.max_spread_ticks as f64
        {
            return (
                false,
                format!("spread_too_wide(y={spr_y_ticks:.1} n={spr_n_ticks:.1})"),
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
        let qty_before = guard.q_yes + guard.q_no;
        if asset_id == yes_asset {
            guard.q_yes = (guard.q_yes + qty).max(0.0);
            guard.c_yes = (guard.c_yes + price * qty).max(0.0);
        } else if self.no_asset.as_deref() == Some(asset_id) {
            guard.q_no = (guard.q_no + qty).max(0.0);
            guard.c_no = (guard.c_no + price * qty).max(0.0);
        } else {
            return false;
        }
        let qty_after = guard.q_yes + guard.q_no;
        let opened_position = side_u == "BUY" && qty_before <= 1e-12 && qty_after > 1e-12;
        let closed_position = qty_after <= 1e-12;
        let mark_first_entry_fill = side_u == "BUY" && qty_after > qty_before + 1e-12;
        let _ = save_state(&self.state_file, &mut guard);
        drop(guard);

        // Clear seed in-flight cooldown on any fill — allows immediate re-seeding
        // of the other side instead of waiting for the hardcoded timeout.
        self._runtime_ts_set("__maker_skew_seed_inflight_until", 0.0);

        let mut opened_reason: Option<String> = None;
        if opened_position {
            let reason = self
                ._take_pending_entry_reason()
                .unwrap_or_else(|| self._default_entry_reason());
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = Some(reason.clone());
            }
            opened_reason = Some(reason);
        } else if closed_position {
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = None;
            }
        }

        if mark_first_entry_fill {
            let fill_ts = crate::db::now_iso_jakarta();
            if let Ok(mut first) = self.first_entry_fill_iso.lock() {
                if first.is_none() {
                    *first = Some(fill_ts);
                }
            }
            if let Ok(mut first_reason) = self.first_entry_reason.lock() {
                if first_reason.is_none() {
                    let reason = opened_reason.unwrap_or_else(|| {
                        self._take_pending_entry_reason()
                            .unwrap_or_else(|| self._default_entry_reason())
                    });
                    *first_reason = Some(reason);
                }
            }
        }
        true
    }

    fn _apply_fill_locked_nodedupe(
        &self,
        guard: &mut BotState,
        asset_id: &str,
        price: f64,
        filled: f64,
        side: &str,
    ) -> Option<ApplyFillMutationMeta> {
        let side_u = side.trim().to_ascii_uppercase();
        if !matches!(side_u.as_str(), "BUY" | "SELL") {
            return None;
        }
        if filled <= 0.0 || price <= 0.0 {
            return None;
        }

        let yes_asset = self.yes_asset.as_deref().unwrap_or_default();
        let sign = if side_u == "BUY" { 1.0 } else { -1.0 };
        let qty = sign * filled;
        let qty_before = guard.q_yes + guard.q_no;
        if asset_id == yes_asset {
            guard.q_yes = (guard.q_yes + qty).max(0.0);
            guard.c_yes = (guard.c_yes + price * qty).max(0.0);
        } else if self.no_asset.as_deref() == Some(asset_id) {
            guard.q_no = (guard.q_no + qty).max(0.0);
            guard.c_no = (guard.c_no + price * qty).max(0.0);
        } else {
            return None;
        }
        let qty_after = guard.q_yes + guard.q_no;
        Some(ApplyFillMutationMeta {
            opened_position: side_u == "BUY" && qty_before <= 1e-12 && qty_after > 1e-12,
            closed_position: qty_after <= 1e-12,
            mark_first_entry_fill: side_u == "BUY" && qty_after > qty_before + 1e-12,
        })
    }

    fn _apply_fill_finalize(&self, meta: ApplyFillMutationMeta) {
        let ApplyFillMutationMeta {
            opened_position,
            closed_position,
            mark_first_entry_fill,
        } = meta;

        // Clear seed in-flight cooldown on any fill; allows immediate re-seeding
        // of the other side instead of waiting for the hardcoded timeout.
        self._runtime_ts_set("__maker_skew_seed_inflight_until", 0.0);

        let mut opened_reason: Option<String> = None;
        if opened_position {
            let reason = self
                ._take_pending_entry_reason()
                .unwrap_or_else(|| self._default_entry_reason());
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = Some(reason.clone());
            }
            opened_reason = Some(reason);
        } else if closed_position {
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = None;
            }
        }

        if mark_first_entry_fill {
            let fill_ts = crate::db::now_iso_jakarta();
            if let Ok(mut first) = self.first_entry_fill_iso.lock() {
                if first.is_none() {
                    *first = Some(fill_ts);
                }
            }
            if let Ok(mut first_reason) = self.first_entry_reason.lock() {
                if first_reason.is_none() {
                    let reason = opened_reason.unwrap_or_else(|| {
                        self._take_pending_entry_reason()
                            .unwrap_or_else(|| self._default_entry_reason())
                    });
                    *first_reason = Some(reason);
                }
            }
        }
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

    pub fn _forget_taker_order(&self, order_id: &str) {
        if order_id.trim().is_empty() {
            return;
        }
        if let Ok(mut m) = self.taker_orders.lock() {
            m.remove(order_id);
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
                self._sniper_record_order_fill(order_id, price, inc);
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

        let mut hedge_progress: Option<(String, String, f64, f64, f64)> = None;
        let mut remove_oid = false;
        if let Ok(mut m) = self.taker_orders.lock() {
            if let Some(rec) = m.get_mut(order_id) {
                rec.applied = rec.applied.max(matched_total);
                rec.ts = now_ts_f64();
                let remaining = (rec.size - rec.applied).max(0.0);
                if self._sniper_is_hedge_order(order_id) {
                    hedge_progress = Some((
                        rec.asset_id.clone(),
                        rec.side.clone(),
                        rec.applied.max(0.0),
                        remaining,
                        rec.size.max(0.0),
                    ));
                }
                if done_hint || (rec.size > 0.0 && rec.applied >= rec.size - 1e-9) {
                    remove_oid = true;
                }
            }
            if remove_oid {
                m.remove(order_id);
            }
        }
        if let Some((asset, side, filled, remaining, total)) = hedge_progress {
            self._sniper_log_hedge_order_progress(
                order_id,
                &asset,
                &side,
                filled,
                remaining,
                total,
                "ORDER_EVT",
                &status,
            );
            if remove_oid || remaining <= 1e-9 {
                self._sniper_clear_hedge_order(order_id);
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
            // Top-level BUY is only trusted when the taker order maps to our local context.
            if !token_id.trim().is_empty()
                && side_top == "BUY"
                && (taker_rec.is_some() || taker_ctx.is_some())
            {
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
                // Strict ownership gate: only our wallet's maker legs can confirm entry.
                if wallet.trim().is_empty() || mo_addr.is_empty() || mo_addr != wallet {
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
                self._sniper_record_order_fill(&taker_oid, price, size);
                self._log_execution_latency_on_fill(&taker_oid, now_ts_f64());
                let mut hedge_progress: Option<(String, String, f64, f64, f64)> = None;
                let mut remove_oid = false;
                if let Ok(mut m) = self.taker_orders.lock() {
                    if let Some(r) = m.get_mut(&taker_oid) {
                        r.applied += size.max(0.0);
                        r.ts = now_ts_f64();
                        let remaining = (r.size - r.applied).max(0.0);
                        if self._sniper_is_hedge_order(&taker_oid) {
                            hedge_progress = Some((
                                r.asset_id.clone(),
                                r.side.clone(),
                                r.applied.max(0.0),
                                remaining,
                                r.size.max(0.0),
                            ));
                        }
                        if r.size > 0.0 && r.applied >= r.size - 1e-9 {
                            remove_oid = true;
                        }
                    }
                    if remove_oid {
                        m.remove(&taker_oid);
                    }
                }
                if let Some((h_asset, h_side, h_filled, h_remaining, h_total)) = hedge_progress {
                    self._sniper_log_hedge_order_progress(
                        &taker_oid,
                        &h_asset,
                        &h_side,
                        h_filled,
                        h_remaining,
                        h_total,
                        "TRADE_EVT",
                        &status,
                    );
                    if remove_oid || h_remaining <= 1e-9 {
                        self._sniper_clear_hedge_order(&taker_oid);
                    }
                }
            }
            return;
        }

        // CASE B: Maker trade event. Apply only if maker leg matches our wallet.
        let wallet = self.wallet_address.to_ascii_lowercase();
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
            if let Some(mo) = maker_leg {
                if let Some(candidate) = self._maker_trade_exec_candidate(msg, &mo) {
                    let maker_oid = candidate.order_id.clone();
                    let trade_id = candidate.trade_id.clone().unwrap_or_default();
                    let tx_hash = candidate.tx_hash.clone().unwrap_or_default();
                    let taker_oid = candidate.taker_order_id.clone().unwrap_or_default();
                    let match_time = candidate.match_time.clone().unwrap_or_default();
                    let qty = candidate.qty;
                    let px = candidate.price;
                    match self._maker_commit_exec_fill(candidate) {
                        MakerExecApplyResult::Applied { canonical_id } => {
                            let alias_kind = Self::_maker_exec_alias_kind(&canonical_id);
                            self.logger.info(&format!(
                                "[FILL][MAKER_APPLY] oid={}.. canonical={} alias_kind={} qty={qty:.6} px={px:.4}",
                                maker_oid.chars().take(10).collect::<String>(),
                                canonical_id,
                                alias_kind
                            ));
                            self._sniper_record_order_fill(&maker_oid, px, qty);
                            self._log_execution_latency_on_fill(&maker_oid, now_ts_f64());
                        }
                        MakerExecApplyResult::Duplicate { canonical_id } => {
                            let alias_kind = Self::_maker_exec_alias_kind(&canonical_id);
                            self.logger.info(&format!(
                                "[FILL][MAKER_DEDUPE] drop oid={}.. canonical={} alias_kind={} qty={qty:.6} px={px:.4} trade_id={} tx={} taker_oid={} match_time={}",
                                maker_oid.chars().take(10).collect::<String>(),
                                canonical_id,
                                alias_kind,
                                trade_id,
                                tx_hash,
                                taker_oid,
                                match_time
                            ));
                        }
                        MakerExecApplyResult::Conflict {
                            canonical_id,
                            reason,
                        } => {
                            let alias_kind = Self::_maker_exec_alias_kind(&canonical_id);
                            self.logger.warning(&format!(
                                "[FILL][MAKER_CONFLICT] oid={}.. canonical={} alias_kind={} reason={} qty={qty:.6} px={px:.4} trade_id={} tx={} taker_oid={} match_time={}",
                                maker_oid.chars().take(10).collect::<String>(),
                                canonical_id,
                                alias_kind,
                                reason,
                                trade_id,
                                tx_hash,
                                taker_oid,
                                match_time
                            ));
                        }
                        MakerExecApplyResult::DroppedWeakId { reason } => {
                            self.logger.warning(&format!(
                                "[FILL][MAKER_DROP_WEAK] oid={}.. reason={} qty={qty:.6} px={px:.4} trade_id={} tx={} taker_oid={} match_time={}",
                                maker_oid.chars().take(10).collect::<String>(),
                                reason,
                                trade_id,
                                tx_hash,
                                taker_oid,
                                match_time
                            ));
                        }
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

        self._maker_order_on_user_event(msg);

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
                    self._maker_order_on_cancel_ack_by_order_id(order_id);
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
        if self._maker_single_inflight_enabled() && !self.cfg.dry_run {
            self._maker_order_reconcile_asset(asset_id, intended_price);
            if !env_bool("RECONCILE_EXCHANGE_ORDERS", true) {
                return;
            }
        }
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
        let origin = if post_only.unwrap_or(false) {
            "LIMIT_GTC_POSTONLY"
        } else {
            "LIMIT_GTC"
        };
        self._place_limit_bid_gtc_with_origin(asset_id, price, size, post_only, origin)
    }

    fn _place_limit_bid_gtc_with_origin(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        post_only: Option<bool>,
        origin: &str,
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
                "origin": origin,
            }),
        );
        Some(oid)
    }

    fn _place_limit_bid_gtc_exact_with_origin(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        post_only: Option<bool>,
        origin: &str,
    ) -> Option<String> {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = clamp(price, tick, 0.99);
        px = round_down(px, tick);
        px = clamp(px, tick, 0.99);
        let size = round_down(size.max(0.0), 0.01);
        if size < 0.01 {
            return None;
        }
        if self.cfg.dry_run {
            let oid = format!("DRY_LIMIT_GTC_EXACT_{}", (now_ts_f64() * 1000.0) as i64);
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    asset_id.to_string(),
                    OpenOrderState {
                        order_id: Some(oid.clone()),
                        price: Some(px),
                        size: Some(size),
                        ts: Some(now_ts_f64()),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
            self.logger.info(&format!(
                "[DRY] limit bid GTC exact asset={} px={px:.3} size={size:.2} post_only={post_only:?}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return Some(oid);
        }

        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": px,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, "GTC", post_only)?;
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.insert(
                asset_id.to_string(),
                OpenOrderState {
                    order_id: Some(oid.clone()),
                    price: Some(px),
                    size: Some(size),
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
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": origin,
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

    pub fn _place_taker_bid_fak_exact(
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
        let size = round_down(size.max(0.0), 0.01);
        if size < 0.01 {
            return None;
        }
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER HEDGE BUY EXACT asset={} price={px:.2} size={size:.2} type={ot}",
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
                "origin": format!("TAKER_{}_BUY_EXACT", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent BUY asset={} px={px:.4} sz={size:.2} oid={oid}",
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

    pub fn _place_taker_ask_fak_exact(
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
        let size = q_down(size.max(0.0), 4);
        if size < 0.0001 {
            return None;
        }
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER SELL EXACT asset={} price={px:.4} size={size:.4} type={ot}",
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
                    "[TAKER {ot}] rejected SELL exact asset={} px={px:.4} sz={size:.4} (no oid)",
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
                "origin": format!("TAKER_{}_SELL_EXACT", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent SELL asset={} px={px:.4} sz={size:.4} oid={oid}",
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

    pub fn _wait_for_pair_order_fills(
        &self,
        y_oid: Option<&str>,
        n_oid: Option<&str>,
        y0: f64,
        n0: f64,
        target_size: i64,
        timeout_s: f64,
    ) -> (f64, f64) {
        let deadline = now_ts_f64() + timeout_s.max(0.01);
        while now_ts_f64() < deadline && !self.stop_flag.load(Ordering::SeqCst) {
            let fy = y_oid
                .map(|oid| (self._maker_exec_applied_qty(oid) - y0).max(0.0))
                .unwrap_or(0.0);
            let fn_ = n_oid
                .map(|oid| (self._maker_exec_applied_qty(oid) - n0).max(0.0))
                .unwrap_or(0.0);
            if fy >= target_size as f64 && fn_ >= target_size as f64 {
                return (fy, fn_);
            }
            let rem = (deadline - now_ts_f64()).max(0.0);
            if rem <= 0.0 {
                break;
            }
            thread::sleep(Duration::from_secs_f64(rem.min(0.05)));
        }
        let fy = y_oid
            .map(|oid| (self._maker_exec_applied_qty(oid) - y0).max(0.0))
            .unwrap_or(0.0);
        let fn_ = n_oid
            .map(|oid| (self._maker_exec_applied_qty(oid) - n0).max(0.0))
            .unwrap_or(0.0);
        (fy, fn_)
    }

    pub fn _handle_exposure_mismatch(&self, filled_yes: f64, filled_no: f64) {
        let fill_delta = filled_yes - filled_no;
        if fill_delta.abs() < 1e-9 {
            return;
        }

        let _ = self._reconcile_state_from_positions("exposure_mismatch");
        let (qy, qn) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no))
            .unwrap_or((0.0, 0.0));
        let state_delta = qy - qn;
        if state_delta.abs() < 1e-9 {
            self.logger.info(&format!(
                "Exposure mismatch cleared after reconcile. filled_yes={filled_yes:.2} filled_no={filled_no:.2}"
            ));
            return;
        }

        let mut delta = fill_delta;
        if fill_delta.signum() != state_delta.signum()
            || (fill_delta - state_delta).abs() >= self.cfg.min_shares
        {
            self.logger.warning(&format!(
                "Exposure mismatch tiebreak: fill_delta={fill_delta:.2} state_delta={state_delta:.2}; using state delta."
            ));
            delta = state_delta;
        }

        if delta.abs() < self.cfg.min_shares {
            if (fill_delta - delta).abs() >= self.cfg.min_shares {
                self.logger.info(&format!(
                    "Exposure mismatch reduced below min_shares after reconcile. fill_delta={fill_delta:.2} state_delta={delta:.2} -> skip stop"
                ));
                return;
            }
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
        if self._maker_single_inflight_enabled() {
            let key = MakerOrderKey::buy(asset_id);
            return self
                ._maker_order_upsert_gtc(&key, price, size, "MAKER_POSTONLY_GTC")
                .is_some();
        }
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
        if self.exec_mode == "MAKER_SKEW_ARB" {
            if let Ok(ms) = self.maker_skew_state.lock() {
                line.push_str(&format!(
                    " | skew downside={:+.3} upside={:+.3} ratio={:.3} cpp={:.3} t_into={:.1}s",
                    ms.downside,
                    ms.upside,
                    ms.skew_ratio,
                    ms.cpp,
                    (now_ts_f64() - self.start_ts as f64).max(0.0)
                ));
                if env_bool("MAKER_STRETCH_BIAS_ENABLED", false) {
                    let rsi_txt = ms
                        .stretch_rsi
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "NA".to_string());
                    let diff_txt = ms
                        .stretch_diff_vs_start
                        .map(|v| format!("{v:+.3}"))
                        .unwrap_or_else(|| "NA".to_string());
                    let default_side = if ms.stretch_default_side.trim().is_empty() {
                        "NA"
                    } else {
                        ms.stretch_default_side.as_str()
                    };
                    let biased_side = if ms.stretch_biased_side.trim().is_empty() {
                        "NA"
                    } else {
                        ms.stretch_biased_side.as_str()
                    };
                    let reason = if ms.stretch_bias_reason.trim().is_empty() {
                        "NA"
                    } else {
                        ms.stretch_bias_reason.as_str()
                    };
                    line.push_str(&format!(
                        " | stretch rsi={} diff={} default={} biased={} reason={}",
                        rsi_txt, diff_txt, default_side, biased_side, reason
                    ));
                }
            }
        } else if self.exec_mode == "TAKER_PAIR" || env_bool("DEBUG_MODE", false) {
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
            self._runtime_ts_set("__max_loss_breach_since", 0.0);
            return false;
        }
        let max_loss = env_float("MAX_LOSS_USD_PER_MARKET", 1.0).max(0.0);
        let s = self.state.lock().map(|v| v.clone()).unwrap_or_default();
        let lp = locked_profit(&s);
        if lp > -max_loss {
            self._runtime_ts_set("__max_loss_breach_since", 0.0);
            return false;
        }

        let now = now_ts_f64();
        let grace_s = env_float("MAX_LOSS_GRACE_SECONDS", 0.0).max(0.0);
        let confirm_s = env_float("MAX_LOSS_CONFIRM_SECONDS", 0.0).max(0.0);
        let mut breach_since = self._runtime_ts_get("__max_loss_breach_since");
        if breach_since <= 0.0 {
            breach_since = now;
            self._runtime_ts_set("__max_loss_breach_since", breach_since);
        }
        let breached_for = now - breach_since;
        if breached_for + 1e-12 < (grace_s + confirm_s) {
            // Do not short-circuit the normal exposure handler during grace/confirm.
            return false;
        }

        self.logger.warning(&format!(
            "max-loss active lp={lp:.4} delta={delta:.4} unhedged_age={unhedged_age:.2}s breached_for={breached_for:.2}s limit={max_loss:.2}"
        ));

        if delta.abs() >= self.cfg.min_shares {
            if self.exec_mode == "TAKER_PAIR" {
                self._emergency_taker_hedge_step(delta, "max_loss");
            } else {
                self._maker_exposure_step(delta, unhedged_age);
            }
        }

        let s2 = self.state.lock().map(|v| v.clone()).unwrap_or_default();
        let lp2 = locked_profit(&s2);
        if lp2 <= -max_loss {
            if let Some(info) = self._flatten_now_best(s2.q_yes - s2.q_no) {
                self._force_flatten_and_stop(s2.q_yes - s2.q_no, &info);
            } else {
                self.logger
                    .warning("max-loss fallback stop (no flatten quote)");
                self._set_exit_reason("MAX_LOSS");
                self.cancel_all_orders_exchange("max-loss");
                self.stop_flag.store(true, Ordering::SeqCst);
            }
            return true;
        }
        false
    }

    pub fn _force_flatten_and_stop(&self, delta: f64, info: &Value) {
        self.logger.warning(&format!(
            "force flatten + stop delta={delta:.4} info={}",
            info
        ));
        self.cancel_all_open_orders_local("force flatten");
        self._maker_ladder_cancel_all("force flatten");
        if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
            self._cancel_exchange_orders_for_assets(
                &[y.clone(), n.clone()],
                "force flatten pre-action",
            );
        }
        let min_need = self.cfg.min_shares.max(1.0);
        let max_passes = env_int("FORCE_FLATTEN_MAX_PASSES", 5).clamp(1, 20) as usize;
        let wait_ms = env_int("FORCE_FLATTEN_WAIT_MS", 350).clamp(50, 5_000) as u64;
        let slip_step = env_int("FORCE_FLATTEN_SLIPPAGE_STEP_TICKS", 1).clamp(0, 50) as f64;
        let tick = self.cfg.tick.max(0.0001);
        let fallback_action = info
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("BUY_MISSING")
            .to_ascii_uppercase();

        for pass in 0..max_passes {
            let _ = self._reconcile_state_from_positions("force_flatten_loop");
            let (qy, qn) = self
                .state
                .lock()
                .map(|s| (s.q_yes, s.q_no))
                .unwrap_or((0.0, 0.0));
            let d = qy - qn;
            if d.abs() < min_need {
                break;
            }
            let flat_info = self._flatten_now_best(d).unwrap_or_else(|| info.clone());
            let action = flat_info
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or(&fallback_action)
                .to_ascii_uppercase();
            let need = flat_info
                .get("need")
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| d.abs())
                .max(0.0);
            if need < min_need {
                break;
            }

            if action == "SELL_HEAVY" {
                let heavy_asset = flat_info
                    .get("heavy_asset")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let heavy_bid = flat_info
                    .get("heavy_bid")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                if !heavy_asset.trim().is_empty() && heavy_bid > 0.0 {
                    let mut px = heavy_bid - (pass as f64 * slip_step * tick);
                    px = round_down(clamp(px, tick, 0.99), tick);
                    let order_type = std::env::var("MAKER_EXPOSURE_UNWIND_ORDER_TYPE")
                        .unwrap_or_else(|_| self.hedge_taker_order_type.clone());
                    let _ = self._place_taker_ask_fak(&heavy_asset, px, need, Some(&order_type));
                }
            } else {
                let missing_asset = flat_info
                    .get("missing_asset")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let miss_ask = flat_info
                    .get("missing_ask")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                if !missing_asset.trim().is_empty() && miss_ask > 0.0 {
                    let mut px = miss_ask
                        + (self.hedge_slippage_ticks as f64 + pass as f64 * slip_step) * tick;
                    px = round_up(clamp(px, tick, 0.99), tick);
                    let _ = self._place_taker_bid_fak(
                        &missing_asset,
                        px,
                        need,
                        Some(&self.hedge_taker_order_type),
                    );
                }
            }
            thread::sleep(Duration::from_millis(wait_ms));
        }
        let _ = self._reconcile_state_from_positions("force_flatten_final");
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Ok(mut r) = self.exit_reason.lock() {
            *r = "FORCED_FLATTEN".to_string();
        }
        self.cancel_all_orders_exchange("force flatten complete");
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
        let mut cap = self._hedge_price_cap();
        let base_cap = cap;
        let t_left = (self.expiry_ts as f64 - now_ts_f64()).max(0.0);
        let force_seconds = env_float("PAIR_BASE_NEAR_EXPIRY_FORCE_TAKER_SECONDS", 0.0).max(0.0);
        let override_max_price = env_float("PAIR_BASE_NEAR_EXPIRY_TAKER_MAX_PRICE", 0.0)
            .clamp(0.0, 0.99);
        if pair_base_near_expiry_taker_override_active(
            reason,
            t_left,
            force_seconds,
            override_max_price,
        ) {
            cap = pair_base_effective_taker_cap(cap, override_max_price);
            if cap > base_cap + 1e-9 {
                self.logger.info(&format!(
                    "[PAIR_BASE] near-expiry taker cap override base_cap={base_cap:.2} override_max={override_max_price:.2} effective_cap={cap:.2} t_left={t_left:.1}s ({reason})"
                ));
            }
        }
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

    fn _pair_base_exact_taker_hedge_step(&self, delta: f64, reason: &str) {
        if delta.abs() < 0.01 {
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
        let (q_yes, q_no) = self._maker_actual_inventory();
        let heavy_asset = if delta > 0.0 {
            self.yes_asset.clone()
        } else {
            self.no_asset.clone()
        };
        let Some(heavy_asset) = heavy_asset else {
            return;
        };
        if self.taker_strict_inflight && self._has_pending_taker_order("SELL", Some(&heavy_asset))
        {
            return;
        }
        let Some((bid, _ask)) = self._best_bid_ask(&heavy_asset) else {
            self.logger.info(&format!(
                "Emergency hedge exact SELL: missing best_bid_ask for {} ({reason})",
                heavy_asset
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
        if bid <= 0.0 {
            self.logger.info(&format!(
                "Emergency hedge exact SELL: missing bid for {} ({reason})",
                heavy_asset
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
        let tick = self.cfg.tick.max(0.0001);
        let size = q_down(delta.abs().min(if delta > 0.0 { q_yes } else { q_no }).max(0.0), 4);
        if size < 0.0001 {
            return;
        }
        let mut px = bid - self.hedge_slippage_ticks as f64 * tick;
        px = round_down(px, tick);
        px = clamp(px, tick, 0.99);
        self.logger.info(&format!(
            "EMERGENCY HEDGE EXACT SELL ({reason}) delta={delta:.4} need={size:.4} sell={} bid={bid:.2} px={px:.2} type={}",
            heavy_asset
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
        let _oid = self._place_taker_ask_fak_exact(
            &heavy_asset,
            px,
            size,
            Some(&self.hedge_taker_order_type),
        );
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
                good.iter()
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(s, _)| s.to_string())
            } else if require_min && price_min > 0.0 {
                return false;
            } else {
                opts.iter()
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
        let endgame_filter_decision =
            self._sniper_filters_eval_entry(&side, "SNIPER_ENDGAME", seconds_left);
        if let Some(decision) = &endgame_filter_decision {
            if !decision.allowed {
                return false;
            }
        }
        let decided_at_ms = (now_ts * 1000.0) as i64;
        let mut pending_breakout_anchor = self._sniper_build_breakout_entry_anchor(
            &side,
            endgame_filter_decision.as_ref(),
            decided_at_ms,
            None,
        );
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
        if let Some(oid) = oid.clone() {
            let (y_bid, y_ask, n_bid, n_ask) = self._sniper_best_snapshot();
            let (bid, ask) = if side == "YES" {
                (y_bid, y_ask)
            } else {
                (n_bid, n_ask)
            };
            self._sniper_trade_decision_record_submit(
                &oid,
                &side,
                seconds_left,
                &asset_id,
                bid,
                ask,
                px,
                size_target as f64,
                endgame_filter_decision.as_ref(),
            );
            if let Some(a) = pending_breakout_anchor.as_mut() {
                a.order_id = Some(oid);
            }
        }
        self._sniper_clear_breakout_entry_anchor_state(false, true);
        self._sniper_set_pending_breakout_entry_anchor(pending_breakout_anchor.clone());
        let endgame_entry_reason = if trigger_mode == "RESOLUTION" {
            "SNIPER_ENDGAME_RESOLUTION"
        } else {
            "SNIPER_ENDGAME_WINDOW"
        };
        self._set_pending_entry_reason(endgame_entry_reason);
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

    fn _sniper_entry_candidate_for_side(
        &self,
        seconds_left: f64,
        ignore_roi_gate: bool,
        preferred_side: Option<&str>,
        bypass_quality_filters: bool,
    ) -> Option<Value> {
        let (yb, ya, nb, na) = self._sniper_best_snapshot();
        let preferred_side = preferred_side.unwrap_or("").trim().to_ascii_uppercase();
        let side_pinned = matches!(preferred_side.as_str(), "YES" | "NO");
        if !side_pinned && (yb <= 0.0 || ya <= 0.0 || nb <= 0.0 || na <= 0.0) {
            return None;
        }
        if side_pinned
            && ((preferred_side == "YES" && (yb <= 0.0 || ya <= 0.0))
                || (preferred_side == "NO" && (nb <= 0.0 || na <= 0.0)))
        {
            return None;
        }
        let y_mid = 0.5 * (yb + ya);
        let n_mid = 0.5 * (nb + na);
        let parity = if yb > 0.0 && ya > 0.0 && nb > 0.0 && na > 0.0 {
            (y_mid + n_mid - 1.0).abs()
        } else {
            0.0
        };
        let sniper_parity_tolerance =
            env_float("SNIPER_PARITY_TOLERANCE", self.parity_tolerance.max(0.0));
        if !bypass_quality_filters && parity > sniper_parity_tolerance {
            return None;
        }
        let tick = self.cfg.tick.max(0.0001);
        let inferred_side = if y_mid >= n_mid { "YES" } else { "NO" };
        let side = if matches!(preferred_side.as_str(), "YES" | "NO") {
            preferred_side
        } else {
            inferred_side.to_string()
        };
        let bid = if side == "YES" { yb } else { nb };
        let ask = if side == "YES" { ya } else { na };
        if bid <= 0.0 || ask <= 0.0 {
            return None;
        }
        let spread_ticks = ((ask - bid) / tick).round() as i64;
        let max_spread_ticks = env_int("SNIPER_MAX_SPREAD_TICKS", self.max_spread_ticks);
        if !bypass_quality_filters && spread_ticks > max_spread_ticks {
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
        if !bypass_quality_filters && !limit_entry && ask > hard_max + 1e-9 {
            return None;
        }
        if !bypass_quality_filters && (entry_px < (price_min - eps) || entry_px > (price_max + eps))
        {
            return None;
        }
        if !ignore_roi_gate && !bypass_quality_filters {
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
            "entry_mode": "SIGNAL",
            "entry_reason": "SIGNAL_ENTRY",
        }))
    }

    pub fn _sniper_entry_candidate(
        &self,
        seconds_left: f64,
        ignore_roi_gate: bool,
    ) -> Option<Value> {
        self._sniper_entry_candidate_for_side(seconds_left, ignore_roi_gate, None, false)
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
            let now_ms = (now_ts_f64() * 1000.0) as i64;
            let metrics_suffix = self
                .sniper_filters
                .lock()
                .ok()
                .map(|f| {
                    let st = f.export_state();
                    let yes = f.evaluate_entry("YES", now_ms);
                    let no = f.evaluate_entry("NO", now_ms);
                    let mom_yes = st
                        .momentum_yes
                        .as_ref()
                        .map(|m| format!("{}/{}:{}", m.checks_passed, m.required_checks, m.reason))
                        .unwrap_or_else(|| "na".to_string());
                    let mom_no = st
                        .momentum_no
                        .as_ref()
                        .map(|m| format!("{}/{}:{}", m.checks_passed, m.required_checks, m.reason))
                        .unwrap_or_else(|| "na".to_string());
                    let breakout_summary = if yes.breakout.applied || no.breakout.applied {
                        let dir = if yes.breakout.direction
                            != crate::sniper_filters::BreakoutDirection::None
                        {
                            yes.breakout.direction.as_str().to_string()
                        } else if no.breakout.direction
                            != crate::sniper_filters::BreakoutDirection::None
                        {
                            no.breakout.direction.as_str().to_string()
                        } else {
                            st.active_trigger.as_str().to_string()
                        };
                        format!(
                            "dir={} y:{} n:{} trig={} cd={}ms",
                            dir,
                            yes.breakout.reason,
                            no.breakout.reason,
                            yes.breakout.triggered || no.breakout.triggered,
                            yes.breakout
                                .cooldown_remaining_ms
                                .max(no.breakout.cooldown_remaining_ms)
                        )
                    } else {
                        "off".to_string()
                    };
                    format!(
                        " | mom[y={},n={}] brk[{}]",
                        mom_yes, mom_no, breakout_summary
                    )
                })
                .unwrap_or_default();
            self.logger.info(&format!(
                "[SNIPER] t_left={seconds_left:6.1}s trades={tc} pnl(mtm)={pnl:+.4} (flat){metrics_suffix}"
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
        if self._sniper_should_block_new_entries() {
            let log_key = "__sniper_post_hedge_block_log_until";
            if now >= self._runtime_ts_get(log_key) {
                self.logger.warning(
                    "[SNIPER] entry blocked: post-hedge protection is active (SNIPER_HEDGED_BLOCK_NEW_ENTRIES=true)",
                );
                self._runtime_ts_set(log_key, now + 2.0);
            }
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

        let mut active_cand = cand.clone();
        let mut ask = active_cand
            .get("ask")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if ask <= 0.0 {
            return false;
        }
        let mut side = active_cand
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let seconds_left = active_cand
            .get("seconds_left")
            .and_then(|v| v.as_f64())
            .unwrap_or(self.expiry_ts as f64 - now);
        let entry_mode = active_cand
            .get("entry_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("NORMAL")
            .to_ascii_uppercase();
        let entry_reason = active_cand
            .get("entry_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let force_diff_override = entry_mode == "FORCE"
            && (entry_reason == "SNIPER_FORCE_DIFF_ENTRY"
                || entry_reason == "RTDS_DIFF_TIME_OVERRIDE");
        let gate_context = if entry_mode == "FORCE" {
            "SNIPER_ENTRY_FORCE"
        } else {
            "SNIPER_ENTRY"
        };
        let (mut gate_ok, gate_threshold_blocked) = if force_diff_override {
            (true, false)
        } else {
            self._rtds_entry_gate_eval_side(&side, seconds_left, gate_context)
        };
        if !gate_ok
            && entry_mode == "FORCE"
            && gate_threshold_blocked
            && env_bool("SNIPER_FORCE_ENTRY_FALLBACK_TO_NORMAL_ON_RTDS_BLOCK", false)
        {
            let now_fallback = now_ts_f64();
            let seconds_left_now = self.expiry_ts as f64 - now_fallback;
            let entry_min = env_float("SNIPER_ENTRY_MIN_SECONDS", 30.0);
            let entry_max = env_float("SNIPER_ENTRY_MAX_SECONDS", 240.0);
            let in_normal_window =
                seconds_left_now + 1e-9 >= entry_min && seconds_left_now <= entry_max + 1e-9;
            let force_exit_window = env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true)
                && seconds_left_now <= env_float("SNIPER_FORCE_EXIT_SECONDS", 8.0) + 1.0;
            if in_normal_window && !force_exit_window {
                if let Some(normal_cand) = self._sniper_entry_candidate(
                    seconds_left_now,
                    env_bool("SNIPER_ENTRY_IGNORE_ROI_GATE", false),
                ) {
                    if self._sniper_entry_confirmed(&normal_cand, now_fallback) {
                        let normal_side = normal_cand
                            .get("side")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let (normal_gate_ok, _) = self._rtds_entry_gate_eval_side(
                            normal_side,
                            seconds_left_now,
                            "SNIPER_ENTRY",
                        );
                        if normal_gate_ok {
                            self.logger.info(&format!(
                                "[SNIPER] FORCE->NORMAL fallback side={} t_left={seconds_left_now:.2}s entry_max={entry_max:.2}s",
                                normal_side
                            ));
                            active_cand = normal_cand;
                            ask = active_cand
                                .get("ask")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0);
                            side = active_cand
                                .get("side")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            gate_ok = true;
                        }
                    }
                }
            }
        }
        if !gate_ok || ask <= 0.0 {
            return false;
        }
        let filter_decision = self._sniper_filters_eval_entry(&side, gate_context, seconds_left);
        if let Some(decision) = &filter_decision {
            if !decision.allowed {
                return false;
            }
        }
        let decided_at_ms = (now * 1000.0) as i64;
        let mut pending_breakout_anchor = self._sniper_build_breakout_entry_anchor(
            &side,
            filter_decision.as_ref(),
            decided_at_ms,
            None,
        );
        let resolved_entry_reason = self._entry_reason_from_candidate(&active_cand);
        let entry_type_name = if force_diff_override {
            "FAK".to_string()
        } else {
            std::env::var("SNIPER_ENTRY_ORDER_TYPE")
                .unwrap_or_else(|_| "FOK".to_string())
                .to_ascii_uppercase()
        };
        let limit_entry = if force_diff_override {
            false
        } else {
            matches!(entry_type_name.as_str(), "GTC" | "LIMIT")
        };
        let mut px = active_cand
            .get("entry_px")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| self._sniper_est_entry_price(ask));
        if force_diff_override {
            px = px.max(ask);
        }
        if px <= 0.0 {
            return false;
        }
        if !limit_entry && px + 1e-12 < ask {
            return false;
        }
        let tick = self.cfg.tick.max(0.0001);
        if force_diff_override {
            let force_hard_max = (1.0 - tick).max(tick);
            px = clamp(round_up(px.max(ask), tick), tick, force_hard_max);
        } else {
            let hard_max = env_float("SNIPER_HARD_MAX_PRICE", env_float("SNIPER_PRICE_MAX", 0.99));
            px = clamp(round_up(px, tick), tick, hard_max.max(tick));
        }

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

        let mut asset_id = active_cand
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
            && !force_diff_override
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
            let oid = oid.unwrap_or_default();
            let bid = active_cand
                .get("bid")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            self._sniper_trade_decision_record_submit(
                &oid,
                &side,
                seconds_left,
                &asset_id,
                bid,
                ask,
                px,
                desired_chunk as f64,
                filter_decision.as_ref(),
            );
            if let Some(a) = pending_breakout_anchor.as_mut() {
                a.order_id = Some(oid.clone());
            }
            self._sniper_clear_breakout_entry_anchor_state(false, true);
            self._sniper_set_pending_breakout_entry_anchor(pending_breakout_anchor.clone());
            self._set_pending_entry_reason(&resolved_entry_reason);
            let pending_key = Self::_sniper_entry_pending_key(&asset_id);
            let confirmed_key = Self::_sniper_entry_confirmed_key(&asset_id);
            self._runtime_ts_set(&pending_key, now_ts_f64());
            self._runtime_ts_set(&confirmed_key, 0.0);
            // Python parity: for resting LIMIT/GTC entry, record last-entry metadata
            // but do not count it as a completed sniper trade until a fill exists.
            if let Ok(mut s) = self.state.lock() {
                s.sniper_last_entry_ts = now_ts_f64();
                s.sniper_last_side = side.clone();
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
        let mut submitted_oid: Option<String> = None;
        let mut submitted_qty = 0.0_f64;
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
            if let Some(oid) = oid {
                any_submitted = true;
                submitted_primary = true;
                submitted_qty = this_chunk as f64;
                submitted_oid = Some(oid);
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
            if let Some(oid) = oid {
                any_submitted = true;
                submitted_qty = fb_chunk as f64;
                submitted_oid = Some(oid);
            }
        }
        if !any_submitted {
            return false;
        }
        if let Some(oid) = submitted_oid {
            let bid = active_cand
                .get("bid")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            self._sniper_trade_decision_record_submit(
                &oid,
                &side,
                seconds_left,
                &asset_id,
                bid,
                ask,
                px,
                submitted_qty.max(0.0),
                filter_decision.as_ref(),
            );
            if let Some(a) = pending_breakout_anchor.as_mut() {
                a.order_id = Some(oid.clone());
            }
        }
        self._sniper_clear_breakout_entry_anchor_state(false, true);
        self._sniper_set_pending_breakout_entry_anchor(pending_breakout_anchor.clone());
        self._set_pending_entry_reason(&resolved_entry_reason);

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
        self._mark_sniper_entry_state(&side);
        true
    }

    fn _sniper_is_flat(&self) -> bool {
        let min_sh = self.cfg.min_shares.max(1.0) - 1e-12;
        self.state
            .lock()
            .map(|s| s.q_yes < min_sh && s.q_no < min_sh)
            .unwrap_or(false)
    }

    fn _sniper_is_paired_hedged(&self) -> bool {
        let min_sh = self.cfg.min_shares.max(1.0) - 1e-12;
        self.state
            .lock()
            .map(|s| {
                let qy = s.q_yes.max(0.0);
                let qn = s.q_no.max(0.0);
                qy >= min_sh && qn >= min_sh && (qy - qn).abs() < self.cfg.min_shares.max(1.0)
            })
            .unwrap_or(false)
    }

    fn _sniper_post_hedge_active(&self) -> bool {
        self._runtime_ts_get("__sniper_post_hedge_active") > 0.5
    }

    fn _sniper_clear_post_hedge_state(&self) {
        self._runtime_ts_set("__sniper_post_hedge_active", 0.0);
        self._runtime_ts_set("__sniper_post_hedge_recover_until", 0.0);
        self._runtime_ts_set("__sniper_post_hedge_unwind_submits", 0.0);
        self._runtime_ts_set("__sniper_post_hedge_hold_mode", 0.0);
    }

    fn _sniper_mark_post_hedge_state(&self, now: f64) {
        self._runtime_ts_set("__sniper_post_hedge_active", 1.0);
        self._runtime_ts_set(
            "__sniper_post_hedge_recover_until",
            now + self.sniper_stop_certainty.post_hedge_recover_window_seconds,
        );
        self._runtime_ts_set("__sniper_post_hedge_unwind_submits", 0.0);
        self._runtime_ts_set("__sniper_post_hedge_hold_mode", 0.0);
        self.logger.warning(&format!(
            "[SNIPER][POST_HEDGE] active policy={} recover_window_s={:.2}",
            self.sniper_stop_certainty.post_hedge_policy.as_str(),
            self.sniper_stop_certainty.post_hedge_recover_window_seconds
        ));
    }

    fn _sniper_should_block_new_entries(&self) -> bool {
        self.sniper_stop_certainty.hedged_block_new_entries && self._sniper_post_hedge_active()
    }

    fn _sniper_bounded_pause_seconds(
        &self,
        base_s: f64,
        reason_u: &str,
        has_open_exposure: bool,
    ) -> f64 {
        let mut out = if base_s.is_finite() && base_s > 0.0 {
            base_s
        } else {
            0.0
        };
        if reason_u == "STOP_LOSS"
            && has_open_exposure
            && self.sniper_stop_certainty.enabled
            && self
                .sniper_stop_certainty
                .stop_loss_open_exposure_max_pause_ms
                > 0
        {
            let cap = self
                .sniper_stop_certainty
                .stop_loss_open_exposure_max_pause_ms as f64
                / 1000.0;
            out = out.min(cap.max(0.0));
        }
        out.max(0.0)
    }

    fn _sniper_set_fail_pause(&self, reason_u: &str, base_s: f64) {
        let has_open_exposure = self._sniper_position().is_some();
        let pause_s = self._sniper_bounded_pause_seconds(base_s, reason_u, has_open_exposure);
        if pause_s > 0.0 {
            self._runtime_ts_set("__taker_fail_pause_until", now_ts_f64() + pause_s);
        }
    }

    fn _sniper_stop_certainty_hedge_phase(
        &self,
        pos: &Value,
        reason_u: &str,
        trigger: &str,
    ) -> bool {
        if !(self.sniper_stop_certainty.enabled && reason_u == "STOP_LOSS") {
            self._sniper_maybe_exit_hedge(pos, reason_u, trigger);
            return self._sniper_position().is_none();
        }
        let deadline =
            now_ts_f64() + (self.sniper_stop_certainty.hedge_budget_ms.max(1) as f64 / 1000.0);
        let mut submits = 0i64;
        while submits < self.sniper_stop_certainty.hedge_max_submits && now_ts_f64() <= deadline {
            let Some(cur) = self._sniper_position() else {
                self._sniper_clear_post_hedge_state();
                return true;
            };
            let _before_qty = cur
                .get("qty")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                .max(0.0);
            let _ = self._sniper_maybe_exit_hedge_with_opts(&cur, reason_u, trigger, true, submits);
            submits += 1;
            let wait_s = (self.sniper_stop_certainty.sell_post_wait_ms.max(1) as f64 / 1000.0)
                .clamp(0.01, 1.0);
            thread::sleep(Duration::from_secs_f64(wait_s));
            if self._sniper_position().is_none() {
                self._sniper_clear_post_hedge_state();
                return true;
            }
            if self._sniper_is_paired_hedged() {
                self._sniper_mark_post_hedge_state(now_ts_f64());
                return false;
            }
        }
        false
    }

    fn _sniper_handle_post_hedge_policy(&self, pos: &Value, now: f64, seconds_left: f64) -> bool {
        if !self._sniper_post_hedge_active() {
            return false;
        }
        if self._sniper_is_flat() {
            self._sniper_clear_post_hedge_state();
            return false;
        }
        if !self._sniper_is_paired_hedged() {
            return false;
        }
        if seconds_left <= self.cfg.stop_buffer_seconds as f64 {
            return true;
        }
        match self.sniper_stop_certainty.post_hedge_policy {
            SniperPostHedgePolicy::HoldToResolution => true,
            SniperPostHedgePolicy::ImmediateUnwind => {
                let submits = self
                    ._runtime_ts_get("__sniper_post_hedge_unwind_submits")
                    .max(0.0) as i64;
                if submits >= self.sniper_stop_certainty.post_hedge_max_unwind_submits {
                    return true;
                }
                self._runtime_ts_set("__sniper_post_hedge_unwind_submits", (submits + 1) as f64);
                if self._sniper_try_exit(pos, "POST_HEDGE_UNWIND") {
                    self._sniper_clear_post_hedge_state();
                    return false;
                }
                true
            }
            SniperPostHedgePolicy::HybridTimed => {
                let recover_until = self._runtime_ts_get("__sniper_post_hedge_recover_until");
                let submits = self
                    ._runtime_ts_get("__sniper_post_hedge_unwind_submits")
                    .max(0.0) as i64;
                if now <= recover_until
                    && submits < self.sniper_stop_certainty.post_hedge_max_unwind_submits
                {
                    self._runtime_ts_set(
                        "__sniper_post_hedge_unwind_submits",
                        (submits + 1) as f64,
                    );
                    if self._sniper_try_exit(pos, "POST_HEDGE_UNWIND") {
                        self._sniper_clear_post_hedge_state();
                        return false;
                    }
                    return true;
                }
                self._runtime_ts_set("__sniper_post_hedge_hold_mode", 1.0);
                true
            }
        }
    }

    fn _sniper_maybe_exit_hedge(&self, pos: &Value, reason: &str, trigger: &str) {
        let _ = self._sniper_maybe_exit_hedge_with_opts(pos, reason, trigger, false, 0);
    }

    fn _sniper_maybe_exit_hedge_with_opts(
        &self,
        pos: &Value,
        reason: &str,
        trigger: &str,
        force: bool,
        pass_idx: i64,
    ) -> bool {
        let reason_u = reason.trim().to_ascii_uppercase();
        let stop_mode = self._sniper_stop_loss_mode();
        let fallback_mode = self._sniper_stop_loss_fallback_mode();
        let stop_mode_hedge =
            reason_u == "STOP_LOSS" && (stop_mode == "HEDGE" || fallback_mode == "HEDGE");
        let forced_stop_loss =
            force && reason_u == "STOP_LOSS" && self.sniper_stop_certainty.enabled;
        if !forced_stop_loss && !env_bool("SNIPER_EXIT_HEDGE_ENABLED", false) && !stop_mode_hedge {
            return false;
        }
        if !forced_stop_loss
            && env_bool("SNIPER_EXIT_HEDGE_STOP_LOSS_ONLY", true)
            && reason_u != "STOP_LOSS"
        {
            return false;
        }
        let side = pos
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if !matches!(side.as_str(), "YES" | "NO") {
            return false;
        }
        let qty = pos
            .get("qty")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .max(0.0);
        let cost = pos
            .get("cost")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .max(0.0);
        let bid = pos.get("bid").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if qty <= 0.0 {
            return false;
        }
        let min_loss = if forced_stop_loss {
            0.0
        } else {
            env_float("SNIPER_EXIT_HEDGE_MIN_LOSS_PCT", 0.0).max(0.0)
        };
        if !forced_stop_loss && cost > 1e-12 {
            let exit_px = self._sniper_est_exit_price(bid, 0.0);
            let pnl_pct = (qty * exit_px - cost) / cost;
            if pnl_pct + 1e-12 > -min_loss {
                return false;
            }
        }

        let now = now_ts_f64();
        let (q_yes, q_no) = self
            .state
            .lock()
            .map(|s| (s.q_yes.max(0.0), s.q_no.max(0.0)))
            .unwrap_or((0.0, 0.0));
        let (primary_qty, opposite_qty, hedge_asset) = if side == "YES" {
            (
                q_yes,
                q_no,
                self.no_asset.clone().unwrap_or_default().to_string(),
            )
        } else {
            (
                q_no,
                q_yes,
                self.yes_asset.clone().unwrap_or_default().to_string(),
            )
        };
        if hedge_asset.trim().is_empty() {
            return false;
        }
        let cooldown_s = env_float("SNIPER_EXIT_HEDGE_COOLDOWN_SECONDS", 1.0).max(0.1);
        let cooldown_key = format!("__sniper_exit_hedge_cooldown_until_{hedge_asset}");
        if !forced_stop_loss && now < self._runtime_ts_get(&cooldown_key) {
            return false;
        }
        let inflight_s = env_float("SNIPER_EXIT_HEDGE_INFLIGHT_SECONDS", 0.75).max(0.1);
        let pending_guard_s = (inflight_s * 2.0).clamp(0.5, 5.0);

        let min_sh = self.cfg.min_shares.max(1.0);
        let net_exposure = (primary_qty - opposite_qty).max(0.0);
        if net_exposure + 1e-12 < min_sh {
            self._runtime_ts_set(&cooldown_key, now + cooldown_s);
            return false;
        }
        if !forced_stop_loss
            && self.taker_strict_inflight
            && self._has_pending_taker_order_recent("BUY", Some(&hedge_asset), pending_guard_s)
        {
            self._runtime_ts_set(&cooldown_key, now + cooldown_s.min(2.0));
            return false;
        }

        let (_bid_h, ask_h) = match self._best_bid_ask(&hedge_asset) {
            Some(v) => v,
            None => return false,
        };
        if ask_h <= 0.0 {
            return false;
        }
        let tick = self.cfg.tick.max(0.0001);
        let slip_ticks = if forced_stop_loss {
            (self.sniper_stop_certainty.hedge_slip_base_ticks
                + pass_idx.max(0) * self.sniper_stop_certainty.hedge_slip_step_ticks)
                .max(0)
        } else {
            env_int("SNIPER_EXIT_HEDGE_SLIPPAGE_TICKS", 2).max(0)
        };
        let mut px = round_up(ask_h + slip_ticks as f64 * tick, tick);
        if forced_stop_loss {
            let mut cap_px = self._hedge_price_cap()
                + self.sniper_stop_certainty.hedge_cap_extra_ticks.max(0) as f64 * tick;
            cap_px = clamp(round_down(cap_px, tick), tick, 0.99);
            if cap_px <= 0.0 {
                return false;
            }
            px = px.min(cap_px);
        }
        px = clamp(px, tick, 0.99);
        if px + 1e-9 < ask_h {
            return false;
        }

        let mut fraction = env_float("SNIPER_EXIT_HEDGE_SIZE_FRACTION", 1.0);
        if !fraction.is_finite() {
            fraction = 1.0;
        }
        if forced_stop_loss {
            fraction = 1.0;
        } else {
            fraction = fraction.clamp(0.0, 1.0);
        }
        if fraction <= 0.0 {
            return false;
        }
        let mut target_shares = net_exposure * fraction;
        let max_shares = env_float("SNIPER_EXIT_HEDGE_MAX_SHARES", 0.0).max(0.0);
        if max_shares > 0.0 {
            target_shares = target_shares.min(max_shares);
        }
        let max_notional = env_float("SNIPER_EXIT_HEDGE_MAX_NOTIONAL_USD", 0.0).max(0.0);
        if max_notional > 0.0 && px > 0.0 {
            target_shares = target_shares.min(max_notional / px);
        }
        let per_order_cap = env_int("SNIPER_EXIT_HEDGE_MAX_SHARES_PER_ORDER", 0);
        if per_order_cap > 0 {
            target_shares = target_shares.min(per_order_cap as f64);
        }
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let mut size_int = (target_shares + 1e-12).floor() as i64;
        size_int = (size_int / min_int) * min_int;
        if size_int < min_int {
            self._runtime_ts_set(&cooldown_key, now + cooldown_s);
            return false;
        }

        let hedge_ot = std::env::var("SNIPER_EXIT_HEDGE_ORDER_TYPE")
            .unwrap_or_else(|_| "FAK".to_string())
            .to_ascii_uppercase();
        self._runtime_ts_set("__taker_inflight_until", now + inflight_s);
        let oid = self._place_taker_bid_fak(&hedge_asset, px, size_int as f64, Some(&hedge_ot));
        let aid_tail: String = hedge_asset
            .chars()
            .rev()
            .take(6)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if oid.is_some() {
            self.logger.warning(&format!(
                "[SNIPER][HEDGE] trigger={trigger} reason={reason} side={side} buy_opp={aid_tail} ask={ask_h:.4} px={px:.4} sz={size_int} type={hedge_ot}"
            ));
            if let Some(oid_s) = oid.as_ref() {
                self._sniper_mark_hedge_order(oid_s);
                self._sniper_log_hedge_order_progress(
                    oid_s,
                    &hedge_asset,
                    "BUY",
                    0.0,
                    size_int as f64,
                    size_int as f64,
                    "SUBMIT",
                    "SUBMITTED",
                );
            }
        } else {
            self.logger.warning(&format!(
                "[SNIPER][HEDGE] trigger={trigger} reason={reason} submit_failed side={side} buy_opp={aid_tail} ask={ask_h:.4} px={px:.4} sz={size_int} type={hedge_ot}"
            ));
        }
        self._runtime_ts_set(&cooldown_key, now + cooldown_s);
        oid.is_some()
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
        let stop_loss_retry_delay_override = if reason_u == "STOP_LOSS" {
            Some(env_float("SNIPER_STOP_LOSS_RETRY_DELAY_SECONDS", 0.0).max(0.0))
        } else {
            None
        };
        let stop_certainty_active = reason_u == "STOP_LOSS" && self.sniper_stop_certainty.enabled;
        let stop_certainty_sell_deadline = if stop_certainty_active {
            now + (self.sniper_stop_certainty.sell_budget_ms.max(1) as f64 / 1000.0)
        } else {
            f64::INFINITY
        };
        let stop_certainty_sell_max_submits = if stop_certainty_active {
            self.sniper_stop_certainty.sell_max_submits.max(1)
        } else {
            3
        };
        let stop_certainty_post_wait_s = if stop_certainty_active {
            (self.sniper_stop_certainty.sell_post_wait_ms.max(1) as f64 / 1000.0).clamp(0.01, 1.0)
        } else {
            0.0
        };
        let stop_certainty_no_derisk_eps = if stop_certainty_active {
            self.sniper_stop_certainty.no_derisk_eps_shares.max(0.0)
        } else {
            0.0
        };
        let retry_pause_s = |base: f64| -> f64 {
            if let Some(v) = stop_loss_retry_delay_override {
                v
            } else if base.is_finite() && base > 0.0 {
                base
            } else {
                0.0
            }
        };
        let retry_sleep_s = |base: f64| -> f64 {
            if let Some(v) = stop_loss_retry_delay_override {
                v
            } else if base.is_finite() && base > 0.0 {
                base
            } else {
                0.0
            }
        };
        if reason_u != "STOP_LOSS" {
            self._sniper_stop_loss_reset_failures(&asset_id);
        }
        let mut mode = self._sniper_stop_loss_mode();
        if reason_u == "STOP_LOSS" {
            let fallback_mode = self._sniper_stop_loss_fallback_mode();
            let fallback_fails = self._sniper_stop_loss_fallback_fails();
            if !fallback_mode.is_empty() {
                let fail_key = Self::_sniper_stop_loss_fail_key(&asset_id);
                let fails = self._runtime_ts_get(&fail_key).max(0.0);
                if fails + 1e-9 >= fallback_fails {
                    if mode != fallback_mode {
                        let now = now_ts_f64();
                        let log_key =
                            format!("__sniper_stop_loss_fallback_active_log_until_{asset_id}");
                        if now >= self._runtime_ts_get(&log_key) {
                            self.logger.warning(&format!(
                                "[SNIPER][STOP_LOSS] fallback_active fails={:.0}/{:.0} mode={} -> {}",
                                fails, fallback_fails, mode, fallback_mode
                            ));
                            self._runtime_ts_set(&log_key, now + 2.0);
                        }
                    }
                    mode = fallback_mode;
                }
            }
            if let Ok(mut cat) = self.stop_loss_category.lock() {
                *cat = Some(mode.clone());
            }
        }
        if reason_u == "STOP_LOSS" && mode == "HEDGE" {
            if stop_certainty_active {
                let done =
                    self._sniper_stop_certainty_hedge_phase(pos, &reason_u, "stop_loss_mode");
                self._sniper_set_fail_pause(&reason_u, retry_pause_s(0.5));
                return done;
            }
            self._sniper_maybe_exit_hedge(pos, &reason_u, "stop_loss_mode");
            self._sniper_set_fail_pause(&reason_u, retry_pause_s(0.5));
            return false;
        }
        let stop_limit_mode = reason_u == "STOP_LOSS" && mode == "LIMIT" && !stop_certainty_active;

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
                if stop_certainty_active {
                    let done =
                        self._sniper_stop_certainty_hedge_phase(pos, &reason_u, "wait_confirmed");
                    self._sniper_set_fail_pause(&reason_u, retry_pause_s(0.5));
                    return done;
                }
                self._sniper_maybe_exit_hedge(pos, &reason_u, "wait_confirmed");
                self._sniper_set_fail_pause(&reason_u, retry_pause_s(0.5));
                return false;
            }
        }

        let remaining = pos.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let start_remaining = remaining.max(0.0);
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

        // Event-driven exit sizing: rely on local position that is updated by
        // MATCHED/MINED/CONFIRMED events, not balance/allowance snapshots.

        if stop_limit_mode {
            let active_entry_reason = self._active_entry_reason_or_default();
            let (_, stop_override_pct) =
                Self::_sniper_tp_sl_overrides_for_entry_reason(&active_entry_reason);
            let stop_pct = stop_override_pct.unwrap_or(env_float("SNIPER_STOP_LOSS_PCT", 0.0));
            let mut ref_px = self._runtime_ts_get("__sniper_entry_ref_price");
            if ref_px <= 0.0 {
                ref_px = pos.get("avg").and_then(|v| v.as_f64()).unwrap_or(0.0);
            }
            if !(ref_px <= 0.0 || stop_pct <= 0.0) {
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
                let cancel_settle_s = retry_sleep_s(0.150);
                if cancel_settle_s > 0.0 {
                    thread::sleep(Duration::from_secs_f64(cancel_settle_s));
                }

                let mut sell_sz = remaining;
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
                    let post_submit_s = retry_sleep_s(1.0);
                    if post_submit_s > 0.0 {
                        thread::sleep(Duration::from_secs_f64(post_submit_s));
                    }
                    if self._sniper_position().is_none() {
                        self._mark_sniper_exit_state();
                        return true;
                    }
                } else {
                    self._sniper_maybe_exit_hedge(pos, &reason_u, "stop_limit_submit_reject");
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
            let cancel_settle_s = retry_sleep_s(0.200);
            if cancel_settle_s > 0.0 {
                thread::sleep(Duration::from_secs_f64(cancel_settle_s));
            }
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
        let exit_slip_ticks = env_int("SNIPER_EXIT_SLIPPAGE_TICKS", 1).max(0);
        let max_passes = stop_certainty_sell_max_submits.max(1);
        let mut sold_any = false;
        let mut submitted_any = false;
        let mut stop_certainty_progress = 0.0f64;
        for pass_i in 0..max_passes {
            if stop_certainty_active && now_ts_f64() > stop_certainty_sell_deadline {
                break;
            }
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
                if sell_sz + 1e-12 < min_exit_size {
                    self._sniper_set_fail_pause(&reason_u, retry_pause_s(1.0));
                    return false;
                }
            } else {
                let mut sell_int = (sell_sz + 1e-12).floor() as i64;
                sell_int = (sell_int / min_int) * min_int;
                if sell_int < min_int {
                    let chunk_i = (chunk + 1e-12).floor() as i64;
                    sell_int = remaining_int.min(chunk_i.max(min_int));
                }
                if sell_int < min_int {
                    self._sniper_set_fail_pause(&reason_u, retry_pause_s(1.0));
                    return false;
                }
                sell_sz = sell_int as f64;
            }
            if stop_certainty_active && now_ts_f64() > stop_certainty_sell_deadline {
                break;
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
                if stop_certainty_active {
                    let done = self._sniper_stop_certainty_hedge_phase(
                        &cur,
                        &reason_u,
                        "sell_submit_reject",
                    );
                    self._sniper_set_fail_pause(&reason_u, retry_pause_s(0.5));
                    return done;
                }
                self._sniper_maybe_exit_hedge(&cur, &reason_u, "exit_submit_reject");
                let fast_retry_s = if reason_u == "STOP_LOSS" {
                    retry_pause_s(0.25)
                } else {
                    retry_pause_s(1.0)
                };
                self._sniper_set_fail_pause(&reason_u, fast_retry_s);
                return false;
            }
            submitted_any = true;
            let post_submit_s = if stop_certainty_active {
                stop_certainty_post_wait_s
            } else {
                retry_sleep_s(1.0)
            };
            if post_submit_s > 0.0 {
                thread::sleep(Duration::from_secs_f64(post_submit_s));
            }
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
            let derisk = (remaining - rem2).max(0.0);
            if derisk > stop_certainty_progress {
                stop_certainty_progress = derisk;
            }
            if stop_certainty_active {
                if derisk + 1e-9 >= stop_certainty_no_derisk_eps {
                    sold_any = true;
                    continue;
                }
                if env_bool("SNIPER_CANCEL_EXIT_ORDERS_BEFORE_RETRY", true) {
                    self._cancel_exchange_orders_for_assets(
                        std::slice::from_ref(&asset_id),
                        &format!("sniper exit {reason_u} no-derisk cancel"),
                    );
                }
                let done =
                    self._sniper_stop_certainty_hedge_phase(&cur, &reason_u, "sell_no_derisk");
                self._sniper_set_fail_pause(&reason_u, retry_pause_s(0.5));
                return done;
            }
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
            self._sniper_set_fail_pause(&reason_u, retry_pause_s(1.0));
            break;
        }
        if stop_certainty_active {
            let Some(cur) = self._sniper_position() else {
                self._mark_sniper_exit_state();
                return true;
            };
            let rem = cur
                .get("qty")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                .max(0.0);
            let derisk_total = (start_remaining - rem)
                .max(0.0)
                .max(stop_certainty_progress);
            let trigger = if derisk_total + 1e-9 >= stop_certainty_no_derisk_eps {
                "sell_stage_residual"
            } else if submitted_any {
                "sell_stage_no_derisk"
            } else {
                "sell_stage_timeout"
            };
            let done = self._sniper_stop_certainty_hedge_phase(&cur, &reason_u, trigger);
            self._sniper_set_fail_pause(&reason_u, retry_pause_s(0.5));
            return done;
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
            "entry_mode": "NORMAL",
            "entry_reason": "SNIPER_ENTRY",
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
        let mut stop_loss_stale_cycles = 0i64;

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
            self._sniper_filters_ingest_latest_tick();
            if !self._market_data_fresh() {
                if self.sniper_stop_certainty.enabled {
                    let stop_loss_active = self._runtime_ts_get("__sniper_stop_loss_active") > 0.5;
                    if stop_loss_active {
                        if let Some(pos_stale) = self._sniper_position() {
                            stop_loss_stale_cycles += 1;
                            self._runtime_ts_set(
                                "__sniper_stop_loss_stale_cycles",
                                stop_loss_stale_cycles as f64,
                            );
                            if stop_loss_stale_cycles
                                >= self.sniper_stop_certainty.stop_loss_stale_cycles_to_hedge
                            {
                                self.logger.warning(&format!(
                                    "[SNIPER][STOP_LOSS] stale_feed hedge fallback cycles={}/{}",
                                    stop_loss_stale_cycles,
                                    self.sniper_stop_certainty.stop_loss_stale_cycles_to_hedge
                                ));
                                let _ = self._sniper_stop_certainty_hedge_phase(
                                    &pos_stale,
                                    "STOP_LOSS",
                                    "stale_feed_signal",
                                );
                                stop_loss_stale_cycles = 0;
                                self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
                            }
                        } else {
                            stop_loss_stale_cycles = 0;
                            self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
                            self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
                        }
                    } else {
                        stop_loss_stale_cycles = 0;
                        self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
                    }
                }
                continue;
            }
            stop_loss_stale_cycles = 0;
            self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
            if now < self._runtime_ts_get("__taker_fail_pause_until") {
                continue;
            }

            let pos = self._sniper_position();
            if pos.is_none() {
                self._runtime_ts_set("__rtds_hold_active", 0.0);
                self._runtime_ts_set("__rtds_hold_side_yes", 0.0);
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
                if sniper_in_pos {
                    sniper_in_pos = false;
                    sniper_pos_open_ts = 0.0;
                    sniper_stop_breach_since = None;
                    self._runtime_ts_set("__sniper_entry_ref_price", 0.0);
                    self._sniper_filters_clear_breakout_invalidation_stop();
                    self._sniper_clear_breakout_entry_anchor_state(true, true);
                }
                self._sniper_clear_post_hedge_state();
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
                let pos_side = pos
                    .as_ref()
                    .and_then(|p| p.get("side"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self._sniper_arm_breakout_invalidation_stop_for_position(
                    pos_side,
                    "SIGNAL_POS_OPEN",
                    seconds_left,
                );
            }

            if pos.is_none() {
                if self._sniper_should_block_new_entries() {
                    continue;
                }
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
            let active_entry_reason = self._active_entry_reason_or_default();
            let (take_profit_pct, stop_pct) =
                self._sniper_tp_sl_for_entry_reason(&active_entry_reason);
            let bypass_hold_for_tp = self._should_bypass_rtds_hold_for_take_profit(
                &active_entry_reason,
                cost,
                pnl_pct,
                take_profit_pct,
            );
            let hold_active =
                self._rtds_hold_till_resolution_active(&pos_side, seconds_left, "SIGNAL_HOLD");
            if hold_active && !bypass_hold_for_tp {
                sniper_stop_breach_since = None;
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
                continue;
            }
            if hold_active && bypass_hold_for_tp {
                self._rtds_gate_log(
                    "hold_bypass_force_diff_tp",
                    &format!(
                        "[RTDS_HOLD] SIGNAL_HOLD bypass: reason={} pnl_pct={:+.6} tp={:.6} t_left={:.2}s",
                        active_entry_reason, pnl_pct, take_profit_pct, seconds_left
                    ),
                );
            }
            if self._sniper_handle_post_hedge_policy(&pos, now, seconds_left) {
                sniper_stop_breach_since = None;
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
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
            if cost > 1e-12 && pnl_pct >= take_profit_pct {
                if self._sniper_try_exit(&pos, "TAKE_PROFIT") {
                    break;
                }
                continue;
            }
            if let Some(stop_decision) = self._sniper_filters_eval_breakout_invalidation_stop(
                &pos_side,
                "SIGNAL_POS",
                seconds_left,
            ) {
                if stop_decision.fired {
                    sniper_stop_breach_since = None;
                    let aid = pos
                        .get("asset_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let sl_mode = self._sniper_stop_loss_mode();
                    self._runtime_ts_set("__sniper_stop_loss_active", 1.0);
                    if self._sniper_try_exit(&pos, "STOP_LOSS") {
                        self._sniper_stop_loss_reset_failures(&aid);
                        break;
                    }
                    if !self._sniper_post_hedge_active() {
                        self._sniper_stop_loss_record_sell_failure(
                            &pos,
                            &aid,
                            &sl_mode,
                            "STOP_LOSS",
                            "stop_loss_breakout_loop_signal",
                        );
                    }
                    continue;
                }
            }
            if cost > 1e-12 && stop_pct > 0.0 {
                let mut stop_loss_active_now = false;
                if pnl_pct <= -stop_pct {
                    let held_s = now - sniper_pos_open_ts.max(0.0);
                    if held_s >= env_float("SNIPER_MIN_HOLD_SECONDS", 0.0).max(0.0) {
                        if sniper_stop_breach_since.is_none() {
                            sniper_stop_breach_since = Some(now);
                        }
                        let confirm_s = env_float("SNIPER_STOP_CONFIRM_SECONDS", 0.0).max(0.0);
                        if now - sniper_stop_breach_since.unwrap_or(now) >= confirm_s {
                            stop_loss_active_now = true;
                            let aid = pos
                                .get("asset_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let sl_mode = self._sniper_stop_loss_mode();
                            if self._sniper_try_exit(&pos, "STOP_LOSS") {
                                self._sniper_stop_loss_reset_failures(&aid);
                                break;
                            }
                            if !self._sniper_post_hedge_active() {
                                self._sniper_stop_loss_record_sell_failure(
                                    &pos,
                                    &aid,
                                    &sl_mode,
                                    "STOP_LOSS",
                                    "stop_loss_loop_signal",
                                );
                            }
                            self._runtime_ts_set(
                                "__sniper_stop_loss_active",
                                if stop_loss_active_now { 1.0 } else { 0.0 },
                            );
                            continue;
                        }
                    }
                } else {
                    sniper_stop_breach_since = None;
                    let aid = pos
                        .get("asset_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    self._sniper_stop_loss_reset_failures(&aid);
                }
                self._runtime_ts_set(
                    "__sniper_stop_loss_active",
                    if stop_loss_active_now { 1.0 } else { 0.0 },
                );
            } else {
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
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
            "SNIPER mode enabled | price=[{:.2},{:.2}] TP={:.1}% SL={:.1}% entry_window=[{}s..{}s] force_exit={}s exit_before_expiry={} force_entry_min={:.2} force_entry_min_diff={:.2} force_entry_max_age={}s force_entry_diff_max_age={:.2}s force_rtds_fallback={} entry_confirm={:.2}s",
            env_float("SNIPER_PRICE_MIN", 0.91),
            env_float("SNIPER_PRICE_MAX", 0.99),
            env_float("SNIPER_TAKE_PROFIT_PCT", 0.01) * 100.0,
            env_float("SNIPER_STOP_LOSS_PCT", 0.02) * 100.0,
            env_float("SNIPER_ENTRY_MIN_SECONDS", 30.0),
            env_float("SNIPER_ENTRY_MAX_SECONDS", 240.0),
            env_float("SNIPER_FORCE_EXIT_SECONDS", 8.0),
            env_bool("SNIPER_EXIT_BEFORE_EXPIRY", true),
            env_float("SNIPER_FORCE_ENTRY_MIN_PRICE", 0.0),
            env_float("SNIPER_FORCE_ENTRY_MIN_DIFF_PRICE", 0.0),
            env_int("SNIPER_FORCE_ENTRY_MAX_AGE_SECONDS", 0),
            env_float(
                "SNIPER_FORCE_ENTRY_MIN_DIFF_PRICE_MAX_AGE_SECONDS",
                env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 2.0),
            ),
            env_bool("SNIPER_FORCE_ENTRY_FALLBACK_TO_NORMAL_ON_RTDS_BLOCK", false),
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
        let mut stop_loss_stale_cycles = 0i64;

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
            self._sniper_filters_ingest_latest_tick();
            if now - last_log >= (self.cfg.log_every as f64).max(0.5) {
                self._log_status_sniper(seconds_left);
                last_log = now;
            }
            if self._sniper_maybe_endgame_blind_post(seconds_left, now) {
                continue;
            }
            if !self._market_data_fresh() {
                if self.sniper_stop_certainty.enabled {
                    let stop_loss_active = self._runtime_ts_get("__sniper_stop_loss_active") > 0.5;
                    if stop_loss_active {
                        if let Some(pos_stale) = self._sniper_position() {
                            stop_loss_stale_cycles += 1;
                            self._runtime_ts_set(
                                "__sniper_stop_loss_stale_cycles",
                                stop_loss_stale_cycles as f64,
                            );
                            if stop_loss_stale_cycles
                                >= self.sniper_stop_certainty.stop_loss_stale_cycles_to_hedge
                            {
                                self.logger.warning(&format!(
                                    "[SNIPER][STOP_LOSS] stale_feed hedge fallback cycles={}/{}",
                                    stop_loss_stale_cycles,
                                    self.sniper_stop_certainty.stop_loss_stale_cycles_to_hedge
                                ));
                                let _ = self._sniper_stop_certainty_hedge_phase(
                                    &pos_stale,
                                    "STOP_LOSS",
                                    "stale_feed",
                                );
                                stop_loss_stale_cycles = 0;
                                self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
                            }
                        } else {
                            stop_loss_stale_cycles = 0;
                            self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
                            self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
                        }
                    } else {
                        stop_loss_stale_cycles = 0;
                        self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
                    }
                }
                continue;
            }
            stop_loss_stale_cycles = 0;
            self._runtime_ts_set("__sniper_stop_loss_stale_cycles", 0.0);
            if now < self._runtime_ts_get("__taker_fail_pause_until") {
                continue;
            }

            let pos = self._sniper_position();
            if pos.is_none() {
                self._runtime_ts_set("__rtds_hold_active", 0.0);
                self._runtime_ts_set("__rtds_hold_side_yes", 0.0);
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
                if sniper_in_pos {
                    sniper_in_pos = false;
                    sniper_pos_open_ts = 0.0;
                    sniper_stop_breach_since = None;
                    self._runtime_ts_set("__sniper_entry_ref_price", 0.0);
                    self._runtime_ts_set("__sniper_entry_gate_since", 0.0);
                    self._sniper_filters_clear_breakout_invalidation_stop();
                    self._sniper_clear_breakout_entry_anchor_state(true, true);
                }
                self._sniper_clear_post_hedge_state();
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
                let pos_side = pos
                    .as_ref()
                    .and_then(|p| p.get("side"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self._sniper_arm_breakout_invalidation_stop_for_position(
                    pos_side,
                    "SNIPER_POS_OPEN",
                    seconds_left,
                );
            } else if self._runtime_ts_get("__sniper_entry_ref_price") <= 0.0 {
                let avg = pos
                    .as_ref()
                    .and_then(|p| p.get("avg"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                self._runtime_ts_set("__sniper_entry_ref_price", avg.max(0.0));
            }

            if pos.is_none() {
                if self._sniper_should_block_new_entries() {
                    continue;
                }
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
                let force_diff_signal = self._sniper_force_entry_diff_signal(seconds_left);
                let force_diff_triggered = force_diff_signal.is_some();
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
                    if !force_diff_triggered
                        && seconds_left < env_float("SNIPER_ENTRY_MIN_SECONDS", 30.0)
                    {
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

                if let Some((force_side, force_diff_price)) = force_diff_signal {
                    let cand = self._sniper_entry_candidate_for_side(
                        seconds_left,
                        true,
                        Some(&force_side),
                        true,
                    );
                    if cand.is_none() {
                        self.logger.info(&format!(
                            "[SNIPER] FORCE-DIFF skipped: side={} diff_price={:+.3} no-valid-side-quote",
                            force_side, force_diff_price
                        ));
                        if env_float("SNIPER_ENTRY_CONFIRM_SECONDS", 0.0) > 0.0 {
                            self._runtime_ts_set("__sniper_entry_gate_since", 0.0);
                        }
                    } else if let Some(mut cand) = cand {
                        if let Value::Object(ref mut o) = cand {
                            o.insert("entry_mode".to_string(), json!("FORCE"));
                            o.insert("entry_reason".to_string(), json!("SNIPER_FORCE_DIFF_ENTRY"));
                        }
                        if !self._sniper_entry_confirmed(&cand, now) {
                            continue;
                        }
                        let min_diff = env_float("SNIPER_FORCE_ENTRY_MIN_DIFF_PRICE", 0.0);
                        self.logger.info(&format!(
                            "[SNIPER] FORCE-DIFF entry triggered side={} diff_price={:+.3}>=min={min_diff:.3} ask={:.3} entry_px={:.3} t_left={seconds_left:.1}s spread_ticks={} parity={:.4}",
                            cand.get("side").and_then(|v| v.as_str()).unwrap_or(""),
                            force_diff_price,
                            cand.get("ask").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            cand.get("entry_px").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            cand.get("spread_ticks").and_then(|v| v.as_i64()).unwrap_or(0),
                            cand.get("parity").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        ));
                        let _ = self._sniper_try_enter(&cand);
                    }
                    continue;
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
                                    o.insert(
                                        "entry_reason".to_string(),
                                        json!("SNIPER_ENTRY_FORCE"),
                                    );
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
            let active_entry_reason = self._active_entry_reason_or_default();
            let (take_profit_pct, stop_pct) =
                self._sniper_tp_sl_for_entry_reason(&active_entry_reason);
            let bypass_hold_for_tp = self._should_bypass_rtds_hold_for_take_profit(
                &active_entry_reason,
                cost,
                pnl_pct,
                take_profit_pct,
            );
            let hold_active =
                self._rtds_hold_till_resolution_active(&pos_side, seconds_left, "SNIPER_HOLD");
            if hold_active && !bypass_hold_for_tp {
                sniper_stop_breach_since = None;
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
                continue;
            }
            if hold_active && bypass_hold_for_tp {
                self._rtds_gate_log(
                    "hold_bypass_force_diff_tp",
                    &format!(
                        "[RTDS_HOLD] SNIPER_HOLD bypass: reason={} pnl_pct={:+.6} tp={:.6} t_left={:.2}s",
                        active_entry_reason, pnl_pct, take_profit_pct, seconds_left
                    ),
                );
            }
            if self._sniper_handle_post_hedge_policy(&pos, now, seconds_left) {
                sniper_stop_breach_since = None;
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
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
            if cost > 1e-12 && pnl_pct >= take_profit_pct {
                if self._sniper_try_exit(&pos, "TAKE_PROFIT") {
                    if repeat_mode {
                        continue;
                    }
                    break;
                }
                continue;
            }
            if let Some(stop_decision) = self._sniper_filters_eval_breakout_invalidation_stop(
                &pos_side,
                "SNIPER_POS",
                seconds_left,
            ) {
                if stop_decision.fired {
                    sniper_stop_breach_since = None;
                    let aid = pos
                        .get("asset_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let sl_mode = self._sniper_stop_loss_mode();
                    self._runtime_ts_set("__sniper_stop_loss_active", 1.0);
                    if self._sniper_try_exit(&pos, "STOP_LOSS") {
                        self._sniper_stop_loss_reset_failures(&aid);
                        if repeat_mode && !repeat_stop_after_sl {
                            continue;
                        }
                        break;
                    }
                    if !self._sniper_post_hedge_active() {
                        self._sniper_stop_loss_record_sell_failure(
                            &pos,
                            &aid,
                            &sl_mode,
                            "STOP_LOSS",
                            "stop_loss_breakout_loop",
                        );
                    }
                    continue;
                }
            }
            if cost > 1e-12 && stop_pct > 0.0 {
                let mut stop_loss_active_now = false;
                if pnl_pct <= -stop_pct {
                    let held_s = now - sniper_pos_open_ts.max(0.0);
                    if held_s >= env_float("SNIPER_MIN_HOLD_SECONDS", 0.0).max(0.0) {
                        if sniper_stop_breach_since.is_none() {
                            sniper_stop_breach_since = Some(now);
                        }
                        if now - sniper_stop_breach_since.unwrap_or(now)
                            >= env_float("SNIPER_STOP_CONFIRM_SECONDS", 0.0).max(0.0)
                        {
                            stop_loss_active_now = true;
                            let aid = pos
                                .get("asset_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let sl_mode = self._sniper_stop_loss_mode();
                            if self._sniper_try_exit(&pos, "STOP_LOSS") {
                                self._sniper_stop_loss_reset_failures(&aid);
                                if repeat_mode && !repeat_stop_after_sl {
                                    continue;
                                }
                                break;
                            }
                            if !self._sniper_post_hedge_active() {
                                self._sniper_stop_loss_record_sell_failure(
                                    &pos,
                                    &aid,
                                    &sl_mode,
                                    "STOP_LOSS",
                                    "stop_loss_loop",
                                );
                            }
                            self._runtime_ts_set(
                                "__sniper_stop_loss_active",
                                if stop_loss_active_now { 1.0 } else { 0.0 },
                            );
                            continue;
                        }
                    }
                } else {
                    sniper_stop_breach_since = None;
                    let aid = pos
                        .get("asset_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    self._sniper_stop_loss_reset_failures(&aid);
                }
                self._runtime_ts_set(
                    "__sniper_stop_loss_active",
                    if stop_loss_active_now { 1.0 } else { 0.0 },
                );
            } else {
                self._runtime_ts_set("__sniper_stop_loss_active", 0.0);
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
        self._sniper_filters_save_state(true);
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
            entry_time_iso: self
                .first_entry_fill_iso
                .lock()
                .ok()
                .and_then(|v| v.clone()),
            entry_reason: self.first_entry_reason.lock().ok().and_then(|v| v.clone()),
            stop_loss_category: self.stop_loss_category.lock().ok().and_then(|v| v.clone()),
            exit_reason: self._get_exit_reason(),
            fill_count: state.seen_trade_keys.len(),
        }
    }

    pub fn trade_decision_snapshot(&self) -> Option<TradeDecisionUpsert> {
        self.sniper_trade_decision
            .lock()
            .ok()
            .and_then(|v| v.clone())
            .map(|v| v.data)
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
        self._maker_ladder_cancel_all("cancel_all_exchange");
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

#[cfg(test)]
mod tests {
    use super::{
        pair_base_early_risk_exit_lead_seconds, pair_base_phase_without_recovery,
        pair_base_remaining_gap, pair_base_should_force_recovery, pair_base_should_latch_risk_exit,
        pair_submit_tracks_taker_fallback, MakerExecCandidate, MakerExecLedger,
        MakerExecRecord, MakerHedgeCapBot, PairBasePhaseState,
    };

    #[test]
    fn maker_payoff_envelope_math() {
        let (downside, upside, skew) = MakerHedgeCapBot::_maker_payoff_envelope(120.0, 80.0, 150.0);
        assert!((downside + 70.0).abs() < 1e-9);
        assert!((upside - (-30.0)).abs() < 1e-9);
        assert!((skew - 1.5).abs() < 1e-9);
    }

    #[test]
    fn maker_fee_formula_peaks_near_mid() {
        let qty = 10.0;
        let low = MakerHedgeCapBot::_maker_poly_fee_formula(qty, 0.1, 0.25, 2.0, 0.0, false, true);
        let mid = MakerHedgeCapBot::_maker_poly_fee_formula(qty, 0.5, 0.25, 2.0, 0.0, false, true);
        let high = MakerHedgeCapBot::_maker_poly_fee_formula(qty, 0.9, 0.25, 2.0, 0.0, false, true);
        assert!(mid > low);
        assert!(mid > high);
    }

    #[test]
    fn maker_fee_formula_maker_path_is_zero_cost() {
        let fee =
            MakerHedgeCapBot::_maker_poly_fee_formula(20.0, 0.52, 0.25, 2.0, 100.0, true, true);
        assert_eq!(fee, 0.0);
    }

    #[test]
    fn maker_bucket_helpers() {
        assert_eq!(MakerHedgeCapBot::_maker_price_bucket(0.0), "NA");
        assert_eq!(MakerHedgeCapBot::_maker_price_bucket(0.19), "LE_020");
        assert_eq!(MakerHedgeCapBot::_maker_price_bucket(0.30), "020_035");
        assert_eq!(MakerHedgeCapBot::_maker_price_bucket(0.55), "035_065");
        assert_eq!(MakerHedgeCapBot::_maker_price_bucket(0.77), "GT_065");
        assert_eq!(MakerHedgeCapBot::_maker_clip_bucket(0.0), "NA");
        assert_eq!(MakerHedgeCapBot::_maker_clip_bucket(10.0), "SMALL");
        assert_eq!(MakerHedgeCapBot::_maker_clip_bucket(20.0), "MID");
        assert_eq!(MakerHedgeCapBot::_maker_clip_bucket(45.0), "LARGE");
    }

    #[test]
    fn maker_exec_aliases_prefer_tx_then_trade_then_match() {
        let candidate = MakerExecCandidate {
            order_id: "oid-1".to_string(),
            asset_id: "asset-1".to_string(),
            side: "BUY".to_string(),
            qty: 5.0,
            price: 0.38,
            tx_hash: Some("0xtx".to_string()),
            trade_id: Some("trade-1".to_string()),
            taker_order_id: Some("taker-1".to_string()),
            match_time: Some("1772749249".to_string()),
        };

        let aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&candidate);
        assert_eq!(aliases.len(), 3);
        assert_eq!(aliases[0], "maker_tx:oid-1:0xtx:5.00000000:0.38000000");
        assert_eq!(aliases[1], "maker_trade:oid-1:trade-1");
        assert_eq!(
            aliases[2],
            "maker_match:oid-1:taker-1:1772749249:5.00000000:0.38000000"
        );
    }

    #[test]
    fn maker_exec_alias_enrichment_resolves_to_existing_trade_canonical() {
        let trade_only = MakerExecCandidate {
            order_id: "oid-1".to_string(),
            asset_id: "asset-1".to_string(),
            side: "BUY".to_string(),
            qty: 5.0,
            price: 0.38,
            tx_hash: None,
            trade_id: Some("trade-1".to_string()),
            taker_order_id: None,
            match_time: None,
        };
        let enriched = MakerExecCandidate {
            tx_hash: Some("0xtx".to_string()),
            taker_order_id: Some("taker-1".to_string()),
            match_time: Some("1772749249".to_string()),
            ..trade_only.clone()
        };

        let trade_aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&trade_only);
        let canonical = trade_aliases[0].clone();
        let mut ledger = MakerExecLedger::default();
        ledger.records.insert(
            canonical.clone(),
            MakerExecRecord {
                canonical_id: canonical.clone(),
                order_id: trade_only.order_id.clone(),
                qty: trade_only.qty,
                price: trade_only.price,
                asset_id: trade_only.asset_id.clone(),
                side: trade_only.side.clone(),
                aliases: Vec::new(),
                applied_ts: 0.0,
            },
        );
        MakerHedgeCapBot::_maker_exec_attach_aliases(&mut ledger, &canonical, &trade_aliases);

        let enriched_aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&enriched);
        let resolved = enriched_aliases
            .iter()
            .find_map(|alias| ledger.alias_to_canonical.get(alias).cloned());
        assert_eq!(resolved.as_deref(), Some(canonical.as_str()));
        assert!(MakerHedgeCapBot::_maker_exec_record_matches(
            ledger.records.get(&canonical).unwrap(),
            &enriched
        ));
    }

    #[test]
    fn maker_exec_alias_enrichment_resolves_to_existing_match_canonical() {
        let match_only = MakerExecCandidate {
            order_id: "oid-2".to_string(),
            asset_id: "asset-2".to_string(),
            side: "SELL".to_string(),
            qty: 2.5,
            price: 0.61,
            tx_hash: None,
            trade_id: None,
            taker_order_id: Some("taker-2".to_string()),
            match_time: Some("1772749257".to_string()),
        };
        let enriched = MakerExecCandidate {
            tx_hash: Some("0xtx-2".to_string()),
            trade_id: Some("trade-2".to_string()),
            ..match_only.clone()
        };

        let match_aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&match_only);
        let canonical = match_aliases[0].clone();
        let mut ledger = MakerExecLedger::default();
        ledger.records.insert(
            canonical.clone(),
            MakerExecRecord {
                canonical_id: canonical.clone(),
                order_id: match_only.order_id.clone(),
                qty: match_only.qty,
                price: match_only.price,
                asset_id: match_only.asset_id.clone(),
                side: match_only.side.clone(),
                aliases: Vec::new(),
                applied_ts: 0.0,
            },
        );
        MakerHedgeCapBot::_maker_exec_attach_aliases(&mut ledger, &canonical, &match_aliases);

        let enriched_aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&enriched);
        let resolved = enriched_aliases
            .iter()
            .find_map(|alias| ledger.alias_to_canonical.get(alias).cloned());
        assert_eq!(resolved.as_deref(), Some(canonical.as_str()));
        assert!(MakerHedgeCapBot::_maker_exec_record_matches(
            ledger.records.get(&canonical).unwrap(),
            &enriched
        ));
    }

    #[test]
    fn maker_projected_gap_allows_light_side_buy_that_stays_below_enter() {
        let projected = MakerHedgeCapBot::_maker_projected_gap_from_inventory(
            25.08,
            24.55,
            0.0,
            0.0,
            "NO",
            5.0,
        );
        assert!((projected - 4.47).abs() < 1e-6);
    }

    #[test]
    fn maker_projected_gap_blocks_same_side_buy_that_would_reopen_recovery() {
        let projected = MakerHedgeCapBot::_maker_projected_gap_from_inventory(
            25.08,
            29.55,
            0.0,
            0.0,
            "NO",
            5.0,
        );
        assert!((projected - 9.47).abs() < 1e-6);
    }

    #[test]
    fn maker_projected_gap_includes_unsettled_buy_risk() {
        let projected = MakerHedgeCapBot::_maker_projected_gap_from_inventory(
            25.0,
            22.0,
            0.0,
            4.0,
            "NO",
            5.0,
        );
        assert!((projected - 6.0).abs() < 1e-6);
    }

    #[test]
    fn pair_base_remaining_gap_respects_live_light_risk() {
        assert!((pair_base_remaining_gap(10.0, 0.0) - 10.0).abs() < 1e-9);
        assert!((pair_base_remaining_gap(10.0, 8.0) - 2.0).abs() < 1e-9);
        assert!((pair_base_remaining_gap(10.0, 10.0) - 0.0).abs() < 1e-9);
        assert!((pair_base_remaining_gap(10.0, 12.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn pair_base_phase_stays_resting_while_pair_orders_are_live() {
        assert_eq!(
            pair_base_phase_without_recovery(false, 0.0, 1.0, true),
            Some(PairBasePhaseState::PairResting)
        );
        assert_eq!(
            pair_base_phase_without_recovery(true, 0.0, 1.0, true),
            Some(PairBasePhaseState::PairResting)
        );
    }

    #[test]
    fn pair_base_phase_only_balances_or_flats_when_no_pair_orders_are_live() {
        assert_eq!(
            pair_base_phase_without_recovery(true, 0.0, 1.0, false),
            Some(PairBasePhaseState::Balanced)
        );
        assert_eq!(
            pair_base_phase_without_recovery(false, 0.0, 1.0, false),
            Some(PairBasePhaseState::Flat)
        );
        assert_eq!(pair_base_phase_without_recovery(true, 5.0, 1.0, false), None);
    }

    #[test]
    fn pair_submit_does_not_track_gtc_orders_as_taker_fallback() {
        assert!(!pair_submit_tracks_taker_fallback("GTC"));
        assert!(!pair_submit_tracks_taker_fallback(" gtc "));
    }

    #[test]
    fn pair_submit_tracks_non_gtc_orders_as_taker_fallback() {
        assert!(pair_submit_tracks_taker_fallback("FAK"));
        assert!(pair_submit_tracks_taker_fallback("FOK"));
    }

    #[test]
    fn pair_base_forces_recovery_when_merge_pending_gap_remains() {
        assert!(pair_base_should_force_recovery(
            PairBasePhaseState::MergePending,
            4.99,
            1.0,
            true
        ));
    }

    #[test]
    fn pair_base_forces_recovery_when_pair_resting_light_leg_is_untrusted() {
        assert!(pair_base_should_force_recovery(
            PairBasePhaseState::PairResting,
            5.0,
            1.0,
            false
        ));
        assert!(!pair_base_should_force_recovery(
            PairBasePhaseState::PairResting,
            5.0,
            1.0,
            true
        ));
    }

    #[test]
    fn pair_base_early_risk_exit_lead_is_ahead_of_stop_buffer() {
        assert!((pair_base_early_risk_exit_lead_seconds(15.0) - 30.0).abs() < 1e-9);
        assert!((pair_base_early_risk_exit_lead_seconds(8.0) - 30.0).abs() < 1e-9);
        assert!((pair_base_early_risk_exit_lead_seconds(20.0) - 40.0).abs() < 1e-9);
    }

    #[test]
    fn pair_base_near_expiry_taker_override_is_reason_and_time_gated() {
        assert!(crate::bot::pair_base_near_expiry_taker_override_active(
            "pair_base_near_expiry",
            9.5,
            10.0,
            0.85
        ));
        assert!(!crate::bot::pair_base_near_expiry_taker_override_active(
            "pair_base_near_expiry",
            12.0,
            10.0,
            0.85
        ));
        assert!(!crate::bot::pair_base_near_expiry_taker_override_active(
            "pair_base_max_loss",
            9.5,
            10.0,
            0.85
        ));
    }

    #[test]
    fn pair_base_near_expiry_taker_override_raises_cap() {
        assert!((crate::bot::pair_base_effective_taker_cap(0.59, 0.85) - 0.85).abs() < 1e-9);
        assert!((crate::bot::pair_base_effective_taker_cap(0.90, 0.85) - 0.90).abs() < 1e-9);
    }

    #[test]
    fn pair_base_near_expiry_risk_exit_latches_terminal_mode() {
        assert!(pair_base_should_latch_risk_exit("near_expiry"));
        assert!(pair_base_should_latch_risk_exit("latched"));
        assert!(!pair_base_should_latch_risk_exit("max_loss"));
    }

    #[test]
    fn pair_base_merge_requote_requires_positive_worst_case_pnl() {
        assert!(crate::bot::pair_base_allows_merge_requote(0.0001));
        assert!(!crate::bot::pair_base_allows_merge_requote(0.0));
        assert!(!crate::bot::pair_base_allows_merge_requote(-0.0001));
    }

    #[test]
    fn pair_base_sub_min_recovery_uses_exact_orders() {
        assert!(crate::bot::pair_base_recovery_uses_exact_order(
            "PAIR_BASE_RECOVERY",
            2.29,
            5.0,
        ));
        assert!(!crate::bot::pair_base_recovery_uses_exact_order(
            "PAIR_BASE_RECOVERY",
            5.0,
            5.0,
        ));
        assert!(!crate::bot::pair_base_recovery_uses_exact_order(
            "PAIR_BASE_GTC_YES",
            2.29,
            5.0,
        ));
    }
}

