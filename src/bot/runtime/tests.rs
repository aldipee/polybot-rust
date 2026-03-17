use super::*;
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
/// Exercises the make BOT runtime test BOT scenario and checks the expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

fn make_bot_runtime_test_bot() -> MakerHedgeCapBot {
    let mut cfg = BotConfig::default();
    cfg.dry_run = true;
    cfg.market_data_stale_seconds = 8;
    cfg.stale_seconds = 3;
    cfg.max_total_cost = 20.0;
    cfg.reserve_usd = 2.0;
    MakerHedgeCapBot {
        cfg,
        logger: Arc::new(BotRuntimeNoopLogger),
        market_slug: "bot-test".to_string(),
        pair_identity: PairIdentity {
            pair_id: canonical_pair_id_from_slug("bot-test"),
            market_slug: "bot-test".to_string(),
            condition_id: None,
            yes_asset_id: Some("yes_asset_id".to_string()),
            no_asset_id: Some("no_asset_id".to_string()),
        },
        state_file: PathBuf::from("__bot_runtime_test_state_nonexistent.json"),
        state: Arc::new(Mutex::new(BotState::default())),
        start_trade_iso: "2024-01-01T00:00:00Z".to_string(),
        first_entry_fill_iso: Arc::new(Mutex::new(None)),
        first_entry_reason: Arc::new(Mutex::new(None)),
        pending_entry_reason: Arc::new(Mutex::new(None)),
        active_entry_reason: Arc::new(Mutex::new(None)),
        stop_loss_category: Arc::new(Mutex::new(None)),
        exit_reason: Arc::new(Mutex::new("RUNNING".to_string())),
        stop_flag: Arc::new(AtomicBool::new(false)),
        wallet_address: "0xtest".to_string(),
        min_maker_notional: 1.0,
        min_taker_notional: 1.0,
        reconcile_sell_credit_mult: 1.0,
        first_clip_shares: 0.0,
        first_hedge_full: false,
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
        loop_wait_seconds_maker: 1.0,
        loop_wait_seconds_taker: 0.2,
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
        bot_runtime_state: Arc::new(Mutex::new(BotRuntimeState::default())),
        bot_runtime_cfg: bot_runtime_config_defaults(),
    }
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
        bot_runtime_phase_from_t_into_s(239.9, &cfg),
        BotRuntimePhase::PairBuild
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(240.0, &cfg),
        BotRuntimePhase::Taper
    );
    assert_eq!(
        bot_runtime_phase_from_t_into_s(300.0, &cfg),
        BotRuntimePhase::HoldSettleRollover
    );
}
/// Exercises the BOT runtime owner routes seed completion and taper scenario and checks the
/// expected BOT behavior.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

#[test]
fn bot_runtime_owner_routes_seed_completion_and_taper() {
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::OpenBoth, 10.0, 0.0),
        (BotRuntimeControlOwner::SeedCompletion, "startup_asymmetry")
    );
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::PairBuild, 12.0, 12.0),
        (BotRuntimeControlOwner::PairBuild, "paired_replenishment")
    );
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::Taper, 12.0, 12.0),
        (BotRuntimeControlOwner::Taper, "late_taper")
    );
    assert_eq!(
        bot_runtime_owner_for_snapshot(BotRuntimePhase::HoldSettleRollover, 12.0, 12.0),
        (
            BotRuntimeControlOwner::HoldSettleRollover,
            "near_expiry_rollover"
        )
    );
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
        BotRuntimeControlOwner::SeedCompletion
    ));
    assert!(!bot_runtime_should_run_open_both_handler(
        BotRuntimeControlOwner::Taper
    ));
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
    if let Ok(mut quotes) = bot.best_quotes.lock() {
        quotes.insert("yes_asset_id".to_string(), (0.40, 0.42, 10.0));
        quotes.insert("no_asset_id".to_string(), (0.55, 0.57, 11.0));
    }
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
