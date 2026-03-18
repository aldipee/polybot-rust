use super::*;
use serde_json::json;
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
    bot._bot_runtime_taper_handler(260.0, 260.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn taper_handler_blocks_balanced_add_at_stop_add_zone_after_runtime_gating() {
    let bot = make_bot_runtime_test_bot();
    let now = now_ts_f64();
    set_pair_quotes(&bot, 0.50, 0.52, 0.50, 0.52, now);

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_taper_handler(260.0, 260.0, 12.0, 20.0, 20.0, 6.0, 6.0, &cfg);

    let state = bot.state.lock().expect("bot state");
    assert!(!state.open_orders.contains_key("yes_asset_id"));
    assert!(!state.open_orders.contains_key("no_asset_id"));
    drop(state);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert!(runtime_state
        .taper_last_hold_reason
        .starts_with("hold:price_zone_stop_add:balanced_add:1.000"));
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
    bot._bot_runtime_taper_handler(260.0, 260.0, 52.8, 40.0, 48.0, 12.0, 40.8, &cfg);

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
    bot._bot_runtime_taper_handler(260.0, 260.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

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
    bot._bot_runtime_taper_handler(260.0, 260.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

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
    bot._bot_runtime_taper_handler(260.0, 260.0, 0.60, 3.5, 2.5, 0.35, 0.25, &cfg);

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
    }

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
