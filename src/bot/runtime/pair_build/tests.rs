use super::super::*;
use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct BotRuntimeNoopLogger;

impl LogLike for BotRuntimeNoopLogger {
    /// Exercises the info scenario and checks the expected BOT behavior.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    fn info(&self, _msg: &str) {}
    /// Exercises the warning scenario and checks the expected BOT behavior.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    fn warning(&self, _msg: &str) {}
    /// Exercises the error scenario and checks the expected BOT behavior.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    fn error(&self, _msg: &str) {}
}

/// Exercises the make pair build test BOT scenario and checks the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

fn make_pair_build_test_bot() -> MakerHedgeCapBot {
    let mut cfg = BotConfig::default();
    cfg.dry_run = true;
    cfg.market_data_stale_seconds = 8;
    cfg.stale_seconds = 3;
    cfg.max_total_cost = 20.0;
    cfg.reserve_usd = 2.0;
    MakerHedgeCapBot {
        cfg,
        logger: Arc::new(BotRuntimeNoopLogger),
        market_slug: "pair-build-test".to_string(),
        pair_identity: PairIdentity {
            pair_id: canonical_pair_id_from_slug("pair-build-test"),
            market_slug: "pair-build-test".to_string(),
            condition_id: None,
            yes_asset_id: Some("yes_asset_id".to_string()),
            no_asset_id: Some("no_asset_id".to_string()),
        },
        state_file: PathBuf::from("__pair_build_test_state_nonexistent.json"),
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

/// Exercises the set quotes scenario and checks the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

fn set_quotes(bot: &MakerHedgeCapBot, y_bid: f64, y_ask: f64, n_bid: f64, n_ask: f64) {
    let mut quotes = bot.best_quotes.lock().expect("quotes lock");
    quotes.insert("yes_asset_id".to_string(), (y_bid, y_ask, 9_999_999_999.0));
    quotes.insert("no_asset_id".to_string(), (n_bid, n_ask, 9_999_999_999.0));
}

/// Exercises the BOT runtime pair build clip bucket boundaries scenario and checks the expected
/// BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_clip_bucket_boundaries() {
    let cfg = bot_runtime_config_defaults();
    assert_eq!(bot_runtime_pair_build_clip_bucket(1.0, &cfg), "small");
    assert_eq!(
        bot_runtime_pair_build_clip_bucket(cfg.large_clip_ladder[0], &cfg),
        "medium"
    );
    assert_eq!(
        bot_runtime_pair_build_clip_bucket(cfg.large_clip_ladder[1], &cfg),
        "large"
    );
}

/// Exercises the BOT runtime pair build paired cost band transitions scenario and checks the
/// expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_paired_cost_band_transitions() {
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(1.03),
        BotRuntimePairedCostBand::Freeze
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(1.01),
        BotRuntimePairedCostBand::RepairOnly
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(0.99),
        BotRuntimePairedCostBand::ReducedGrowth
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(0.95),
        BotRuntimePairedCostBand::NormalGrowth
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(0.90),
        BotRuntimePairedCostBand::StrongGrowth
    );
}

/// Exercises the BOT runtime pair build optional buy requires below snapshot scenario and
/// checks the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_optional_buy_requires_below_snapshot() {
    let decision = BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::PairedGrowth,
        side: None,
        clip: 5,
        requested_clip: 5.0,
        clip_bucket: "small",
        cpp_hint: BotRuntimePairBuildCppHint::Normal,
        pair_sum: 0.90,
        pair_coverage: 1.0,
        skew_ratio: 1.0,
        current_base: 4.0,
        qty_gap: 0.0,
        inventory_vwap_sum: 0.90,
        market_snapshot_vwap_sum: 0.92,
    };
    let cfg = bot_runtime_config_defaults();
    let policy = bot_runtime_pair_build_optional_buy_policy(
        &decision,
        0.46,
        0.46,
        0.46,
        0.46,
        BotRuntimePairedCostBand::NormalGrowth,
        1.0,
        &cfg,
    )
    .expect("optional buy policy");
    assert!(policy.hold_reason.is_some());
}

/// Exercises the BOT runtime pair build repair policy and reserve blocks scenario and checks
/// the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_repair_policy_and_reserve_blocks() {
    let repair_policy = bot_runtime_pair_build_lighter_repair_policy(
        &BotRuntimePairBuildDecision {
            mode: BotRuntimePairBuildMode::LighterSideFirst,
            side: Some(OutcomeSide::Yes),
            clip: 3,
            requested_clip: 3.0,
            clip_bucket: "small",
            cpp_hint: BotRuntimePairBuildCppHint::Normal,
            pair_sum: 0.80,
            pair_coverage: 0.5,
            skew_ratio: 2.0,
            current_base: 2.0,
            qty_gap: 1.0,
            inventory_vwap_sum: 0.80,
            market_snapshot_vwap_sum: 0.82,
        },
        0.45,
        0.20,
        1.0,
        1.0,
    )
    .expect("repair policy");
    assert_eq!(repair_policy.clip, 0);
    assert!(repair_policy.hold_reason.is_some());

    let cfg = bot_runtime_config_defaults();
    let reserve_policy = bot_runtime_pair_build_repair_reserve_policy(
        &BotRuntimePairBuildDecision {
            mode: BotRuntimePairBuildMode::PairedGrowth,
            side: None,
            clip: 4,
            requested_clip: 4.0,
            clip_bucket: "medium",
            cpp_hint: BotRuntimePairBuildCppHint::Normal,
            pair_sum: 0.82,
            pair_coverage: 0.80,
            skew_ratio: 1.2,
            current_base: 3.0,
            qty_gap: 4.0,
            inventory_vwap_sum: 0.88,
            market_snapshot_vwap_sum: 0.84,
        },
        3.0,
        7.0,
        0.40,
        0.42,
        0.50,
        1.0,
        1.0,
        &cfg,
    )
    .expect("reserve policy");
    assert!(reserve_policy.hold_reason.is_some());
}

/// Exercises the BOT runtime pair build repost and reject cooldowns scenario and checks the
/// expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_repost_and_reject_cooldowns() {
    assert!(!bot_runtime_pair_build_price_moved_meaningfully(
        0.40, 0.40, 0.01
    ));
    assert!(bot_runtime_pair_build_price_moved_meaningfully(
        0.40, 0.41, 0.01
    ));

    let slot = MakerOrderSlot {
        last_reject_ts: 10.0,
        consecutive_rejects: 3,
        last_reject_origin: "BOT_PAIR_BUILD".to_string(),
        ..MakerOrderSlot::default()
    };
    let cooldown =
        maker_order_effective_reject_cooldown_seconds("BOT_PAIR_BUILD", &slot, 1.0, 60.0);
    assert_eq!(cooldown, 4.0);
}

/// Exercises the BOT runtime pair build handler sets hold reason when quotes missing scenario
/// and checks the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_handler_sets_hold_reason_when_quotes_missing() {
    let bot = make_pair_build_test_bot();
    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(40.0, 40.0, 0.0, 4.0, 4.0, 1.6, 1.6, &cfg);
    let state = bot.bot_runtime_state.lock().expect("runtime state");
    assert_eq!(
        state.pair_build_last_hold_reason,
        "hold:quote_inputs_unready:missing_quotes_YES"
    );
}

/// Exercises the BOT runtime pair build handler submits paired growth orders scenario and
/// checks the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_handler_submits_paired_growth_orders() {
    let bot = make_pair_build_test_bot();
    set_quotes(&bot, 0.30, 0.35, 0.30, 0.35);
    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(40.0, 40.0, 0.0, 5.0, 5.0, 0.0, 0.0, &cfg);

    let contexts = bot.order_exec_context.lock().expect("exec context");
    let origins: Vec<String> = contexts
        .values()
        .filter_map(|value| {
            value
                .get("origin")
                .and_then(|origin| origin.as_str())
                .map(ToString::to_string)
        })
        .collect();
    assert!(origins.iter().any(|origin| origin == "BOT_PAIR_BUILD_YES"));
    assert!(origins.iter().any(|origin| origin == "BOT_PAIR_BUILD_NO"));
    for value in contexts.values() {
        assert_eq!(
            value.get("pair_id").and_then(|field| field.as_str()),
            Some("pair-build-test")
        );
        assert_eq!(
            value.get("market_slug").and_then(|field| field.as_str()),
            Some("pair-build-test")
        );
    }
}

/// Exercises the BOT runtime pair build handler submits lighter side repair order scenario and
/// checks the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_handler_submits_lighter_side_repair_order() {
    let bot = make_pair_build_test_bot();
    let context = BotRuntimePairBuildMarketContext {
        yes_asset: "yes_asset_id".to_string(),
        no_asset: "no_asset_id".to_string(),
        yes_key: MakerOrderKey::buy("yes_asset_id"),
        no_key: MakerOrderKey::buy("no_asset_id"),
        yes_slot: MakerOrderSlot::default(),
        no_slot: MakerOrderSlot::default(),
        y_bid: 0.25,
        y_ask: 0.30,
        n_bid: 0.55,
        n_ask: 0.60,
    };
    let decision = BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::LighterSideFirst,
        side: Some(OutcomeSide::Yes),
        clip: 15,
        requested_clip: 15.0,
        clip_bucket: "small",
        cpp_hint: BotRuntimePairBuildCppHint::Normal,
        pair_sum: 0.80,
        pair_coverage: 0.25,
        skew_ratio: 4.0,
        current_base: 5.0,
        qty_gap: 15.0,
        inventory_vwap_sum: 0.60,
        market_snapshot_vwap_sum: 0.90,
    };
    let plan = BotRuntimePairBuildPlan {
        decision,
        budget_snapshot: BotRuntimeBudgetSnapshot {
            cumulative_min_fraction: 0.0,
            cumulative_max_fraction: 1.0,
            cumulative_min_cost: 0.0,
            cumulative_max_cost: 20.0,
            remaining_to_max_cost: 10.5,
            under_min_target: false,
        },
        lighter_repair_policy: bot_runtime_pair_build_lighter_repair_policy(
            &decision,
            context.y_bid,
            10.5,
            bot.cfg.min_shares,
            bot.min_maker_notional,
        ),
        repair_reserve_policy: None,
        optional_growth_policy: None,
        optional_buy_policy: None,
        paired_cost_observation: None,
        bad_regime_shutdown: (false, 0.0, 0, 0),
    };
    bot._bot_runtime_pair_build_handle_lighter_side_repair(
        40.0, 40.0, 7.5, 5.0, 20.0, 1.5, 6.0, &context, &plan,
    );

    let state = bot.state.lock().expect("bot state");
    assert!(state.open_orders.contains_key("yes_asset_id"));
    assert!(!state.open_orders.contains_key("no_asset_id"));
}
