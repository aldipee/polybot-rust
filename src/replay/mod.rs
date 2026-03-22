use crate::bot::{BotRuntimeMarketDataStaleStage, MakerHedgeCapBot};
use crate::config::{
    resolve_versioned_config_bundle_from_snapshot, BotConfig, BotExecutionConfigSnapshot,
    BotRuntimeConfigSnapshotV1, ResolvedVersionedConfigBundle, VersionedConfigSnapshotV1,
};
use crate::db::{TradeDecisionEventInsert, TradeRuntimeEventInsert};
use crate::helpers::{save_daily_liquidity_state, save_state};
use crate::latency_log::JsonlFileService;
use crate::logging::{setup_item_logger, LogLike};
use crate::rtds::ResolutionSnapshot;
use anyhow::{anyhow, Context, Result};
use chrono::{TimeZone, Utc};
use chrono_tz::Asia::Jakarta;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

thread_local! {
    static REPLAY_RUNTIME: RefCell<ReplayRuntimeState> = RefCell::new(ReplayRuntimeState::default());
}

#[derive(Debug, Clone, Default)]
struct ReplayRuntimeState {
    active: bool,
    now_ns: i64,
    uuid_counter: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReplayManifest {
    pub schema_version: String,
    pub market_slug: String,
    pub configured_order_mode: String,
    pub state_file_name: String,
    #[serde(default)]
    pub trade_id: Option<String>,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, Option<String>>,
    pub condition_id: Option<String>,
    pub yes_asset_id: Option<String>,
    pub no_asset_id: Option<String>,
    #[serde(default)]
    pub market_start_ts: Option<i64>,
    #[serde(default)]
    pub market_expiry_ts: Option<i64>,
    pub start_ts_ns: i64,
    pub end_ts_ns: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReplayResolvedConfig {
    pub snapshot: VersionedConfigSnapshotV1,
    #[serde(default)]
    pub effective_bot_config: Option<BotConfig>,
    #[serde(default)]
    pub runtime_config: Option<BotRuntimeConfigSnapshotV1>,
    #[serde(default)]
    pub execution_config: Option<BotExecutionConfigSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReplayEventRecord {
    pub ts_ns: i64,
    pub seq: u64,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug)]
pub(crate) struct ReplayRecorder {
    pub root_dir: PathBuf,
    manifest_path: PathBuf,
    manifest: Mutex<ReplayManifest>,
    events_writer: Arc<JsonlFileService>,
    decisions_writer: Arc<JsonlFileService>,
    runtime_events_writer: Arc<JsonlFileService>,
    seq: AtomicU64,
}

#[derive(Debug, Clone)]
pub(crate) struct ReplayOracle {
    pub runtime_events: Vec<TradeRuntimeEventInsert>,
    pub decisions: Vec<TradeDecisionEventInsert>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReplayOrderAck {
    pub order_id: String,
    pub submit_timing: Option<Value>,
    pub asset_id: Option<String>,
    pub side: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ReplayScenario {
    pub root_dir: PathBuf,
    pub manifest: ReplayManifest,
    pub resolved: ResolvedVersionedConfigBundle,
    pub events: Vec<ReplayEventRecord>,
    pub oracle: ReplayOracle,
    pub resolution_snapshot: Option<ResolutionSnapshot>,
}

pub(crate) fn replay_capture_enabled() -> bool {
    matches!(
        std::env::var("REPLAY_CAPTURE_ENABLED")
            .ok()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) fn replay_capture_dir() -> Option<PathBuf> {
    let raw = std::env::var("REPLAY_CAPTURE_DIR").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

pub(crate) fn replay_runtime_active() -> bool {
    REPLAY_RUNTIME.with(|state| state.borrow().active)
}

pub(crate) fn replay_runtime_activate(start_ts_ns: i64) {
    REPLAY_RUNTIME.with(|state| {
        let mut state = state.borrow_mut();
        state.active = true;
        state.now_ns = start_ts_ns.max(0);
        state.uuid_counter = 0;
    });
}

pub(crate) fn replay_runtime_deactivate() {
    REPLAY_RUNTIME.with(|state| {
        *state.borrow_mut() = ReplayRuntimeState::default();
    });
}

pub(crate) fn replay_runtime_set_now_ns(now_ns: i64) {
    REPLAY_RUNTIME.with(|state| {
        let mut state = state.borrow_mut();
        if state.active {
            state.now_ns = now_ns.max(0);
        }
    });
}

pub(crate) fn replay_runtime_now_ns() -> Option<i64> {
    REPLAY_RUNTIME.with(|state| {
        let state = state.borrow();
        state.active.then_some(state.now_ns)
    })
}

pub(crate) fn replay_runtime_new_uuid() -> Option<String> {
    REPLAY_RUNTIME.with(|state| {
        let mut state = state.borrow_mut();
        if !state.active {
            return None;
        }
        state.uuid_counter = state.uuid_counter.saturating_add(1);
        Some(format!(
            "00000000-0000-0000-0000-{:012x}",
            state.uuid_counter
        ))
    })
}

pub(crate) fn runtime_now_ns() -> i64 {
    replay_runtime_now_ns().unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0)
    })
}

pub(crate) fn runtime_now_ts_f64() -> f64 {
    (runtime_now_ns() as f64) / 1_000_000_000.0
}

pub(crate) fn runtime_now_ts() -> i64 {
    runtime_now_ts_f64().floor() as i64
}

pub(crate) fn runtime_now_iso_jakarta() -> String {
    let now_ns = runtime_now_ns();
    let sec = now_ns.div_euclid(1_000_000_000);
    let nsec = now_ns.rem_euclid(1_000_000_000) as u32;
    Utc.timestamp_opt(sec, nsec)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&Jakarta)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(crate) fn runtime_utc_day_key() -> String {
    let ts = runtime_now_ts_f64();
    let sec = ts.floor() as i64;
    let nsec = ((ts - sec as f64).max(0.0) * 1_000_000_000.0) as u32;
    Utc.timestamp_opt(sec, nsec)
        .single()
        .unwrap_or_else(Utc::now)
        .format("%Y-%m-%d")
        .to_string()
}

fn load_json_lines<T>(path: &Path) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed reading replay jsonl {}", path.display()))?;
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = serde_json::from_str::<T>(trimmed).with_context(|| {
            format!(
                "failed parsing replay jsonl {} line {}",
                path.display(),
                idx + 1
            )
        })?;
        out.push(value);
    }
    Ok(out)
}

fn replay_behavior_env_keys() -> &'static [&'static str] {
    &[
        "REQUIRE_USER_WS_CONNECTED",
        "RECONCILE_EXCHANGE_ORDERS",
        "RECONCILE_USE_DATA_API",
        "MISMATCH_RECONCILE_FROM_BALANCE",
        "RECONCILE_MIN_INTERVAL_SECONDS",
        "RECONCILE_CONFIRM_DELAY_SECONDS",
        "RECONCILE_NEVER_ZERO_WITHOUT_CONFIRM",
        "AUTO_DETECT_MARKET_PARAMS",
        "MAKER_SINGLE_INFLIGHT_PER_SIDE",
        "MAKER_SUBMIT_PENDING_TTL_SECONDS",
        "MAKER_CANCEL_PENDING_TTL_SECONDS",
        "MAKER_WORKING_MISSING_TTL_SECONDS",
        "MAKER_SUBMIT_REJECT_COOLDOWN_SECONDS",
        "MAKER_REPLACE_MIN_INTERVAL_SECONDS",
        "MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS",
        "MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET",
        "PAIR_ARB_IMBALANCE_RELEASE_SHARES",
        "STALE_SECONDS",
        "REPLACE_IF_PRICE_MOVES_TICKS",
        "MAKER_UNDERDOG_FLOOR_PRICE",
        "MAKER_HEDGE_FLOOR_PRICE",
        "WS_PING_INTERVAL",
        "WS_IO_TIMEOUT_SECONDS",
        "DEBUG_THROTTLE_SECONDS",
        "ORDERBOOK_HTTP_TIMEOUT",
        "BOOK_CACHE_TTL_SECONDS",
        "UNWIND_CHUNK_SHARES",
        "UNWIND_MAX_PASSES",
        "UNWIND_WAIT_AFTER_ORDER_SECONDS",
        "MAKER_EXPOSURE_UNWIND_SLIPPAGE_TICKS",
        "UNWIND_DEPTH_GATE_ENABLED",
        "DEPTH_GATE_LEVELS",
        "DEPTH_GATE_MAX_AGE_SECONDS",
        "MIN_SHARES",
        "CLIP_SHARES",
        "MAX_TOTAL_COST",
        "RESERVE_USD",
        "DRY_RUN",
        "LOG_EVERY_SECONDS",
        "MARKET_DATA_STALE_ADD_BLOCK_SECONDS",
        "MARKET_DATA_STALE_HARD_PAUSE_SECONDS",
        "STOP_BUFFER_SECONDS",
        "ENTRY_EDGE_TICKS",
        "HEDGE_BUFFER_TICKS",
        "MAKER_BUFFER_TICKS",
        "IMPROVE_BID_TICKS",
        "MAKER_SKEW_PEAK_CLIP_MULT",
    ]
}

fn replay_capture_env_overrides() -> BTreeMap<String, Option<String>> {
    let mut out = BTreeMap::new();
    for key in replay_behavior_env_keys() {
        out.insert((*key).to_string(), std::env::var(key).ok());
    }
    out
}

fn replay_apply_env_overrides(
    overrides: &BTreeMap<String, Option<String>>,
) -> BTreeMap<String, Option<String>> {
    let mut previous = BTreeMap::new();
    for key in replay_behavior_env_keys() {
        let key_string = (*key).to_string();
        previous.insert(key_string.clone(), std::env::var(key).ok());
        match overrides.get(*key).cloned().flatten() {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
    previous
}

fn replay_restore_env_overrides(previous: &BTreeMap<String, Option<String>>) {
    for key in replay_behavior_env_keys() {
        match previous.get(*key).cloned().flatten() {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

fn sanitize_slug(value: &str) -> String {
    let mut out = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "scenario".to_string()
    } else {
        trimmed.to_string()
    }
}

fn copy_file_if_exists(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst).with_context(|| {
        format!(
            "failed copying replay initial file {} -> {}",
            src.display(),
            dst.display()
        )
    })?;
    Ok(())
}

fn compare_optional_jsonl_oracle(
    expected_path: &Path,
    actual_path: &Path,
    label: &str,
) -> Result<()> {
    if !expected_path.exists() {
        return Ok(());
    }
    let expected = load_json_lines::<Value>(expected_path)?;
    let actual = load_json_lines::<Value>(actual_path)?;
    if expected == actual {
        return Ok(());
    }
    Err(anyhow!(
        "replay oracle mismatch for {label}: expected {} rows, got {}",
        expected.len(),
        actual.len()
    ))
}

fn compare_optional_json_oracle(
    expected_path: &Path,
    actual_path: &Path,
    label: &str,
) -> Result<()> {
    if !expected_path.exists() {
        return Ok(());
    }
    let expected = serde_json::from_slice::<Value>(&fs::read(expected_path)?)
        .with_context(|| format!("failed parsing {}", expected_path.display()))?;
    let actual = serde_json::from_slice::<Value>(&fs::read(actual_path)?)
        .with_context(|| format!("failed parsing {}", actual_path.display()))?;
    if expected == actual {
        return Ok(());
    }
    Err(anyhow!("replay oracle mismatch for {label}"))
}

fn load_optional_json<T>(path: &Path) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read(path).with_context(|| format!("failed reading {}", path.display()))?;
    let value = serde_json::from_slice::<T>(&raw)
        .with_context(|| format!("failed parsing {}", path.display()))?;
    Ok(Some(value))
}

fn write_optional_resolution_snapshot(
    root_dir: &Path,
    snapshot: Option<&ResolutionSnapshot>,
) -> Result<()> {
    let path = root_dir.join("resolution_snapshot.json");
    if let Some(snapshot) = snapshot {
        fs::write(&path, serde_json::to_vec_pretty(snapshot)?)
            .with_context(|| format!("failed writing {}", path.display()))?;
    }
    Ok(())
}

fn replay_extract_order_id_from_user_order(payload: &Value) -> Option<String> {
    payload
        .get("order_id")
        .or_else(|| payload.get("id"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn replay_extract_asset_id(payload: &Value) -> Option<String> {
    payload
        .get("asset_id")
        .or_else(|| payload.get("token_id"))
        .or_else(|| payload.get("assetId"))
        .or_else(|| payload.get("tokenId"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn replay_extract_side(payload: &Value) -> Option<String> {
    payload
        .get("side")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| matches!(value.as_str(), "BUY" | "SELL"))
}

fn replay_extract_taker_order_id_from_user_trade(payload: &Value) -> Option<String> {
    payload
        .get("taker_order_id")
        .or_else(|| payload.get("takerOrderId"))
        .or_else(|| payload.get("taker_orderId"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn replay_push_ack_if_new(
    out: &mut VecDeque<ReplayOrderAck>,
    seen: &mut HashSet<String>,
    order_id: String,
    submit_timing: Option<Value>,
    asset_id: Option<String>,
    side: Option<String>,
) {
    if seen.insert(order_id.clone()) {
        out.push_back(ReplayOrderAck {
            order_id,
            submit_timing,
            asset_id,
            side,
        });
    }
}

pub(crate) fn replay_take_order_ack(
    queue: &mut VecDeque<ReplayOrderAck>,
    asset_id: &str,
    side: &str,
) -> Option<ReplayOrderAck> {
    let expected_asset = asset_id.trim();
    let expected_side = side.trim().to_ascii_uppercase();
    if !expected_asset.is_empty() && matches!(expected_side.as_str(), "BUY" | "SELL") {
        if let Some(pos) = queue.iter().position(|ack| {
            ack.asset_id.as_deref() == Some(expected_asset)
                && ack.side.as_deref() == Some(expected_side.as_str())
        }) {
            return queue.remove(pos);
        }
    }
    queue.pop_front()
}

fn replay_payout_from_resolution_diff(diff_price: f64, q_yes: f64, q_no: f64) -> (f64, f64) {
    if diff_price >= 0.0 {
        (q_yes.max(0.0), 0.0)
    } else {
        (0.0, q_no.max(0.0))
    }
}

fn replay_realized_lp_from_resolution_record(
    snapshot: &crate::rtds::ResolutionSnapshot,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
) -> Option<f64> {
    if snapshot.source_ts_ms + 1 < snapshot.resolution_ts_ms {
        return None;
    }
    let diff_price = snapshot
        .diff_vs_price_to_beat
        .or_else(|| {
            snapshot
                .price_to_beat
                .map(|ptb| snapshot.resolution_price - ptb)
        })
        .filter(|v| v.is_finite())?;
    let (yes_payout, no_payout) = replay_payout_from_resolution_diff(diff_price, q_yes, q_no);
    Some(yes_payout + no_payout - total_cost)
}

fn replay_emit_post_run_settlement_events(
    bot: &MakerHedgeCapBot,
    run_reason: &str,
    resolution_snapshot: Option<&ResolutionSnapshot>,
) {
    let metrics = bot.trade_metrics_snapshot();
    let has_trade_activity = metrics.fill_count > 0
        || metrics.total_cost > 1e-9
        || metrics.q_yes > 1e-9
        || metrics.q_no > 1e-9;
    if !has_trade_activity {
        return;
    }

    let trade_id = bot._replay_trade_id();
    let raw_exit_reason = if metrics.exit_reason.trim().is_empty()
        || metrics.exit_reason.eq_ignore_ascii_case("RUNNING")
    {
        run_reason.to_string()
    } else {
        metrics.exit_reason.clone()
    };
    let end_trade_iso = runtime_now_iso_jakarta();

    bot._audit_record_settlement_event(
        "await_settlement_handoff",
        json!({
            "trade_id": trade_id,
            "reason_code": "await_settlement_handoff",
            "raw_exit_reason": raw_exit_reason,
            "end_trade_iso": end_trade_iso,
            "q_yes": metrics.q_yes,
            "q_no": metrics.q_no,
            "total_cost": metrics.total_cost,
            "cpp": metrics.cpp,
        }),
    );

    if let Some(snapshot) = resolution_snapshot {
        if let Some(realized_lp) = replay_realized_lp_from_resolution_record(
            snapshot,
            metrics.q_yes,
            metrics.q_no,
            metrics.total_cost,
        ) {
            bot._audit_record_settlement_event(
                "settled",
                json!({
                    "trade_id": trade_id,
                    "reason_code": "settled",
                    "resolved_lp": realized_lp,
                    "total_cost": metrics.total_cost,
                    "cpp": metrics.cpp,
                    "q_yes": metrics.q_yes,
                    "q_no": metrics.q_no,
                    "snapshot_market_slug": snapshot.market_slug,
                    "snapshot_resolution_price": snapshot.resolution_price,
                }),
            );
            return;
        }
    }

    bot._audit_record_settlement_event(
        "resolution_snapshot_unavailable",
        json!({
            "trade_id": trade_id,
            "reason_code": "resolution_snapshot_unavailable",
            "pair_id": metrics.pair_id,
            "q_yes": metrics.q_yes,
            "q_no": metrics.q_no,
            "total_cost": metrics.total_cost,
        }),
    );
}

impl ReplayRecorder {
    pub(crate) fn from_env(
        bot: &MakerHedgeCapBot,
        bundle: &ResolvedVersionedConfigBundle,
    ) -> Result<Option<Arc<Self>>> {
        if !replay_capture_enabled() {
            return Ok(None);
        }
        let Some(base_dir) = replay_capture_dir() else {
            return Ok(None);
        };
        fs::create_dir_all(&base_dir)?;
        let capture_ts_ns = runtime_now_ns().max(0);
        let scenario_dir = base_dir.join(format!(
            "{}_{}",
            sanitize_slug(bot.market_slug.as_str()),
            capture_ts_ns
        ));
        fs::create_dir_all(&scenario_dir)?;
        let manifest = ReplayManifest {
            schema_version: "replay_v1".to_string(),
            market_slug: bot.market_slug.clone(),
            configured_order_mode: bot.configured_order_mode.clone(),
            state_file_name: bot
                .state_file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("maker_hedgecap_state.json")
                .to_string(),
            trade_id: bot._active_trade_id_opt(),
            env_overrides: replay_capture_env_overrides(),
            condition_id: bot.pair_identity().condition_id.clone(),
            yes_asset_id: bot.pair_identity().yes_asset_id.clone(),
            no_asset_id: bot.pair_identity().no_asset_id.clone(),
            market_start_ts: Some(bot.start_ts),
            market_expiry_ts: Some(bot.expiry_ts),
            start_ts_ns: capture_ts_ns,
            end_ts_ns: None,
        };
        let manifest_path = scenario_dir.join("manifest.json");
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)
            .with_context(|| format!("failed writing {}", manifest_path.display()))?;
        let resolved_path = scenario_dir.join("resolved_config.json");
        let resolved = ReplayResolvedConfig {
            snapshot: bundle.snapshot.clone(),
            effective_bot_config: Some(bundle.effective_bot_config.clone()),
            runtime_config: Some(BotRuntimeConfigSnapshotV1::from(&bundle.runtime_config)),
            execution_config: Some(bundle.execution_config.clone()),
        };
        fs::write(&resolved_path, serde_json::to_vec_pretty(&resolved)?)
            .with_context(|| format!("failed writing {}", resolved_path.display()))?;

        let initial_workdir = scenario_dir.join("initial_state").join("workdir");
        let initial_shared = scenario_dir.join("initial_state").join("shared-state");
        fs::create_dir_all(&initial_workdir)?;
        fs::create_dir_all(&initial_shared)?;
        if let Ok(mut state) = bot.state.lock() {
            save_state(
                &initial_workdir.join(manifest.state_file_name.as_str()),
                &mut state,
            )?;
        }
        if let Ok(mut daily) = bot.daily_liquidity_state.lock() {
            let file_name = bot
                .daily_liquidity_state_file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("maker_hedgecap_daily_liquidity.json");
            save_daily_liquidity_state(&initial_shared.join(file_name), &mut daily)?;
        }
        let pending_taker_path = MakerHedgeCapBot::pending_taker_state_file_for_wallet(
            bot.wallet_address.as_str(),
            bot.configured_order_mode.as_str(),
        );
        let gross_state_path = MakerHedgeCapBot::gross_exposure_state_file_for_wallet(
            bot.wallet_address.as_str(),
            bot.configured_order_mode.as_str(),
        );
        copy_file_if_exists(
            &pending_taker_path,
            &initial_shared.join(
                pending_taker_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("maker_hedgecap_pending_takers.json"),
            ),
        )?;
        copy_file_if_exists(
            &gross_state_path,
            &initial_shared.join(
                gross_state_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("maker_hedgecap_gross_exposure.json"),
            ),
        )?;

        Ok(Some(Arc::new(Self {
            root_dir: scenario_dir.clone(),
            manifest_path,
            manifest: Mutex::new(manifest),
            events_writer: Arc::new(JsonlFileService::new(
                scenario_dir
                    .join("events.jsonl")
                    .to_string_lossy()
                    .to_string(),
                true,
            )),
            decisions_writer: Arc::new(JsonlFileService::new(
                scenario_dir
                    .join("oracle_decisions.jsonl")
                    .to_string_lossy()
                    .to_string(),
                true,
            )),
            runtime_events_writer: Arc::new(JsonlFileService::new(
                scenario_dir
                    .join("oracle_runtime_events.jsonl")
                    .to_string_lossy()
                    .to_string(),
                true,
            )),
            seq: AtomicU64::new(0),
        })))
    }

    pub(crate) fn record_event(&self, ts_ns: i64, kind: &str, payload: Value) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        self.events_writer.append(&json!(ReplayEventRecord {
            ts_ns,
            seq,
            kind: kind.to_string(),
            payload,
        }));
    }

    pub(crate) fn record_runtime_event(&self, row: &TradeRuntimeEventInsert) {
        self.runtime_events_writer.append(&json!({
            "event_id": row.event_id,
            "trade_id": row.trade_id,
            "pair_id": row.pair_id,
            "market_slug": row.market_slug,
            "condition_id": row.condition_id,
            "yes_asset_id": row.yes_asset_id,
            "no_asset_id": row.no_asset_id,
            "config_version": row.config_version,
            "event_kind": row.event_kind,
            "event_ts": row.event_ts,
            "decision_event_id": row.decision_event_id,
            "order_id": row.order_id,
            "asset_id": row.asset_id,
            "side": row.side,
            "reason_code": row.reason_code,
            "payload_json": row.payload_json,
        }));
    }

    pub(crate) fn record_decision_event(&self, row: &TradeDecisionEventInsert) {
        self.decisions_writer.append(&json!({
            "decision_event_id": row.decision_event_id,
            "trade_id": row.trade_id,
            "pair_id": row.pair_id,
            "market_slug": row.market_slug,
            "condition_id": row.condition_id,
            "yes_asset_id": row.yes_asset_id,
            "no_asset_id": row.no_asset_id,
            "config_version": row.config_version,
            "decision_scope": row.decision_scope,
            "decision_ts": row.decision_ts,
            "phase": row.phase,
            "owner": row.owner,
            "approved": row.approved,
            "reason_code": row.reason_code,
            "submit_origin": row.submit_origin,
            "submit_side": row.submit_side,
            "payload_json": row.payload_json,
        }));
    }

    pub(crate) fn finalize(&self, bot: &MakerHedgeCapBot, exit_reason: &str) -> Result<()> {
        if let Ok(mut manifest) = self.manifest.lock() {
            manifest.trade_id = bot
                ._active_trade_id_opt()
                .or_else(|| manifest.trade_id.clone());
            manifest.end_ts_ns = Some(runtime_now_ns().max(manifest.start_ts_ns));
            fs::write(&self.manifest_path, serde_json::to_vec_pretty(&*manifest)?)
                .with_context(|| format!("failed writing {}", self.manifest_path.display()))?;
        }
        let final_state_path = self.root_dir.join("oracle_final_state.json");
        let state = bot
            .state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let daily = bot
            .daily_liquidity_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        let metrics = bot.trade_metrics_snapshot();
        let runtime = bot._replay_runtime_state_snapshot_json();
        fs::write(
            &final_state_path,
            serde_json::to_vec_pretty(&json!({
                "exit_reason": exit_reason,
                "state": state,
                "daily_liquidity_state": daily,
                "runtime_state": runtime,
                "trade_metrics": metrics,
            }))?,
        )
        .with_context(|| format!("failed writing {}", final_state_path.display()))?;
        if !bot._replay_mode_active() {
            let resolution_snapshot =
                crate::rtds::get_resolution_snapshot_for_market(bot.market_slug.as_str());
            write_optional_resolution_snapshot(&self.root_dir, resolution_snapshot.as_ref())?;
        }
        Ok(())
    }
}

impl ReplayScenario {
    pub(crate) fn load(root_dir: &Path) -> Result<Self> {
        let manifest_path = root_dir.join("manifest.json");
        let manifest = serde_json::from_slice::<ReplayManifest>(
            &fs::read(&manifest_path)
                .with_context(|| format!("failed reading {}", manifest_path.display()))?,
        )
        .with_context(|| format!("failed parsing {}", manifest_path.display()))?;
        let resolved_path = root_dir.join("resolved_config.json");
        let resolved = serde_json::from_slice::<ReplayResolvedConfig>(
            &fs::read(&resolved_path)
                .with_context(|| format!("failed reading {}", resolved_path.display()))?,
        )
        .with_context(|| format!("failed parsing {}", resolved_path.display()))?;
        let bundle =
            if let (Some(effective_bot_config), Some(runtime_config), Some(execution_config)) = (
                resolved.effective_bot_config.clone(),
                resolved.runtime_config.clone(),
                resolved.execution_config.clone(),
            ) {
                ResolvedVersionedConfigBundle {
                    snapshot: resolved.snapshot.clone(),
                    effective_bot_config,
                    runtime_config: runtime_config.to_runtime_config(),
                    execution_config,
                }
            } else {
                resolve_versioned_config_bundle_from_snapshot(resolved.snapshot.clone())?
            };
        let events_path = root_dir.join("events.jsonl");
        if !events_path.exists() {
            return Err(anyhow!(
                "required replay file missing: {}",
                events_path.display()
            ));
        }
        let events = load_json_lines::<ReplayEventRecord>(&events_path)?;
        let decisions = load_json_lines::<Value>(&root_dir.join("oracle_decisions.jsonl"))?
            .into_iter()
            .map(|value| serde_json::from_value::<TradeDecisionEventInsert>(value))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|err| anyhow!(err))?;
        let runtime_events =
            load_json_lines::<Value>(&root_dir.join("oracle_runtime_events.jsonl"))?
                .into_iter()
                .map(|value| serde_json::from_value::<TradeRuntimeEventInsert>(value))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|err| anyhow!(err))?;
        let resolution_snapshot =
            load_optional_json::<ResolutionSnapshot>(&root_dir.join("resolution_snapshot.json"))?;
        let mut previous: Option<(i64, u64)> = None;
        for event in &events {
            if let Some((prev_ts, prev_seq)) = previous {
                if (event.ts_ns, event.seq) <= (prev_ts, prev_seq) {
                    return Err(anyhow!(
                        "replay events must be strictly sorted by (ts_ns, seq)"
                    ));
                }
            }
            previous = Some((event.ts_ns, event.seq));
        }
        Ok(Self {
            root_dir: root_dir.to_path_buf(),
            manifest,
            resolved: bundle,
            events,
            oracle: ReplayOracle {
                runtime_events,
                decisions,
            },
            resolution_snapshot,
        })
    }

    pub(crate) fn replay_order_acks(&self) -> VecDeque<ReplayOrderAck> {
        let mut out = VecDeque::new();
        let mut seen = HashSet::new();
        for event in &self.oracle.runtime_events {
            if event.event_kind == "order_ack" {
                if let Some(order_id) = event.order_id.as_ref().filter(|value| !value.is_empty()) {
                    let submit_timing = serde_json::from_str::<Value>(&event.payload_json)
                        .ok()
                        .and_then(|payload| payload.get("meta_json").cloned());
                    replay_push_ack_if_new(
                        &mut out,
                        &mut seen,
                        order_id.clone(),
                        submit_timing,
                        event.asset_id.clone(),
                        event.side.clone(),
                    );
                }
            }
        }
        for event in &self.events {
            match event.kind.as_str() {
                "user_order" => {
                    if let Some(order_id) = replay_extract_order_id_from_user_order(&event.payload)
                    {
                        replay_push_ack_if_new(
                            &mut out,
                            &mut seen,
                            order_id,
                            None,
                            replay_extract_asset_id(&event.payload),
                            replay_extract_side(&event.payload),
                        );
                    }
                }
                "user_trade" => {
                    if let Some(order_id) =
                        replay_extract_taker_order_id_from_user_trade(&event.payload)
                    {
                        replay_push_ack_if_new(
                            &mut out,
                            &mut seen,
                            order_id,
                            None,
                            replay_extract_asset_id(&event.payload),
                            replay_extract_side(&event.payload),
                        );
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub(crate) fn replay_trade_id(&self) -> Option<String> {
        self.manifest
            .trade_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.oracle
                    .runtime_events
                    .iter()
                    .map(|row| row.trade_id.trim())
                    .find(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .or_else(|| {
                        self.oracle
                            .decisions
                            .iter()
                            .map(|row| row.trade_id.trim())
                            .find(|value| !value.is_empty())
                            .map(|value| value.to_string())
                    })
            })
    }
}

impl MakerHedgeCapBot {
    pub(crate) fn _replay_capture_event(&self, kind: &str, payload: Value) {
        let Some(recorder) = self.replay_recorder.as_ref() else {
            return;
        };
        recorder.record_event(runtime_now_ns(), kind, payload);
    }

    pub(crate) fn _init_replay_capture(
        &mut self,
        bundle: &ResolvedVersionedConfigBundle,
    ) -> Result<()> {
        self.replay_recorder = ReplayRecorder::from_env(self, bundle)?;
        Ok(())
    }

    pub fn finalize_replay_capture(&self, exit_reason: &str) {
        if let Some(recorder) = self.replay_recorder.as_ref() {
            let _ = recorder.finalize(self, exit_reason);
        }
    }
}

pub fn run_replay_scenario(root_dir: &Path) -> Result<()> {
    let scenario = ReplayScenario::load(root_dir)?;
    let logger: Arc<dyn LogLike> = setup_item_logger("replay");
    let temp_root = std::env::temp_dir().join(format!(
        "polybot_replay_{}_{}",
        sanitize_slug(scenario.manifest.market_slug.as_str()),
        runtime_now_ns().max(0)
    ));
    let prior_shared_state_dir = std::env::var("POLYBOT_SHARED_STATE_DIR").ok();
    let prior_capture_enabled = std::env::var("REPLAY_CAPTURE_ENABLED").ok();
    let prior_capture_dir = std::env::var("REPLAY_CAPTURE_DIR").ok();
    let prior_behavior_env = replay_capture_env_overrides();
    let prior_cwd = std::env::current_dir().ok();

    let result = (|| -> Result<()> {
        let workdir = temp_root.join("workdir");
        let shared = temp_root.join("shared-state");
        let capture_dir = temp_root.join("capture");
        fs::create_dir_all(&workdir)?;
        fs::create_dir_all(&shared)?;
        fs::create_dir_all(&capture_dir)?;
        let initial_root = scenario.root_dir.join("initial_state");
        let initial_workdir = initial_root.join("workdir");
        let initial_shared = initial_root.join("shared-state");
        if initial_workdir.exists() {
            for entry in fs::read_dir(&initial_workdir)? {
                let entry = entry?;
                copy_file_if_exists(&entry.path(), &workdir.join(entry.file_name()))?;
            }
        }
        if initial_shared.exists() {
            for entry in fs::read_dir(&initial_shared)? {
                let entry = entry?;
                copy_file_if_exists(&entry.path(), &shared.join(entry.file_name()))?;
            }
        }

        std::env::set_var(
            "POLYBOT_SHARED_STATE_DIR",
            shared.to_string_lossy().to_string(),
        );
        std::env::set_var("REPLAY_CAPTURE_ENABLED", "true");
        std::env::set_var(
            "REPLAY_CAPTURE_DIR",
            capture_dir.to_string_lossy().to_string(),
        );
        replay_apply_env_overrides(&scenario.manifest.env_overrides);
        std::env::set_current_dir(&workdir)?;
        replay_runtime_activate(scenario.manifest.start_ts_ns);

        let mut bot = MakerHedgeCapBot::new(
            scenario.resolved.clone(),
            &scenario.manifest.market_slug,
            logger.clone(),
        )?;
        bot.runtime_flags
            .insert("__replay_mode".to_string(), json!(true));
        bot._replay_seed_market_metadata(
            scenario.manifest.condition_id.clone(),
            scenario.manifest.yes_asset_id.clone(),
            scenario.manifest.no_asset_id.clone(),
            scenario.manifest.market_start_ts,
            scenario.manifest.market_expiry_ts,
        );
        bot._replay_seed_active_trade_id(scenario.replay_trade_id());
        bot._init_replay_capture(&scenario.resolved)?;
        let replay_output_root = bot
            .replay_recorder
            .as_ref()
            .map(|recorder| recorder.root_dir.clone())
            .ok_or_else(|| anyhow!("failed to initialize replay output recorder"))?;

        if let Ok(mut queue) = bot.replay_order_acks.lock() {
            *queue = scenario
                .replay_order_acks()
                .into_iter()
                .collect::<VecDeque<_>>();
        }
        bot._bot_runtime_mark_startup_reconciliation_pending(runtime_now_ts_f64());

        let mut last_gross_reservation_refresh = 0.0;
        let mut stale_stage_logged = BotRuntimeMarketDataStaleStage::Fresh;
        let tick_ns =
            (bot.loop_wait_seconds_maker.max(0.01).min(0.5) * 1_000_000_000.0).round() as i64;
        let mut next_internal_tick_ns = scenario.manifest.start_ts_ns;
        let mut exit_reason: Option<String> = None;

        for event in &scenario.events {
            while next_internal_tick_ns < event.ts_ns {
                replay_runtime_set_now_ns(next_internal_tick_ns);
                if let Some(reason) = bot._run_bot_runtime_tick(
                    runtime_now_ts_f64(),
                    &mut last_gross_reservation_refresh,
                    &mut stale_stage_logged,
                ) {
                    if reason != bot._get_exit_reason() {
                        return Err(anyhow!("replay exited early: {reason}"));
                    }
                    exit_reason = Some(reason);
                    break;
                }
                next_internal_tick_ns = next_internal_tick_ns.saturating_add(tick_ns.max(1));
            }
            if exit_reason.is_some() {
                break;
            }
            replay_runtime_set_now_ns(event.ts_ns);
            match event.kind.as_str() {
                "market_best_bid_ask" | "market_tick_size" => {
                    bot._handle_market_event(&event.payload)
                }
                "user_order" | "user_trade" => bot._handle_user_event(&event.payload),
                "ws_open" => {
                    if let Some(channel) = event
                        .payload
                        .get("channel")
                        .and_then(|value| value.as_str())
                    {
                        bot._on_open(channel);
                    }
                }
                "ws_close" => {
                    if let Some(channel) = event
                        .payload
                        .get("channel")
                        .and_then(|value| value.as_str())
                    {
                        let code = event
                            .payload
                            .get("code")
                            .and_then(|value| value.as_i64())
                            .unwrap_or(1006);
                        let msg = event
                            .payload
                            .get("message")
                            .and_then(|value| value.as_str())
                            .unwrap_or("replay_close");
                        bot._on_close(channel, code, msg);
                    }
                }
                "reconcile_snapshot" => {
                    let orders = event
                        .payload
                        .get("orders")
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
                        *cache = orders;
                    }
                }
                other => return Err(anyhow!("unsupported replay event kind: {other}")),
            }
            if let Some(reason) = bot._run_bot_runtime_tick(
                runtime_now_ts_f64(),
                &mut last_gross_reservation_refresh,
                &mut stale_stage_logged,
            ) {
                if reason != bot._get_exit_reason() {
                    return Err(anyhow!("replay exited early: {reason}"));
                }
                exit_reason = Some(reason);
                break;
            }
            next_internal_tick_ns = event.ts_ns.saturating_add(tick_ns.max(1));
        }

        if exit_reason.is_none() {
            let end_ts_ns = scenario
                .manifest
                .end_ts_ns
                .unwrap_or_else(|| next_internal_tick_ns.max(scenario.manifest.start_ts_ns));
            while next_internal_tick_ns <= end_ts_ns {
                replay_runtime_set_now_ns(next_internal_tick_ns);
                if let Some(reason) = bot._run_bot_runtime_tick(
                    runtime_now_ts_f64(),
                    &mut last_gross_reservation_refresh,
                    &mut stale_stage_logged,
                ) {
                    if reason != bot._get_exit_reason() {
                        return Err(anyhow!("replay exited early: {reason}"));
                    }
                    exit_reason = Some(reason);
                    break;
                }
                next_internal_tick_ns = next_internal_tick_ns.saturating_add(tick_ns.max(1));
            }
        }

        let exit_reason = exit_reason.unwrap_or_else(|| {
            bot._set_exit_reason("REPLAY_COMPLETE");
            "REPLAY_COMPLETE".to_string()
        });
        replay_emit_post_run_settlement_events(
            &bot,
            exit_reason.as_str(),
            scenario.resolution_snapshot.as_ref(),
        );
        bot.finalize_replay_capture(exit_reason.as_str());
        write_optional_resolution_snapshot(
            &replay_output_root,
            scenario.resolution_snapshot.as_ref(),
        )?;

        compare_optional_jsonl_oracle(
            &scenario.root_dir.join("oracle_decisions.jsonl"),
            &replay_output_root.join("oracle_decisions.jsonl"),
            "decision events",
        )?;
        compare_optional_jsonl_oracle(
            &scenario.root_dir.join("oracle_runtime_events.jsonl"),
            &replay_output_root.join("oracle_runtime_events.jsonl"),
            "runtime events",
        )?;
        compare_optional_json_oracle(
            &scenario.root_dir.join("oracle_final_state.json"),
            &replay_output_root.join("oracle_final_state.json"),
            "final state",
        )?;
        Ok(())
    })();

    replay_runtime_deactivate();
    match prior_shared_state_dir {
        Some(value) => std::env::set_var("POLYBOT_SHARED_STATE_DIR", value),
        None => std::env::remove_var("POLYBOT_SHARED_STATE_DIR"),
    }
    match prior_capture_enabled {
        Some(value) => std::env::set_var("REPLAY_CAPTURE_ENABLED", value),
        None => std::env::remove_var("REPLAY_CAPTURE_ENABLED"),
    }
    match prior_capture_dir {
        Some(value) => std::env::set_var("REPLAY_CAPTURE_DIR", value),
        None => std::env::remove_var("REPLAY_CAPTURE_DIR"),
    }
    replay_restore_env_overrides(&prior_behavior_env);
    if let Some(path) = prior_cwd {
        let _ = std::env::set_current_dir(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_runtime_clock_and_uuid_are_stable() {
        replay_runtime_activate(1_000_000_000);
        assert_eq!(runtime_now_ts(), 1);
        assert!((runtime_now_ts_f64() - 1.0).abs() < 1e-9);
        let first = replay_runtime_new_uuid().expect("uuid");
        let second = replay_runtime_new_uuid().expect("uuid");
        assert_ne!(first, second);
        replay_runtime_deactivate();
    }

    #[test]
    fn replay_scenario_loader_rejects_unsorted_events() {
        let root =
            std::env::temp_dir().join(format!("replay_loader_unsorted_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        let snapshot = VersionedConfigSnapshotV1 {
            schema_version: "v1".to_string(),
            source: "test".to_string(),
            loaded_at: "2024-01-01T00:00:00+07:00".to_string(),
            config_version: "cfgv1".to_string(),
            config_hash: "hash".to_string(),
            bot_config: crate::config::BotConfig::default(),
            runtime_config: crate::config::BotRuntimeConfigSnapshotV1::from(
                &crate::bot::bot_runtime_config_defaults(),
            ),
            execution_config: crate::config::BotExecutionConfigSnapshot {
                wallet_address: "paper".to_string(),
                min_maker_notional: 1.0,
                min_taker_notional: 1.0,
                reconcile_sell_credit_mult: 1.0,
                first_clip_shares: 0.0,
                first_hedge_full: false,
                warmup_seconds: 0,
                max_spread_ticks: 6,
                parity_tolerance: 0.025,
                unhedged_timeout_seconds: 2.0,
                hedge_slippage_ticks: 1,
                hedge_taker_order_type: "FAK".to_string(),
                taker_order_ttl_seconds: 120,
                taker_fill_fallback_from_order_events: true,
                taker_strict_inflight: true,
                taker_hedge_min_interval: 1.0,
                exec_mode: "BOT".to_string(),
                loop_wait_seconds_maker: 1.0,
                loop_wait_seconds_taker: 0.2,
                min_entry_edge_ticks: 0,
                exec_latency_log_enabled: false,
                exec_latency_file_log_enabled: false,
                exec_latency_jsonl_enabled: false,
                exec_latency_csv_enabled: false,
                exec_latency_log_dir: String::new(),
                exec_latency_jsonl_path: String::new(),
                exec_latency_csv_path: String::new(),
                clob_gamma_host: String::new(),
                clob_order_meta_warmup: true,
                order_mode: crate::bot::BotOrderMode::Paper.as_str().to_string(),
                live_enabled: false,
            },
        };
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&ReplayManifest {
                schema_version: "replay_v1".to_string(),
                market_slug: "bot-test".to_string(),
                configured_order_mode: "paper".to_string(),
                state_file_name: "maker_hedgecap_state_bot-test_paper.json".to_string(),
                trade_id: None,
                env_overrides: BTreeMap::new(),
                condition_id: None,
                yes_asset_id: None,
                no_asset_id: None,
                market_start_ts: None,
                market_expiry_ts: None,
                start_ts_ns: 1,
                end_ts_ns: Some(3),
            })
            .expect("manifest bytes"),
        )
        .expect("manifest");
        fs::write(
            root.join("resolved_config.json"),
            serde_json::to_vec_pretty(&ReplayResolvedConfig {
                snapshot,
                effective_bot_config: None,
                runtime_config: None,
                execution_config: None,
            })
            .expect("resolved bytes"),
        )
        .expect("resolved");
        fs::write(
            root.join("events.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&ReplayEventRecord {
                    ts_ns: 2,
                    seq: 1,
                    kind: "ws_open".to_string(),
                    payload: json!({"channel":"market"}),
                })
                .expect("line 1"),
                serde_json::to_string(&ReplayEventRecord {
                    ts_ns: 2,
                    seq: 0,
                    kind: "ws_open".to_string(),
                    payload: json!({"channel":"user"}),
                })
                .expect("line 2")
            ),
        )
        .expect("events");
        let err = ReplayScenario::load(&root).expect_err("unsorted should fail");
        assert!(err.to_string().contains("strictly sorted by (ts_ns, seq)"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replay_scenario_loader_requires_events_file() {
        let root = std::env::temp_dir().join(format!(
            "replay_loader_missing_events_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        let snapshot = VersionedConfigSnapshotV1 {
            schema_version: "v1".to_string(),
            source: "test".to_string(),
            loaded_at: "2024-01-01T00:00:00+07:00".to_string(),
            config_version: "cfgv1".to_string(),
            config_hash: "hash".to_string(),
            bot_config: crate::config::BotConfig::default(),
            runtime_config: crate::config::BotRuntimeConfigSnapshotV1::from(
                &crate::bot::bot_runtime_config_defaults(),
            ),
            execution_config: crate::config::BotExecutionConfigSnapshot {
                wallet_address: "paper".to_string(),
                min_maker_notional: 1.0,
                min_taker_notional: 1.0,
                reconcile_sell_credit_mult: 1.0,
                first_clip_shares: 0.0,
                first_hedge_full: false,
                warmup_seconds: 0,
                max_spread_ticks: 6,
                parity_tolerance: 0.025,
                unhedged_timeout_seconds: 2.0,
                hedge_slippage_ticks: 1,
                hedge_taker_order_type: "FAK".to_string(),
                taker_order_ttl_seconds: 120,
                taker_fill_fallback_from_order_events: true,
                taker_strict_inflight: true,
                taker_hedge_min_interval: 1.0,
                exec_mode: "BOT".to_string(),
                loop_wait_seconds_maker: 1.0,
                loop_wait_seconds_taker: 0.2,
                min_entry_edge_ticks: 0,
                exec_latency_log_enabled: false,
                exec_latency_file_log_enabled: false,
                exec_latency_jsonl_enabled: false,
                exec_latency_csv_enabled: false,
                exec_latency_log_dir: String::new(),
                exec_latency_jsonl_path: String::new(),
                exec_latency_csv_path: String::new(),
                clob_gamma_host: String::new(),
                clob_order_meta_warmup: true,
                order_mode: crate::bot::BotOrderMode::Paper.as_str().to_string(),
                live_enabled: false,
            },
        };
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&ReplayManifest {
                schema_version: "replay_v1".to_string(),
                market_slug: "bot-test".to_string(),
                configured_order_mode: "paper".to_string(),
                state_file_name: "maker_hedgecap_state_bot-test_paper.json".to_string(),
                trade_id: None,
                env_overrides: BTreeMap::new(),
                condition_id: None,
                yes_asset_id: None,
                no_asset_id: None,
                market_start_ts: None,
                market_expiry_ts: None,
                start_ts_ns: 1,
                end_ts_ns: Some(3),
            })
            .expect("manifest bytes"),
        )
        .expect("manifest");
        fs::write(
            root.join("resolved_config.json"),
            serde_json::to_vec_pretty(&ReplayResolvedConfig {
                snapshot,
                effective_bot_config: None,
                runtime_config: None,
                execution_config: None,
            })
            .expect("resolved bytes"),
        )
        .expect("resolved");
        let err = ReplayScenario::load(&root).expect_err("missing events should fail");
        assert!(err.to_string().contains("required replay file missing"));
        assert!(err.to_string().contains("events.jsonl"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replay_env_override_roundtrip_restores_machine_env() {
        let key = "REQUIRE_USER_WS_CONNECTED";
        let prior = std::env::var(key).ok();
        std::env::remove_var(key);
        let captured_none = replay_capture_env_overrides();
        assert_eq!(captured_none.get(key).cloned().flatten(), None);
        std::env::set_var(key, "false");
        let previous = replay_apply_env_overrides(&captured_none);
        assert_eq!(std::env::var(key).ok(), None);
        replay_restore_env_overrides(&previous);
        assert_eq!(std::env::var(key).ok().as_deref(), Some("false"));
        match prior {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn run_replay_scenario_smoke_completes_offline() {
        let root = std::env::temp_dir().join(format!("replay_smoke_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        let snapshot = VersionedConfigSnapshotV1 {
            schema_version: "v1".to_string(),
            source: "test".to_string(),
            loaded_at: "2024-01-01T00:00:00+07:00".to_string(),
            config_version: "cfgv1".to_string(),
            config_hash: "hash".to_string(),
            bot_config: crate::config::BotConfig::default(),
            runtime_config: crate::config::BotRuntimeConfigSnapshotV1::from(
                &crate::bot::bot_runtime_config_defaults(),
            ),
            execution_config: crate::config::BotExecutionConfigSnapshot {
                wallet_address: "paper".to_string(),
                min_maker_notional: 1.0,
                min_taker_notional: 1.0,
                reconcile_sell_credit_mult: 1.0,
                first_clip_shares: 0.0,
                first_hedge_full: false,
                warmup_seconds: 0,
                max_spread_ticks: 6,
                parity_tolerance: 0.025,
                unhedged_timeout_seconds: 2.0,
                hedge_slippage_ticks: 1,
                hedge_taker_order_type: "FAK".to_string(),
                taker_order_ttl_seconds: 120,
                taker_fill_fallback_from_order_events: true,
                taker_strict_inflight: true,
                taker_hedge_min_interval: 1.0,
                exec_mode: "BOT".to_string(),
                loop_wait_seconds_maker: 1.0,
                loop_wait_seconds_taker: 0.2,
                min_entry_edge_ticks: 0,
                exec_latency_log_enabled: false,
                exec_latency_file_log_enabled: false,
                exec_latency_jsonl_enabled: false,
                exec_latency_csv_enabled: false,
                exec_latency_log_dir: String::new(),
                exec_latency_jsonl_path: String::new(),
                exec_latency_csv_path: String::new(),
                clob_gamma_host: String::new(),
                clob_order_meta_warmup: true,
                order_mode: crate::bot::BotOrderMode::Paper.as_str().to_string(),
                live_enabled: false,
            },
        };
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&ReplayManifest {
                schema_version: "replay_v1".to_string(),
                market_slug: "bot-test".to_string(),
                configured_order_mode: "paper".to_string(),
                state_file_name: "maker_hedgecap_state_bot-test_paper.json".to_string(),
                trade_id: None,
                env_overrides: BTreeMap::new(),
                condition_id: Some("cond_test".to_string()),
                yes_asset_id: Some("yes_asset_id".to_string()),
                no_asset_id: Some("no_asset_id".to_string()),
                market_start_ts: Some(1),
                market_expiry_ts: Some(3),
                start_ts_ns: 1_000_000_000,
                end_ts_ns: Some(1_000_000_000),
            })
            .expect("manifest bytes"),
        )
        .expect("manifest");
        fs::write(
            root.join("resolved_config.json"),
            serde_json::to_vec_pretty(&ReplayResolvedConfig {
                snapshot,
                effective_bot_config: None,
                runtime_config: None,
                execution_config: None,
            })
            .expect("resolved bytes"),
        )
        .expect("resolved");
        fs::write(root.join("events.jsonl"), "").expect("events");

        run_replay_scenario(&root).expect("replay smoke");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replay_order_ack_extracts_submit_timing_meta() {
        let row = TradeRuntimeEventInsert {
            event_id: "evt1".to_string(),
            trade_id: "trade1".to_string(),
            pair_id: "pair1".to_string(),
            market_slug: "slug1".to_string(),
            condition_id: None,
            yes_asset_id: None,
            no_asset_id: None,
            config_version: "cfg1".to_string(),
            event_kind: "order_ack".to_string(),
            event_ts: "2024-01-01T00:00:00+07:00".to_string(),
            decision_event_id: Some("dec1".to_string()),
            order_id: Some("oid1".to_string()),
            asset_id: Some("asset1".to_string()),
            side: Some("BUY".to_string()),
            reason_code: Some("reason".to_string()),
            payload_json: json!({
                "order_id": "oid1",
                "meta_json": {
                    "prep_start_ts": 1.0,
                    "post_end_ts": 2.0,
                    "order_submit_ts": 2.0
                }
            })
            .to_string(),
        };
        let scenario = ReplayScenario {
            root_dir: std::env::temp_dir(),
            manifest: ReplayManifest {
                schema_version: "replay_v1".to_string(),
                market_slug: "slug1".to_string(),
                configured_order_mode: "paper".to_string(),
                state_file_name: "state.json".to_string(),
                trade_id: None,
                env_overrides: BTreeMap::new(),
                condition_id: None,
                yes_asset_id: None,
                no_asset_id: None,
                market_start_ts: None,
                market_expiry_ts: None,
                start_ts_ns: 1,
                end_ts_ns: Some(2),
            },
            resolved: resolve_versioned_config_bundle_from_snapshot(VersionedConfigSnapshotV1 {
                schema_version: "v1".to_string(),
                source: "test".to_string(),
                loaded_at: "2024-01-01T00:00:00+07:00".to_string(),
                config_version: "cfg1".to_string(),
                config_hash: "hash".to_string(),
                bot_config: crate::config::BotConfig::default(),
                runtime_config: crate::config::BotRuntimeConfigSnapshotV1::from(
                    &crate::bot::bot_runtime_config_defaults(),
                ),
                execution_config: crate::config::BotExecutionConfigSnapshot {
                    wallet_address: "paper".to_string(),
                    min_maker_notional: 1.0,
                    min_taker_notional: 1.0,
                    reconcile_sell_credit_mult: 1.0,
                    first_clip_shares: 0.0,
                    first_hedge_full: false,
                    warmup_seconds: 0,
                    max_spread_ticks: 6,
                    parity_tolerance: 0.025,
                    unhedged_timeout_seconds: 2.0,
                    hedge_slippage_ticks: 1,
                    hedge_taker_order_type: "FAK".to_string(),
                    taker_order_ttl_seconds: 120,
                    taker_fill_fallback_from_order_events: true,
                    taker_strict_inflight: true,
                    taker_hedge_min_interval: 1.0,
                    exec_mode: "BOT".to_string(),
                    loop_wait_seconds_maker: 1.0,
                    loop_wait_seconds_taker: 0.2,
                    min_entry_edge_ticks: 0,
                    exec_latency_log_enabled: false,
                    exec_latency_file_log_enabled: false,
                    exec_latency_jsonl_enabled: false,
                    exec_latency_csv_enabled: false,
                    exec_latency_log_dir: String::new(),
                    exec_latency_jsonl_path: String::new(),
                    exec_latency_csv_path: String::new(),
                    clob_gamma_host: String::new(),
                    clob_order_meta_warmup: true,
                    order_mode: crate::bot::BotOrderMode::Paper.as_str().to_string(),
                    live_enabled: false,
                },
            })
            .expect("bundle"),
            events: Vec::new(),
            oracle: ReplayOracle {
                runtime_events: vec![row],
                decisions: Vec::new(),
            },
            resolution_snapshot: None,
        };
        let mut acks = scenario.replay_order_acks();
        let ack = acks.pop_front().expect("ack");
        assert_eq!(ack.order_id, "oid1");
        assert_eq!(ack.asset_id.as_deref(), Some("asset1"));
        assert_eq!(ack.side.as_deref(), Some("BUY"));
        assert_eq!(
            ack.submit_timing
                .as_ref()
                .and_then(|value| value.get("order_submit_ts"))
                .and_then(|value| value.as_f64()),
            Some(2.0)
        );
    }

    #[test]
    fn replay_loader_prefers_persisted_exact_resolved_bundle() {
        let root = std::env::temp_dir().join(format!("replay_loader_exact_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        let mut snapshot_bot_config = crate::config::BotConfig::default();
        snapshot_bot_config.market_duration_seconds = 60;
        let mut exact_bot_config = snapshot_bot_config.clone();
        exact_bot_config.market_duration_seconds = 999;
        let runtime_snapshot = crate::config::BotRuntimeConfigSnapshotV1::from(
            &crate::bot::bot_runtime_config_defaults(),
        );
        let snapshot = VersionedConfigSnapshotV1 {
            schema_version: "v1".to_string(),
            source: "test".to_string(),
            loaded_at: "2024-01-01T00:00:00+07:00".to_string(),
            config_version: "cfgv1".to_string(),
            config_hash: "hash".to_string(),
            bot_config: snapshot_bot_config,
            runtime_config: runtime_snapshot.clone(),
            execution_config: crate::config::BotExecutionConfigSnapshot {
                wallet_address: "paper".to_string(),
                min_maker_notional: 1.0,
                min_taker_notional: 1.0,
                reconcile_sell_credit_mult: 1.0,
                first_clip_shares: 0.0,
                first_hedge_full: false,
                warmup_seconds: 0,
                max_spread_ticks: 6,
                parity_tolerance: 0.025,
                unhedged_timeout_seconds: 2.0,
                hedge_slippage_ticks: 1,
                hedge_taker_order_type: "FAK".to_string(),
                taker_order_ttl_seconds: 120,
                taker_fill_fallback_from_order_events: true,
                taker_strict_inflight: true,
                taker_hedge_min_interval: 1.0,
                exec_mode: "BOT".to_string(),
                loop_wait_seconds_maker: 1.0,
                loop_wait_seconds_taker: 0.2,
                min_entry_edge_ticks: 0,
                exec_latency_log_enabled: false,
                exec_latency_file_log_enabled: false,
                exec_latency_jsonl_enabled: false,
                exec_latency_csv_enabled: false,
                exec_latency_log_dir: String::new(),
                exec_latency_jsonl_path: String::new(),
                exec_latency_csv_path: String::new(),
                clob_gamma_host: String::new(),
                clob_order_meta_warmup: true,
                order_mode: crate::bot::BotOrderMode::Shadow.as_str().to_string(),
                live_enabled: false,
            },
        };
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&ReplayManifest {
                schema_version: "replay_v1".to_string(),
                market_slug: "bot-test".to_string(),
                configured_order_mode: "shadow".to_string(),
                state_file_name: "state.json".to_string(),
                trade_id: Some("manifest_trade".to_string()),
                env_overrides: BTreeMap::new(),
                condition_id: None,
                yes_asset_id: None,
                no_asset_id: None,
                market_start_ts: None,
                market_expiry_ts: None,
                start_ts_ns: 1,
                end_ts_ns: Some(2),
            })
            .expect("manifest bytes"),
        )
        .expect("manifest");
        fs::write(
            root.join("resolved_config.json"),
            serde_json::to_vec_pretty(&ReplayResolvedConfig {
                snapshot,
                effective_bot_config: Some(exact_bot_config),
                runtime_config: Some(runtime_snapshot),
                execution_config: Some(crate::config::BotExecutionConfigSnapshot {
                    wallet_address: "paper".to_string(),
                    min_maker_notional: 1.0,
                    min_taker_notional: 1.0,
                    reconcile_sell_credit_mult: 1.0,
                    first_clip_shares: 0.0,
                    first_hedge_full: false,
                    warmup_seconds: 0,
                    max_spread_ticks: 6,
                    parity_tolerance: 0.025,
                    unhedged_timeout_seconds: 2.0,
                    hedge_slippage_ticks: 1,
                    hedge_taker_order_type: "FAK".to_string(),
                    taker_order_ttl_seconds: 120,
                    taker_fill_fallback_from_order_events: true,
                    taker_strict_inflight: true,
                    taker_hedge_min_interval: 1.0,
                    exec_mode: "BOT".to_string(),
                    loop_wait_seconds_maker: 1.0,
                    loop_wait_seconds_taker: 0.2,
                    min_entry_edge_ticks: 0,
                    exec_latency_log_enabled: false,
                    exec_latency_file_log_enabled: false,
                    exec_latency_jsonl_enabled: false,
                    exec_latency_csv_enabled: false,
                    exec_latency_log_dir: String::new(),
                    exec_latency_jsonl_path: String::new(),
                    exec_latency_csv_path: String::new(),
                    clob_gamma_host: String::new(),
                    clob_order_meta_warmup: true,
                    order_mode: crate::bot::BotOrderMode::Paper.as_str().to_string(),
                    live_enabled: false,
                }),
            })
            .expect("resolved bytes"),
        )
        .expect("resolved");
        fs::write(root.join("events.jsonl"), "").expect("events");
        let scenario = ReplayScenario::load(&root).expect("load");
        assert_eq!(
            scenario
                .resolved
                .effective_bot_config
                .market_duration_seconds,
            999
        );
        assert_eq!(scenario.resolved.execution_config.order_mode, "paper");
        assert_eq!(
            scenario.replay_trade_id().as_deref(),
            Some("manifest_trade")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replay_order_acks_fallback_to_event_tape_ids_without_oracle_rows() {
        let bundle = resolve_versioned_config_bundle_from_snapshot(VersionedConfigSnapshotV1 {
            schema_version: "v1".to_string(),
            source: "test".to_string(),
            loaded_at: "2024-01-01T00:00:00+07:00".to_string(),
            config_version: "cfg1".to_string(),
            config_hash: "hash".to_string(),
            bot_config: crate::config::BotConfig::default(),
            runtime_config: crate::config::BotRuntimeConfigSnapshotV1::from(
                &crate::bot::bot_runtime_config_defaults(),
            ),
            execution_config: crate::config::BotExecutionConfigSnapshot {
                wallet_address: "paper".to_string(),
                min_maker_notional: 1.0,
                min_taker_notional: 1.0,
                reconcile_sell_credit_mult: 1.0,
                first_clip_shares: 0.0,
                first_hedge_full: false,
                warmup_seconds: 0,
                max_spread_ticks: 6,
                parity_tolerance: 0.025,
                unhedged_timeout_seconds: 2.0,
                hedge_slippage_ticks: 1,
                hedge_taker_order_type: "FAK".to_string(),
                taker_order_ttl_seconds: 120,
                taker_fill_fallback_from_order_events: true,
                taker_strict_inflight: true,
                taker_hedge_min_interval: 1.0,
                exec_mode: "BOT".to_string(),
                loop_wait_seconds_maker: 1.0,
                loop_wait_seconds_taker: 0.2,
                min_entry_edge_ticks: 0,
                exec_latency_log_enabled: false,
                exec_latency_file_log_enabled: false,
                exec_latency_jsonl_enabled: false,
                exec_latency_csv_enabled: false,
                exec_latency_log_dir: String::new(),
                exec_latency_jsonl_path: String::new(),
                exec_latency_csv_path: String::new(),
                clob_gamma_host: String::new(),
                clob_order_meta_warmup: true,
                order_mode: crate::bot::BotOrderMode::Paper.as_str().to_string(),
                live_enabled: false,
            },
        })
        .expect("bundle");
        let scenario = ReplayScenario {
            root_dir: std::env::temp_dir(),
            manifest: ReplayManifest {
                schema_version: "replay_v1".to_string(),
                market_slug: "slug1".to_string(),
                configured_order_mode: "paper".to_string(),
                state_file_name: "state.json".to_string(),
                trade_id: None,
                env_overrides: BTreeMap::new(),
                condition_id: None,
                yes_asset_id: None,
                no_asset_id: None,
                market_start_ts: None,
                market_expiry_ts: None,
                start_ts_ns: 1,
                end_ts_ns: Some(2),
            },
            resolved: bundle,
            events: vec![
                ReplayEventRecord {
                    ts_ns: 10,
                    seq: 0,
                    kind: "user_trade".to_string(),
                    payload: json!({
                        "taker_order_id": "taker_oid_1",
                        "asset_id": "asset_taker",
                        "side": "BUY"
                    }),
                },
                ReplayEventRecord {
                    ts_ns: 11,
                    seq: 0,
                    kind: "user_order".to_string(),
                    payload: json!({
                        "order_id": "maker_oid_1",
                        "asset_id": "asset_maker",
                        "side": "BUY"
                    }),
                },
            ],
            oracle: ReplayOracle {
                runtime_events: Vec::new(),
                decisions: Vec::new(),
            },
            resolution_snapshot: None,
        };
        let acks = scenario.replay_order_acks();
        assert_eq!(acks.len(), 2);
        assert_eq!(acks[0].order_id, "taker_oid_1");
        assert_eq!(acks[0].asset_id.as_deref(), Some("asset_taker"));
        assert_eq!(acks[0].side.as_deref(), Some("BUY"));
        assert_eq!(acks[1].order_id, "maker_oid_1");
        assert_eq!(acks[1].asset_id.as_deref(), Some("asset_maker"));
        assert_eq!(acks[1].side.as_deref(), Some("BUY"));
    }

    #[test]
    fn replay_order_ack_matching_prefers_asset_and_side_over_fifo() {
        let mut queue = VecDeque::from(vec![
            ReplayOrderAck {
                order_id: "no_oid".to_string(),
                submit_timing: None,
                asset_id: Some("no_asset".to_string()),
                side: Some("BUY".to_string()),
            },
            ReplayOrderAck {
                order_id: "yes_oid".to_string(),
                submit_timing: None,
                asset_id: Some("yes_asset".to_string()),
                side: Some("BUY".to_string()),
            },
        ]);
        let yes_ack = replay_take_order_ack(&mut queue, "yes_asset", "BUY").expect("yes ack");
        let no_ack = replay_take_order_ack(&mut queue, "no_asset", "BUY").expect("no ack");
        assert_eq!(yes_ack.order_id, "yes_oid");
        assert_eq!(no_ack.order_id, "no_oid");
    }

    #[test]
    fn replay_trade_id_falls_back_to_manifest_without_oracle_files() {
        let bundle = resolve_versioned_config_bundle_from_snapshot(VersionedConfigSnapshotV1 {
            schema_version: "v1".to_string(),
            source: "test".to_string(),
            loaded_at: "2024-01-01T00:00:00+07:00".to_string(),
            config_version: "cfg1".to_string(),
            config_hash: "hash".to_string(),
            bot_config: crate::config::BotConfig::default(),
            runtime_config: crate::config::BotRuntimeConfigSnapshotV1::from(
                &crate::bot::bot_runtime_config_defaults(),
            ),
            execution_config: crate::config::BotExecutionConfigSnapshot {
                wallet_address: "paper".to_string(),
                min_maker_notional: 1.0,
                min_taker_notional: 1.0,
                reconcile_sell_credit_mult: 1.0,
                first_clip_shares: 0.0,
                first_hedge_full: false,
                warmup_seconds: 0,
                max_spread_ticks: 6,
                parity_tolerance: 0.025,
                unhedged_timeout_seconds: 2.0,
                hedge_slippage_ticks: 1,
                hedge_taker_order_type: "FAK".to_string(),
                taker_order_ttl_seconds: 120,
                taker_fill_fallback_from_order_events: true,
                taker_strict_inflight: true,
                taker_hedge_min_interval: 1.0,
                exec_mode: "BOT".to_string(),
                loop_wait_seconds_maker: 1.0,
                loop_wait_seconds_taker: 0.2,
                min_entry_edge_ticks: 0,
                exec_latency_log_enabled: false,
                exec_latency_file_log_enabled: false,
                exec_latency_jsonl_enabled: false,
                exec_latency_csv_enabled: false,
                exec_latency_log_dir: String::new(),
                exec_latency_jsonl_path: String::new(),
                exec_latency_csv_path: String::new(),
                clob_gamma_host: String::new(),
                clob_order_meta_warmup: true,
                order_mode: crate::bot::BotOrderMode::Paper.as_str().to_string(),
                live_enabled: false,
            },
        })
        .expect("bundle");
        let scenario = ReplayScenario {
            root_dir: std::env::temp_dir(),
            manifest: ReplayManifest {
                schema_version: "replay_v1".to_string(),
                market_slug: "slug1".to_string(),
                configured_order_mode: "paper".to_string(),
                state_file_name: "state.json".to_string(),
                trade_id: Some("manifest_trade_only".to_string()),
                env_overrides: BTreeMap::new(),
                condition_id: None,
                yes_asset_id: None,
                no_asset_id: None,
                market_start_ts: None,
                market_expiry_ts: None,
                start_ts_ns: 1,
                end_ts_ns: Some(2),
            },
            resolved: bundle,
            events: Vec::new(),
            oracle: ReplayOracle {
                runtime_events: Vec::new(),
                decisions: Vec::new(),
            },
            resolution_snapshot: None,
        };
        assert_eq!(
            scenario.replay_trade_id().as_deref(),
            Some("manifest_trade_only")
        );
    }
}
