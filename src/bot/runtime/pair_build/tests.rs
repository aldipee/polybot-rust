use super::super::*;
use super::*;
use std::collections::HashMap;
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
    let state_file = std::env::temp_dir().join(format!(
        "pair_build_test_state_{}.json",
        uuid::Uuid::new_v4()
    ));
    let daily_liquidity_state_file = std::env::temp_dir().join(format!(
        "pair_build_test_daily_liquidity_state_{}.json",
        uuid::Uuid::new_v4()
    ));
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
        bot_runtime_pair_build_clip_bucket(cfg.clip_ladder[1], &cfg),
        "medium"
    );
    assert_eq!(
        bot_runtime_pair_build_clip_bucket(cfg.clip_ladder[2], &cfg),
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
        BotRuntimePairedCostBand::Danger
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(1.01),
        BotRuntimePairedCostBand::StopAdd
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(0.99),
        BotRuntimePairedCostBand::Caution
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(0.95),
        BotRuntimePairedCostBand::Acceptable
    );
    assert_eq!(
        bot_runtime_pair_build_projected_paired_cost_band(0.90),
        BotRuntimePairedCostBand::Preferred
    );
}

#[test]
fn pair_build_decision_surfaces_balanced_add_stop_add_zone_for_runtime_gating() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        60.0, 20.0, 20.0, 6.0, 6.0, 0.50, 0.52, 0.50, 0.52, 40.0, 12.0, 1.0, 1.0, &cfg, false,
    )
    .expect("raw decision should surface the stop-add zone for later runtime gating");
    assert_eq!(decision.mode, BotRuntimePairBuildMode::PairedGrowth);
    assert_eq!(decision.price_zone, BotRuntimePairedCostBand::StopAdd);
    assert_eq!(
        decision.marginal_cost_mode,
        BotRuntimeMarginalCostMode::BalancedAdd
    );
    assert!((decision.effective_marginal_pair_cost - 1.0).abs() < 1e-9);
}

#[test]
fn pair_build_decision_allows_balanced_add_below_one_even_with_high_inventory_vwap() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        60.0, 20.0, 20.0, 10.4, 10.4, 0.49, 0.51, 0.49, 0.51, 80.0, 20.8, 1.0, 1.0, &cfg, false,
    )
    .expect("caution-zone balanced add should remain legal");
    assert_eq!(decision.mode, BotRuntimePairBuildMode::PairedGrowth);
    assert_eq!(decision.price_zone, BotRuntimePairedCostBand::Caution);
    assert_eq!(
        decision.marginal_cost_mode,
        BotRuntimeMarginalCostMode::BalancedAdd
    );
    assert!((decision.effective_marginal_pair_cost - 0.98).abs() < 1e-9);
}

#[test]
fn pair_build_decision_uses_rebalance_effective_marginal_pair_cost() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        60.0, 14.0, 10.0, 8.4, 6.0, 0.60, 0.62, 0.35, 0.37, 80.0, 14.4, 1.0, 1.0, &cfg, false,
    )
    .expect("sub-one rebalance add should remain legal");
    assert_eq!(decision.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(decision.side, Some(OutcomeSide::No));
    assert_eq!(decision.price_zone, BotRuntimePairedCostBand::Acceptable);
    assert_eq!(
        decision.marginal_cost_mode,
        BotRuntimeMarginalCostMode::RebalanceAdd
    );
    assert_eq!(decision.residual_unit_cost, Some(0.6));
    assert_eq!(decision.lagging_side_quote, Some(0.35));
    assert!((decision.effective_marginal_pair_cost - 0.95).abs() < 1e-9);
}

#[test]
fn pair_build_growth_downgrades_to_normal_20_when_large_clip_is_not_green() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        200.0, 25.0, 25.0, 7.5, 7.5, 0.30, 0.32, 0.30, 0.32, 500.0, 15.0, 1.0, 1.0, &cfg, false,
    )
    .expect("non-green paired growth should downgrade to the normal rung");
    assert_eq!(decision.requested_rung, BotRuntimeClipRung::Large1);
    assert_eq!(decision.selected_rung, BotRuntimeClipRung::Normal);
    assert_eq!(decision.clip, 20);
    assert!(!decision.requested_large_clip || !decision.green_conditions_met);
    assert!(!decision.green_time_ok);
}

#[test]
fn pair_build_growth_allows_green_large_40_and_80_progressively() {
    let cfg = bot_runtime_config_defaults();
    let large_40 = bot_runtime_pair_build_decision(
        60.0, 20.0, 20.0, 6.0, 6.0, 0.30, 0.32, 0.30, 0.32, 500.0, 12.0, 1.0, 1.0, &cfg, false,
    )
    .expect("green paired growth should allow the 40-share rung");
    assert_eq!(large_40.requested_rung, BotRuntimeClipRung::Large1);
    assert_eq!(large_40.selected_rung, BotRuntimeClipRung::Large1);
    assert_eq!(large_40.clip, 40);
    assert!(large_40.green_conditions_met);

    let large_80 = bot_runtime_pair_build_decision(
        60.0, 40.0, 40.0, 12.0, 12.0, 0.30, 0.32, 0.30, 0.32, 500.0, 24.0, 1.0, 1.0, &cfg, false,
    )
    .expect("green paired growth should allow the 80-share rung");
    assert_eq!(large_80.requested_rung, BotRuntimeClipRung::Large2);
    assert_eq!(large_80.selected_rung, BotRuntimeClipRung::Large2);
    assert_eq!(large_80.clip, 80);
    assert!(large_80.green_conditions_met);
}

#[test]
fn pair_build_growth_budget_downgrades_80_to_40() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        60.0, 40.0, 40.0, 12.0, 12.0, 0.30, 0.32, 0.30, 0.32, 60.0, 24.0, 1.0, 1.0, &cfg, false,
    )
    .expect("budget-constrained paired growth should downgrade to the next legal rung");
    assert_eq!(decision.requested_rung, BotRuntimeClipRung::Large2);
    assert_eq!(decision.selected_rung, BotRuntimeClipRung::Large1);
    assert_eq!(decision.clip, 40);
}

#[test]
fn pair_build_repair_uses_largest_legal_rung_or_exact_gap_clip() {
    let cfg = bot_runtime_config_defaults();
    let rung_repair = bot_runtime_pair_build_decision(
        60.0, 110.0, 75.0, 33.0, 22.5, 0.30, 0.32, 0.30, 0.32, 500.0, 55.5, 1.0, 1.0, &cfg, false,
    )
    .expect("repair should use the largest legal rung that does not overshoot the gap");
    assert_eq!(rung_repair.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(rung_repair.requested_rung, BotRuntimeClipRung::Normal);
    assert_eq!(rung_repair.selected_rung, BotRuntimeClipRung::Normal);
    assert_eq!(rung_repair.clip, 20);

    let exact_gap = bot_runtime_pair_build_decision(
        60.0, 20.0, 14.0, 6.0, 4.2, 0.30, 0.32, 0.30, 0.32, 500.0, 10.2, 1.0, 1.0, &cfg, false,
    )
    .expect("sub-ladder repair should use the exact gap clip");
    assert_eq!(exact_gap.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(exact_gap.requested_rung, BotRuntimeClipRung::ExactGapRepair);
    assert_eq!(exact_gap.selected_rung, BotRuntimeClipRung::ExactGapRepair);
    assert_eq!(exact_gap.clip, 6);
}

#[test]
fn pair_build_repair_downgrades_large_repair_to_20_when_green_conditions_fail() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        220.0, 280.0, 200.0, 84.0, 60.0, 0.30, 0.32, 0.30, 0.32, 500.0, 144.0, 1.0, 1.0, &cfg,
        false,
    )
    .expect("non-green large repair should downgrade to the normal rung");
    assert_eq!(decision.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(decision.requested_rung, BotRuntimeClipRung::Large2);
    assert_eq!(decision.selected_rung, BotRuntimeClipRung::Normal);
    assert_eq!(decision.clip, 20);
    assert!(!decision.green_time_ok);
}

#[test]
fn pair_build_repair_budget_cap_uses_lagging_side_order_price() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        60.0, 100.0, 80.0, 80.0, 8.0, 0.30, 0.32, 0.10, 0.12, 90.0, 88.0, 1.0, 1.0, &cfg, false,
    )
    .expect("repair should be affordable from the actual lagging-side order spend");
    assert_eq!(decision.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(decision.side, Some(OutcomeSide::No));
    assert_eq!(decision.selected_rung, BotRuntimeClipRung::Normal);
    assert_eq!(decision.clip, 20);
    assert_eq!(decision.lagging_side_quote, Some(0.10));
    assert!((decision.effective_marginal_pair_cost - 0.90).abs() < 1e-9);
    assert!(decision.green_budget_ok);
}

#[test]
fn pair_build_decision_surfaces_rebalance_add_stop_add_zone_for_runtime_gating() {
    let cfg = bot_runtime_config_defaults();
    let decision = bot_runtime_pair_build_decision(
        60.0, 14.0, 10.0, 8.4, 6.0, 0.60, 0.62, 0.40, 0.42, 80.0, 14.4, 1.0, 1.0, &cfg, false,
    )
    .expect("raw repair decision should surface the stop-add zone for later runtime gating");
    assert_eq!(decision.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(decision.side, Some(OutcomeSide::No));
    assert_eq!(decision.price_zone, BotRuntimePairedCostBand::StopAdd);
    assert_eq!(
        decision.marginal_cost_mode,
        BotRuntimeMarginalCostMode::RebalanceAdd
    );
    assert!((decision.effective_marginal_pair_cost - 1.0).abs() < 1e-9);
}

#[test]
fn tail_repair_priority_recomputes_rebalance_price_zone_fields() {
    let cfg = bot_runtime_config_defaults();
    let decision = BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::PairedGrowth,
        side: None,
        clip: 12,
        selected_rung: BotRuntimeClipRung::Seed,
        requested_rung: BotRuntimeClipRung::Seed,
        requested_clip: 12.0,
        requested_large_clip: false,
        clip_bucket: "small",
        cpp_hint: BotRuntimePairBuildCppHint::Normal,
        marginal_cost_mode: BotRuntimeMarginalCostMode::BalancedAdd,
        effective_marginal_pair_cost: 1.02,
        price_zone: BotRuntimePairedCostBand::StopAdd,
        residual_unit_cost: None,
        lagging_side_quote: None,
        pair_sum: 1.02,
        current_unmatched_fraction: unmatched_fraction(4.0, 8.0),
        projected_unmatched_fraction: unmatched_fraction(4.0, 8.0),
        match_ratio: match_ratio(4.0, 8.0),
        imbalance_state: BotRuntimeImbalanceState::Normal,
        reduces_imbalance: false,
        green_both_sides_filled: false,
        green_price_ok: false,
        green_imbalance_ok: false,
        green_time_ok: false,
        green_budget_ok: false,
        green_conditions_met: false,
        pair_coverage: pair_coverage(4.0, 8.0),
        skew_ratio: share_skew_ratio(4.0, 8.0),
        current_base: 4.0,
        qty_gap: 4.0,
        inventory_vwap_sum: inventory_vwap_sum(4.0, 8.0, 1.6, 4.8),
        market_snapshot_vwap_sum: market_snapshot_vwap_sum(0.25, 0.27, 0.77, 0.79),
    };

    let rewritten = bot_runtime_pair_build_apply_tail_repair_priority(
        decision, 4.0, 8.0, 1.6, 4.8, 0.25, 0.77, 20.0, 1.0, 1.0, 50.0, &cfg,
    );

    assert_eq!(rewritten.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(rewritten.side, Some(OutcomeSide::Yes));
    assert_eq!(
        rewritten.marginal_cost_mode,
        BotRuntimeMarginalCostMode::RebalanceAdd
    );
    assert_eq!(rewritten.price_zone, BotRuntimePairedCostBand::Preferred);
    assert_eq!(rewritten.residual_unit_cost, Some(0.6));
    assert_eq!(rewritten.lagging_side_quote, Some(0.25));
    assert!((rewritten.effective_marginal_pair_cost - 0.85).abs() < 1e-9);
}

#[test]
fn tail_repair_priority_uses_lagging_side_order_price_for_budget_cap() {
    let cfg = bot_runtime_config_defaults();
    let decision = BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::PairedGrowth,
        side: None,
        clip: 20,
        selected_rung: BotRuntimeClipRung::Normal,
        requested_rung: BotRuntimeClipRung::Normal,
        requested_clip: 20.0,
        requested_large_clip: false,
        clip_bucket: "medium",
        cpp_hint: BotRuntimePairBuildCppHint::Normal,
        marginal_cost_mode: BotRuntimeMarginalCostMode::BalancedAdd,
        effective_marginal_pair_cost: 1.10,
        price_zone: BotRuntimePairedCostBand::Danger,
        residual_unit_cost: None,
        lagging_side_quote: None,
        pair_sum: 1.10,
        current_unmatched_fraction: unmatched_fraction(100.0, 80.0),
        projected_unmatched_fraction: unmatched_fraction(100.0, 80.0),
        match_ratio: match_ratio(100.0, 80.0),
        imbalance_state: BotRuntimeImbalanceState::Normal,
        reduces_imbalance: false,
        green_both_sides_filled: false,
        green_price_ok: false,
        green_imbalance_ok: false,
        green_time_ok: false,
        green_budget_ok: false,
        green_conditions_met: false,
        pair_coverage: pair_coverage(100.0, 80.0),
        skew_ratio: share_skew_ratio(100.0, 80.0),
        current_base: 80.0,
        qty_gap: 20.0,
        inventory_vwap_sum: inventory_vwap_sum(100.0, 80.0, 80.0, 8.0),
        market_snapshot_vwap_sum: market_snapshot_vwap_sum(0.30, 0.32, 0.10, 0.12),
    };

    let rewritten = bot_runtime_pair_build_apply_tail_repair_priority(
        decision, 100.0, 80.0, 80.0, 8.0, 0.30, 0.10, 2.0, 1.0, 1.0, 250.0, &cfg,
    );

    assert_eq!(rewritten.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(rewritten.side, Some(OutcomeSide::No));
    assert_eq!(rewritten.clip, 20);
    assert_eq!(rewritten.selected_rung, BotRuntimeClipRung::Normal);
    assert!(rewritten.green_budget_ok);
}

#[test]
fn repair_requested_rung_rejects_ladder_when_min_valid_clip_exceeds_gap() {
    let cfg = bot_runtime_config_defaults();
    assert!(bot_runtime_repair_requested_rung(12.0, None, Some(20.0), &cfg).is_none());
    assert!(bot_runtime_repair_clip_choice(f64::INFINITY, 12.0, None, Some(20.0), &cfg).is_none());
}

#[test]
fn tail_repair_priority_keeps_paired_growth_when_min_notional_would_overshoot_gap() {
    let cfg = bot_runtime_config_defaults();
    let decision = BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::PairedGrowth,
        side: None,
        clip: 20,
        selected_rung: BotRuntimeClipRung::Normal,
        requested_rung: BotRuntimeClipRung::Normal,
        requested_clip: 20.0,
        requested_large_clip: false,
        clip_bucket: "medium",
        cpp_hint: BotRuntimePairBuildCppHint::Normal,
        marginal_cost_mode: BotRuntimeMarginalCostMode::BalancedAdd,
        effective_marginal_pair_cost: 0.65,
        price_zone: BotRuntimePairedCostBand::Acceptable,
        residual_unit_cost: None,
        lagging_side_quote: None,
        pair_sum: 0.65,
        current_unmatched_fraction: unmatched_fraction(24.0, 12.0),
        projected_unmatched_fraction: unmatched_fraction(24.0, 12.0),
        match_ratio: match_ratio(24.0, 12.0),
        imbalance_state: BotRuntimeImbalanceState::Normal,
        reduces_imbalance: false,
        green_both_sides_filled: false,
        green_price_ok: false,
        green_imbalance_ok: false,
        green_time_ok: false,
        green_budget_ok: false,
        green_conditions_met: false,
        pair_coverage: pair_coverage(24.0, 12.0),
        skew_ratio: share_skew_ratio(24.0, 12.0),
        current_base: 12.0,
        qty_gap: 12.0,
        inventory_vwap_sum: inventory_vwap_sum(24.0, 12.0, 7.2, 3.6),
        market_snapshot_vwap_sum: market_snapshot_vwap_sum(0.30, 0.32, 0.05, 0.07),
    };

    let rewritten = bot_runtime_pair_build_apply_tail_repair_priority(
        decision, 24.0, 12.0, 7.2, 3.6, 0.30, 0.05, 20.0, 1.0, 1.0, 250.0, &cfg,
    );

    assert_eq!(rewritten.mode, BotRuntimePairBuildMode::PairedGrowth);
    assert_eq!(rewritten.side, None);
    assert_eq!(rewritten.clip, 20);
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
        selected_rung: BotRuntimeClipRung::Seed,
        requested_rung: BotRuntimeClipRung::Seed,
        requested_clip: 5.0,
        requested_large_clip: false,
        clip_bucket: "small",
        cpp_hint: BotRuntimePairBuildCppHint::Normal,
        marginal_cost_mode: BotRuntimeMarginalCostMode::BalancedAdd,
        effective_marginal_pair_cost: 0.90,
        price_zone: BotRuntimePairedCostBand::Preferred,
        residual_unit_cost: None,
        lagging_side_quote: None,
        pair_sum: 0.90,
        current_unmatched_fraction: 0.0,
        projected_unmatched_fraction: 0.0,
        match_ratio: 1.0,
        imbalance_state: BotRuntimeImbalanceState::Normal,
        reduces_imbalance: false,
        green_both_sides_filled: false,
        green_price_ok: false,
        green_imbalance_ok: false,
        green_time_ok: false,
        green_budget_ok: false,
        green_conditions_met: false,
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
        BotRuntimePairedCostBand::Acceptable,
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
    let cfg = bot_runtime_config_defaults();
    let repair_policy = bot_runtime_pair_build_lighter_repair_policy(
        &BotRuntimePairBuildDecision {
            mode: BotRuntimePairBuildMode::LighterSideFirst,
            side: Some(OutcomeSide::Yes),
            clip: 3,
            selected_rung: BotRuntimeClipRung::ExactGapRepair,
            requested_rung: BotRuntimeClipRung::ExactGapRepair,
            requested_clip: 3.0,
            requested_large_clip: false,
            clip_bucket: "small",
            cpp_hint: BotRuntimePairBuildCppHint::Normal,
            marginal_cost_mode: BotRuntimeMarginalCostMode::RebalanceAdd,
            effective_marginal_pair_cost: 0.85,
            price_zone: BotRuntimePairedCostBand::Acceptable,
            residual_unit_cost: Some(0.40),
            lagging_side_quote: Some(0.45),
            pair_sum: 0.80,
            current_unmatched_fraction: 0.3333333333,
            projected_unmatched_fraction: 0.25,
            match_ratio: 0.5,
            imbalance_state: BotRuntimeImbalanceState::Warning,
            reduces_imbalance: true,
            green_both_sides_filled: false,
            green_price_ok: false,
            green_imbalance_ok: false,
            green_time_ok: false,
            green_budget_ok: false,
            green_conditions_met: false,
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
        &cfg,
    )
    .expect("repair policy");
    assert_eq!(repair_policy.clip, 0);
    assert!(repair_policy.hold_reason.is_some());

    let reserve_policy = bot_runtime_pair_build_repair_reserve_policy(
        &BotRuntimePairBuildDecision {
            mode: BotRuntimePairBuildMode::PairedGrowth,
            side: None,
            clip: 4,
            selected_rung: BotRuntimeClipRung::Seed,
            requested_rung: BotRuntimeClipRung::Seed,
            requested_clip: 4.0,
            requested_large_clip: false,
            clip_bucket: "medium",
            cpp_hint: BotRuntimePairBuildCppHint::Normal,
            marginal_cost_mode: BotRuntimeMarginalCostMode::BalancedAdd,
            effective_marginal_pair_cost: 0.82,
            price_zone: BotRuntimePairedCostBand::Preferred,
            residual_unit_cost: None,
            lagging_side_quote: None,
            pair_sum: 0.82,
            current_unmatched_fraction: 0.1111111111,
            projected_unmatched_fraction: 0.0769230769,
            match_ratio: 0.8,
            imbalance_state: BotRuntimeImbalanceState::Throttle,
            reduces_imbalance: true,
            green_both_sides_filled: false,
            green_price_ok: false,
            green_imbalance_ok: false,
            green_time_ok: false,
            green_budget_ok: false,
            green_conditions_met: false,
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
    let mut bot = make_pair_build_test_bot();
    bot.cfg.max_total_cost = 500.0;
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
    let hold_reason = bot
        .bot_runtime_state
        .lock()
        .map(|state| state.pair_build_last_hold_reason.clone())
        .unwrap_or_default();
    assert!(
        origins.iter().any(|origin| origin == "BOT_PAIR_BUILD_YES"),
        "origins={origins:?} hold_reason={hold_reason}"
    );
    assert!(
        origins.iter().any(|origin| origin == "BOT_PAIR_BUILD_NO"),
        "origins={origins:?} hold_reason={hold_reason}"
    );
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

#[test]
fn pair_build_handler_blocks_balanced_add_at_stop_add_zone_without_tail_repair_priority() {
    let mut bot = make_pair_build_test_bot();
    bot.cfg.max_total_cost = 100.0;
    set_quotes(&bot, 0.50, 0.52, 0.50, 0.52);

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(40.0, 40.0, 0.0, 20.0, 20.0, 6.0, 6.0, &cfg);

    let state = bot.state.lock().expect("bot state");
    assert!(!state.open_orders.contains_key("yes_asset_id"));
    assert!(!state.open_orders.contains_key("no_asset_id"));
    drop(state);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert!(
        runtime_state
            .pair_build_last_hold_reason
            .contains("price_zone_stop_add:balanced_add:1.000"),
        "actual_reason={}",
        runtime_state.pair_build_last_hold_reason
    );
}

#[test]
fn pair_build_handler_allows_tail_repair_priority_before_balanced_add_stop_add_hold() {
    let mut bot = make_pair_build_test_bot();
    bot.cfg.max_total_cost = 500.0;
    set_quotes(&bot, 0.35, 0.37, 0.70, 0.72);

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(250.0, 250.0, 0.0, 200.0, 205.0, 120.0, 123.0, &cfg);

    let state = bot.state.lock().expect("bot state");
    assert!(state.open_orders.contains_key("yes_asset_id"));
    assert!(!state.open_orders.contains_key("no_asset_id"));
}

/// Exercises the BOT runtime pair build handler submits lighter side repair order scenario and
/// checks the expected BOT behavior.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

#[test]
fn bot_runtime_pair_build_handler_submits_lighter_side_repair_order() {
    let bot = make_pair_build_test_bot();
    let cfg = *bot._bot_runtime_cfg();
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
        clip: 12,
        selected_rung: BotRuntimeClipRung::Seed,
        requested_rung: BotRuntimeClipRung::Seed,
        requested_clip: 12.0,
        requested_large_clip: false,
        clip_bucket: "small",
        cpp_hint: BotRuntimePairBuildCppHint::Normal,
        marginal_cost_mode: BotRuntimeMarginalCostMode::RebalanceAdd,
        effective_marginal_pair_cost: 0.55,
        price_zone: BotRuntimePairedCostBand::Preferred,
        residual_unit_cost: Some(0.30),
        lagging_side_quote: Some(0.25),
        pair_sum: 0.80,
        current_unmatched_fraction: 0.60,
        projected_unmatched_fraction: 0.0,
        match_ratio: 0.25,
        imbalance_state: BotRuntimeImbalanceState::HardDisable,
        reduces_imbalance: true,
        green_both_sides_filled: false,
        green_price_ok: false,
        green_imbalance_ok: false,
        green_time_ok: false,
        green_budget_ok: false,
        green_conditions_met: false,
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
            &cfg,
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

#[test]
fn tail_repair_priority_blocks_live_repair_when_rebalance_zone_is_danger() {
    let mut bot = make_pair_build_test_bot();
    bot.cfg.max_total_cost = 500.0;
    set_quotes(&bot, 0.20, 0.22, 0.70, 0.72);

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(40.0, 40.0, 0.0, 50.0, 56.0, 17.5, 47.6, &cfg);

    let state = bot.state.lock().expect("bot state");
    assert!(!state.open_orders.contains_key("yes_asset_id"));
    assert!(!state.open_orders.contains_key("no_asset_id"));
    drop(state);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert!(
        runtime_state
            .pair_build_last_hold_reason
            .starts_with("hold:price_zone_danger:rebalance_add:1.050"),
        "actual_reason={}",
        runtime_state.pair_build_last_hold_reason
    );
}

#[test]
fn rebalance_price_zone_hold_cancels_live_pair_build_lighter_order() {
    let mut bot = make_pair_build_test_bot();
    bot.cfg.max_total_cost = 100.0;
    set_quotes(&bot, 0.20, 0.22, 0.70, 0.72);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-lighter-yes".to_string()),
                origin: "BOT_PAIR_BUILD_LIGHTER".to_string(),
                last_submit_ts: 42.0,
                price: 0.20,
                remaining: 8.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(60.0, 60.0, 0.0, 40.0, 48.0, 12.0, 40.8, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert!(
        runtime_state
            .pair_build_last_hold_reason
            .contains("price_zone_danger:rebalance_add:1.050"),
        "actual_reason={}",
        runtime_state.pair_build_last_hold_reason
    );
}

#[test]
fn tail_rewrite_price_zone_hold_cancels_live_pair_build_lighter_order() {
    let mut bot = make_pair_build_test_bot();
    bot.cfg.max_total_cost = 500.0;
    bot.cfg.min_shares = 1.0;
    set_quotes(&bot, 0.34, 0.36, 0.60, 0.62);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-lighter-yes".to_string()),
                origin: "BOT_PAIR_BUILD_LIGHTER".to_string(),
                last_submit_ts: 240.0,
                price: 0.34,
                remaining: 3.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(250.0, 250.0, 0.0, 100.0, 103.0, 35.0, 77.25, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);

    let runtime_state = bot.bot_runtime_state.lock().expect("runtime state");
    assert!(runtime_state
        .pair_build_last_hold_reason
        .contains("price_zone_danger:rebalance_add:1.090"));
}

#[test]
fn hard_disable_cancels_lingering_open_both_orders_before_returning() {
    let bot = make_pair_build_test_bot();
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.imbalance_state = BotRuntimeImbalanceState::HardDisable;
    }
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-open-both-yes".to_string()),
                origin: "BOT_OPEN_BOTH_YES".to_string(),
                last_submit_ts: 12.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(40.0, 40.0, 5.0, 12.0, 8.0, 4.2, 2.8, &cfg);

    let slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    assert_eq!(slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn hard_disable_cancels_multiple_bot_families_without_short_circuiting() {
    let bot = make_pair_build_test_bot();
    if let Ok(mut runtime_state) = bot.bot_runtime_state.lock() {
        runtime_state.imbalance_state = BotRuntimeImbalanceState::HardDisable;
    }
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-open-both-yes".to_string()),
                origin: "BOT_OPEN_BOTH_YES".to_string(),
                last_submit_ts: 12.0,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-await-second-fill-no".to_string()),
                origin: "BOT_AWAIT_SECOND_FILL_NO".to_string(),
                last_submit_ts: 12.5,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(40.0, 40.0, 5.0, 12.0, 8.0, 4.2, 2.8, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn imbalance_repair_unavailable_cancels_live_pair_build_orders() {
    let bot = make_pair_build_test_bot();
    set_quotes(&bot, 0.10, 0.12, 0.10, 0.12);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-yes".to_string()),
                origin: "BOT_PAIR_BUILD_YES".to_string(),
                last_submit_ts: 18.0,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-no".to_string()),
                origin: "BOT_PAIR_BUILD_NO".to_string(),
                last_submit_ts: 18.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(60.0, 60.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn imbalance_hold_cancels_growth_but_keeps_live_lighter_repair() {
    let bot = make_pair_build_test_bot();
    set_quotes(&bot, 0.10, 0.12, 0.10, 0.12);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-lighter-yes".to_string()),
                origin: "BOT_PAIR_BUILD_LIGHTER".to_string(),
                last_submit_ts: 18.0,
                price: 0.10,
                remaining: 0.50,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-no".to_string()),
                origin: "BOT_PAIR_BUILD_NO".to_string(),
                last_submit_ts: 18.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(60.0, 60.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::Working);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn imbalance_hold_cancels_oversized_live_lighter_repair() {
    let bot = make_pair_build_test_bot();
    set_quotes(&bot, 0.10, 0.12, 0.10, 0.12);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-lighter-yes".to_string()),
                origin: "BOT_PAIR_BUILD_LIGHTER".to_string(),
                last_submit_ts: 18.0,
                price: 0.10,
                remaining: 1.50,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-no".to_string()),
                origin: "BOT_PAIR_BUILD_NO".to_string(),
                last_submit_ts: 18.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(60.0, 60.0, 0.60, 2.5, 3.5, 0.25, 0.35, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn imbalance_hold_cancels_wrong_side_live_lighter_repair_after_side_flip() {
    let bot = make_pair_build_test_bot();
    set_quotes(&bot, 0.10, 0.12, 0.10, 0.12);
    if let Ok(mut slots) = bot.maker_order_slots.lock() {
        slots.insert(
            MakerOrderKey::buy("yes_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-lighter-yes".to_string()),
                origin: "BOT_PAIR_BUILD_LIGHTER".to_string(),
                last_submit_ts: 18.0,
                price: 0.10,
                remaining: 0.50,
                ..MakerOrderSlot::default()
            },
        );
        slots.insert(
            MakerOrderKey::buy("no_asset_id"),
            MakerOrderSlot {
                state: MakerOrderLifecycle::Working,
                order_id: Some("oid-pair-build-no".to_string()),
                origin: "BOT_PAIR_BUILD_NO".to_string(),
                last_submit_ts: 18.0,
                ..MakerOrderSlot::default()
            },
        );
    }

    let cfg = *bot._bot_runtime_cfg();
    bot._bot_runtime_pair_build_handler(60.0, 60.0, 0.60, 3.5, 2.5, 0.35, 0.25, &cfg);

    let yes_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("yes_asset_id"));
    let no_slot = bot._maker_order_slot_get(&MakerOrderKey::buy("no_asset_id"));
    assert_eq!(yes_slot.state, MakerOrderLifecycle::CancelPending);
    assert_eq!(no_slot.state, MakerOrderLifecycle::CancelPending);
}

#[test]
fn pair_build_decision_uses_exact_unmatched_fraction_thresholds() {
    let cfg = bot_runtime_config_defaults();
    let normal = bot_runtime_pair_build_decision(
        100.0, 100.0, 100.0, 35.0, 35.0, 0.30, 0.32, 0.30, 0.32, 200.0, 70.0, 1.0, 1.0, &cfg, false,
    )
    .expect("normal paired growth");
    assert_eq!(normal.mode, BotRuntimePairBuildMode::PairedGrowth);
    assert_eq!(normal.imbalance_state, BotRuntimeImbalanceState::Normal);

    let throttle = bot_runtime_pair_build_decision(
        100.0, 100.0, 85.0, 35.0, 29.75, 0.30, 0.32, 0.30, 0.32, 200.0, 64.75, 1.0, 1.0, &cfg,
        false,
    )
    .expect("throttle repair");
    assert_eq!(throttle.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(throttle.imbalance_state, BotRuntimeImbalanceState::Throttle);
    assert!(throttle.reduces_imbalance);

    let warning = bot_runtime_pair_build_decision(
        100.0, 100.0, 75.0, 35.0, 26.25, 0.30, 0.32, 0.30, 0.32, 200.0, 61.25, 1.0, 1.0, &cfg,
        false,
    )
    .expect("warning repair");
    assert_eq!(warning.mode, BotRuntimePairBuildMode::LighterSideFirst);
    assert_eq!(warning.imbalance_state, BotRuntimeImbalanceState::Warning);

    let hard_disable = bot_runtime_pair_build_decision(
        100.0, 12.0, 8.0, 4.2, 2.8, 0.30, 0.32, 0.30, 0.32, 100.0, 7.0, 1.0, 1.0, &cfg, false,
    )
    .expect_err("hard disable should block");
    assert!(hard_disable.starts_with("hard_imbalance_disable:"));
}
