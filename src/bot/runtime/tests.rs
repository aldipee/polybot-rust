use super::*;
use proptest::prelude::*;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::sync::OnceLock;
struct BotRuntimeNoopLogger;
impl LogLike for BotRuntimeNoopLogger {
    /// Exercises the info scenario and checks the expected BOT behavior.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    fn info(&self, _msg: &str) {}
    /// Exercises the warning scenario and checks the expected BOT behavior.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    fn warning(&self, _msg: &str) {}
    /// Exercises the error scenario and checks the expected BOT behavior.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    fn error(&self, _msg: &str) {}

    fn event(&self, _level: &str, _record: &serde_json::Value) {}
}
/// Exercises the env lock scenario and checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

fn env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
/// Exercises the with exec mode scenario and checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

fn with_exec_mode<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
    let _guard = env_lock().lock().expect("env lock");
    let prior = std::env::var("EXEC_MODE").ok();
    match value {
        Some(v) => std::env::set_var("EXEC_MODE", v),
        None => std::env::remove_var("EXEC_MODE"),
    }
    let out = f();
    match prior {
        Some(v) => std::env::set_var("EXEC_MODE", v),
        None => std::env::remove_var("EXEC_MODE"),
    }
    out
}

fn with_env_var<T>(key: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
    let _guard = env_lock().lock().expect("env lock");
    let prior = std::env::var(key).ok();
    match value {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    let out = f();
    match prior {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    out
}

fn with_shared_state_dir<T>(path: &PathBuf, f: impl FnOnce() -> T) -> T {
    let _ = path;
    f()
}

fn set_shared_state_dir_override(bot: &mut MakerHedgeCapBot, path: &PathBuf) {
    bot.runtime_flags.insert(
        "__shared_state_dir_override".to_string(),
        json!(path.to_string_lossy().to_string()),
    );
}

/// Exercises the make BOT runtime test BOT scenario and checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

fn make_bot_runtime_test_bot() -> MakerHedgeCapBot {
    let mut cfg = BotConfig::default();
    cfg.dry_run = true;
    cfg.market_data_stale_seconds = 8;
    cfg.market_data_stale_add_block_seconds = 2;
    cfg.market_data_stale_hard_pause_seconds = 5;
    cfg.stale_seconds = 3;
    cfg.max_total_cost = 20.0;
    cfg.reserve_usd = 2.0;
    let state_file = std::env::temp_dir().join(format!(
        "bot_runtime_test_state_{}.json",
        uuid::Uuid::new_v4()
    ));
    let daily_liquidity_state_file = std::env::temp_dir().join(format!(
        "bot_runtime_test_daily_liquidity_state_{}.json",
        uuid::Uuid::new_v4()
    ));
    MakerHedgeCapBot {
        cfg,
        logger: Arc::new(BotRuntimeNoopLogger),
        market_slug: "bot-test".to_string(),
        config_version: "cfgv1_test".to_string(),
        audit_repo: None,
        active_trade_id: None,
        audit_runtime_tx: None,
        pair_identity: PairIdentity {
            pair_id: canonical_pair_id_from_slug("bot-test"),
            market_slug: "bot-test".to_string(),
            condition_id: None,
            yes_asset_id: Some("yes_asset_id".to_string()),
            no_asset_id: Some("no_asset_id".to_string()),
        },
        state_file,
        state: Arc::new(Mutex::new(BotState::default())),
        daily_liquidity_state_file,
        daily_liquidity_state: Arc::new(Mutex::new(DailyLiquidityState::default())),
        start_trade_iso: "2024-01-01T00:00:00Z".to_string(),
        first_entry_fill_iso: Arc::new(Mutex::new(None)),
        first_entry_reason: Arc::new(Mutex::new(None)),
        pending_entry_reason: Arc::new(Mutex::new(None)),
        active_entry_reason: Arc::new(Mutex::new(None)),
        stop_loss_category: Arc::new(Mutex::new(None)),
        exit_reason: Arc::new(Mutex::new("RUNNING".to_string())),
        stop_flag: Arc::new(AtomicBool::new(false)),
        wallet_address: format!("0xtest{}", uuid::Uuid::new_v4().simple()),
        min_maker_notional: 1.0,
        min_taker_notional: 1.0,
        reconcile_sell_credit_mult: 1.0,
        first_clip_shares: 0.0,
        first_hedge_full: false,
        min_entry_edge_ticks: 0,
        start_ts: 0,
        expiry_ts: 300,
        warmup_seconds: 0,
        max_spread_ticks: 6,
        parity_tolerance: 0.025,
        unhedged_timeout_seconds: 2.0,
        hedge_slippage_ticks: 1,
        hedge_taker_order_type: "FAK".to_string(),
        taker_order_ttl_seconds: 120,
        taker_fill_fallback_from_order_events: true,
        taker_strict_inflight: true,
        last_taker_hedge_ts: 0.0,
        taker_hedge_min_interval: 1.0,
        exec_mode: "BOT".to_string(),
        configured_order_mode: "shadow".to_string(),
        live_enabled: false,
        loop_wait_seconds_maker: 1.0,
        loop_wait_seconds_taker: 0.2,
        clob_order_meta_warmup: true,
        condition_id: None,
        market_fees_enabled: None,
        yes_asset: Some("yes_asset_id".to_string()),
        no_asset: Some("no_asset_id".to_string()),
        runtime_flags: HashMap::new(),
        market_last_update_ts: Arc::new(Mutex::new(0.0)),
        best_quotes: Arc::new(Mutex::new(HashMap::new())),
        market_connected: Arc::new(AtomicBool::new(true)),
        user_connected: Arc::new(AtomicBool::new(true)),
        book_cache: Arc::new(Mutex::new(HashMap::new())),
        debug_last_ts: Arc::new(Mutex::new(HashMap::new())),
        fsm_state: Arc::new(Mutex::new("ACCUMULATE".to_string())),
        order_exec_context: Arc::new(Mutex::new(HashMap::new())),
        submit_timing_cache: Arc::new(Mutex::new(HashMap::new())),
        taker_orders: Arc::new(Mutex::new(HashMap::new())),
        latency_log: None,
        clob_rt: None,
        clob_client: None,
        clob_api_creds: None,
        balance_allowance_cache: Arc::new(Mutex::new(HashMap::new())),
        reconcile_suspect_yes: Arc::new(Mutex::new(None)),
        reconcile_suspect_no: Arc::new(Mutex::new(None)),
        reconcile_last_ts: Arc::new(Mutex::new(0.0)),
        exchange_orders_cache: Arc::new(Mutex::new(Vec::new())),
        maker_ladder_open_orders: Arc::new(Mutex::new(HashMap::new())),
        maker_order_slots: Arc::new(Mutex::new(HashMap::new())),
        maker_order_index: Arc::new(Mutex::new(HashMap::new())),
        maker_exec_ledger: Arc::new(Mutex::new(MakerExecLedger::default())),
        replay_recorder: None,
        replay_order_acks: Arc::new(Mutex::new(VecDeque::new())),
        bot_runtime_state: Arc::new(Mutex::new(BotRuntimeState::default())),
        bot_runtime_cfg: bot_runtime_config_defaults(),
    }
}

fn set_pair_quotes(
    bot: &MakerHedgeCapBot,
    yes_bid: f64,
    yes_ask: f64,
    no_bid: f64,
    no_ask: f64,
    ts: f64,
) {
    if let Ok(mut quotes) = bot.best_quotes.lock() {
        quotes.insert("yes_asset_id".to_string(), (yes_bid, yes_ask, ts));
        quotes.insert("no_asset_id".to_string(), (no_bid, no_ask, ts));
    }
}

fn set_daily_liquidity_state(
    bot: &MakerHedgeCapBot,
    maker_fill_shares: f64,
    taker_fill_shares: f64,
) {
    if let Ok(mut state) = bot.daily_liquidity_state.lock() {
        state.day_key_utc = crate::helpers::current_utc_day_key();
        state.maker_fill_shares = maker_fill_shares;
        state.taker_fill_shares = taker_fill_shares;
        let _ =
            crate::helpers::save_daily_liquidity_state(&bot.daily_liquidity_state_file, &mut state);
    }
}

#[test]
fn configured_live_disarmed_uses_effective_shadow_mode() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = false;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.40, 0.41, 0.40, 0.41, now);

    assert!(matches!(
        bot._bot_runtime_effective_order_mode(),
        BotOrderMode::Shadow
    ));
    assert_eq!(
        bot._bot_runtime_live_block_reason().as_deref(),
        Some("live_mode_disarmed")
    );
    let oid = bot
        ._place_limit_bid_gtc_with_origin(
            "yes_asset_id",
            0.40,
            12.0,
            Some(true),
            "BOT_OPEN_BOTH_YES",
        )
        .expect("shadow-routed submit");
    assert!(oid.starts_with("SHADOW_INTENT_"));
}

#[test]
fn shadow_direct_reconcile_preserves_persisted_shadow_intent() {
    let mut bot = make_bot_runtime_test_bot();
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let oid = bot
        ._place_limit_bid_gtc_with_origin(
            "yes_asset_id",
            0.40,
            12.0,
            Some(true),
            "BOT_OPEN_BOTH_YES",
        )
        .expect("shadow direct submit");
    assert!(oid.starts_with("SHADOW_INTENT_"));

    if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
        *cache = vec![json!({
            "id": "real_order_yes_1",
            "order_id": "real_order_yes_1",
            "asset_id": "yes_asset_id",
            "token_id": "yes_asset_id",
            "side": "BUY",
            "price": 0.39,
            "size": 7.0,
            "remaining_size": 7.0,
            "original_size": 7.0,
        })];
    }

    bot._reconcile_exchange_orders_for_asset("yes_asset_id", Some(0.40), true);

    let open_order = bot
        .state
        .lock()
        .ok()
        .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
        .expect("shadow local open order");
    assert_eq!(open_order.order_id.as_deref(), Some(oid.as_str()));
    let cached_ids = bot
        .exchange_orders_cache
        .lock()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| bot._extract_order_id(row))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(cached_ids.iter().any(|id| id == oid.as_str()));
}

#[test]
fn shadow_single_inflight_reconcile_preserves_persisted_shadow_intent() {
    let bot = make_bot_runtime_test_bot();
    let key = MakerOrderKey::buy("yes_asset_id");

    let oid = bot
        ._maker_order_upsert_gtc(&key, 0.40, 12.0, "BOT_OPEN_BOTH_YES")
        .expect("shadow single-inflight submit");
    assert!(oid.starts_with("SHADOW_INTENT_"));

    if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
        *cache = vec![json!({
            "id": "real_order_yes_2",
            "order_id": "real_order_yes_2",
            "asset_id": "yes_asset_id",
            "token_id": "yes_asset_id",
            "side": "BUY",
            "price": 0.39,
            "size": 6.0,
            "remaining_size": 6.0,
            "original_size": 6.0,
        })];
    }

    bot._maker_order_reconcile_asset("yes_asset_id", Some(0.40));

    let open_order = bot
        .state
        .lock()
        .ok()
        .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
        .expect("shadow local single-inflight order");
    assert_eq!(open_order.order_id.as_deref(), Some(oid.as_str()));
    let slot = bot._maker_order_slot_get(&key);
    assert_eq!(slot.order_id.as_deref(), Some(oid.as_str()));
    assert_eq!(slot.state, MakerOrderLifecycle::Working);
}

#[test]
fn downgraded_live_shadow_intent_cancel_clears_local_state() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = true;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.40, 0.41, 0.40, 0.41, now);
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.safety_gate = BotRuntimeSafetyGate::DependencyPaused;
        runtime_state.safety_gate_reason = "dependency_pause:test".to_string();
        runtime_state.live_order_write_armed_once = true;
    }

    assert!(matches!(
        bot._bot_runtime_effective_order_mode(),
        BotOrderMode::Shadow
    ));
    assert!(bot._bot_runtime_live_cancel_allowed());

    let key = MakerOrderKey::buy("yes_asset_id");
    let oid = bot
        ._maker_order_upsert_gtc(&key, 0.40, 12.0, "BOT_OPEN_BOTH_YES")
        .expect("downgraded-live shadow submit");
    assert!(oid.starts_with("SHADOW_INTENT_"));

    if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
        cache.clear();
    }

    assert!(bot._cancel(oid.as_str()));
    assert!(bot
        .state
        .lock()
        .ok()
        .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
        .is_none());
    let slot = bot._maker_order_slot_get(&key);
    assert_eq!(slot.state, MakerOrderLifecycle::Idle);
    assert!(slot.order_id.is_none());
}

#[test]
fn configured_live_with_fresh_quotes_and_clean_gate_is_effectively_live() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = true;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.40, 0.41, 0.40, 0.41, now);

    assert!(matches!(
        bot._bot_runtime_effective_order_mode(),
        BotOrderMode::Live
    ));
    assert!(bot._bot_runtime_live_write_allowed());
}

#[test]
fn startup_reconciliation_pending_keeps_configured_live_in_shadow() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = true;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.40, 0.41, 0.40, 0.41, now);
    bot._bot_runtime_mark_startup_reconciliation_pending(now);

    assert!(matches!(
        bot._bot_runtime_effective_order_mode(),
        BotOrderMode::Shadow
    ));
    assert_eq!(
        bot._bot_runtime_live_block_reason().as_deref(),
        Some("startup_reconciliation_pending")
    );
}

#[test]
fn hard_stale_demotes_configured_live_back_to_shadow() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = true;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.40, 0.41, 0.40, 0.41, now - 10.0);

    assert!(matches!(
        bot._bot_runtime_effective_order_mode(),
        BotOrderMode::Shadow
    ));
    assert_eq!(
        bot._bot_runtime_live_block_reason().as_deref(),
        Some("market_data_stale:hard_paused")
    );
}

#[test]
fn paper_taker_fak_buy_fills_immediately_on_cross() {
    let mut bot = make_bot_runtime_test_bot();
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_paper_taker_fill_shared_{}",
        uuid::Uuid::new_v4()
    ));
    set_shared_state_dir_override(&mut bot, &shared_dir);
    bot.configured_order_mode = "paper".to_string();
    bot.active_trade_id = Some("paper_taker_trade".to_string());
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.maker_fill_shares = 100.0;
    }
    set_daily_liquidity_state(&bot, 100.0, 0.0);
    bot._bot_runtime_refresh_daily_liquidity_counters();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.39, 0.40, 0.59, 0.60, now);

    let oid = bot
        ._place_taker_bid_fak(
            "yes_asset_id",
            0.41,
            5.0,
            Some("FAK"),
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        )
        .expect("paper taker order");
    assert!(oid.starts_with("PAPER_INTENT_"));
    let state = bot
        .state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert!((state.q_yes - 5.0).abs() < 1e-9);
    assert!(bot
        .taker_orders
        .lock()
        .map(|orders| !orders.contains_key(oid.as_str()))
        .unwrap_or(true));
    let shared = crate::helpers::load_shared_gross_exposure_state(
        &bot._gross_exposure_state_file(),
        bot.cfg.gross_cap_shared_state_ttl_seconds,
    )
    .expect("paper gross state");
    assert!(shared.pending_orders.is_empty());
}

#[test]
fn paper_taker_fak_buy_non_cross_clears_shared_gross_reservation() {
    let mut bot = make_bot_runtime_test_bot();
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_paper_taker_miss_shared_{}",
        uuid::Uuid::new_v4()
    ));
    set_shared_state_dir_override(&mut bot, &shared_dir);
    bot.configured_order_mode = "paper".to_string();
    bot.active_trade_id = Some("paper_taker_trade_miss".to_string());
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.maker_fill_shares = 100.0;
    }
    set_daily_liquidity_state(&bot, 100.0, 0.0);
    bot._bot_runtime_refresh_daily_liquidity_counters();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.39, 0.50, 0.49, 0.60, now);

    let oid = bot
        ._place_taker_bid_fak(
            "yes_asset_id",
            0.41,
            5.0,
            Some("FAK"),
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        )
        .expect("paper taker order");
    assert!(oid.starts_with("PAPER_INTENT_"));
    assert!(bot
        .taker_orders
        .lock()
        .map(|orders| !orders.contains_key(oid.as_str()))
        .unwrap_or(true));
    let shared = crate::helpers::load_shared_gross_exposure_state(
        &bot._gross_exposure_state_file(),
        bot.cfg.gross_cap_shared_state_ttl_seconds,
    )
    .expect("paper gross state");
    assert!(shared.pending_orders.is_empty());
}

#[test]
fn shadow_taker_fak_buy_auto_clears_pending_tracking() {
    let mut bot = make_bot_runtime_test_bot();
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_shadow_taker_cleanup_shared_{}",
        uuid::Uuid::new_v4()
    ));
    set_shared_state_dir_override(&mut bot, &shared_dir);
    bot.active_trade_id = Some("shadow_taker_trade".to_string());
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.maker_fill_shares = 100.0;
    }
    set_daily_liquidity_state(&bot, 100.0, 0.0);
    bot._bot_runtime_refresh_daily_liquidity_counters();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.39, 0.50, 0.49, 0.60, now);

    let oid = bot
        ._place_taker_bid_fak(
            "yes_asset_id",
            0.41,
            5.0,
            Some("FAK"),
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        )
        .expect("shadow taker order");
    assert!(oid.starts_with("SHADOW_INTENT_"));
    let state = bot
        .state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert!(state.q_yes.abs() < 1e-9);
    assert!(!state
        .open_orders
        .values()
        .any(|order| order.order_id.as_deref() == Some(oid.as_str())));
    assert!(bot
        .taker_orders
        .lock()
        .map(|orders| !orders.contains_key(oid.as_str()))
        .unwrap_or(true));
    assert!(bot
        .exchange_orders_cache
        .lock()
        .map(|rows| {
            !rows
                .iter()
                .any(|row| bot._extract_order_id(row).as_deref() == Some(oid.as_str()))
        })
        .unwrap_or(true));
    let pending = crate::helpers::load_shared_pending_taker_state(
        &bot._pending_taker_state_file(),
        bot.taker_order_ttl_seconds as f64,
    )
    .expect("shadow pending taker state");
    assert!(pending.orders.is_empty());
    let shared = crate::helpers::load_shared_gross_exposure_state(
        &bot._gross_exposure_state_file(),
        bot.cfg.gross_cap_shared_state_ttl_seconds,
    )
    .expect("shadow gross state");
    assert!(shared.pending_orders.is_empty());
}

#[test]
fn paper_maker_order_only_fills_on_touch_and_updates_remaining() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "paper".to_string();
    let oid = bot
        ._maker_order_upsert_gtc(
            &MakerOrderKey::buy("yes_asset_id"),
            0.40,
            12.0,
            "BOT_PAIR_BUILD_YES",
        )
        .expect("paper maker order");
    assert!(oid.starts_with("PAPER_INTENT_"));

    set_pair_quotes(&bot, 0.38, 0.42, 0.58, 0.62, now_ts_f64());
    bot._paper_runtime_simulate_fills(now_ts_f64());
    let before_touch = bot
        .state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert!(before_touch.q_yes.abs() < 1e-9);

    set_pair_quotes(&bot, 0.39, 0.40, 0.59, 0.60, now_ts_f64());
    bot._paper_runtime_simulate_fills(now_ts_f64());
    let after_touch = bot
        .state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert!((after_touch.q_yes - bot.cfg.min_shares).abs() < 1e-9);
    let remaining = after_touch
        .open_orders
        .get("yes_asset_id")
        .and_then(|order| order.size)
        .expect("paper maker order remaining");
    assert!((remaining - (12.0 - bot.cfg.min_shares)).abs() < 1e-9);
}

#[test]
fn non_live_cancel_clears_taker_tracking_and_pending_files() {
    let mut bot = make_bot_runtime_test_bot();
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_non_live_taker_cancel_shared_{}",
        uuid::Uuid::new_v4()
    ));
    set_shared_state_dir_override(&mut bot, &shared_dir);
    bot.configured_order_mode = "paper".to_string();
    bot.active_trade_id = Some("paper_taker_cancel_trade".to_string());
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.maker_fill_shares = 100.0;
    }
    set_daily_liquidity_state(&bot, 100.0, 0.0);
    bot._bot_runtime_refresh_daily_liquidity_counters();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.39, 0.50, 0.49, 0.60, now);

    let oid = bot
        ._place_taker_bid_fak(
            "yes_asset_id",
            0.41,
            5.0,
            Some("GTC"),
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        )
        .expect("paper taker gtc order");
    assert!(bot._cancel(oid.as_str()));
    assert!(bot
        .taker_orders
        .lock()
        .map(|orders| !orders.contains_key(oid.as_str()))
        .unwrap_or(true));
    let pending_takers = crate::helpers::load_shared_pending_taker_state(
        &bot._pending_taker_state_file(),
        bot.taker_order_ttl_seconds as f64,
    )
    .expect("pending taker state");
    assert!(pending_takers.orders.is_empty());
    let shared = crate::helpers::load_shared_gross_exposure_state(
        &bot._gross_exposure_state_file(),
        bot.cfg.gross_cap_shared_state_ttl_seconds,
    )
    .expect("paper gross state");
    assert!(shared.pending_orders.is_empty());
}

#[test]
fn audit_runtime_event_payload_includes_order_modes() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = false;
    bot.active_trade_id = Some("trade_audit_modes".to_string());
    let (tx, rx) = sync_channel(1);
    bot.audit_runtime_tx = Some(tx);

    let event_id = bot._audit_insert_runtime_event(
        "risk_block",
        None,
        None,
        None,
        None,
        Some("test_reason"),
        json!({"foo": "bar"}),
    );
    assert!(event_id.is_some());
    let task = rx.recv().expect("audit runtime task");
    let row = match task {
        AuditWriteTask::Runtime(row) => row,
        other => panic!("expected runtime audit task, got {other:?}"),
    };
    let payload: serde_json::Value =
        serde_json::from_str(&row.payload_json).expect("audit payload json");
    assert_eq!(
        payload
            .get("configured_order_mode")
            .and_then(|value| value.as_str()),
        Some("live")
    );
    assert_eq!(
        payload
            .get("effective_order_mode")
            .and_then(|value| value.as_str()),
        Some("shadow")
    );
    assert_eq!(
        payload
            .get("live_order_mode_block_reason")
            .and_then(|value| value.as_str()),
        Some("live_mode_disarmed")
    );
}

#[test]
fn startup_reconciliation_gate_clears_to_healthy_when_local_validation_is_clean() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now);
    bot._bot_runtime_mark_startup_reconciliation_pending(now);
    bot._bot_runtime_run_reconciliation_gate("startup", now + 1.0)
        .expect("startup reconciliation should pass in local-only mode");

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.safety_gate, BotRuntimeSafetyGate::Healthy);
    assert!(runtime_state.last_clean_reconcile_ts > 0.0);
    assert!(runtime_state
        .safety_gate_reason
        .contains("reconciliation_clean"));
}

#[test]
fn startup_reconciliation_republishes_recovered_live_maker_buy_after_gate_clears() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_startup_reconcile_republish_recovered_maker_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xstartuprecongrossmaker".to_string();
        bot.active_trade_id = Some("trade_startup_reconcile_republish_maker".to_string());
        let now = now_ts_f64();
        bot._bot_runtime_mark_startup_reconciliation_pending(now);

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_startup_recovered_maker_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now - 300.0),
                    submit_ts: Some(now - 300.0),
                    kind: Some("maker".to_string()),
                },
            );
        }
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.push(json!({
                "id": "oid_startup_recovered_maker_yes",
                "order_id": "oid_startup_recovered_maker_yes",
                "asset_id": "yes_asset_id",
                "token_id": "yes_asset_id",
                "side": "BUY",
                "price": 0.41,
                "size": 12.0,
                "remaining_size": 12.0,
                "status": "LIVE",
            }));
        }

        bot._bot_runtime_run_reconciliation_gate("startup", now + 1.0)
            .expect("startup reconciliation should republish recovered maker buy");

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_startup_recovered_maker_yes")
            .expect("recovered maker buy should be republished after clean gate");
        assert_eq!(reservation.kind, "maker");
        assert!((reservation.remaining_gross() - (0.41 * 12.0)).abs() < 1e-9);

        let runtime_state = bot
            .bot_runtime_state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        assert_eq!(runtime_state.safety_gate, BotRuntimeSafetyGate::Healthy);
    });
}

#[test]
fn websocket_disconnect_and_reopen_moves_runtime_through_pause_and_reconnect_gate() {
    let bot = make_bot_runtime_test_bot();
    bot._on_open("market");
    bot._on_close("market", 1006, "test_disconnect");
    let paused = bot
        .bot_runtime_state
        .lock()
        .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
        .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
    assert_eq!(paused.0, BotRuntimeSafetyGate::DependencyPaused);
    assert!(paused.1.contains("dependency_pause:market_ws"));

    bot._on_open("market");
    let reconnected = bot
        .bot_runtime_state
        .lock()
        .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
        .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
    assert_eq!(reconnected.0, BotRuntimeSafetyGate::ReconnectReconPending);
    assert!(reconnected
        .1
        .contains("reconnect_reconciliation_pending:market_ws"));
}

#[test]
fn database_dependency_pause_stays_latched_until_state_save_recovers() {
    let mut bot = make_bot_runtime_test_bot();
    let missing_dir = std::env::temp_dir()
        .join(format!(
            "bot_runtime_missing_state_dir_{}",
            uuid::Uuid::new_v4()
        ))
        .join("nested");
    bot.state_file = missing_dir.join("state.json");
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now);
    bot._bot_runtime_enter_dependency_pause("database", "test", now);

    let health_before = bot._bot_runtime_dependency_healthy();
    assert!(health_before.is_err());
    let paused = bot
        .bot_runtime_state
        .lock()
        .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
        .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
    assert_eq!(paused.0, BotRuntimeSafetyGate::DependencyPaused);
    assert!(paused.1.starts_with("dependency_pause:database"));

    std::fs::create_dir_all(&missing_dir).expect("create recovered state dir");
    bot._bot_runtime_dependency_healthy()
        .expect("database pause should clear only after state writes recover");
    bot._bot_runtime_run_reconciliation_gate("recovery", now + 1.0)
        .expect("recovery reconciliation should pass once persistence recovers");

    let recovered = bot
        .bot_runtime_state
        .lock()
        .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
        .unwrap_or((BotRuntimeSafetyGate::DependencyPaused, String::new()));
    assert_eq!(recovered.0, BotRuntimeSafetyGate::Healthy);
    assert!(recovered.1.contains("reconciliation_clean:recovery"));
}

#[test]
fn daily_liquidity_database_pause_stays_latched_until_daily_file_recovers() {
    let mut bot = make_bot_runtime_test_bot();
    let state_dir =
        std::env::temp_dir().join(format!("bot_runtime_state_ok_dir_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    bot.state_file = state_dir.join("state.json");

    let blocker_path = std::env::temp_dir().join(format!(
        "bot_runtime_daily_blocker_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&blocker_path, "block daily liquidity dir").expect("write blocker");
    bot.daily_liquidity_state_file = blocker_path.join("daily_liquidity.json");

    if let Ok(mut daily_state) = bot.daily_liquidity_state.lock() {
        daily_state.day_key_utc = crate::helpers::current_utc_day_key();
        daily_state.maker_fill_shares = 4.0;
        daily_state.taker_fill_shares = 1.0;
    }

    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now);
    bot._bot_runtime_enter_dependency_pause("database", "daily_liquidity", now);

    let health_before = bot._bot_runtime_dependency_healthy();
    assert!(health_before.is_err());
    let paused = bot
        .bot_runtime_state
        .lock()
        .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
        .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
    assert_eq!(paused.0, BotRuntimeSafetyGate::DependencyPaused);
    assert!(paused
        .1
        .starts_with("dependency_pause:database:daily_liquidity"));

    let recovered_file = std::env::temp_dir()
        .join(format!("bot_runtime_daily_ok_dir_{}", uuid::Uuid::new_v4()))
        .join("daily_liquidity.json");
    bot.daily_liquidity_state_file = recovered_file;
    bot._bot_runtime_dependency_healthy()
        .expect("daily liquidity pause should clear only after daily file writes recover");
    bot._bot_runtime_run_reconciliation_gate("recovery", now + 1.0)
        .expect("recovery reconciliation should pass once daily liquidity persistence recovers");

    let recovered = bot
        .bot_runtime_state
        .lock()
        .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
        .unwrap_or((BotRuntimeSafetyGate::DependencyPaused, String::new()));
    assert_eq!(recovered.0, BotRuntimeSafetyGate::Healthy);
    assert!(recovered.1.contains("reconciliation_clean:recovery"));
}

#[test]
fn dependency_pause_cancels_direct_bot_orders_without_single_inflight() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;

    let yes_oid = bot
        ._place_limit_bid_gtc_with_origin(
            "yes_asset_id",
            0.40,
            12.0,
            Some(true),
            "BOT_PAIR_BUILD_YES",
        )
        .expect("direct yes bot order");
    let no_oid = bot
        ._place_limit_bid_gtc_with_origin("no_asset_id", 0.41, 12.0, Some(true), "BOT_TAPER_NO")
        .expect("direct no bot order");

    assert_eq!(
        bot.state
            .lock()
            .map(|state| state.open_orders.len())
            .unwrap_or_default(),
        2
    );
    assert_eq!(
        bot.exchange_orders_cache
            .lock()
            .map(|orders| orders.len())
            .unwrap_or_default(),
        2
    );

    bot._bot_runtime_cancel_new_risk_orders("dependency_pause:test");

    assert!(bot
        .state
        .lock()
        .map(|state| state.open_orders.is_empty())
        .unwrap_or(false));
    assert!(bot
        .exchange_orders_cache
        .lock()
        .map(|orders| orders.is_empty())
        .unwrap_or(false));
    let yes_ctx = bot
        ._get_order_execution_context(&yes_oid)
        .expect("yes order context");
    let no_ctx = bot
        ._get_order_execution_context(&no_oid)
        .expect("no order context");
    assert_eq!(
        yes_ctx
            .get("direct_cancel_requested")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        no_ctx
            .get("direct_cancel_requested")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn maker_refresh_cycle_cap_blocks_second_yes_cycle_within_interval() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = true;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.40, 0.41, 0.40, 0.41, now);
    let key = MakerOrderKey::buy("yes_asset_id");

    let oid = bot
        ._maker_order_upsert_gtc(&key, 0.40, 12.0, "BOT_PAIR_BUILD_YES")
        .expect("initial yes order");
    assert!(!oid.is_empty());

    let first_cycle = bot
        ._maker_order_request_refresh_cancel(&key, "test_refresh_cycle")
        .expect("first refresh cycle should start");
    assert!(first_cycle);

    let blocked = bot
        ._maker_order_request_refresh_cancel(&key, "test_refresh_cycle")
        .expect_err("second refresh cycle should be blocked");
    assert!(blocked.starts_with("refresh_cadence_cap:YES:"));

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.yes_refresh_cycles_started, 1);
    assert_eq!(runtime_state.yes_refresh_cap_block_count, 1);
    assert_eq!(runtime_state.no_refresh_cycles_started, 0);
    assert_eq!(runtime_state.no_refresh_cap_block_count, 0);
}

#[test]
fn direct_refresh_cycle_cap_blocks_same_side_without_latency_logging() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let first_oid = bot
        ._place_limit_bid_gtc_with_origin("yes_asset_id", 0.40, 12.0, Some(true), "BOT_TAPER_YES")
        .expect("initial direct yes order");
    let second_oid = bot
        ._place_limit_bid_gtc_with_origin("yes_asset_id", 0.41, 12.0, Some(true), "BOT_TAPER_YES")
        .expect("first direct refresh cycle");
    let third_oid = bot._place_limit_bid_gtc_with_origin(
        "yes_asset_id",
        0.42,
        12.0,
        Some(true),
        "BOT_TAPER_YES",
    );

    assert_ne!(second_oid, first_oid);
    assert_eq!(third_oid.as_deref(), Some(second_oid.as_str()));
    let open_order_id = bot
        .state
        .lock()
        .ok()
        .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
        .and_then(|row| row.order_id)
        .expect("live open order after capped no-op");
    assert_eq!(open_order_id, second_oid);

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.yes_refresh_cycles_started, 1);
    assert_eq!(runtime_state.yes_refresh_cap_block_count, 1);
}

#[test]
fn direct_refresh_decision_does_not_start_cycle_before_submit_succeeds() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let first_oid = bot
        ._place_limit_bid_gtc_with_origin("yes_asset_id", 0.40, 12.0, Some(true), "BOT_TAPER_YES")
        .expect("initial direct yes order");
    assert!(!first_oid.is_empty());

    let decision =
        bot._bot_runtime_direct_refresh_decision("yes_asset_id", "BOT_TAPER_YES", now_ts_f64());
    match decision {
        MakerDirectRefreshDecision::Started(OutcomeSide::Yes) => {}
        other => panic!("expected started direct refresh decision, got {other:?}"),
    }

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.yes_refresh_cycles_started, 0);
    assert_eq!(runtime_state.yes_refresh_cycle.last_cycle_started_ts, 0.0);
}

#[test]
fn protective_refresh_cancel_bypasses_cadence_cap() {
    let mut bot = make_bot_runtime_test_bot();
    bot.configured_order_mode = "live".to_string();
    bot.live_enabled = true;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.40, 0.41, 0.40, 0.41, now);
    let key = MakerOrderKey::buy("yes_asset_id");

    let oid = bot
        ._maker_order_upsert_gtc(&key, 0.40, 12.0, "BOT_PAIR_BUILD_YES")
        .expect("initial yes order");
    assert!(!oid.is_empty());

    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.yes_refresh_cycle.last_cycle_started_ts = now_ts_f64();
        runtime_state.yes_refresh_cycle.last_origin = "BOT_PAIR_BUILD_YES".to_string();
        runtime_state.yes_refresh_cycle.last_reason = "test_recent_cycle".to_string();
    }

    let canceled = bot
        ._maker_order_request_refresh_cancel(
            &key,
            "bot_runtime_pair_build_asymmetric_submit_invalid",
        )
        .expect("protective cancel should bypass cadence cap");
    assert!(canceled);

    let slot = bot._maker_order_slot_get(&key);
    assert_eq!(slot.state, MakerOrderLifecycle::CancelPending);
    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.yes_refresh_cap_block_count, 0);
}

#[test]
fn direct_refresh_missing_context_uses_live_order_timestamp_for_cap() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let first_oid = bot
        ._place_limit_bid_gtc_with_origin("yes_asset_id", 0.40, 12.0, Some(true), "BOT_TAPER_YES")
        .expect("initial direct yes order");
    if let Ok(mut map) = bot.order_exec_context.lock() {
        map.clear();
    }

    let decision =
        bot._bot_runtime_direct_refresh_decision("yes_asset_id", "BOT_TAPER_YES", now_ts_f64());
    match decision {
        MakerDirectRefreshDecision::Blocked {
            existing_order_id,
            reason,
        } => {
            assert_eq!(existing_order_id, first_oid);
            assert!(reason.starts_with("refresh_cadence_cap:YES:"));
        }
        other => panic!("expected blocked direct refresh decision, got {other:?}"),
    }
}

#[test]
fn direct_refresh_missing_context_uses_stable_submit_ts_not_last_update_ts() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let first_oid = bot
        ._place_limit_bid_gtc_with_origin("yes_asset_id", 0.40, 12.0, Some(true), "BOT_TAPER_YES")
        .expect("initial direct yes order");
    if let Ok(mut state) = bot.state.lock() {
        if let Some(order) = state.open_orders.get_mut("yes_asset_id") {
            order.submit_ts = Some(now_ts_f64() - 5.0);
            order.ts = Some(now_ts_f64());
        }
    }
    if let Ok(mut map) = bot.order_exec_context.lock() {
        map.clear();
    }

    let decision =
        bot._bot_runtime_direct_refresh_decision("yes_asset_id", "BOT_TAPER_YES", now_ts_f64());
    match decision {
        MakerDirectRefreshDecision::Started(OutcomeSide::Yes) => {}
        other => panic!("expected started direct refresh decision, got {other:?}"),
    }

    let live_order_id = bot
        .state
        .lock()
        .ok()
        .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
        .and_then(|row| row.order_id)
        .expect("live order after stable-submit-ts check");
    assert_eq!(live_order_id, first_oid);
}

#[test]
fn user_event_replacement_does_not_inherit_previous_order_submit_ts() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let old_submit_ts = now_ts_f64() - 5.0;
    if let Ok(mut state) = bot.state.lock() {
        state.open_orders.insert(
            "yes_asset_id".to_string(),
            OpenOrderState {
                order_id: Some("old_yes_oid".to_string()),
                price: Some(0.40),
                size: Some(12.0),
                ts: Some(now_ts_f64()),
                submit_ts: Some(old_submit_ts),
                kind: None,
            },
        );
    }
    if let Ok(mut map) = bot.order_exec_context.lock() {
        map.clear();
    }

    bot._handle_user_order_event(&json!({
        "event_type": "order",
        "asset_id": "yes_asset_id",
        "order_id": "replacement_yes_oid",
        "side": "BUY",
        "type": "PLACEMENT",
        "price": 0.41,
        "original_size": 12.0,
        "size_matched": 0.0,
        "status": "LIVE",
    }));

    let replacement = bot
        .state
        .lock()
        .ok()
        .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
        .expect("replacement order in state");
    assert_eq!(replacement.order_id.as_deref(), Some("replacement_yes_oid"));
    assert!(replacement.submit_ts.unwrap_or(0.0) > old_submit_ts + 1.0);

    let decision =
        bot._bot_runtime_direct_refresh_decision("yes_asset_id", "BOT_TAPER_YES", now_ts_f64());
    match decision {
        MakerDirectRefreshDecision::Blocked { .. } => {}
        other => panic!("expected blocked direct refresh decision, got {other:?}"),
    }
}

#[test]
fn user_order_event_size_only_update_preserves_remaining_size() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_user_order_size_only_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xuserordersizeonly".to_string();
        bot.active_trade_id = Some("trade_user_order_size_only".to_string());
        bot._track_order_execution_context(
            "size_only_yes_oid",
            &json!({
                "order_id": "size_only_yes_oid",
                "asset_id": "yes_asset_id",
                "side": "BUY",
                "origin": "BOT_PAIR_BUILD_YES",
                "liquidity_intent": LiquidityIntent::Maker.as_str(),
            }),
        );

        bot._handle_user_order_event(&json!({
            "event_type": "order",
            "asset_id": "yes_asset_id",
            "order_id": "size_only_yes_oid",
            "side": "BUY",
            "type": "UPDATE",
            "price": 0.41,
            "size": 7.0,
            "size_matched": 5.0,
            "status": "LIVE",
        }));

        let open_order = bot
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
            .expect("open order from size-only update");
        assert_eq!(open_order.order_id.as_deref(), Some("size_only_yes_oid"));
        assert!((open_order.size.unwrap_or(0.0) - 7.0).abs() < 1e-9);

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("size_only_yes_oid")
            .expect("shared gross reservation for size-only update");
        assert!((reservation.size - 7.0).abs() < 1e-9);
        assert!(reservation.applied_size.abs() < 1e-9);
        assert!((reservation.remaining_size() - 7.0).abs() < 1e-9);
    });
}

#[test]
fn user_order_event_restores_taker_kind_from_pending_taker_state_without_exec_context() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_user_order_restore_taker_kind_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xuserordertakerkind".to_string();
        bot.active_trade_id = Some("trade_user_order_taker_kind".to_string());
        bot.cfg.gross_cap_include_pending_maker = false;
        bot.cfg.gross_cap_include_pending_taker = true;
        if let Ok(mut map) = bot.order_exec_context.lock() {
            map.clear();
        }

        bot._remember_shared_pending_taker_order(
            "user_event_taker_yes",
            "yes_asset_id",
            8.0,
            0.0,
            "BUY",
            now_ts_f64() - 40.0,
        );

        bot._handle_user_order_event(&json!({
            "event_type": "order",
            "asset_id": "yes_asset_id",
            "order_id": "user_event_taker_yes",
            "side": "BUY",
            "type": "UPDATE",
            "price": 0.47,
            "original_size": 8.0,
            "size_matched": 0.0,
            "status": "LIVE",
        }));

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("user_event_taker_yes")
            .expect("shared gross reservation from user order event");
        assert_eq!(reservation.kind, "taker");
        assert!((reservation.remaining_gross() - (0.47 * 8.0)).abs() < 1e-9);

        let snapshot = bot
            ._gross_cap_snapshot(0.0, &[])
            .expect("gross snapshot with restored taker kind");
        assert!(snapshot.current_pair_pending_maker_gross_usd.abs() < 1e-9);
        assert!((snapshot.current_pair_pending_taker_gross_usd - (0.47 * 8.0)).abs() < 1e-9);
    });
}

#[test]
fn user_order_event_preserves_taker_kind_from_open_order_state_after_long_restart_gap() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_user_order_restore_taker_kind_from_open_order_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xuserordertakerkindopenorder".to_string();
        bot.active_trade_id = Some("trade_user_order_taker_kind_open_order".to_string());
        bot.cfg.gross_cap_include_pending_maker = false;
        bot.cfg.gross_cap_include_pending_taker = true;

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("user_event_restart_taker_yes".to_string()),
                    price: Some(0.47),
                    size: Some(8.0),
                    ts: Some(now_ts_f64() - 300.0),
                    submit_ts: Some(now_ts_f64() - 300.0),
                    kind: Some("taker".to_string()),
                },
            );
        }
        if let Ok(mut map) = bot.order_exec_context.lock() {
            map.clear();
        }
        bot._forget_shared_gross_order_reservation("user_event_restart_taker_yes");
        bot._forget_shared_pending_taker_order("user_event_restart_taker_yes");

        bot._handle_user_order_event(&json!({
            "event_type": "order",
            "asset_id": "yes_asset_id",
            "order_id": "user_event_restart_taker_yes",
            "side": "BUY",
            "type": "UPDATE",
            "price": 0.47,
            "original_size": 8.0,
            "size_matched": 0.0,
            "status": "LIVE",
        }));

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("user_event_restart_taker_yes")
            .expect("shared gross reservation after long-gap user event");
        assert_eq!(reservation.kind, "taker");

        let open_order = bot
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
            .expect("open order after long-gap user event");
        assert_eq!(open_order.kind.as_deref(), Some("taker"));

        let snapshot = bot
            ._gross_cap_snapshot(0.0, &[])
            .expect("gross snapshot with open-order taker kind");
        assert!(snapshot.current_pair_pending_maker_gross_usd.abs() < 1e-9);
        assert!((snapshot.current_pair_pending_taker_gross_usd - (0.47 * 8.0)).abs() < 1e-9);
    });
}

#[test]
fn remember_taker_order_persists_open_order_kind_across_long_restart_gap() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_taker_accept_persist_open_order_kind_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xtakeracceptpersistkind".to_string();
        bot.active_trade_id = Some("trade_taker_accept_persist_kind".to_string());
        bot.cfg.gross_cap_include_pending_maker = false;
        bot.cfg.gross_cap_include_pending_taker = true;

        assert!(bot._remember_taker_order(
            "accept_restart_taker_yes",
            "yes_asset_id",
            8.0,
            0.47,
            "BUY",
            LiquidityIntent::TakerException,
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        ));

        let saved_open_order = bot
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
            .expect("accepted taker order saved into local open_orders");
        assert_eq!(
            saved_open_order.order_id.as_deref(),
            Some("accept_restart_taker_yes")
        );
        assert_eq!(saved_open_order.kind.as_deref(), Some("taker"));

        bot._forget_shared_gross_order_reservation("accept_restart_taker_yes");
        bot._forget_shared_pending_taker_order("accept_restart_taker_yes");

        let mut restarted = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut restarted, &shared_dir);
        restarted.cfg.dry_run = false;
        restarted.wallet_address = bot.wallet_address.clone();
        restarted.active_trade_id = Some("trade_taker_accept_persist_kind_restart".to_string());
        restarted.cfg.gross_cap_include_pending_maker = false;
        restarted.cfg.gross_cap_include_pending_taker = true;
        restarted.state_file = bot.state_file.clone();
        restarted.state = Arc::new(Mutex::new(
            crate::helpers::load_state(&restarted.state_file)
                .expect("load persisted state after restart"),
        ));
        let loaded_open_order = restarted
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
            .expect("loaded persisted taker open order");
        assert_eq!(
            loaded_open_order.order_id.as_deref(),
            Some("accept_restart_taker_yes")
        );
        assert_eq!(loaded_open_order.kind.as_deref(), Some("taker"));
        if let Ok(mut cache) = restarted.exchange_orders_cache.lock() {
            cache.push(json!({
                "id": "accept_restart_taker_yes",
                "order_id": "accept_restart_taker_yes",
                "asset_id": "yes_asset_id",
                "token_id": "yes_asset_id",
                "side": "BUY",
                "price": 0.47,
                "size": 8.0,
                "remaining_size": 8.0,
                "status": "LIVE",
            }));
        }

        restarted._reconcile_exchange_orders_for_asset("yes_asset_id", None, true);

        let open_order = restarted
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
            .expect("open order after restart reconciliation");
        assert_eq!(
            open_order.order_id.as_deref(),
            Some("accept_restart_taker_yes")
        );
        assert_eq!(open_order.kind.as_deref(), Some("taker"));

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &restarted._gross_exposure_state_file(),
            restarted.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state after restart reconciliation");
        let reservation = shared
            .pending_orders
            .get("accept_restart_taker_yes")
            .expect("reconciled shared gross reservation");
        assert_eq!(reservation.kind, "taker");

        let snapshot = restarted
            ._gross_cap_snapshot(0.0, &[])
            .expect("gross snapshot with persisted taker open-order kind");
        assert!(snapshot.current_pair_pending_maker_gross_usd.abs() < 1e-9);
        assert!((snapshot.current_pair_pending_taker_gross_usd - (0.47 * 8.0)).abs() < 1e-9);
    });
}

#[test]
fn direct_refresh_cycle_cap_blocks_family_handoff_without_single_inflight() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let first_oid = bot
        ._place_limit_bid_gtc_with_origin(
            "yes_asset_id",
            0.40,
            12.0,
            Some(true),
            "BOT_PAIR_BUILD_YES",
        )
        .expect("initial direct yes order");
    let second_oid = bot
        ._place_limit_bid_gtc_with_origin("yes_asset_id", 0.41, 12.0, Some(true), "BOT_TAPER_YES")
        .expect("direct family handoff should be cadence-capped");

    assert_eq!(second_oid, first_oid);
    let open_order_id = bot
        .state
        .lock()
        .ok()
        .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
        .and_then(|row| row.order_id)
        .expect("live open order after capped handoff");
    assert_eq!(open_order_id, first_oid);

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.yes_refresh_cycles_started, 0);
    assert_eq!(runtime_state.yes_refresh_cap_block_count, 1);
}

#[test]
fn maker_refresh_cycle_cap_blocks_family_handoff_within_interval() {
    let bot = make_bot_runtime_test_bot();
    let key = MakerOrderKey::buy("yes_asset_id");

    let live_oid = bot
        ._maker_order_upsert_gtc(&key, 0.40, 12.0, "BOT_PAIR_BUILD_YES")
        .expect("initial pair-build order");
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.yes_refresh_cycle.last_cycle_started_ts = now_ts_f64();
        runtime_state.yes_refresh_cycle.last_origin = "BOT_PAIR_BUILD_YES".to_string();
        runtime_state.yes_refresh_cycle.last_reason = "test_recent_handoff".to_string();
    }
    let third_oid = bot
        ._maker_order_upsert_gtc(&key, 0.45, 12.0, "BOT_TAPER_YES")
        .expect("capped family handoff reuses live order");

    assert_eq!(third_oid, live_oid);

    let slot = bot._maker_order_slot_get(&key);
    assert_eq!(slot.state, MakerOrderLifecycle::Working);
    assert_eq!(slot.order_id.as_deref(), Some(live_oid.as_str()));
    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.yes_refresh_cycles_started, 0);
    assert_eq!(runtime_state.yes_refresh_cap_block_count, 1);
}

#[test]
fn pair_build_submit_bookkeeping_skips_when_both_legs_are_refresh_noops() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));
    let cfg = bot_runtime_config_defaults();
    let submit_started = now_ts_f64();

    let first_yes_oid = bot
        ._place_limit_bid_gtc_with_origin(
            "yes_asset_id",
            0.40,
            12.0,
            Some(true),
            "BOT_PAIR_BUILD_YES",
        )
        .expect("initial yes direct order");
    let second_yes_oid = bot
        ._place_limit_bid_gtc_with_origin(
            "yes_asset_id",
            0.41,
            12.0,
            Some(true),
            "BOT_PAIR_BUILD_YES",
        )
        .expect("first yes refresh cycle");
    let first_no_oid = bot
        ._place_limit_bid_gtc_with_origin(
            "no_asset_id",
            0.40,
            12.0,
            Some(true),
            "BOT_PAIR_BUILD_NO",
        )
        .expect("initial no direct order");
    let second_no_oid = bot
        ._place_limit_bid_gtc_with_origin(
            "no_asset_id",
            0.41,
            12.0,
            Some(true),
            "BOT_PAIR_BUILD_NO",
        )
        .expect("first no refresh cycle");
    assert_ne!(first_yes_oid, second_yes_oid);
    assert_ne!(first_no_oid, second_no_oid);

    let decision = bot_runtime_pair_build_decision(
        60.0, 12.0, 12.0, 12.0, 12.0, 0.40, 0.42, 0.40, 0.42, 20.0, 10.0, 1.0, 1.0, 0.01, &cfg,
        false,
    )
    .expect("paired-growth decision");
    assert_eq!(decision.mode, BotRuntimePairBuildMode::PairedGrowth);
    let optional_growth_policy = bot_runtime_pair_build_optional_growth_policy(
        &decision, 12.0, 12.0, 12.0, 12.0, 0.40, 0.40, 1.0, &cfg,
    );
    let optional_buy_policy = bot_runtime_pair_build_optional_buy_policy(
        &decision,
        0.40,
        0.42,
        0.40,
        0.42,
        BotRuntimePairedCostBand::Acceptable,
        1.0,
        &cfg,
    );
    let plan = BotRuntimePairBuildPlan {
        decision,
        budget_snapshot: BotRuntimeBudgetSnapshot {
            cumulative_min_fraction: 0.0,
            cumulative_max_fraction: 1.0,
            cumulative_min_cost: 0.0,
            cumulative_max_cost: 100.0,
            remaining_to_max_cost: 100.0,
            under_min_target: false,
        },
        lighter_repair_policy: None,
        repair_reserve_policy: None,
        optional_growth_policy,
        optional_buy_policy,
        paired_cost_observation: Some((0.82, BotRuntimePairedCostBand::Acceptable)),
        bad_regime_shutdown: (false, 0.0, 0, 0),
    };
    let context = BotRuntimePairBuildMarketContext {
        yes_asset: "yes_asset_id".to_string(),
        no_asset: "no_asset_id".to_string(),
        yes_key: MakerOrderKey::buy("yes_asset_id"),
        no_key: MakerOrderKey::buy("no_asset_id"),
        yes_slot: bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id")),
        no_slot: bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id")),
        y_bid: 0.40,
        y_ask: 0.42,
        n_bid: 0.40,
        n_ask: 0.42,
    };

    bot._bot_runtime_pair_build_handle_paired_growth(
        submit_started,
        60.0,
        10.0,
        12.0,
        12.0,
        &context,
        &plan,
        &cfg,
    );

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.pair_build_last_optional_growth_submit_ts, 0.0);
    assert_eq!(runtime_state.paired_size_delta_by_state, [0.0; 5]);
    assert_eq!(runtime_state.yes_refresh_cap_block_count, 1);
    assert_eq!(runtime_state.no_refresh_cap_block_count, 1);
}

#[test]
fn maker_submit_pair_orders_preserves_refresh_noop_markers_for_direct_capped_legs() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    bot.cfg.maker_replace_min_interval_seconds = 1.0;
    bot.runtime_flags
        .insert("maker_single_inflight_per_side".to_string(), json!(false));

    let (first_yes_oid, first_no_oid) =
        bot._maker_submit_pair_orders(12, 0.40, 0.40, "GTC", Some(true), "BOT_PAIR_BUILD");
    let first_yes_oid = first_yes_oid.expect("initial yes pair order");
    let first_no_oid = first_no_oid.expect("initial no pair order");

    let (second_yes_oid, second_no_oid) =
        bot._maker_submit_pair_orders(12, 0.41, 0.41, "GTC", Some(true), "BOT_PAIR_BUILD");
    let second_yes_oid = second_yes_oid.expect("first yes refresh cycle");
    let second_no_oid = second_no_oid.expect("first no refresh cycle");

    let (third_yes_oid, third_no_oid) =
        bot._maker_submit_pair_orders(12, 0.42, 0.42, "GTC", Some(true), "BOT_PAIR_BUILD");
    let third_yes_oid = third_yes_oid.expect("capped yes pair leg");
    let third_no_oid = third_no_oid.expect("capped no pair leg");

    assert_ne!(second_yes_oid, first_yes_oid);
    assert_ne!(second_no_oid, first_no_oid);
    assert_eq!(third_yes_oid, second_yes_oid);
    assert_eq!(third_no_oid, second_no_oid);
    assert!(bot._consume_refresh_cadence_noop_marker(&third_yes_oid));
    assert!(bot._consume_refresh_cadence_noop_marker(&third_no_oid));
}

#[test]
fn stale_add_block_blocks_new_risk_without_entering_dependency_pause() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(
        &bot,
        0.42,
        0.43,
        0.41,
        0.42,
        now - (bot.cfg.market_data_stale_add_block_seconds as f64 + 0.25),
    );

    let stale = bot._bot_runtime_market_data_stale_status();
    assert_eq!(stale.stage, BotRuntimeMarketDataStaleStage::AddBlocked);
    assert!(stale.age_seconds >= bot.cfg.market_data_stale_add_block_seconds as f64);

    bot._bot_runtime_dependency_healthy()
        .expect("warning stale should not enter dependency pause on its own");
    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
        .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
    assert_eq!(runtime_state.0, BotRuntimeSafetyGate::Healthy);
    assert!(runtime_state.1.is_empty());
}

#[test]
fn hard_stale_enters_dependency_pause_and_stays_latched_until_quotes_are_fresh() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(
        &bot,
        0.42,
        0.43,
        0.41,
        0.42,
        now - (bot.cfg.market_data_stale_hard_pause_seconds as f64 + 0.25),
    );
    let stale = bot._bot_runtime_market_data_stale_status();
    assert_eq!(stale.stage, BotRuntimeMarketDataStaleStage::HardPaused);

    bot._bot_runtime_enter_dependency_pause("market_data_stale", "", now);
    let health_before = bot._bot_runtime_dependency_healthy();
    assert!(health_before.is_err());
    assert!(health_before
        .err()
        .unwrap_or_default()
        .contains("dependency_pause:market_data_stale"));

    set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now_ts_f64());
    bot._bot_runtime_dependency_healthy()
        .expect("hard stale pause should clear only once quotes are fully fresh");
}

#[test]
fn hard_stale_recovery_stays_latched_across_other_pause_reasons() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(
        &bot,
        0.42,
        0.43,
        0.41,
        0.42,
        now - (bot.cfg.market_data_stale_hard_pause_seconds as f64 + 0.25),
    );
    bot._bot_runtime_enter_dependency_pause("market_data_stale", "", now);

    set_pair_quotes(
        &bot,
        0.42,
        0.43,
        0.41,
        0.42,
        now - (bot.cfg.market_data_stale_add_block_seconds as f64 + 0.25),
    );
    bot._bot_runtime_enter_dependency_pause("market_ws", "closed", now + 0.1);

    let health = bot._bot_runtime_dependency_healthy();
    assert!(health.is_err());
    assert!(health
        .err()
        .unwrap_or_default()
        .contains("dependency_pause:market_data_stale"));
}

#[test]
fn hard_stale_does_not_overwrite_existing_database_pause() {
    let mut bot = make_bot_runtime_test_bot();
    let missing_dir = std::env::temp_dir()
        .join(format!(
            "bot_runtime_missing_state_dir_stale_precedence_{}",
            uuid::Uuid::new_v4()
        ))
        .join("nested");
    bot.state_file = missing_dir.join("state.json");
    let now = now_ts_f64();

    set_pair_quotes(
        &bot,
        0.42,
        0.43,
        0.41,
        0.42,
        now - (bot.cfg.market_data_stale_hard_pause_seconds as f64 + 0.25),
    );
    bot._bot_runtime_enter_dependency_pause("database", "test", now);

    let preserve_existing_database_pause = bot
        .bot_runtime_state
        .lock()
        .map(|st| {
            st.safety_gate == BotRuntimeSafetyGate::DependencyPaused
                && st
                    .safety_gate_reason
                    .starts_with("dependency_pause:database")
        })
        .unwrap_or(false);
    if preserve_existing_database_pause {
        if let Ok(mut st) = bot.bot_runtime_state.lock() {
            st.market_data_hard_pause_latched = true;
        }
    } else {
        bot._bot_runtime_enter_dependency_pause("market_data_stale", "", now);
    }

    let paused = bot
        .bot_runtime_state
        .lock()
        .map(|state| {
            (
                state.safety_gate,
                state.safety_gate_reason.clone(),
                state.market_data_hard_pause_latched,
            )
        })
        .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new(), false));
    assert_eq!(paused.0, BotRuntimeSafetyGate::DependencyPaused);
    assert!(paused.1.starts_with("dependency_pause:database"));
    assert!(paused.2);

    std::fs::create_dir_all(&missing_dir).expect("create recovered state dir");
    set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now - 3.25);
    let health = bot._bot_runtime_dependency_healthy();
    assert!(health.is_err());
    assert!(health
        .err()
        .unwrap_or_default()
        .contains("dependency_pause:market_data_stale"));
}

#[test]
fn quote_input_status_uses_add_block_threshold_not_legacy_compat_field() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.market_data_stale_seconds = 8;
    bot.cfg.market_data_stale_add_block_seconds = 10;
    bot.cfg.market_data_stale_hard_pause_seconds = 20;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now - 9.0);

    let (ready, reason) = bot._bot_runtime_quote_input_status();
    assert!(
        ready,
        "quotes should remain usable until add-block threshold"
    );
    assert_eq!(reason, "ok");
}

#[test]
fn direct_bot_order_cancel_still_works_when_latency_logging_is_disabled() {
    with_env_var("EXEC_LATENCY_LOG_ENABLED", Some("false"), || {
        let mut bot = make_bot_runtime_test_bot();
        bot.cfg.dry_run = false;

        let order_id = bot
            ._place_limit_bid_gtc_with_origin(
                "yes_asset_id",
                0.40,
                12.0,
                Some(true),
                "BOT_PAIR_BUILD_YES",
            )
            .expect("bot direct order");
        assert!(
            bot._get_order_execution_context(&order_id).is_some(),
            "order execution context should still be stored when latency logging is disabled"
        );

        bot._bot_runtime_cancel_new_risk_orders("dependency_pause:test");

        assert!(bot
            .state
            .lock()
            .map(|state| state.open_orders.is_empty())
            .unwrap_or(false));
        let ctx = bot
            ._get_order_execution_context(&order_id)
            .expect("order context after cancellation");
        assert_eq!(
            ctx.get("direct_cancel_requested")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    });
}

#[test]
fn identical_local_order_retry_reuses_deterministic_intent_oid() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;

    let oid_a = bot
        ._place_limit_bid_gtc_exact_with_origin(
            "yes_asset_id",
            0.40,
            12.0,
            Some(false),
            "BOT_PAIR_BUILD_YES",
        )
        .expect("first local fallback order id");
    let oid_b = bot
        ._place_limit_bid_gtc_exact_with_origin(
            "yes_asset_id",
            0.40,
            12.0,
            Some(false),
            "BOT_PAIR_BUILD_YES",
        )
        .expect("retry local fallback order id");
    let oid_c = bot
        ._place_limit_bid_gtc_exact_with_origin(
            "yes_asset_id",
            0.40,
            20.0,
            Some(false),
            "BOT_PAIR_BUILD_YES",
        )
        .expect("replacement local fallback order id");

    assert_eq!(oid_a, oid_b);
    assert_ne!(oid_a, oid_c);
}

#[test]
fn pair_gross_cap_blocks_combined_pair_seed_submit() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_pair_cap_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.cfg.pair_gross_deployed_cost_cap_usd = 8.0;
        bot.cfg.portfolio_gross_deployed_cost_cap_usd = 100.0;
        bot.active_trade_id = Some("trade_pair_cap".to_string());

        let (yes_oid, no_oid) =
            bot._maker_submit_pair_orders(12, 0.40, 0.40, "GTC", Some(true), "BOT_PAIR_BUILD");

        assert!(yes_oid.is_none());
        assert!(no_oid.is_none());
        assert!(bot
            .state
            .lock()
            .map(|state| state.open_orders.is_empty())
            .unwrap_or(false));
        assert!(bot
            .exchange_orders_cache
            .lock()
            .map(|orders| orders.is_empty())
            .unwrap_or(false));
    });
}

#[test]
fn pair_gross_cap_preapproval_counts_retained_live_pair_leg() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_pair_retained_leg_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.cfg.pair_gross_deployed_cost_cap_usd = 7.0;
        bot.cfg.portfolio_gross_deployed_cost_cap_usd = 100.0;
        bot.runtime_flags
            .insert("maker_single_inflight_per_side".to_string(), json!(false));
        bot.active_trade_id = Some("trade_pair_retained_leg".to_string());

        let yes_oid = bot
            ._place_limit_bid_gtc_with_origin(
                "yes_asset_id",
                0.50,
                12.0,
                Some(true),
                "BOT_PAIR_BUILD_YES",
            )
            .expect("existing oversized yes order");
        bot._remember_shared_gross_order_reservation(
            &yes_oid,
            "yes_asset_id",
            "BUY",
            0.50,
            12.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        );
        if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
            runtime_state.yes_refresh_cycle.last_cycle_started_ts = now_ts_f64();
            runtime_state.yes_refresh_cycle.last_origin = "BOT_PAIR_BUILD_YES".to_string();
            runtime_state.yes_refresh_cycle.last_reason = "test_recent_refresh".to_string();
        }

        let (next_yes_oid, next_no_oid) =
            bot._maker_submit_pair_orders(4, 0.50, 0.50, "GTC", Some(true), "BOT_PAIR_BUILD");

        assert!(next_yes_oid.is_none());
        assert!(next_no_oid.is_none());
        let open_orders = bot
            .state
            .lock()
            .map(|state| state.open_orders.clone())
            .unwrap_or_default();
        assert!(open_orders
            .get("yes_asset_id")
            .and_then(|order| order.order_id.clone())
            .is_some());
        assert!(!open_orders.contains_key("no_asset_id"));
    });
}

#[test]
fn pair_gross_cap_preapproval_keeps_direct_live_pair_legs_additive_until_cancelled() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_pair_replace_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.cfg.pair_gross_deployed_cost_cap_usd = 10.0;
        bot.cfg.portfolio_gross_deployed_cost_cap_usd = 100.0;
        bot.cfg.maker_replace_min_interval_seconds = 0.0;
        bot.runtime_flags
            .insert("maker_single_inflight_per_side".to_string(), json!(false));
        bot.active_trade_id = Some("trade_pair_replace".to_string());

        let (yes_oid, no_oid) =
            bot._maker_submit_pair_orders(10, 0.50, 0.50, "GTC", Some(true), "BOT_PAIR_BUILD");
        let yes_oid = yes_oid.expect("initial yes order");
        let no_oid = no_oid.expect("initial no order");
        let shared_after_first = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("shared gross state after first pair submit");
        assert!(shared_after_first
            .pending_orders
            .contains_key(yes_oid.as_str()));
        assert!(shared_after_first
            .pending_orders
            .contains_key(no_oid.as_str()));

        let (next_yes_oid, next_no_oid) =
            bot._maker_submit_pair_orders(10, 0.50, 0.50, "GTC", Some(true), "BOT_PAIR_BUILD");

        assert!(next_yes_oid.is_none());
        assert!(next_no_oid.is_none());
        let open_orders = bot
            .state
            .lock()
            .map(|state| state.open_orders.clone())
            .unwrap_or_default();
        assert_eq!(
            open_orders
                .get("yes_asset_id")
                .and_then(|order| order.order_id.clone()),
            Some(yes_oid)
        );
        assert_eq!(
            open_orders
                .get("no_asset_id")
                .and_then(|order| order.order_id.clone()),
            Some(no_oid)
        );
    });
}

#[test]
fn pair_gross_cap_preapproval_keeps_direct_refresh_legs_additive_until_cancelled() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_pair_direct_started_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.cfg.pair_gross_deployed_cost_cap_usd = 7.0;
        bot.cfg.portfolio_gross_deployed_cost_cap_usd = 100.0;
        bot.cfg.maker_replace_min_interval_seconds = 0.0;
        bot.runtime_flags
            .insert("maker_single_inflight_per_side".to_string(), json!(false));
        bot.active_trade_id = Some("trade_pair_direct_started".to_string());

        let yes_oid = bot
            ._place_limit_bid_gtc_with_origin(
                "yes_asset_id",
                0.50,
                12.0,
                Some(true),
                "BOT_PAIR_BUILD_YES",
            )
            .expect("existing direct yes order");

        let (next_yes_oid, next_no_oid) =
            bot._maker_submit_pair_orders(4, 0.50, 0.50, "GTC", Some(true), "BOT_PAIR_BUILD");

        assert!(next_yes_oid.is_none());
        assert!(next_no_oid.is_none());
        let open_orders = bot
            .state
            .lock()
            .map(|state| state.open_orders.clone())
            .unwrap_or_default();
        assert_eq!(
            open_orders
                .get("yes_asset_id")
                .and_then(|order| order.order_id.clone()),
            Some(yes_oid)
        );
        assert!(!open_orders.contains_key("no_asset_id"));
    });
}

#[test]
fn portfolio_gross_cap_blocks_second_bot_on_same_wallet() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_portfolio_cap_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let wallet = "0xsharedgrosswallet".to_string();

        let mut bot_a = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot_a, &shared_dir);
        bot_a.wallet_address = wallet.clone();
        bot_a.market_slug = "gross-other-pair".to_string();
        bot_a.pair_identity = PairIdentity {
            pair_id: canonical_pair_id_from_slug("gross-other-pair"),
            market_slug: "gross-other-pair".to_string(),
            condition_id: None,
            yes_asset_id: Some("gross_other_yes_asset_id".to_string()),
            no_asset_id: Some("gross_other_no_asset_id".to_string()),
        };
        bot_a.yes_asset = Some("gross_other_yes_asset_id".to_string());
        bot_a.no_asset = Some("gross_other_no_asset_id".to_string());
        bot_a.active_trade_id = Some("trade_a".to_string());
        if let Ok(mut state) = bot_a.state.lock() {
            state.c_yes = 15.0;
            state.c_no = 0.0;
        }
        assert!(bot_a._refresh_shared_gross_trade_snapshot());

        let mut bot_b = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot_b, &shared_dir);
        bot_b.cfg.dry_run = false;
        bot_b.wallet_address = wallet;
        bot_b.active_trade_id = Some("trade_b".to_string());
        bot_b.cfg.pair_gross_deployed_cost_cap_usd = 50.0;
        bot_b.cfg.portfolio_gross_deployed_cost_cap_usd = 16.0;

        let (yes_oid, no_oid) =
            bot_b._maker_submit_pair_orders(12, 0.10, 0.10, "GTC", Some(true), "BOT_PAIR_BUILD");

        assert!(yes_oid.is_none());
        assert!(no_oid.is_none());
        assert!(bot_b
            .state
            .lock()
            .map(|state| state.open_orders.is_empty())
            .unwrap_or(false));
    });
}

#[test]
fn single_inflight_pair_submit_does_not_consume_first_leg_after_submit_ack_publish_failure() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_pair_submit_ack_gross_publish_failure_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xpairsubmitackgrossfail".to_string();
        bot.active_trade_id = Some("trade_pair_submit_ack_gross_fail".to_string());

        let gross_state_file = bot._gross_exposure_state_file();
        if let Some(parent) = gross_state_file.parent() {
            std::fs::create_dir_all(parent).expect("create shared gross state dir");
        }
        let mut shared = crate::helpers::SharedGrossExposureState::default();
        crate::helpers::save_shared_gross_exposure_state(&gross_state_file, &mut shared)
            .expect("seed readable shared gross state");
        let mut permissions = std::fs::metadata(&gross_state_file)
            .expect("shared gross state metadata")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&gross_state_file, permissions)
            .expect("make shared gross state read-only");

        let (yes_oid, no_oid) =
            bot._maker_submit_pair_orders(12, 0.10, 0.10, "GTC", Some(true), "BOT_PAIR_BUILD");

        assert!(yes_oid.is_none());
        assert!(no_oid.is_none());
        assert!(bot
            .state
            .lock()
            .map(|state| state.open_orders.is_empty())
            .unwrap_or(false));
        assert!(bot
            .exchange_orders_cache
            .lock()
            .map(|cache| cache.is_empty())
            .unwrap_or(false));
        let paused = bot
            .bot_runtime_state
            .lock()
            .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
            .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
        assert_eq!(paused.0, BotRuntimeSafetyGate::DependencyPaused);
        assert!(paused
            .1
            .starts_with("dependency_pause:database:gross_cap_state"));
    });
}

#[test]
fn gross_cap_snapshot_prefers_local_trade_cost_over_stale_shared_copy() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_self_override_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.wallet_address = "0xgrossself".to_string();
        bot.active_trade_id = Some("trade_self_new".to_string());
        if let Ok(mut state) = bot.state.lock() {
            state.c_yes = 2.0;
            state.c_no = 3.0;
        }

        let mut shared = crate::helpers::SharedGrossExposureState::default();
        let now = now_ts_f64();
        shared.upsert_trade_snapshot(
            "trade_self_old",
            bot.pair_identity().pair_id.as_str(),
            bot._gross_cap_instance_key().as_str(),
            99.0,
            now,
        );
        shared.upsert_trade_snapshot("trade_other", "pair_other", "other-instance", 7.0, now);
        crate::helpers::save_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            &mut shared,
        )
        .expect("write stale shared gross state");

        let snapshot = bot
            ._gross_cap_snapshot(1.0, &[])
            .expect("gross snapshot should load");

        assert!((snapshot.current_pair_filled_gross_usd - 5.0).abs() < 1e-9);
        assert!((snapshot.current_portfolio_filled_gross_usd - 12.0).abs() < 1e-9);
        assert!((snapshot.projected_portfolio_gross_usd - 13.0).abs() < 1e-9);
    });
}

#[test]
fn gross_cap_snapshot_counts_same_pair_sibling_trade_with_different_instance_key() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_same_pair_sibling_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let instance_dir_a = shared_dir.join("instance_a");
        let instance_dir_b = shared_dir.join("instance_b");
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.runtime_flags.insert(
            "__instance_working_dir_override".to_string(),
            json!(instance_dir_a.to_string_lossy().to_string()),
        );
        bot.state_file = PathBuf::from("maker_hedgecap_state_same_market.json");
        bot.wallet_address = "0xgrosssamepairsibling".to_string();
        bot.active_trade_id = Some("trade_self_new".to_string());
        if let Ok(mut state) = bot.state.lock() {
            state.c_yes = 2.0;
            state.c_no = 3.0;
        }

        let mut sibling = make_bot_runtime_test_bot();
        sibling.runtime_flags.insert(
            "__instance_working_dir_override".to_string(),
            json!(instance_dir_b.to_string_lossy().to_string()),
        );
        sibling.state_file = PathBuf::from("maker_hedgecap_state_same_market.json");

        let mut shared = crate::helpers::SharedGrossExposureState::default();
        let now = now_ts_f64();
        shared.upsert_trade_snapshot(
            "trade_same_pair_sibling",
            bot.pair_identity().pair_id.as_str(),
            sibling._gross_cap_instance_key().as_str(),
            7.0,
            now,
        );
        crate::helpers::save_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            &mut shared,
        )
        .expect("write sibling same-pair shared state");

        let snapshot = bot
            ._gross_cap_snapshot(1.0, &[])
            .expect("gross snapshot should load");

        assert!((snapshot.current_pair_filled_gross_usd - 5.0).abs() < 1e-9);
        assert!((snapshot.current_portfolio_filled_gross_usd - 12.0).abs() < 1e-9);
        assert!((snapshot.projected_portfolio_gross_usd - 13.0).abs() < 1e-9);
    });
}

#[test]
fn gross_cap_snapshot_excludes_replaced_order_reservation() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_replace_exclude_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.wallet_address = "0xgrossreplace".to_string();
        bot.active_trade_id = Some("trade_replace".to_string());

        bot._remember_shared_gross_order_reservation(
            "old_yes",
            "yes_asset_id",
            "BUY",
            0.50,
            10.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        );

        let included = bot
            ._gross_cap_snapshot(5.0, &[])
            .expect("snapshot with existing reservation");
        assert!((included.current_pair_pending_maker_gross_usd - 5.0).abs() < 1e-9);
        assert!((included.current_portfolio_pending_gross_usd - 5.0).abs() < 1e-9);
        assert!((included.projected_pair_gross_usd - 10.0).abs() < 1e-9);

        let excluded = bot
            ._gross_cap_snapshot(5.0, &[String::from("old_yes")])
            .expect("snapshot excluding replaced reservation");
        assert!(excluded.current_pair_pending_maker_gross_usd.abs() < 1e-9);
        assert!(excluded.current_portfolio_pending_gross_usd.abs() < 1e-9);
        assert!((excluded.projected_pair_gross_usd - 5.0).abs() < 1e-9);
        assert!((excluded.projected_portfolio_gross_usd - 5.0).abs() < 1e-9);
    });
}

#[test]
fn reconciliation_republishes_live_bot_orders_into_shared_gross_state() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_republish_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrossrepublish".to_string();
        bot.active_trade_id = Some("trade_republish".to_string());
        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_republish_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now_ts_f64()),
                    submit_ts: Some(now_ts_f64()),
                    kind: None,
                },
            );
        }
        bot._track_order_execution_context(
            "oid_republish_yes",
            &json!({
                "order_id": "oid_republish_yes",
                "asset_id": "yes_asset_id",
                "side": "BUY",
                "origin": "BOT_PAIR_BUILD_YES",
                "liquidity_intent": LiquidityIntent::Maker.as_str(),
            }),
        );

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load republished shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_republish_yes")
            .expect("republished reservation");
        assert_eq!(reservation.trade_id, "trade_republish");
        assert_eq!(reservation.origin, "BOT_PAIR_BUILD_YES");
        assert_eq!(reservation.kind, "maker");
        assert!((reservation.price - 0.41).abs() < 1e-9);
        assert!((reservation.size - 12.0).abs() < 1e-9);
    });
}

#[test]
fn republish_shared_gross_reservations_skips_dry_run_local_orders() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_republish_dry_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.configured_order_mode = "paper".to_string();
        bot.cfg.dry_run = true;
        bot.wallet_address = "0xgrossrepublishdry".to_string();
        bot.active_trade_id = Some("trade_republish_dry".to_string());
        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("PAPER_INTENT_123".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now_ts_f64()),
                    submit_ts: Some(now_ts_f64()),
                    kind: Some("maker".to_string()),
                },
            );
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        assert!(shared.pending_orders.contains_key("PAPER_INTENT_123"));
        let live_shared = crate::helpers::load_shared_gross_exposure_state(
            &MakerHedgeCapBot::gross_exposure_state_file_for_wallet(&bot.wallet_address, "live"),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load live shared gross state");
        assert!(live_shared.pending_orders.is_empty());
    });
}

#[test]
fn dry_run_single_inflight_maker_submit_does_not_publish_shared_gross_reservation() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_dry_single_inflight_gross_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.configured_order_mode = "paper".to_string();
        bot.cfg.dry_run = true;
        bot.wallet_address = "0xdrysingleinflightgross".to_string();
        bot.active_trade_id = Some("trade_dry_single_inflight_gross".to_string());
        let key = MakerOrderKey::buy("yes_asset_id");

        let oid = bot
            ._maker_order_upsert_gtc(&key, 0.41, 12.0, "BOT_PAIR_BUILD_YES")
            .expect("dry-run maker submit");

        assert!(oid.starts_with("PAPER_INTENT_"));
        let tracked_oid = bot
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get("yes_asset_id").cloned())
            .and_then(|order| order.order_id)
            .expect("local dry-run order tracked");
        assert_eq!(tracked_oid, oid);

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        assert!(shared.pending_orders.contains_key(oid.as_str()));
        let live_shared = crate::helpers::load_shared_gross_exposure_state(
            &MakerHedgeCapBot::gross_exposure_state_file_for_wallet(&bot.wallet_address, "live"),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load live shared gross state");
        assert!(live_shared.pending_orders.is_empty());
    });
}

#[test]
fn republish_shared_gross_reservations_refreshes_all_live_direct_buy_orders() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_republish_direct_multi_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrossrepublishdirect".to_string();
        bot.active_trade_id = Some("trade_republish_direct".to_string());
        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_direct_new".to_string()),
                    price: Some(0.43),
                    size: Some(10.0),
                    ts: Some(now_ts_f64()),
                    submit_ts: Some(now_ts_f64()),
                    kind: None,
                },
            );
        }
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.push(json!({
                "id": "oid_direct_old",
                "order_id": "oid_direct_old",
                "asset_id": "yes_asset_id",
                "token_id": "yes_asset_id",
                "side": "BUY",
                "price": 0.41,
                "size": 12.0,
                "remaining_size": 12.0,
            }));
            cache.push(json!({
                "id": "oid_direct_new",
                "order_id": "oid_direct_new",
                "asset_id": "yes_asset_id",
                "token_id": "yes_asset_id",
                "side": "BUY",
                "price": 0.43,
                "size": 10.0,
                "remaining_size": 10.0,
            }));
        }
        bot._track_order_execution_context(
            "oid_direct_old",
            &json!({
                "order_id": "oid_direct_old",
                "asset_id": "yes_asset_id",
                "side": "BUY",
                "origin": "BOT_PAIR_BUILD_YES",
                "liquidity_intent": LiquidityIntent::Maker.as_str(),
            }),
        );
        bot._track_order_execution_context(
            "oid_direct_new",
            &json!({
                "order_id": "oid_direct_new",
                "asset_id": "yes_asset_id",
                "side": "BUY",
                "origin": "BOT_TAPER_YES",
                "liquidity_intent": LiquidityIntent::Maker.as_str(),
            }),
        );

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let old_order = shared
            .pending_orders
            .get("oid_direct_old")
            .expect("old direct reservation");
        let new_order = shared
            .pending_orders
            .get("oid_direct_new")
            .expect("new direct reservation");
        assert_eq!(old_order.kind, "maker");
        assert_eq!(new_order.kind, "maker");
        assert!((old_order.price - 0.41).abs() < 1e-9);
        assert!((new_order.price - 0.43).abs() < 1e-9);
        assert!((old_order.size - 12.0).abs() < 1e-9);
        assert!((new_order.size - 10.0).abs() < 1e-9);
    });
}

#[test]
fn republish_shared_gross_reservations_preserves_existing_maker_applied_progress() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_republish_preserve_applied_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrossrepublishapplied".to_string();
        bot.active_trade_id = Some("trade_republish_applied".to_string());
        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_republish_applied_yes".to_string()),
                    price: Some(0.41),
                    size: Some(7.0),
                    ts: Some(now_ts_f64()),
                    submit_ts: Some(now_ts_f64()),
                    kind: None,
                },
            );
        }
        bot._track_order_execution_context(
            "oid_republish_applied_yes",
            &json!({
                "order_id": "oid_republish_applied_yes",
                "asset_id": "yes_asset_id",
                "side": "BUY",
                "origin": "BOT_PAIR_BUILD_YES",
                "liquidity_intent": LiquidityIntent::Maker.as_str(),
            }),
        );
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_republish_applied_yes",
            "yes_asset_id",
            "BUY",
            0.41,
            12.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        ));
        bot._add_shared_gross_order_applied("oid_republish_applied_yes", 5.0);

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_republish_applied_yes")
            .expect("republished reservation");
        assert!((reservation.size - 12.0).abs() < 1e-9);
        assert!((reservation.applied_size - 5.0).abs() < 1e-9);
        assert!((reservation.remaining_size() - 7.0).abs() < 1e-9);
    });
}

#[test]
fn republish_shared_gross_reservations_preserves_existing_kind_without_exec_context() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_republish_preserve_kind_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrossrepublishkind".to_string();
        bot.active_trade_id = Some("trade_republish_kind".to_string());

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_republish_kind_yes".to_string()),
                    price: Some(0.52),
                    size: Some(9.0),
                    ts: Some(now_ts_f64()),
                    submit_ts: Some(now_ts_f64()),
                    kind: None,
                },
            );
        }
        if let Ok(mut map) = bot.order_exec_context.lock() {
            map.clear();
        }
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_republish_kind_yes",
            "yes_asset_id",
            "BUY",
            0.52,
            9.0,
            "BOT_AWAIT_SECOND_FILL_YES",
            "taker",
        ));

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_republish_kind_yes")
            .expect("republished reservation");
        assert_eq!(reservation.kind, "taker");
        assert_eq!(reservation.origin, "BOT_AWAIT_SECOND_FILL_YES");
    });
}

#[test]
fn republish_shared_gross_reservations_restores_taker_kind_from_pending_taker_state() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_republish_restore_taker_kind_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrossrestoretakerkind".to_string();
        bot.active_trade_id = Some("trade_restore_taker_kind".to_string());
        bot.cfg.gross_cap_include_pending_maker = false;
        bot.cfg.gross_cap_include_pending_taker = true;

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_restart_taker_yes".to_string()),
                    price: Some(0.47),
                    size: Some(8.0),
                    ts: Some(now_ts_f64()),
                    submit_ts: Some(now_ts_f64() - 40.0),
                    kind: None,
                },
            );
        }
        if let Ok(mut map) = bot.order_exec_context.lock() {
            map.clear();
        }
        bot._remember_shared_pending_taker_order(
            "oid_restart_taker_yes",
            "yes_asset_id",
            8.0,
            0.0,
            "BUY",
            now_ts_f64() - 40.0,
        );

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_restart_taker_yes")
            .expect("republished taker reservation");
        assert_eq!(reservation.kind, "taker");
        assert!((reservation.remaining_gross() - (0.47 * 8.0)).abs() < 1e-9);

        let snapshot = bot
            ._gross_cap_snapshot(0.0, &[])
            .expect("gross snapshot with restored taker kind");
        assert!(snapshot.current_pair_pending_maker_gross_usd.abs() < 1e-9);
        assert!((snapshot.current_pair_pending_taker_gross_usd - (0.47 * 8.0)).abs() < 1e-9);
    });
}

#[test]
fn runtime_shared_gross_refresh_republishes_live_reservations_before_ttl_expiry() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_refresh_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrossrefresh".to_string();
        bot.active_trade_id = Some("trade_gross_refresh".to_string());
        bot.cfg.gross_cap_shared_state_ttl_seconds = 3.0;
        let now = now_ts_f64();
        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_refresh_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now - 10.0),
                    submit_ts: Some(now - 10.0),
                    kind: None,
                },
            );
        }
        bot._track_order_execution_context(
            "oid_refresh_yes",
            &json!({
                "order_id": "oid_refresh_yes",
                "asset_id": "yes_asset_id",
                "side": "BUY",
                "origin": "BOT_PAIR_BUILD_YES",
                "liquidity_intent": LiquidityIntent::Maker.as_str(),
            }),
        );

        let mut last_refresh = now;
        bot._bot_runtime_refresh_shared_gross_state(now + 0.5, &mut last_refresh);
        let shared_before = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state before refresh interval");
        assert!(!shared_before.pending_orders.contains_key("oid_refresh_yes"));
        assert!((last_refresh - now).abs() < 1e-9);

        bot._bot_runtime_refresh_shared_gross_state(now + 1.1, &mut last_refresh);
        let shared_after = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state after refresh interval");
        let reservation = shared_after
            .pending_orders
            .get("oid_refresh_yes")
            .expect("republished reservation after refresh interval");
        assert_eq!(reservation.trade_id, "trade_gross_refresh");
        assert!((last_refresh - (now + 1.1)).abs() < 1e-6);
    });
}

#[test]
fn taker_buy_submit_is_blocked_by_pair_gross_cap() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_gross_taker_cap_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrosstaker".to_string();
        bot.active_trade_id = Some("trade_taker_cap".to_string());
        bot.cfg.pair_gross_deployed_cost_cap_usd = 4.0;
        bot.cfg.portfolio_gross_deployed_cost_cap_usd = 100.0;

        let oid = bot._place_taker_bid_fak(
            "yes_asset_id",
            0.40,
            12.0,
            Some("FAK"),
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        );

        assert!(oid.is_none());
        assert!(bot
            .taker_orders
            .lock()
            .map(|orders| orders.is_empty())
            .unwrap_or(false));
    });
}

#[test]
fn gross_cap_shared_state_failure_enters_dependency_pause_and_recovers() {
    let shared_root = std::env::temp_dir().join(format!(
        "bot_runtime_gross_state_blocker_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&shared_root, "blocked").expect("write blocker file");
    with_shared_state_dir(&shared_root, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_root);
        bot.wallet_address = "0xgrosspause".to_string();
        bot.active_trade_id = Some("trade_gross_pause".to_string());
        let now = now_ts_f64();
        set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now);

        assert!(!bot._refresh_shared_gross_trade_snapshot());
        let paused = bot
            .bot_runtime_state
            .lock()
            .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
            .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
        assert_eq!(paused.0, BotRuntimeSafetyGate::DependencyPaused);
        assert!(paused
            .1
            .starts_with("dependency_pause:database:gross_cap_state"));

        std::fs::remove_file(&shared_root).expect("remove blocker file");
        std::fs::create_dir_all(&shared_root).expect("create recovered shared dir");

        bot._bot_runtime_dependency_healthy()
            .expect("gross-cap shared state pause should clear only after file recovers");
        bot._bot_runtime_run_reconciliation_gate("gross_cap_recovery", now + 1.0)
            .expect("reconciliation after gross-cap recovery");
        let recovered = bot
            .bot_runtime_state
            .lock()
            .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
            .unwrap_or((BotRuntimeSafetyGate::DependencyPaused, String::new()));
        assert_eq!(recovered.0, BotRuntimeSafetyGate::Healthy);
        assert!(recovered
            .1
            .contains("reconciliation_clean:gross_cap_recovery"));
    });
}

#[test]
fn direct_submit_unwinds_when_shared_gross_reservation_publish_fails() {
    let shared_root = std::env::temp_dir().join(format!(
        "bot_runtime_gross_submit_blocker_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&shared_root, "blocked").expect("write blocker file");
    with_shared_state_dir(&shared_root, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_root);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xgrosssubmitpause".to_string();
        bot.active_trade_id = Some("trade_gross_submit_pause".to_string());
        bot.runtime_flags
            .insert("maker_single_inflight_per_side".to_string(), json!(false));

        let oid = bot._place_limit_bid_gtc_with_origin(
            "yes_asset_id",
            0.41,
            12.0,
            Some(false),
            "BOT_PAIR_BUILD_YES",
        );

        assert!(oid.is_none());
        let open_order = bot
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get("yes_asset_id").cloned());
        assert!(open_order.is_none());
        let cached_orders = bot
            .exchange_orders_cache
            .lock()
            .map(|orders| orders.clone())
            .unwrap_or_default();
        assert!(cached_orders.is_empty());
    });
}

#[test]
fn taker_buy_submit_unwinds_when_shared_gross_reservation_publish_fails() {
    let shared_root = std::env::temp_dir().join(format!(
        "bot_runtime_taker_gross_submit_blocker_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&shared_root, "blocked").expect("write blocker file");
    with_shared_state_dir(&shared_root, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_root);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xtakergrosssubmitpause".to_string();
        bot.active_trade_id = Some("trade_taker_gross_submit_pause".to_string());
        let now = now_ts_f64();
        set_pair_quotes(&bot, 0.42, 0.43, 0.41, 0.42, now);

        let oid = bot._place_taker_bid_fak(
            "yes_asset_id",
            0.41,
            12.0,
            Some("GTC"),
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        );

        assert!(oid.is_none());
        let taker_orders = bot
            .taker_orders
            .lock()
            .map(|orders| orders.clone())
            .unwrap_or_default();
        assert!(taker_orders.is_empty());
        let cached_orders = bot
            .exchange_orders_cache
            .lock()
            .map(|orders| orders.clone())
            .unwrap_or_default();
        assert!(cached_orders.is_empty());
    });
}

#[test]
fn republish_shared_gross_reservations_includes_live_taker_buys() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_taker_gross_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishtakergross".to_string();
        bot.active_trade_id = Some("trade_republish_taker_gross".to_string());
        assert!(bot._remember_taker_order(
            "oid_republish_taker_yes",
            "yes_asset_id",
            12.0,
            0.41,
            "BUY",
            LiquidityIntent::TakerException,
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        ));
        bot._forget_shared_gross_order_reservation("oid_republish_taker_yes");

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_republish_taker_yes")
            .expect("republished taker reservation");
        assert_eq!(reservation.trade_id, "trade_republish_taker_gross");
        assert_eq!(reservation.kind, "taker");
        assert_eq!(reservation.asset_id, "yes_asset_id");
        assert!((reservation.price - 0.41).abs() < 1e-9);
        assert!((reservation.size - 12.0).abs() < 1e-9);
    });
}

#[test]
fn cancel_open_order_local_forgets_taker_tracking_after_successful_cancel() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_cancel_local_taker_cleanup_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xcanceltakercleanup".to_string();
        bot.active_trade_id = Some("trade_cancel_taker_cleanup".to_string());

        assert!(bot._remember_taker_order(
            "oid_cancel_taker_yes",
            "yes_asset_id",
            12.0,
            0.41,
            "BUY",
            LiquidityIntent::TakerException,
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        ));
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.push(json!({
                "id": "oid_cancel_taker_yes",
                "order_id": "oid_cancel_taker_yes",
                "asset_id": "yes_asset_id",
                "token_id": "yes_asset_id",
                "side": "BUY",
                "price": 0.41,
                "size": 12.0,
                "remaining_size": 12.0,
                "status": "LIVE",
            }));
        }

        bot._cancel_open_order_local("yes_asset_id", "test_cancel_taker_cleanup");

        assert!(bot
            .taker_orders
            .lock()
            .map(|orders| !orders.contains_key("oid_cancel_taker_yes"))
            .unwrap_or(false));
        assert!(bot
            .state
            .lock()
            .map(|state| !state.open_orders.contains_key("yes_asset_id"))
            .unwrap_or(false));

        let pending_takers = crate::helpers::load_shared_pending_taker_state(
            &bot._pending_taker_state_file(),
            bot.taker_order_ttl_seconds as f64,
        )
        .expect("load shared pending taker state after local cancel");
        assert!(!pending_takers.orders.contains_key("oid_cancel_taker_yes"));

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state after local cancel");
        assert!(!shared.pending_orders.contains_key("oid_cancel_taker_yes"));
    });
}

#[test]
fn republish_shared_gross_reservations_forgets_expired_taker_buys() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_expired_taker_gross_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishexpiredtakergross".to_string();
        bot.active_trade_id = Some("trade_republish_expired_taker_gross".to_string());
        bot.taker_order_ttl_seconds = 5;
        assert!(bot._remember_taker_order(
            "oid_republish_expired_taker_yes",
            "yes_asset_id",
            12.0,
            0.41,
            "BUY",
            LiquidityIntent::TakerException,
            Some(TakerExceptionReason::AwaitSecondFillRescue),
            TakerCapPolicy::EnforceCap,
        ));
        if let Ok(mut orders) = bot.taker_orders.lock() {
            if let Some(record) = orders.get_mut("oid_republish_expired_taker_yes") {
                record.ts = now_ts_f64() - 10.0;
            }
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        assert!(!shared
            .pending_orders
            .contains_key("oid_republish_expired_taker_yes"));
    });
}

#[test]
fn republish_shared_gross_reservations_preserves_existing_unconfirmed_taker_buys_before_startup_reconcile(
) {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_unconfirmed_persisted_taker_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishunconfirmedtakergross".to_string();
        bot.active_trade_id = Some("trade_republish_unconfirmed_taker_gross".to_string());
        bot._bot_runtime_mark_startup_reconciliation_pending(now_ts_f64());

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_unconfirmed_taker_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now_ts_f64() - 300.0),
                    submit_ts: Some(now_ts_f64() - 300.0),
                    kind: Some("taker".to_string()),
                },
            );
        }
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_unconfirmed_taker_yes",
            "yes_asset_id",
            "BUY",
            0.41,
            12.0,
            "BOT_AWAIT_SECOND_FILL_YES",
            "taker",
        ));
        bot._forget_shared_pending_taker_order("oid_unconfirmed_taker_yes");
        if let Ok(mut takers) = bot.taker_orders.lock() {
            takers.clear();
        }
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.clear();
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_unconfirmed_taker_yes")
            .expect("existing taker reservation should be preserved until startup reconcile");
        assert_eq!(reservation.kind, "taker");
        assert!((reservation.remaining_gross() - (0.41 * 12.0)).abs() < 1e-9);
    });
}

#[test]
fn republish_shared_gross_reservations_preserves_existing_unconfirmed_maker_buys_before_startup_reconcile(
) {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_unconfirmed_persisted_maker_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishunconfirmedmakergross".to_string();
        bot.active_trade_id = Some("trade_republish_unconfirmed_maker_gross".to_string());
        bot._bot_runtime_mark_startup_reconciliation_pending(now_ts_f64());

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_unconfirmed_maker_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now_ts_f64() - 300.0),
                    submit_ts: Some(now_ts_f64() - 300.0),
                    kind: Some("maker".to_string()),
                },
            );
        }
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_unconfirmed_maker_yes",
            "yes_asset_id",
            "BUY",
            0.41,
            12.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        ));
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.clear();
        }
        if let Ok(mut ctx) = bot.order_exec_context.lock() {
            ctx.clear();
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_unconfirmed_maker_yes")
            .expect("existing maker reservation should be preserved until startup reconcile");
        assert_eq!(reservation.kind, "maker");
        assert!((reservation.remaining_gross() - (0.41 * 12.0)).abs() < 1e-9);
    });
}

#[test]
fn republish_shared_gross_reservations_does_not_create_unconfirmed_maker_buys_before_startup_reconcile(
) {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_unconfirmed_maker_no_create_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishunconfirmedmakernocreate".to_string();
        bot.active_trade_id = Some("trade_republish_unconfirmed_maker_no_create".to_string());
        bot._bot_runtime_mark_startup_reconciliation_pending(now_ts_f64());

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_unconfirmed_maker_no_create_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now_ts_f64() - 300.0),
                    submit_ts: Some(now_ts_f64() - 300.0),
                    kind: Some("maker".to_string()),
                },
            );
        }
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.clear();
        }
        if let Ok(mut ctx) = bot.order_exec_context.lock() {
            ctx.clear();
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        assert!(!shared
            .pending_orders
            .contains_key("oid_unconfirmed_maker_no_create_yes"));
    });
}

#[test]
fn republish_shared_gross_reservations_skips_stale_cached_buys_during_reconnect_reconcile() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_reconnect_stale_cache_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishreconnectgross".to_string();
        bot.active_trade_id = Some("trade_republish_reconnect_stale_cache".to_string());
        bot._bot_runtime_mark_reconnect_reconciliation_pending("market_ws", now_ts_f64());

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_reconnect_stale_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(now_ts_f64() - 300.0),
                    submit_ts: Some(now_ts_f64() - 300.0),
                    kind: Some("maker".to_string()),
                },
            );
        }
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_reconnect_stale_yes",
            "yes_asset_id",
            "BUY",
            0.41,
            12.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        ));
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.push(json!({
                "id": "oid_reconnect_stale_yes",
                "order_id": "oid_reconnect_stale_yes",
                "asset_id": "yes_asset_id",
                "token_id": "yes_asset_id",
                "side": "BUY",
                "price": 0.41,
                "size": 12.0,
                "remaining_size": 12.0,
                "status": "LIVE",
            }));
        }
        if let Ok(mut ctx) = bot.order_exec_context.lock() {
            ctx.clear();
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        assert!(!shared
            .pending_orders
            .contains_key("oid_reconnect_stale_yes"));
    });
}

#[test]
fn republish_shared_gross_reservations_ignores_stale_exec_context_during_reconnect_reconcile() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_reconnect_stale_context_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishreconnectcontext".to_string();
        bot.active_trade_id = Some("trade_republish_reconnect_stale_context".to_string());
        let disconnect_ts = now_ts_f64() - 30.0;
        bot._bot_runtime_enter_dependency_pause("market_ws", "closed", disconnect_ts);
        bot._bot_runtime_mark_reconnect_reconciliation_pending("market_ws", disconnect_ts + 5.0);

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_reconnect_context_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(disconnect_ts - 10.0),
                    submit_ts: Some(disconnect_ts - 10.0),
                    kind: Some("maker".to_string()),
                },
            );
        }
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_reconnect_context_yes",
            "yes_asset_id",
            "BUY",
            0.41,
            12.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        ));
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.clear();
        }
        if let Ok(mut ctx) = bot.order_exec_context.lock() {
            ctx.insert(
                "oid_reconnect_context_yes".to_string(),
                json!({
                    "order_id": "oid_reconnect_context_yes",
                    "origin": "BOT_PAIR_BUILD_YES",
                    "ts": disconnect_ts - 1.0,
                }),
            );
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        assert!(!shared
            .pending_orders
            .contains_key("oid_reconnect_context_yes"));
    });
}

#[test]
fn republish_shared_gross_reservations_keeps_fresh_exec_context_after_reconnect_event() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_reconnect_fresh_context_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishreconnectfreshctx".to_string();
        bot.active_trade_id = Some("trade_republish_reconnect_fresh_context".to_string());
        let disconnect_ts = now_ts_f64() - 30.0;
        let reconnect_ts = disconnect_ts + 5.0;
        bot._bot_runtime_enter_dependency_pause("market_ws", "closed", disconnect_ts);
        bot._bot_runtime_mark_reconnect_reconciliation_pending("market_ws", reconnect_ts);

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_reconnect_fresh_ctx_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(disconnect_ts - 10.0),
                    submit_ts: Some(disconnect_ts - 10.0),
                    kind: Some("maker".to_string()),
                },
            );
        }
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_reconnect_fresh_ctx_yes",
            "yes_asset_id",
            "BUY",
            0.41,
            12.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        ));
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.clear();
        }
        if let Ok(mut ctx) = bot.order_exec_context.lock() {
            ctx.insert(
                "oid_reconnect_fresh_ctx_yes".to_string(),
                json!({
                    "order_id": "oid_reconnect_fresh_ctx_yes",
                    "origin": "BOT_PAIR_BUILD_YES",
                    "ts": reconnect_ts + 1.0,
                }),
            );
        }

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_reconnect_fresh_ctx_yes")
            .expect("fresh reconnect context should preserve reservation");
        assert_eq!(reservation.kind, "maker");
        assert!((reservation.remaining_gross() - (0.41 * 12.0)).abs() < 1e-9);
    });
}

#[test]
fn republish_shared_gross_reservations_preserves_existing_maker_buy_during_partial_reconnect() {
    let shared_dir = std::env::temp_dir().join(format!(
        "bot_runtime_republish_partial_reconnect_preserve_maker_{}",
        uuid::Uuid::new_v4()
    ));
    with_shared_state_dir(&shared_dir, || {
        let mut bot = make_bot_runtime_test_bot();
        set_shared_state_dir_override(&mut bot, &shared_dir);
        bot.cfg.dry_run = false;
        bot.wallet_address = "0xrepublishpartialreconnectmaker".to_string();
        bot.active_trade_id = Some("trade_republish_partial_reconnect_maker".to_string());
        let disconnect_ts = now_ts_f64() - 30.0;
        bot._bot_runtime_enter_dependency_pause("user_ws", "closed", disconnect_ts);
        bot._bot_runtime_mark_reconnect_reconciliation_pending("market_ws", disconnect_ts + 5.0);
        bot.market_connected.store(true, Ordering::SeqCst);
        bot.user_connected.store(false, Ordering::SeqCst);

        if let Ok(mut state) = bot.state.lock() {
            state.open_orders.insert(
                "yes_asset_id".to_string(),
                OpenOrderState {
                    order_id: Some("oid_partial_reconnect_yes".to_string()),
                    price: Some(0.41),
                    size: Some(12.0),
                    ts: Some(disconnect_ts - 10.0),
                    submit_ts: Some(disconnect_ts - 10.0),
                    kind: Some("maker".to_string()),
                },
            );
        }
        assert!(bot._remember_shared_gross_order_reservation(
            "oid_partial_reconnect_yes",
            "yes_asset_id",
            "BUY",
            0.41,
            12.0,
            "BOT_PAIR_BUILD_YES",
            "maker",
        ));
        if let Ok(mut cache) = bot.exchange_orders_cache.lock() {
            cache.clear();
        }
        if let Ok(mut ctx) = bot.order_exec_context.lock() {
            ctx.clear();
        }

        let before = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state before partial reconnect refresh");
        let before_updated_ts = before
            .pending_orders
            .get("oid_partial_reconnect_yes")
            .map(|reservation| reservation.updated_ts)
            .expect("existing maker reservation before partial reconnect");
        std::thread::sleep(std::time::Duration::from_millis(20));

        assert!(bot._republish_shared_gross_reservations_from_local_state());

        let shared = crate::helpers::load_shared_gross_exposure_state(
            &bot._gross_exposure_state_file(),
            bot.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .expect("load shared gross state");
        let reservation = shared
            .pending_orders
            .get("oid_partial_reconnect_yes")
            .expect("existing maker reservation should be preserved during partial reconnect");
        assert_eq!(reservation.kind, "maker");
        assert!((reservation.remaining_gross() - (0.41 * 12.0)).abs() < 1e-9);
        assert!(reservation.updated_ts > before_updated_ts);
    });
}
/// Exercises the exec mode defaults to BOT runtime scenario and checks the expected BOT
/// behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn exec_mode_defaults_to_bot_runtime() {
    with_exec_mode(None, || {
        assert_eq!(require_bot_exec_mode().expect("default exec mode"), "BOT");
    });
}
/// Exercises the exec mode rejects unsupported modes scenario and checks the expected BOT
/// behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn exec_mode_rejects_unsupported_modes() {
    with_exec_mode(Some("SETTLEMENT_SHAPER"), || {
        let err = require_bot_exec_mode().expect_err("unsupported mode should fail");
        assert!(err.to_string().contains("Only BOT is supported"));
    });
}

#[test]
fn wallet_daily_liquidity_file_uses_shared_state_dir_override() {
    let _guard = env_lock().lock().expect("env lock");
    let prior = std::env::var("POLYBOT_SHARED_STATE_DIR").ok();
    std::env::set_var("POLYBOT_SHARED_STATE_DIR", "__shared_state_test");

    let path = MakerHedgeCapBot::daily_liquidity_state_file_for_wallet("0xAbC", "live");

    match prior {
        Some(value) => std::env::set_var("POLYBOT_SHARED_STATE_DIR", value),
        None => std::env::remove_var("POLYBOT_SHARED_STATE_DIR"),
    }

    assert_eq!(
        path,
        PathBuf::from("__shared_state_test").join("maker_hedgecap_daily_liquidity_0xabc.json")
    );
}
/// Exercises the BOT runtime phase routing covers runtime segments scenario and checks the
/// expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn bot_runtime_phase_routing_covers_runtime_segments() {
    let cfg = bot_runtime_config_defaults();
    assert_eq!(
        bot_runtime_phase_from_t_into_s(-0.1, &cfg),
        BotRuntimePhase::PreArm
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(0.0, &cfg),
        BotRuntimePhase::OpenBoth
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(29.9, &cfg),
        BotRuntimePhase::OpenBoth
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(30.0, &cfg),
        BotRuntimePhase::PairBuild
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(179.9, &cfg),
        BotRuntimePhase::PairBuild
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(180.0, &cfg),
        BotRuntimePhase::Taper
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(224.9, &cfg),
        BotRuntimePhase::Taper
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(225.0, &cfg),
        BotRuntimePhase::Taper
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(239.9, &cfg),
        BotRuntimePhase::Taper
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(240.0, &cfg),
        BotRuntimePhase::AwaitSettlement
    );
}

#[test]
fn bot_runtime_config_defaults_include_exact_open_time_targets() {
    let cfg = bot_runtime_config_defaults();
    assert_eq!(cfg.open_both_seed_deadline_seconds, 5.0);
    assert_eq!(cfg.open_both_submit_delta_max_seconds, 1.0);
    assert!(cfg.open_both_allow_single_late_seed);
    assert_eq!(cfg.imbalance_target_fraction, 0.07);
    assert_eq!(cfg.imbalance_warning_fraction, 0.12);
    assert_eq!(cfg.imbalance_disable_fraction, 0.20);
    assert_eq!(cfg.clip_ladder, [12.0, 20.0, 40.0, 80.0]);
    assert_eq!(cfg.late_reduce_start_seconds, 180.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 225.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 240.0);
}

#[test]
fn bot_runtime_validate_config_rejects_invalid_open_time_targets() {
    let mut cfg = bot_runtime_config_defaults();
    cfg.open_both_seed_deadline_seconds = 0.0;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_open_both_seed_deadline_seconds")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.open_both_submit_delta_max_seconds = 0.0;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_open_both_submit_delta_max_seconds")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.open_both_submit_delta_max_seconds = 6.0;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("open_both_submit_delta_exceeds_deadline")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.late_reduce_start_seconds = 30.0;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_late_reduce_start_seconds")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.late_balance_only_start_seconds = cfg.late_reduce_start_seconds;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_late_balance_only_start_seconds")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.late_stop_new_orders_start_seconds = cfg.late_balance_only_start_seconds;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_late_stop_new_orders_start_seconds")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.imbalance_target_fraction = 0.0;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_imbalance_target_fraction")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.imbalance_warning_fraction = cfg.imbalance_target_fraction;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_imbalance_warning_fraction")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.imbalance_disable_fraction = cfg.imbalance_warning_fraction;
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_imbalance_disable_fraction")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.clip_ladder = [12.0, 12.0, 40.0, 80.0];
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_clip_ladder")
    );

    let mut cfg = bot_runtime_config_defaults();
    cfg.clip_ladder = [12.0, 20.0, 40.0, 81.0];
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("clip_ladder_exceeds_hard_cap")
    );
}

#[test]
fn bot_runtime_config_reader_uses_bot_clip_ladder_and_ignores_legacy_split_clip_envs() {
    let env = HashMap::from([
        ("BOT_CLIP_LADDER".to_string(), "14,22,44,80".to_string()),
        ("BOT_SEED_CLIP_SMALL".to_string(), "99".to_string()),
        ("BOT_REPAIR_CLIP_SMALL".to_string(), "77".to_string()),
        ("BOT_CLIP_LADDER_LARGE".to_string(), "55,80".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.clip_ladder, [14.0, 22.0, 44.0, 80.0]);

    let legacy_only = HashMap::from([
        ("BOT_SEED_CLIP_SMALL".to_string(), "99".to_string()),
        ("BOT_REPAIR_CLIP_SMALL".to_string(), "77".to_string()),
        ("BOT_CLIP_LADDER_LARGE".to_string(), "55,80".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| legacy_only.get(key).cloned());
    assert_eq!(cfg.clip_ladder, [12.0, 20.0, 40.0, 80.0]);
}

#[test]
fn bot_runtime_config_reader_uses_late_window_env_overrides() {
    let env = HashMap::from([
        (
            "BOT_LATE_REDUCE_START_SECONDS".to_string(),
            "185".to_string(),
        ),
        (
            "BOT_LATE_BALANCE_ONLY_START_SECONDS".to_string(),
            "230".to_string(),
        ),
        (
            "BOT_LATE_STOP_NEW_ORDERS_START_SECONDS".to_string(),
            "245".to_string(),
        ),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.late_reduce_start_seconds, 185.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 230.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 245.0);
}

#[test]
fn bot_runtime_config_reader_honors_legacy_late_window_env_names_when_new_ones_absent() {
    let env = HashMap::from([
        ("BOT_TAPER_START_SECONDS".to_string(), "210".to_string()),
        ("BOT_FINAL_QUIET_SECONDS".to_string(), "20".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.late_reduce_start_seconds, 210.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 280.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 300.0);
    assert!(cfg.legacy_late_window_budget_mode);
}

#[test]
fn legacy_late_window_envs_allow_taper_start_below_30_seconds() {
    let env = HashMap::from([
        ("BOT_TAPER_START_SECONDS".to_string(), "20".to_string()),
        ("BOT_FINAL_QUIET_SECONDS".to_string(), "30".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.late_reduce_start_seconds, 20.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 270.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 300.0);
    assert!(cfg.legacy_late_window_budget_mode);
    assert_eq!(bot_runtime_validate_config(&cfg), Ok(()));
}

#[test]
fn legacy_late_window_envs_clamp_pre_taper_final_quiet_to_taper_start() {
    let env = HashMap::from([
        ("BOT_TAPER_START_SECONDS".to_string(), "240".to_string()),
        ("BOT_FINAL_QUIET_SECONDS".to_string(), "90".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.late_reduce_start_seconds, 240.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 240.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 300.0);
    assert!(cfg.legacy_late_window_budget_mode);
    assert_eq!(bot_runtime_validate_config(&cfg), Ok(()));
    assert_eq!(
        bot_runtime_taper_mode(240.0, &cfg),
        BotRuntimeTaperMode::BalanceOnly
    );
}

#[test]
fn legacy_late_window_envs_allow_balance_only_to_start_at_taper_start() {
    let env = HashMap::from([
        ("BOT_TAPER_START_SECONDS".to_string(), "240".to_string()),
        ("BOT_FINAL_QUIET_SECONDS".to_string(), "60".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.late_reduce_start_seconds, 240.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 240.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 300.0);
    assert!(cfg.legacy_late_window_budget_mode);
    assert_eq!(bot_runtime_validate_config(&cfg), Ok(()));
    assert_eq!(
        bot_runtime_taper_mode(240.0, &cfg),
        BotRuntimeTaperMode::BalanceOnly
    );
}

#[test]
fn legacy_late_window_envs_allow_zero_length_final_quiet_window() {
    let env = HashMap::from([
        ("BOT_TAPER_START_SECONDS".to_string(), "240".to_string()),
        ("BOT_FINAL_QUIET_SECONDS".to_string(), "0".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.late_reduce_start_seconds, 240.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 300.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 300.0);
    assert!(cfg.legacy_late_window_budget_mode);
    assert_eq!(bot_runtime_validate_config(&cfg), Ok(()));
}

#[test]
fn partial_late_window_migration_preserves_legacy_base_thresholds() {
    let env = HashMap::from([
        ("BOT_TAPER_START_SECONDS".to_string(), "210".to_string()),
        ("BOT_FINAL_QUIET_SECONDS".to_string(), "20".to_string()),
        (
            "BOT_LATE_STOP_NEW_ORDERS_START_SECONDS".to_string(),
            "250".to_string(),
        ),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    assert_eq!(cfg.late_reduce_start_seconds, 210.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 280.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 250.0);
    assert!(cfg.legacy_late_window_budget_mode);
    assert_eq!(
        bot_runtime_validate_config(&cfg),
        Err("invalid_late_stop_new_orders_start_seconds")
    );
}

#[test]
fn legacy_late_window_envs_preserve_old_budget_bands() {
    let env = HashMap::from([
        ("BOT_TAPER_START_SECONDS".to_string(), "240".to_string()),
        ("BOT_FINAL_QUIET_SECONDS".to_string(), "30".to_string()),
    ]);
    let cfg = bot_runtime_config_from_reader(|key| env.get(key).cloned());
    let seed_early_main =
        cfg.seed_budget_min_fraction + cfg.early_budget_min_fraction + cfg.main_budget_min_fraction;
    let with_late = seed_early_main + cfg.late_budget_min_fraction;
    let with_taper = with_late + cfg.taper_budget_min_fraction;

    assert_eq!(cfg.late_reduce_start_seconds, 240.0);
    assert_eq!(cfg.late_balance_only_start_seconds, 270.0);
    assert_eq!(cfg.late_stop_new_orders_start_seconds, 300.0);
    assert!(cfg.legacy_late_window_budget_mode);

    let (min_before_late, _) = bot_runtime_cumulative_budget_fractions(179.9, &cfg);
    let (min_at_late, _) = bot_runtime_cumulative_budget_fractions(180.0, &cfg);
    let (min_before_taper, _) = bot_runtime_cumulative_budget_fractions(239.9, &cfg);
    let (min_at_taper, _) = bot_runtime_cumulative_budget_fractions(240.0, &cfg);

    assert!((min_before_late - seed_early_main).abs() < 1e-9);
    assert!((min_at_late - with_late).abs() < 1e-9);
    assert!((min_before_taper - with_late).abs() < 1e-9);
    assert!((min_at_taper - with_taper).abs() < 1e-9);
}

#[test]
fn late_metric_labels_follow_configured_thresholds() {
    assert_eq!(
        bot_runtime_late_metric_label("fills", 180.0),
        "fills_after_180"
    );
    assert_eq!(
        bot_runtime_late_metric_label("new_orders", 225.0),
        "new_orders_after_225"
    );
    assert_eq!(
        bot_runtime_late_metric_label("fills", 230.5),
        "fills_after_230_5"
    );
    assert_eq!(
        bot_runtime_late_metric_label("late_new_orders", 300.0),
        "late_new_orders_after_300"
    );
}

#[test]
fn bot_runtime_taper_mode_uses_exact_late_window_boundaries() {
    let cfg = bot_runtime_config_defaults();
    assert_eq!(
        bot_runtime_taper_mode(180.0, &cfg),
        BotRuntimeTaperMode::ReduceClips
    );
    assert_eq!(
        bot_runtime_taper_mode(224.9, &cfg),
        BotRuntimeTaperMode::ReduceClips
    );
    assert_eq!(
        bot_runtime_taper_mode(225.0, &cfg),
        BotRuntimeTaperMode::BalanceOnly
    );
    assert_eq!(
        bot_runtime_taper_mode(239.9, &cfg),
        BotRuntimeTaperMode::BalanceOnly
    );
}

#[test]
fn taper_maintenance_decision_downshifts_paired_growth_to_min_lot() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        60.0, 40.0, 40.0, 12.0, 12.0, 0.30, 0.32, 0.30, 0.32, 500.0, 24.0, 1.0, 1.0, 0.01, &cfg,
        false,
    )
    .expect("green paired growth should start at the large rung before taper maintenance");
    assert_eq!(decision.clip, 80);

    let tapered =
        bot_runtime_taper_maintenance_decision(decision, 5.0, 40.0, 40.0, 100.0, 200.0, &cfg);
    assert_eq!(tapered.mode, BotRuntimePairBuildMode::PairedGrowth);
    assert_eq!(tapered.clip, 5);
    assert_eq!(tapered.selected_rung, BotRuntimeClipRung::Seed);
    assert_eq!(tapered.clip_bucket, "small");
    assert!(!tapered.green_time_ok);
}

#[test]
fn late_window_fill_and_submit_counters_follow_180_225_240_thresholds() {
    let cfg = bot_runtime_config_defaults();
    let mut state = BotRuntimeState::default();
    bot_runtime_note_fill_event(&mut state, 179.9, 1.0, true, &cfg);
    assert_eq!(state.late_fill_events_after_180, 0);
    assert_eq!(state.late_fill_events_after_225, 0);

    bot_runtime_note_fill_event(&mut state, 180.0, 1.0, true, &cfg);
    assert_eq!(state.late_fill_events_after_180, 1);
    assert_eq!(state.late_fill_events_after_225, 0);

    bot_runtime_note_fill_event(&mut state, 225.0, 1.0, true, &cfg);
    assert_eq!(state.late_fill_events_after_180, 2);
    assert_eq!(state.late_fill_events_after_225, 1);

    let bot = make_bot_runtime_test_bot();
    bot._bot_runtime_note_taper_submit(224.9, &cfg);
    bot._bot_runtime_note_taper_submit(225.0, &cfg);
    bot._bot_runtime_note_taper_submit(240.0, &cfg);
    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert_eq!(state.late_new_orders_after_225, 2);
    assert_eq!(state.late_new_orders_after_240, 1);
}

#[test]
fn unmatched_fraction_match_ratio_and_imbalance_state_follow_requirement_thresholds() {
    let cfg = bot_runtime_config_defaults();
    assert_eq!(unmatched_fraction(0.0, 0.0), 0.0);
    assert_eq!(match_ratio(0.0, 0.0), 1.0);
    assert!((unmatched_fraction(10.0, 10.0) - 0.0).abs() < 1e-9);
    assert!((match_ratio(10.0, 10.0) - 1.0).abs() < 1e-9);
    assert!((unmatched_fraction(12.0, 8.0) - 0.20).abs() < 1e-9);
    assert!((match_ratio(12.0, 8.0) - (8.0 / 12.0)).abs() < 1e-9);

    assert_eq!(
        bot_runtime_imbalance_state_from_fraction(0.069, &cfg),
        BotRuntimeImbalanceState::Normal
    );
    assert_eq!(
        bot_runtime_imbalance_state_from_fraction(0.07, &cfg),
        BotRuntimeImbalanceState::Throttle
    );
    assert_eq!(
        bot_runtime_imbalance_state_from_fraction(0.1200001, &cfg),
        BotRuntimeImbalanceState::Warning
    );
    assert_eq!(
        bot_runtime_imbalance_state_from_fraction(0.20, &cfg),
        BotRuntimeImbalanceState::HardDisable
    );
}

#[test]
fn projected_unmatched_fraction_math_matches_paired_and_repair_cases() {
    let paired = bot_runtime_projected_unmatched_fraction(
        BotRuntimePairBuildMode::PairedGrowth,
        None,
        10.0,
        12.0,
        8.0,
    );
    assert!((paired - (4.0 / 40.0)).abs() < 1e-9);

    let repair = bot_runtime_projected_unmatched_fraction(
        BotRuntimePairBuildMode::LighterSideFirst,
        Some(OutcomeSide::No),
        3.0,
        12.0,
        8.0,
    );
    assert!((repair - (1.0 / 23.0)).abs() < 1e-9);
    assert!(bot_runtime_order_reduces_imbalance(
        unmatched_fraction(12.0, 8.0),
        repair
    ));
}
/// Exercises the BOT runtime owner routes seed completion and taper scenario and checks the
/// expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn bot_runtime_owner_routes_await_second_fill_and_taper() {
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::OpenBoth, 10.0, 0.0, false),
        (BotRuntimeControlOwner::AwaitSecondFill, "startup_asymmetry")
    );
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::PairBuild, 12.0, 12.0, false),
        (BotRuntimeControlOwner::PairBuild, "paired_replenishment")
    );
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::Taper, 12.0, 12.0, false),
        (BotRuntimeControlOwner::Taper, "late_taper")
    );
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::AwaitSettlement, 12.0, 12.0, false),
        (BotRuntimeControlOwner::AwaitSettlement, "await_settlement")
    );
}

#[test]
fn owner_invariant_routes_any_one_sided_inventory_to_await_second_fill() {
    let one_sided_cases = [(5.0, 0.0), (0.0, 5.0), (12.0, 0.0), (0.0, 9.0)];

    for phase in [
        BotRuntimePhase::OpenBoth,
        BotRuntimePhase::PairBuild,
        BotRuntimePhase::Taper,
    ] {
        for (q_yes, q_no) in one_sided_cases {
            let (owner, reason) = bot_runtime_owner_for_snapshot(phase, q_yes, q_no, false);
            assert_eq!(
                owner,
                BotRuntimeControlOwner::AwaitSecondFill,
                "phase={phase:?} q_yes={q_yes} q_no={q_no} should never scale before both sides fill"
            );
            assert_eq!(
                reason, "startup_asymmetry",
                "phase={phase:?} q_yes={q_yes} q_no={q_no} should surface the stable asymmetry reason"
            );
        }
    }
}

#[test]
fn owner_routing_boundaries_cover_zero_live_both_live_and_startup_pause_cases() {
    let cases = [
        (
            BotRuntimePhase::OpenBoth,
            0.0,
            0.0,
            false,
            BotRuntimeControlOwner::OpenBoth,
            "seed_both_sides",
        ),
        (
            BotRuntimePhase::OpenBoth,
            4.0,
            4.0,
            false,
            BotRuntimeControlOwner::PairBuild,
            "both_sides_live",
        ),
        (
            BotRuntimePhase::PairBuild,
            0.0,
            0.0,
            false,
            BotRuntimeControlOwner::OpenBoth,
            "seed_both_sides",
        ),
        (
            BotRuntimePhase::PairBuild,
            4.0,
            4.0,
            false,
            BotRuntimeControlOwner::PairBuild,
            "paired_replenishment",
        ),
        (
            BotRuntimePhase::Taper,
            0.0,
            0.0,
            false,
            BotRuntimeControlOwner::Taper,
            "late_taper",
        ),
        (
            BotRuntimePhase::Taper,
            4.0,
            4.0,
            false,
            BotRuntimeControlOwner::Taper,
            "late_taper",
        ),
        (
            BotRuntimePhase::OpenBoth,
            4.0,
            4.0,
            true,
            BotRuntimeControlOwner::AwaitSecondFill,
            "startup_hard_paused",
        ),
        (
            BotRuntimePhase::PairBuild,
            4.0,
            4.0,
            true,
            BotRuntimeControlOwner::AwaitSecondFill,
            "startup_hard_paused",
        ),
        (
            BotRuntimePhase::Taper,
            4.0,
            4.0,
            true,
            BotRuntimeControlOwner::AwaitSecondFill,
            "startup_hard_paused",
        ),
    ];

    for (phase, q_yes, q_no, hard_paused, expected_owner, expected_reason) in cases {
        assert_eq!(
            bot_runtime_owner_for_snapshot(phase, q_yes, q_no, hard_paused),
            (expected_owner, expected_reason),
            "phase={phase:?} q_yes={q_yes} q_no={q_no} hard_paused={hard_paused}"
        );
    }
}
/// Exercises the BOT runtime open both handler only runs for open both owner scenario and
/// checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn bot_runtime_open_both_handler_only_runs_for_open_both_owner() {
    assert!(bot_runtime_should_run_open_both_handler(
        BotRuntimeControlOwner::OpenBoth
    ));
    assert!(!bot_runtime_should_run_open_both_handler(
        BotRuntimeControlOwner::AwaitSecondFill
    ));
    assert!(!bot_runtime_should_run_open_both_handler(
        BotRuntimeControlOwner::Taper
    ));
}

#[test]
fn await_second_fill_thresholds_and_rescue_helpers_follow_requirement_constants() {
    assert_eq!(bot_runtime_await_second_fill_target_seconds(), 15.0);
    assert_eq!(bot_runtime_await_second_fill_deadline_seconds(), 30.0);
    assert_eq!(
        bot_runtime_await_second_fill_missing_side(5.0, 2.0, 0.0, 0.0),
        Some(OutcomeSide::No)
    );
    assert_eq!(
        bot_runtime_await_second_fill_missing_side(0.0, 0.0, 3.0, 1.2),
        Some(OutcomeSide::Yes)
    );
    assert_eq!(
        bot_runtime_await_second_fill_missing_side(3.0, 1.2, 3.0, 1.1),
        None
    );
    assert_eq!(
        bot_runtime_await_second_fill_rescue_size(15, 9.0, 6.0, 1.0),
        Some(6)
    );
    assert_eq!(
        bot_runtime_await_second_fill_rescue_size(15, 0.5, 6.0, 1.0),
        None
    );
    let pair_sum =
        bot_runtime_await_second_fill_marginal_pair_sum(OutcomeSide::No, 5.0, 0.0, 2.0, 0.0, 0.39)
            .expect("pair sum");
    assert!((pair_sum - 0.79).abs() < 1e-9);
}

#[test]
fn startup_hard_pause_keeps_owner_in_await_second_fill_even_after_both_sides_fill() {
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::PairBuild, 4.0, 4.0, true),
        (
            BotRuntimeControlOwner::AwaitSecondFill,
            "startup_hard_paused"
        )
    );
}

#[test]
fn open_both_seed_anchor_prefers_earliest_nonzero_timestamp() {
    assert_eq!(bot_runtime_open_both_seed_anchor_ts(0.0, 0.0), 0.0);
    assert_eq!(bot_runtime_open_both_seed_anchor_ts(10.0, 0.0), 10.0);
    assert_eq!(bot_runtime_open_both_seed_anchor_ts(0.0, 12.0), 12.0);
    assert_eq!(bot_runtime_open_both_seed_anchor_ts(10.0, 12.0), 10.0);
    assert_eq!(
        bot_runtime_open_both_seed_deadline_ts(10.0, &bot_runtime_config_defaults()),
        15.0
    );
}

#[test]
fn post_open_pair_quote_status_requires_post_open_quote_timestamps() {
    let now = 105.0;
    let stale_s = 8.0;
    let open_confirmed_ts = 100.0;
    let pre_open = bot_runtime_post_open_pair_quote_status(
        Some((0.40, 0.42, 99.5)),
        Some((0.55, 0.57, 101.0)),
        open_confirmed_ts,
        now,
        stale_s,
    );
    assert_eq!(pre_open, (false, "yes_quote_pre_open".to_string()));

    let post_open = bot_runtime_post_open_pair_quote_status(
        Some((0.40, 0.42, 100.1)),
        Some((0.55, 0.57, 100.2)),
        open_confirmed_ts,
        now,
        stale_s,
    );
    assert_eq!(post_open, (true, "ok".to_string()));
}

#[test]
fn ask_snapshot_status_allows_fresh_ask_only_quotes() {
    let now = 105.0;
    let stale_s = 8.0;
    let ask_only = bot_runtime_ask_snapshot_status("NO", Some((0.0, 0.39, 104.5)), now, stale_s);
    assert_eq!(ask_only, (true, "ok".to_string()));

    let missing_ask = bot_runtime_ask_snapshot_status("NO", Some((0.0, 0.0, 104.5)), now, stale_s);
    assert_eq!(missing_ask, (false, "zero_ask_NO".to_string()));
}

#[test]
fn open_both_submit_delta_math_only_exists_after_both_first_submits() {
    assert_eq!(bot_runtime_open_both_submit_delta_ms(0.0, 101.0), None);
    let delta = bot_runtime_open_both_submit_delta_ms(100.0, 101.2).expect("delta");
    assert!((delta - 1200.0).abs() < 1e-6);
}
/// Exercises the trade metrics snapshot reports BOT runtime fields scenario and checks the
/// expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn trade_metrics_snapshot_reports_bot_runtime_fields() {
    let bot = make_bot_runtime_test_bot();
    if let Ok(mut state) = bot.state.lock() {
        state.q_yes = 4.0;
        state.q_no = 6.0;
        state.c_yes = 1.2;
        state.c_no = 2.8;
        state.seen_trade_keys = vec!["a".to_string(), "b".to_string()];
    }
    if let Ok(mut first_fill) = bot.first_entry_fill_iso.lock() {
        *first_fill = Some("2024-01-01T00:00:10Z".to_string());
    }
    if let Ok(mut first_reason) = bot.first_entry_reason.lock() {
        *first_reason = Some("BOT_ENTRY".to_string());
    }
    if let Ok(mut stop_loss) = bot.stop_loss_category.lock() {
        *stop_loss = Some("none".to_string());
    }
    if let Ok(mut exit_reason) = bot.exit_reason.lock() {
        *exit_reason = "DONE".to_string();
    }
    let snapshot = bot.trade_metrics_snapshot();
    assert_eq!(snapshot.pair_id, "bot-test");
    assert_eq!(snapshot.market_slug, "bot-test");
    assert_eq!(snapshot.yes_asset_id.as_deref(), Some("yes_asset_id"));
    assert_eq!(snapshot.no_asset_id.as_deref(), Some("no_asset_id"));
    assert_eq!(snapshot.total_cost, 4.0);
    assert_eq!(snapshot.q_yes, 4.0);
    assert_eq!(snapshot.q_no, 6.0);
    assert_eq!(snapshot.fill_count, 2);
    assert_eq!(
        snapshot.entry_time_iso.as_deref(),
        Some("2024-01-01T00:00:10Z")
    );
    assert_eq!(snapshot.entry_reason.as_deref(), Some("BOT_ENTRY"));
    assert_eq!(snapshot.stop_loss_category.as_deref(), Some("none"));
    assert_eq!(snapshot.exit_reason, "DONE");
}

#[test]
fn metrics_snapshot_reports_exact_unmatched_fraction_and_state() {
    let mut state = BotRuntimeState::default();
    state.imbalance_state = BotRuntimeImbalanceState::Warning;
    let snapshot = bot_runtime_metrics_snapshot(&state, 14.0, 10.0, 5.6, 4.0, 9.6);
    assert_eq!(snapshot.unmatched_size, 4.0);
    assert!((snapshot.unmatched_fraction - (4.0 / 24.0)).abs() < 1e-9);
    assert!((snapshot.match_ratio - (10.0 / 14.0)).abs() < 1e-9);
    assert_eq!(snapshot.imbalance_state, BotRuntimeImbalanceState::Warning);
}

#[test]
fn startup_one_sided_fill_does_not_latch_hard_disable_forever() {
    let bot = make_bot_runtime_test_bot();
    let cfg = bot_runtime_config_defaults();

    assert_eq!(
        bot._bot_runtime_note_imbalance_state(10.0, 5.0, 0.0, &cfg),
        BotRuntimeImbalanceState::Normal
    );
    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert_eq!(state.imbalance_state, BotRuntimeImbalanceState::Normal);

    assert_eq!(
        bot._bot_runtime_note_imbalance_state(12.0, 5.0, 5.0, &cfg),
        BotRuntimeImbalanceState::Normal
    );
    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert_eq!(state.imbalance_state, BotRuntimeImbalanceState::Normal);
}

#[test]
fn post_completion_hard_disable_remains_sticky() {
    let bot = make_bot_runtime_test_bot();
    let cfg = bot_runtime_config_defaults();

    assert_eq!(
        bot._bot_runtime_note_imbalance_state(20.0, 12.0, 8.0, &cfg),
        BotRuntimeImbalanceState::HardDisable
    );
    assert_eq!(
        bot._bot_runtime_note_imbalance_state(25.0, 12.0, 12.0, &cfg),
        BotRuntimeImbalanceState::HardDisable
    );
}

/// Exercises the pair identity normalization scenario and checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn pair_identity_is_present_and_carries_market_metadata() {
    let bot = make_bot_runtime_test_bot();
    let pair = bot.pair_identity();
    assert_eq!(pair.pair_id, "bot-test");
    assert_eq!(pair.market_slug, "bot-test");
    assert_eq!(pair.yes_asset_id.as_deref(), Some("yes_asset_id"));
    assert_eq!(pair.no_asset_id.as_deref(), Some("no_asset_id"));
}

/// Exercises the pair snapshot math scenario and checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn pair_snapshot_reports_position_cost_and_quote_state() {
    let bot = make_bot_runtime_test_bot();
    set_pair_quotes(&bot, 0.40, 0.42, 0.55, 0.57, 10.0);
    let snapshot =
        bot._pair_snapshot_from_inputs(BotRuntimePhase::PairBuild, 42.0, 4.0, 6.0, 1.2, 2.8);
    assert_eq!(snapshot.identity.pair_id, "bot-test");
    assert_eq!(snapshot.phase, "PairBuild");
    assert_eq!(snapshot.t_into_s, 42.0);
    assert_eq!(snapshot.total_cost, 4.0);
    assert_eq!(snapshot.paired_size, 4.0);
    assert_eq!(snapshot.unmatched_size, 2.0);
    assert_eq!(snapshot.yes_quote.map(|quote| quote.bid), Some(0.40));
    assert_eq!(snapshot.no_quote.map(|quote| quote.ask), Some(0.57));
}

/// Exercises the pair-owned fill accounting scenario and checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn apply_fill_updates_pair_owned_position_without_side_orphans() {
    let bot = make_bot_runtime_test_bot();
    assert!(bot._apply_fill("yes_asset_id", 0.40, 5.0, "fill-yes", "BUY"));
    let one_sided = bot._pair_snapshot_from_state(BotRuntimePhase::OpenBoth, 12.0);
    assert_eq!(one_sided.position.q_yes, 5.0);
    assert_eq!(one_sided.position.q_no, 0.0);
    assert_eq!(one_sided.paired_size, 0.0);
    assert_eq!(one_sided.unmatched_size, 5.0);

    assert!(bot._apply_fill("no_asset_id", 0.45, 5.0, "fill-no", "BUY"));
    let paired = bot._pair_snapshot_from_state(BotRuntimePhase::PairBuild, 18.0);
    assert_eq!(paired.position.q_yes, 5.0);
    assert_eq!(paired.position.q_no, 5.0);
    assert!((paired.total_cost - 4.25).abs() < 1e-9);
    assert_eq!(paired.paired_size, 5.0);
    assert_eq!(paired.unmatched_size, 0.0);
}

#[test]
fn apply_fill_invariant_preserves_quantity_and_cost_through_multi_fill_sequence() {
    struct FillStep {
        asset_id: &'static str,
        price: f64,
        filled: f64,
        trade_key: &'static str,
        side: &'static str,
        expected_applied: bool,
        expected_q_yes: f64,
        expected_q_no: f64,
        expected_c_yes: f64,
        expected_c_no: f64,
    }

    let bot = make_bot_runtime_test_bot();
    let steps = [
        FillStep {
            asset_id: "yes_asset_id",
            price: 0.40,
            filled: 5.0,
            trade_key: "seq-fill-1",
            side: "BUY",
            expected_applied: true,
            expected_q_yes: 5.0,
            expected_q_no: 0.0,
            expected_c_yes: 2.0,
            expected_c_no: 0.0,
        },
        FillStep {
            asset_id: "yes_asset_id",
            price: 0.40,
            filled: 5.0,
            trade_key: "seq-fill-1",
            side: "BUY",
            expected_applied: false,
            expected_q_yes: 5.0,
            expected_q_no: 0.0,
            expected_c_yes: 2.0,
            expected_c_no: 0.0,
        },
        FillStep {
            asset_id: "no_asset_id",
            price: 0.45,
            filled: 3.0,
            trade_key: "seq-fill-2",
            side: "BUY",
            expected_applied: true,
            expected_q_yes: 5.0,
            expected_q_no: 3.0,
            expected_c_yes: 2.0,
            expected_c_no: 1.35,
        },
        FillStep {
            asset_id: "no_asset_id",
            price: 0.55,
            filled: 4.0,
            trade_key: "seq-fill-3",
            side: "BUY",
            expected_applied: true,
            expected_q_yes: 5.0,
            expected_q_no: 7.0,
            expected_c_yes: 2.0,
            expected_c_no: 3.55,
        },
        FillStep {
            asset_id: "yes_asset_id",
            price: 0.60,
            filled: 2.0,
            trade_key: "seq-fill-4",
            side: "SELL",
            expected_applied: true,
            expected_q_yes: 3.0,
            expected_q_no: 7.0,
            expected_c_yes: 0.8,
            expected_c_no: 3.55,
        },
        FillStep {
            asset_id: "no_asset_id",
            price: 0.30,
            filled: 1.0,
            trade_key: "seq-fill-5",
            side: "SELL",
            expected_applied: true,
            expected_q_yes: 3.0,
            expected_q_no: 6.0,
            expected_c_yes: 0.8,
            expected_c_no: 3.25,
        },
    ];

    for step in steps {
        assert_eq!(
            bot._apply_fill(
                step.asset_id,
                step.price,
                step.filled,
                step.trade_key,
                step.side,
            ),
            step.expected_applied,
            "trade_key={} side={} should match the expected dedupe/apply outcome",
            step.trade_key,
            step.side
        );
        let snapshot = bot._pair_snapshot_from_state(BotRuntimePhase::PairBuild, 60.0);
        assert!((snapshot.position.q_yes - step.expected_q_yes).abs() < 1e-9);
        assert!((snapshot.position.q_no - step.expected_q_no).abs() < 1e-9);
        assert!((snapshot.position.c_yes - step.expected_c_yes).abs() < 1e-9);
        assert!((snapshot.position.c_no - step.expected_c_no).abs() < 1e-9);
        assert!(
            (snapshot.total_cost - (step.expected_c_yes + step.expected_c_no)).abs() < 1e-9,
            "trade_key={} should preserve total cost conservation",
            step.trade_key
        );
        assert!(
            (snapshot.paired_size - step.expected_q_yes.min(step.expected_q_no)).abs() < 1e-9,
            "trade_key={} should preserve paired size conservation",
            step.trade_key
        );
        assert!(
            (snapshot.unmatched_size - (step.expected_q_yes - step.expected_q_no).abs()).abs()
                < 1e-9,
            "trade_key={} should preserve unmatched size conservation",
            step.trade_key
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        rng_seed: proptest::test_runner::RngSeed::Fixed(0xC0DEC0DE),
        .. ProptestConfig::default()
    })]

    #[test]
    fn fill_stream_property_preserves_quantity_conservation_and_dedupe(
        ops in prop::collection::vec(
            (
                any::<bool>(),
                1u32..100u32,
                1u32..13u32,
                0u8..8u8,
                any::<bool>(),
            ),
            1..=16
        )
    ) {
        let bot = make_bot_runtime_test_bot();
        let mut seen = HashSet::<String>::new();
        let mut expected_q_yes = 0.0f64;
        let mut expected_q_no = 0.0f64;
        let mut expected_c_yes = 0.0f64;
        let mut expected_c_no = 0.0f64;

        for (idx, (is_yes, cents, shares, trade_key_ix, is_buy)) in ops.into_iter().enumerate() {
            let asset_id = if is_yes { "yes_asset_id" } else { "no_asset_id" };
            let side = if is_buy { "BUY" } else { "SELL" };
            let price = f64::from(cents) / 100.0;
            let filled = f64::from(shares);
            let trade_key = format!("prop-fill-{trade_key_ix}");
            let expected_applied = seen.insert(trade_key.clone());

            let (applied, actual_q_yes, actual_q_no, actual_c_yes, actual_c_no) = {
                let mut guard = bot.state.lock().expect("state lock");
                let applied = if guard.has_seen_trade_key(&trade_key) {
                    false
                } else {
                    guard.record_seen_trade_key(&trade_key, idx as f64 + 1.0);
                    bot._apply_fill_locked_nodedupe(&mut guard, asset_id, price, filled, side)
                        .is_some()
                };
                (applied, guard.q_yes, guard.q_no, guard.c_yes, guard.c_no)
            };

            prop_assert_eq!(applied, expected_applied);

            if expected_applied {
                let qty = if is_buy { filled } else { -filled };
                if is_yes {
                    expected_q_yes = (expected_q_yes + qty).max(0.0);
                    expected_c_yes = (expected_c_yes + price * qty).max(0.0);
                } else {
                    expected_q_no = (expected_q_no + qty).max(0.0);
                    expected_c_no = (expected_c_no + price * qty).max(0.0);
                }
            }

            prop_assert!((actual_q_yes - expected_q_yes).abs() < 1e-9);
            prop_assert!((actual_q_no - expected_q_no).abs() < 1e-9);
            prop_assert!((actual_c_yes - expected_c_yes).abs() < 1e-9);
            prop_assert!((actual_c_no - expected_c_no).abs() < 1e-9);

            let paired_qty = actual_q_yes.min(actual_q_no);
            let residual_qty = (actual_q_yes - actual_q_no).abs();
            let dominant_side_qty = actual_q_yes.max(actual_q_no);
            let total_side_qty = actual_q_yes + actual_q_no;
            let unmatched = unmatched_fraction(actual_q_yes, actual_q_no);

            prop_assert!(actual_q_yes >= -1e-9);
            prop_assert!(actual_q_no >= -1e-9);
            prop_assert!(actual_c_yes >= -1e-9);
            prop_assert!(actual_c_no >= -1e-9);
            prop_assert!((paired_qty + residual_qty - dominant_side_qty).abs() < 1e-9);
            prop_assert!(((2.0 * paired_qty) + residual_qty - total_side_qty).abs() < 1e-9);
            prop_assert!(unmatched >= -1e-9 && unmatched <= 1.0 + 1e-9);
        }
    }
}

#[test]
fn hydrate_runtime_liquidity_counters_restores_pair_and_daily_taker_history() {
    let bot = make_bot_runtime_test_bot();
    if let Ok(mut state) = bot.state.lock() {
        state.pair_total_fill_events = 3;
        state.pair_total_fill_shares = 30.0;
        state.pair_maker_fill_events = 2;
        state.pair_maker_fill_shares = 27.0;
        state.pair_taker_fill_events = 1;
        state.pair_taker_fill_shares = 3.0;
    }
    set_daily_liquidity_state(&bot, 90.0, 9.0);

    bot._hydrate_runtime_liquidity_counters_from_state();

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(runtime_state.total_fill_events, 3);
    assert!((runtime_state.maker_fill_shares - 27.0).abs() < 1e-9);
    assert!((runtime_state.taker_fill_shares - 3.0).abs() < 1e-9);
    assert!((runtime_state.daily_maker_fill_shares - 90.0).abs() < 1e-9);
    assert!((runtime_state.daily_taker_fill_shares - 9.0).abs() < 1e-9);

    let snapshot = bot._taker_share_snapshot(1.0);
    assert!((snapshot.pair_taker_share - 0.1).abs() < 1e-9);
    assert!((snapshot.daily_taker_share - (9.0 / 99.0)).abs() < 1e-9);
}

#[test]
fn shared_daily_liquidity_state_reloads_before_cap_checks_and_writes() {
    let mut bot_a = make_bot_runtime_test_bot();
    let mut bot_b = make_bot_runtime_test_bot();
    let shared_file = std::env::temp_dir().join(format!(
        "polybot_daily_liquidity_reload_{}.json",
        uuid::Uuid::new_v4()
    ));
    let _ = std::fs::remove_file(&shared_file);
    bot_a.daily_liquidity_state_file = shared_file.clone();
    bot_b.daily_liquidity_state_file = shared_file.clone();

    let mut persisted = DailyLiquidityState {
        day_key_utc: crate::helpers::current_utc_day_key(),
        maker_fill_shares: 90.0,
        taker_fill_shares: 9.0,
    };
    crate::helpers::save_daily_liquidity_state(&shared_file, &mut persisted)
        .expect("seed shared daily state");

    if let Ok(mut state) = bot_b.daily_liquidity_state.lock() {
        *state = DailyLiquidityState::default();
    }
    let (maker_qty, taker_qty) = bot_b._bot_runtime_refresh_daily_liquidity_counters();
    assert!((maker_qty - 90.0).abs() < 1e-9);
    assert!((taker_qty - 9.0).abs() < 1e-9);

    if let Ok(mut state) = bot_b.daily_liquidity_state.lock() {
        state.day_key_utc = crate::helpers::current_utc_day_key();
        state.maker_fill_shares = 0.0;
        state.taker_fill_shares = 0.0;
    }
    bot_b._record_daily_liquidity_fill_global(1.0, false, Some(now_ts_f64()));

    let reloaded = crate::helpers::load_daily_liquidity_state(&shared_file)
        .expect("reload merged shared daily state");
    assert!((reloaded.maker_fill_shares - 90.0).abs() < 1e-9);
    assert!((reloaded.taker_fill_shares - 10.0).abs() < 1e-9);

    let _ = std::fs::remove_file(&shared_file);
}

#[test]
fn taker_share_snapshot_counts_sibling_pending_orders_only_for_daily_share_when_other_pair() {
    let _guard = env_lock().lock().expect("env lock");
    let shared_dir =
        std::env::temp_dir().join(format!("polybot_shared_pending_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&shared_dir).expect("create shared pending dir");
    let prior_shared_dir = std::env::var("POLYBOT_SHARED_STATE_DIR").ok();
    std::env::set_var("POLYBOT_SHARED_STATE_DIR", &shared_dir);

    let mut bot_a = make_bot_runtime_test_bot();
    let mut bot_b = make_bot_runtime_test_bot();
    let shared_wallet = format!("0xshared{}", uuid::Uuid::new_v4().simple());
    bot_a.wallet_address = shared_wallet.clone();
    bot_b.wallet_address = shared_wallet.clone();
    bot_b.market_slug = "other-pair".to_string();
    bot_b.pair_identity = PairIdentity {
        pair_id: canonical_pair_id_from_slug("other-pair"),
        market_slug: "other-pair".to_string(),
        condition_id: None,
        yes_asset_id: Some("other_yes_asset_id".to_string()),
        no_asset_id: Some("other_no_asset_id".to_string()),
    };
    bot_b.yes_asset = Some("other_yes_asset_id".to_string());
    bot_b.no_asset = Some("other_no_asset_id".to_string());
    let daily_file =
        MakerHedgeCapBot::daily_liquidity_state_file_for_wallet(&shared_wallet, "live");
    bot_a.daily_liquidity_state_file = daily_file.clone();
    bot_b.daily_liquidity_state_file = daily_file.clone();
    set_daily_liquidity_state(&bot_a, 90.0, 0.0);
    if let Ok(mut runtime_state) = bot_a.bot_runtime_state.lock() {
        runtime_state.maker_fill_shares = 90.0;
        runtime_state.taker_fill_shares = 0.0;
    }

    assert!(bot_b._remember_taker_order(
        "shared-pending-oid",
        "other_yes_asset_id",
        9.0,
        0.40,
        "BUY",
        LiquidityIntent::TakerException,
        Some(TakerExceptionReason::AwaitSecondFillRescue),
        TakerCapPolicy::EnforceCap,
    ));

    let snapshot = bot_a._taker_share_snapshot(2.0);
    assert!((snapshot.daily_taker_share - 0.0).abs() < 1e-9);
    assert!((snapshot.projected_pair_taker_share - (2.0 / 92.0)).abs() < 1e-9);
    assert!((snapshot.projected_daily_taker_share - (11.0 / 101.0)).abs() < 1e-9);

    bot_b._forget_taker_order("shared-pending-oid");
    let _ = std::fs::remove_file(MakerHedgeCapBot::pending_taker_state_file_for_wallet(
        &shared_wallet,
        "live",
    ));
    let _ = std::fs::remove_file(&daily_file);
    let _ = std::fs::remove_dir_all(&shared_dir);
    match prior_shared_dir {
        Some(value) => std::env::set_var("POLYBOT_SHARED_STATE_DIR", value),
        None => std::env::remove_var("POLYBOT_SHARED_STATE_DIR"),
    }
}

#[test]
fn apply_fill_with_fill_ts_attributes_daily_liquidity_to_fill_day() {
    let bot = make_bot_runtime_test_bot();
    let fill_ts = now_ts_f64() + 86_400.0;
    let expected_day_key = crate::helpers::utc_day_key_from_ts(fill_ts);
    if let Ok(mut daily_state) = bot.daily_liquidity_state.lock() {
        daily_state.day_key_utc = crate::helpers::current_utc_day_key();
        daily_state.maker_fill_shares = 0.0;
        daily_state.taker_fill_shares = 0.0;
        let _ = crate::helpers::save_daily_liquidity_state(
            &bot.daily_liquidity_state_file,
            &mut daily_state,
        );
    }

    assert!(bot._apply_fill_with_fill_ts(
        "yes_asset_id",
        0.40,
        5.0,
        "fill-with-ts",
        "BUY",
        Some(fill_ts),
        None,
        None,
    ));

    let state = bot.state.lock().expect("state lock");
    assert_eq!(state.pair_taker_fill_events, 1);
    assert!((state.pair_taker_fill_shares - 5.0).abs() < 1e-9);
    drop(state);

    let daily_state = bot
        .daily_liquidity_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert_eq!(daily_state.day_key_utc, expected_day_key);
    assert_eq!(daily_state.maker_fill_shares, 0.0);
    assert!((daily_state.taker_fill_shares - 5.0).abs() < 1e-9);
}

#[test]
fn await_settlement_handler_requests_cancel_then_exits_with_stable_reason() {
    let bot = make_bot_runtime_test_bot();
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-yes".to_string()),
                origin: "BOT_PAIR_BUILD_YES".to_string(),
                last_submit_ts: 10.0,
                ..MakerOrderSlot::default()
            },
        );
    }
    assert!(!bot._bot_runtime_await_settlement_handler(100.0, 8.0));
    let slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    assert_eq!(slot.state, MakerOrderLifecycle::CancelPending);
    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();
    assert!(runtime_state.await_settlement_cancel_requested);
    assert_eq!(runtime_state.await_settlement_started_ts, 100.0);
    assert_eq!(runtime_state.await_settlement_orders_cleared_ts, 0.0);
    assert_eq!(bot._get_exit_reason(), "RUNNING");

    assert!(bot._bot_runtime_await_settlement_handler(104.5, 3.5));
    assert_eq!(bot._get_exit_reason(), "AWAIT_SETTLEMENT");
}

#[test]
fn post_order_compat_rejects_bot_strategy_sell_origin() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    let rejected = bot._post_order_compat(
        &json!({
            "asset_id": "yes_asset_id",
            "side": "SELL",
            "price": 0.40,
            "size": 3.0,
            "origin": "BOT_TAPER_EXIT",
        }),
        "FAK",
        None,
    );
    assert!(rejected.is_none());

    let allowed = bot._post_order_compat(
        &json!({
            "asset_id": "yes_asset_id",
            "side": "SELL",
            "price": 0.40,
            "size": 3.0,
            "origin": "TAKER_FAK_SELL",
        }),
        "FAK",
        None,
    );
    assert!(allowed.is_some());
}

#[test]
fn imbalance_repair_unavailable_cancels_live_taper_orders() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.10, 0.12, 0.10, 0.12, now);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-yes".to_string()),
                origin: "BOT_TAPER_YES".to_string(),
                last_submit_ts: 200.0,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-no".to_string()),
                origin: "BOT_TAPER_NO".to_string(),
                last_submit_ts: 200.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(200.0, 200.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn taper_handler_blocks_balanced_add_at_stop_add_zone_after_runtime_gating() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.max_total_cost = 500.0;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.50, 0.52, 0.50, 0.52, now);

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(200.0, 200.0, 12.0, 20.0, 20.0, 6.0, 6.0, &cfg);

    let state = bot.state.lock().expect("bot state");
    assert!(!state.open_orders.contains_key("yes_asset_id"));
    assert!(!state.open_orders.contains_key("no_asset_id"));
    drop(state);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert!(
        runtime_state
            .taper_last_hold_reason
            .starts_with("hold:price_zone_stop_add:balanced_add:1.000"),
        "actual_reason={}",
        runtime_state.taper_last_hold_reason
    );
}

#[test]
fn rebalance_price_zone_hold_cancels_live_taper_lighter_order() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.max_total_cost = 100.0;
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.20, 0.22, 0.70, 0.72, now);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-lighter-yes".to_string()),
                origin: "BOT_TAPER_LIGHTER".to_string(),
                last_submit_ts: 240.0,
                price: 0.20,
                remaining: 8.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(200.0, 200.0, 52.8, 40.0, 48.0, 12.0, 40.8, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert!(
        runtime_state
            .taper_last_hold_reason
            .contains("price_zone_danger:rebalance_add:1.050"),
        "actual_reason={}",
        runtime_state.taper_last_hold_reason
    );
}

#[test]
fn balance_only_window_cancels_live_taper_growth_orders_before_new_repair_work() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.30, 0.32, 0.30, 0.32, now);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-yes".to_string()),
                origin: "BOT_TAPER_YES".to_string(),
                last_submit_ts: 220.0,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-no".to_string()),
                origin: "BOT_TAPER_NO".to_string(),
                last_submit_ts: 220.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(230.0, 230.0, 12.0, 20.0, 20.0, 6.0, 6.0, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert_eq!(
        runtime_state.taper_last_hold_reason,
        "rest:late_balance_only_growth_handoff"
    );
}

#[test]
fn late_action_hold_cancels_stale_taper_lighter_orders_after_inventory_rebalances() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.30, 0.32, 0.30, 0.32, now);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-lighter-yes".to_string()),
                origin: "BOT_TAPER_LIGHTER".to_string(),
                last_submit_ts: 220.0,
                price: 0.30,
                remaining: 5.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(230.0, 230.0, 12.0, 20.0, 20.0, 6.0, 6.0, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert_eq!(
        runtime_state.taper_last_hold_reason, "rest:stale_lighter_repair_balanced",
        "actual_reason={}",
        runtime_state.taper_last_hold_reason
    );
}

#[test]
fn imbalance_hold_keeps_live_taper_lighter_repair_orders() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.10, 0.12, 0.10, 0.12, now);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-lighter-yes".to_string()),
                origin: "BOT_TAPER_LIGHTER".to_string(),
                last_submit_ts: 200.0,
                price: 0.10,
                remaining: 0.50,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-no".to_string()),
                origin: "BOT_TAPER_NO".to_string(),
                last_submit_ts: 200.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(200.0, 200.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::Working);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn imbalance_hold_cancels_oversized_live_taper_lighter_repair() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.10, 0.12, 0.10, 0.12, now);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-lighter-yes".to_string()),
                origin: "BOT_TAPER_LIGHTER".to_string(),
                last_submit_ts: 200.0,
                price: 0.10,
                remaining: 1.50,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-no".to_string()),
                origin: "BOT_TAPER_NO".to_string(),
                last_submit_ts: 200.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(200.0, 200.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn imbalance_hold_cancels_wrong_side_live_taper_lighter_repair_after_side_flip() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.10, 0.12, 0.10, 0.12, now);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-lighter-yes".to_string()),
                origin: "BOT_TAPER_LIGHTER".to_string(),
                last_submit_ts: 200.0,
                price: 0.10,
                remaining: 0.50,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-taper-no".to_string()),
                origin: "BOT_TAPER_NO".to_string(),
                last_submit_ts: 200.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(200.0, 200.0, 0.60, 3.5, 2.5, 0.35, 0.25, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn prearm_ready_before_open_and_open_confirmed_are_recorded() {
    let mut bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    bot.start_ts = now.ceil() as i64 + 5;
    bot.expiry_ts = bot.start_ts + 300;
    bot.condition_id = Some("condition-test".to_string());
    set_pair_quotes(&bot, 0.40, 0.42, 0.55, 0.57, now);

    let status = bot._bot_runtime_prearm_status(-1.0);
    assert!(status.ready);
    bot._bot_runtime_note_prearm_ready_before_open();
    assert!(bot
        .bot_runtime_state
        .lock()
        .map(|st| st.prearm_ready_before_open)
        .unwrap_or(false));

    assert!(bot._bot_runtime_note_open_confirmed(now + 5.0));
    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert!((state.open_confirmed_ts - (now + 5.0)).abs() < 1e-9);
}

#[test]
fn first_tradable_post_open_ignores_pre_open_quotes() {
    let mut bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    bot.start_ts = now.floor() as i64;
    bot.expiry_ts = bot.start_ts + 300;
    bot.condition_id = Some("condition-test".to_string());
    bot._bot_runtime_note_open_confirmed(now);

    set_pair_quotes(&bot, 0.40, 0.42, 0.55, 0.57, now - 0.5);
    assert!(!bot._bot_runtime_note_first_tradable_post_open(now + 0.1));

    set_pair_quotes(&bot, 0.40, 0.42, 0.55, 0.57, now + 0.2);
    assert!(bot._bot_runtime_note_first_tradable_post_open(now + 0.3));
}

#[test]
fn open_both_handler_rejects_pre_open_cached_quotes_even_when_startup_pair_status_passes() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    let now = now_ts_f64();
    bot.start_ts = now.floor() as i64;
    bot.expiry_ts = bot.start_ts + 300;
    bot.condition_id = Some("condition-test".to_string());
    set_pair_quotes(&bot, 0.40, 0.42, 0.55, 0.57, now - 0.2);
    if let Ok(mut st) = bot.bot_runtime_state.lock() {
        st.open_confirmed_ts = now;
        st.open_both_seed_anchor_ts = now;
    }

    bot._bot_runtime_open_both_handler(
        now + 0.1,
        now + 0.1 - bot.start_ts as f64,
        0.0,
        0.0,
        0.0,
        &bot_runtime_config_defaults(),
    );

    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert_eq!(state.open_both_attempt_count, 0);
    assert_eq!(state.open_both_first_tradable_post_open_ts, 0.0);
    assert_eq!(
        state.open_both_last_hold_reason,
        "post_open_quotes_unready:yes_quote_pre_open"
    );
}

#[test]
fn open_both_submit_timing_kpis_track_same_cycle_submits() {
    let bot = make_bot_runtime_test_bot();
    let cfg = bot_runtime_config_defaults();
    let open_ts = 100.0;
    let deadline_ts = open_ts + cfg.open_both_seed_deadline_seconds;

    let (attempts, first_submit) =
        bot._bot_runtime_note_open_both_submit(open_ts + 2.0, true, true, deadline_ts, &cfg);
    assert_eq!(attempts, 1);
    assert!(first_submit);

    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert_eq!(state.open_both_first_yes_submit_ts, open_ts + 2.0);
    assert_eq!(state.open_both_first_no_submit_ts, open_ts + 2.0);
    assert_eq!(state.open_both_first_submit_delta_ms, 0.0);
    assert!(state.open_both_seed_by_deadline_met);
    assert!(state.open_both_submit_delta_met);
}

#[test]
fn open_both_submit_timing_distinguishes_deadline_vs_delta_failures() {
    let bot = make_bot_runtime_test_bot();
    let cfg = bot_runtime_config_defaults();
    let open_ts = 100.0;
    let deadline_ts = open_ts + cfg.open_both_seed_deadline_seconds;

    let _ = bot._bot_runtime_note_open_both_submit(open_ts + 0.5, true, false, deadline_ts, &cfg);
    let _ = bot._bot_runtime_note_open_both_submit(open_ts + 1.7, false, true, deadline_ts, &cfg);
    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert!(state.open_both_seed_by_deadline_met);
    assert!(!state.open_both_submit_delta_met);
    assert!((state.open_both_first_submit_delta_ms - 1200.0).abs() < 1e-6);

    let bot = make_bot_runtime_test_bot();
    let _ = bot._bot_runtime_note_open_both_submit(open_ts + 0.5, true, false, deadline_ts, &cfg);
    let _ = bot._bot_runtime_note_open_both_submit(open_ts + 5.6, false, true, deadline_ts, &cfg);
    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert!(!state.open_both_seed_by_deadline_met);
    assert!(!state.open_both_submit_delta_met);
}

#[test]
fn late_seed_unlock_can_only_be_granted_once_after_deadline_miss() {
    let bot = make_bot_runtime_test_bot();
    let open_ts = 100.0;
    let deadline_ts = open_ts + bot_runtime_config_defaults().open_both_seed_deadline_seconds;

    bot._bot_runtime_note_open_both_deadline_miss(open_ts + 6.0, deadline_ts);
    assert!(bot._bot_runtime_unlock_late_seed_once(open_ts + 6.0, deadline_ts));
    assert!(!bot._bot_runtime_unlock_late_seed_once(open_ts + 6.2, deadline_ts));

    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert_eq!(state.open_both_seed_deadline_missed_ts, open_ts + 6.0);
    assert!(state.open_both_late_seed_unlock_used);
    assert!(!state.open_both_late_seed_exhausted);
}

#[test]
fn late_seed_exhaustion_blocks_repeated_unlocks() {
    let bot = make_bot_runtime_test_bot();
    let open_ts = 100.0;
    let deadline_ts = open_ts + bot_runtime_config_defaults().open_both_seed_deadline_seconds;

    assert!(bot._bot_runtime_unlock_late_seed_once(open_ts + 6.0, deadline_ts));
    bot._bot_runtime_mark_late_seed_exhausted(open_ts + 6.1);
    assert!(!bot._bot_runtime_unlock_late_seed_once(open_ts + 6.2, deadline_ts));

    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert!(state.open_both_late_seed_unlock_used);
    assert!(state.open_both_late_seed_exhausted);
}

#[test]
fn open_both_missing_leg_followup_does_not_require_late_unlock_once_one_side_exists() {
    let bot = make_bot_runtime_test_bot();
    let cfg = bot_runtime_config_defaults();
    let open_ts = 100.0;
    let deadline_ts = open_ts + cfg.open_both_seed_deadline_seconds;
    if let Ok(mut st) = bot.bot_runtime_state.lock() {
        st.open_confirmed_ts = open_ts;
        st.open_both_seed_anchor_ts = open_ts;
        st.open_both_first_yes_submit_ts = open_ts + 2.0;
        st.open_both_first_submit_ts = open_ts + 2.0;
    }

    let _ = bot._bot_runtime_note_open_both_submit(open_ts + 6.0, false, true, deadline_ts, &cfg);
    let state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert!(!state.open_both_late_seed_unlock_used);
    assert!(state.open_both_first_no_submit_ts > 0.0);
    assert!(!state.open_both_seed_by_deadline_met);
}

#[test]
fn await_second_fill_deadline_rescue_can_use_ask_only_missing_side_quote() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    let now = now_ts_f64();
    bot.start_ts = now.floor() as i64 - 60;
    bot.expiry_ts = bot.start_ts + 300;
    if let Ok(mut state) = bot.state.lock() {
        state.q_yes = 5.0;
        state.c_yes = 2.0;
        state.q_no = 0.0;
        state.c_no = 0.0;
    }
    set_pair_quotes(&bot, 0.40, 0.42, 0.0, 0.39, now);
    if let Ok(mut books) = bot.book_cache.lock() {
        books.insert(
            "no_asset_id".to_string(),
            (
                json!({
                    "asks": [
                        { "price": 0.39, "size": 8.0 }
                    ],
                    "bids": []
                }),
                now,
            ),
        );
    }
    if let Ok(mut st) = bot.bot_runtime_state.lock() {
        st.open_both_first_fill_ts = now - 31.0;
        st.await_second_fill_started_ts = now - 31.0;
        st.await_second_fill_missing_side = Some(OutcomeSide::No);
        st.maker_fill_shares = 100.0;
        st.daily_maker_fill_shares = 100.0;
    }
    set_daily_liquidity_state(&bot, 100.0, 0.0);

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_await_second_fill_handler(now, 31.0, 2.0, 5.0, 0.0, 2.0, 0.0, &cfg);

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert!(runtime_state.await_second_fill_rescue_used);
    assert!(!runtime_state.await_second_fill_hard_paused);

    let contexts = bot.order_exec_context.lock().expect("exec context");
    assert!(contexts.values().any(|value| {
        value
            .get("bot_runtime_await_second_fill_rescue")
            .and_then(|field| field.as_bool())
            .unwrap_or(false)
    }));
}

#[test]
fn taker_share_helpers_compute_current_and_projected_share() {
    assert_eq!(bot_runtime_taker_share(0.0, 0.0), 0.0);
    assert!((bot_runtime_taker_share(95.0, 5.0) - 0.05).abs() < 1e-9);
    assert!((bot_runtime_projected_taker_share(90.0, 5.0, 3.0, 2.0) - 0.10).abs() < 1e-9);
}

#[test]
fn taker_submit_requires_exception_reason() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;

    let oid = bot._place_taker_bid_fak(
        "yes_asset_id",
        0.42,
        5.0,
        Some("FAK"),
        None,
        TakerCapPolicy::EnforceCap,
    );

    assert!(oid.is_none());
}

#[test]
fn taker_submit_blocks_projected_market_cap() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.maker_fill_shares = 90.0;
        runtime_state.taker_fill_shares = 9.0;
    }

    let oid = bot._place_taker_bid_fak(
        "no_asset_id",
        0.40,
        2.0,
        Some("FAK"),
        Some(TakerExceptionReason::AwaitSecondFillRescue),
        TakerCapPolicy::EnforceCap,
    );

    assert!(oid.is_none());
}

#[test]
fn taker_submit_blocks_daily_cap() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    set_daily_liquidity_state(&bot, 90.0, 9.0);
    bot._bot_runtime_refresh_daily_liquidity_counters();

    let oid = bot._place_taker_bid_fak(
        "no_asset_id",
        0.40,
        2.0,
        Some("FAK"),
        Some(TakerExceptionReason::AwaitSecondFillRescue),
        TakerCapPolicy::EnforceCap,
    );

    assert!(oid.is_none());
}

#[test]
fn recovery_bypass_taker_submit_is_allowed_above_cap_and_records_metadata() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.maker_fill_shares = 10.0;
        runtime_state.taker_fill_shares = 5.0;
    }
    set_daily_liquidity_state(&bot, 10.0, 5.0);
    bot._bot_runtime_refresh_daily_liquidity_counters();

    let oid = bot._place_taker_ask_fak(
        "yes_asset_id",
        0.40,
        5.0,
        Some("FAK"),
        Some(TakerExceptionReason::RecoveryBypass),
        TakerCapPolicy::RecoveryBypass,
    );

    let oid = oid.expect("recovery bypass taker order");
    let ctx = bot
        .order_exec_context
        .lock()
        .ok()
        .and_then(|map| map.get(&oid).cloned())
        .expect("taker exec context");
    assert_eq!(
        ctx.get("liquidity_intent")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        LiquidityIntent::TakerException.as_str()
    );
    assert_eq!(
        ctx.get("taker_exception_reason")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        TakerExceptionReason::RecoveryBypass.as_str()
    );
    assert_eq!(
        ctx.get("config_version")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        "cfgv1_test"
    );
}

#[test]
fn runtime_metrics_snapshot_exposes_pair_and_daily_taker_share() {
    let mut state = BotRuntimeState::default();
    state.total_fill_events = 3;
    state.total_fill_shares = 100.0;
    state.maker_fill_shares = 95.0;
    state.taker_fill_events = 1;
    state.taker_fill_shares = 5.0;
    state.daily_maker_fill_shares = 190.0;
    state.daily_taker_fill_shares = 10.0;
    state.yes_refresh_cycles_started = 2;
    state.no_refresh_cycles_started = 1;
    state.yes_refresh_cap_block_count = 3;
    state.no_refresh_cap_block_count = 4;

    let metrics = bot_runtime_metrics_snapshot(&state, 10.0, 10.0, 4.0, 4.0, 8.0);

    assert!((metrics.pair_taker_share - 0.05).abs() < 1e-9);
    assert!((metrics.daily_taker_share - 0.05).abs() < 1e-9);
    assert_eq!(metrics.taker_fill_events, 1);
    assert!((metrics.taker_fill_shares - 5.0).abs() < 1e-9);
    assert_eq!(metrics.yes_refresh_cycles_started, 2);
    assert_eq!(metrics.no_refresh_cycles_started, 1);
    assert_eq!(metrics.yes_refresh_cap_block_count, 3);
    assert_eq!(metrics.no_refresh_cap_block_count, 4);
}

#[test]
fn await_second_fill_rescue_hard_pauses_when_taker_cap_is_breached() {
    let mut bot = make_bot_runtime_test_bot();
    bot.cfg.dry_run = false;
    let now = now_ts_f64();
    bot.start_ts = now.floor() as i64 - 60;
    bot.expiry_ts = bot.start_ts + 300;
    if let Ok(mut state) = bot.state.lock() {
        state.q_yes = 5.0;
        state.c_yes = 2.0;
        state.q_no = 0.0;
        state.c_no = 0.0;
    }
    set_daily_liquidity_state(&bot, 5.0, 0.0);
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.open_both_first_fill_ts = now - 31.0;
        runtime_state.await_second_fill_started_ts = now - 31.0;
        runtime_state.await_second_fill_missing_side = Some(OutcomeSide::No);
        runtime_state.maker_fill_shares = 5.0;
        runtime_state.daily_maker_fill_shares = 5.0;
    }
    set_pair_quotes(&bot, 0.40, 0.42, 0.0, 0.39, now);
    if let Ok(mut books) = bot.book_cache.lock() {
        books.insert(
            "no_asset_id".to_string(),
            (
                json!({
                    "asks": [{ "price": 0.39, "size": 8.0 }],
                    "bids": []
                }),
                now,
            ),
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_await_second_fill_handler(now, 31.0, 2.0, 5.0, 0.0, 2.0, 0.0, &cfg);

    let runtime_state = bot
        .bot_runtime_state
        .lock()
        .map(|st| st.clone())
        .unwrap_or_default();
    assert!(!runtime_state.await_second_fill_rescue_used);
    assert!(runtime_state.await_second_fill_hard_paused);
}
