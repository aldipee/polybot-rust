use super::*;
use std::sync::atomic::AtomicBool;

// ───────────────────────────────────────────────────────────────────
// Test helper: NoopLogger + minimal MakerHedgeCapBot constructor
// ───────────────────────────────────────────────────────────────────

struct NoopLogger;

impl LogLike for NoopLogger {
    fn info(&self, _msg: &str) {}
    fn warning(&self, _msg: &str) {}
    fn error(&self, _msg: &str) {}
}

fn make_test_bot() -> MakerHedgeCapBot {
    make_test_bot_with_mode("MAKER")
}

fn make_test_bot_with_mode(exec_mode: &str) -> MakerHedgeCapBot {
    let cfg = BotConfig::default();
    make_test_bot_with_cfg(cfg, exec_mode)
}

fn make_test_bot_with_cfg(cfg: BotConfig, exec_mode: &str) -> MakerHedgeCapBot {
    let logger: Arc<dyn LogLike> = Arc::new(NoopLogger);
    MakerHedgeCapBot {
        cfg,
        logger,
        market_slug: "test-slug".to_string(),
        signal_hub: None,
        state_file: PathBuf::from("__p2_test_state_nonexistent.json"),
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
        expiry_ts: 86400,
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
        exec_mode: exec_mode.to_string(),
        loop_wait_seconds_maker: 1.0,
        loop_wait_seconds_taker: 0.2,
        loop_wait_seconds_sniper: 0.05,
        sniper_stop_certainty: SniperStopCertaintyConfig::from_env(),
        condition_id: None,
        market_fees_enabled: None,
        yes_asset: Some("yes_asset_id".to_string()),
        no_asset: Some("no_asset_id".to_string()),
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
        latency_log: None,
        clob_rt: None,
        clob_client: None,
        clob_api_creds: None,
        balance_allowance_cache: Arc::new(Mutex::new(HashMap::new())),
        reconcile_suspect_yes: Arc::new(Mutex::new(None)),
        reconcile_suspect_no: Arc::new(Mutex::new(None)),
        reconcile_last_ts: Arc::new(Mutex::new(0.0)),
        exchange_orders_cache: Arc::new(Mutex::new(Vec::new())),
        binance_feed: None,
        sniper_filters: Arc::new(Mutex::new(SniperFilterEngine::new(""))),
        sniper_filters_persist_enabled: false,
        sniper_filters_state_path: None,
        sniper_filters_persist_min_interval_ms: 250,
        sniper_trade_decision: Arc::new(Mutex::new(None)),
        sniper_order_fill_agg: Arc::new(Mutex::new(HashMap::new())),
        maker_skew_state: Arc::new(Mutex::new(MakerSkewArbState::default())),
        maker_ladder_open_orders: Arc::new(Mutex::new(HashMap::new())),
        maker_order_slots: Arc::new(Mutex::new(HashMap::new())),
        maker_order_index: Arc::new(Mutex::new(HashMap::new())),
        maker_exec_ledger: Arc::new(Mutex::new(MakerExecLedger::default())),
        pair_arb_pending_imbalance: Arc::new(Mutex::new(None)),
        pair_base_state: Arc::new(Mutex::new(PairBaseRuntimeState::default())),
    }
}

// ═══════════════════════════════════════════════════════════════════
// 1. _lat_ms
// ═══════════════════════════════════════════════════════════════════

#[test]
fn lat_ms_normal_diff() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_ms(1.500, 1.000), Some(500));
}

#[test]
fn lat_ms_zero_diff() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_ms(5.0, 5.0), Some(0));
}

#[test]
fn lat_ms_negative_diff() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_ms(1.0, 2.0), Some(-1000));
}

#[test]
fn lat_ms_nan_returns_none() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_ms(f64::NAN, 1.0), None);
    assert_eq!(bot._lat_ms(1.0, f64::NAN), None);
}

#[test]
fn lat_ms_infinity_returns_none() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_ms(f64::INFINITY, 1.0), None);
    assert_eq!(bot._lat_ms(1.0, f64::NEG_INFINITY), None);
}

#[test]
fn lat_ms_sub_millisecond_rounds() {
    let bot = make_test_bot();
    // 0.0004 seconds = 0.4 ms -> rounds to 0
    assert_eq!(bot._lat_ms(1.0004, 1.0), Some(0));
    // 0.0006 seconds = 0.6 ms -> rounds to 1
    assert_eq!(bot._lat_ms(1.0006, 1.0), Some(1));
}

// ═══════════════════════════════════════════════════════════════════
// 2. _lat_us
// ═══════════════════════════════════════════════════════════════════

#[test]
fn lat_us_normal_diff() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_us(1.001, 1.000), Some(1000));
}

#[test]
fn lat_us_zero_diff() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_us(5.0, 5.0), Some(0));
}

#[test]
fn lat_us_nan_returns_none() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_us(f64::NAN, 1.0), None);
}

#[test]
fn lat_us_infinity_returns_none() {
    let bot = make_test_bot();
    assert_eq!(bot._lat_us(f64::INFINITY, 1.0), None);
}

// ═══════════════════════════════════════════════════════════════════
// 3. _utc_iso
// ═══════════════════════════════════════════════════════════════════

#[test]
fn utc_iso_epoch_zero() {
    let bot = make_test_bot();
    let iso = bot._utc_iso(0.0);
    assert!(iso.starts_with("1970-01-01T00:00:00"));
}

#[test]
fn utc_iso_known_timestamp() {
    let bot = make_test_bot();
    // 2024-01-01T00:00:00Z in epoch seconds
    let iso = bot._utc_iso(1704067200.0);
    assert!(iso.starts_with("2024-01-01T00:00:00"));
}

#[test]
fn utc_iso_fractional_seconds() {
    let bot = make_test_bot();
    let iso = bot._utc_iso(1704067200.5);
    assert!(iso.starts_with("2024-01-01T00:00:00"));
}

#[test]
fn utc_iso_negative_ts_returns_something() {
    let bot = make_test_bot();
    let iso = bot._utc_iso(-100.0);
    // Should still produce a valid RFC3339 string (1969 or fallback to now)
    assert!(iso.contains('T'));
}

// ═══════════════════════════════════════════════════════════════════
// 4. _runtime_ts_get / _runtime_ts_set
// ═══════════════════════════════════════════════════════════════════

#[test]
fn runtime_ts_get_returns_zero_when_unset() {
    let bot = make_test_bot();
    assert!((bot._runtime_ts_get("__p2_nonexistent_key") - 0.0).abs() < 1e-12);
}

#[test]
fn runtime_ts_set_then_get() {
    let bot = make_test_bot();
    bot._runtime_ts_set("__p2_test_key_1", 42.5);
    assert!((bot._runtime_ts_get("__p2_test_key_1") - 42.5).abs() < 1e-12);
}

#[test]
fn runtime_ts_set_overwrites() {
    let bot = make_test_bot();
    bot._runtime_ts_set("__p2_test_key_2", 10.0);
    bot._runtime_ts_set("__p2_test_key_2", 20.0);
    assert!((bot._runtime_ts_get("__p2_test_key_2") - 20.0).abs() < 1e-12);
}

#[test]
fn runtime_ts_independent_keys() {
    let bot = make_test_bot();
    bot._runtime_ts_set("__p2_key_a", 1.0);
    bot._runtime_ts_set("__p2_key_b", 2.0);
    assert!((bot._runtime_ts_get("__p2_key_a") - 1.0).abs() < 1e-12);
    assert!((bot._runtime_ts_get("__p2_key_b") - 2.0).abs() < 1e-12);
}

// ═══════════════════════════════════════════════════════════════════
// 5. _set_exit_reason / _get_exit_reason
// ═══════════════════════════════════════════════════════════════════

#[test]
fn exit_reason_default_is_running() {
    let bot = make_test_bot();
    assert_eq!(bot._get_exit_reason(), "RUNNING");
}

#[test]
fn exit_reason_set_and_get() {
    let bot = make_test_bot();
    bot._set_exit_reason("STOP_LOSS");
    assert_eq!(bot._get_exit_reason(), "STOP_LOSS");
}

#[test]
fn exit_reason_overwrite() {
    let bot = make_test_bot();
    bot._set_exit_reason("TAKE_PROFIT");
    bot._set_exit_reason("MAX_LOSS");
    assert_eq!(bot._get_exit_reason(), "MAX_LOSS");
}

// ═══════════════════════════════════════════════════════════════════
// 6. _set_pending_entry_reason / _take_pending_entry_reason
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pending_entry_reason_initially_none() {
    let bot = make_test_bot();
    assert!(bot._take_pending_entry_reason().is_none());
}

#[test]
fn pending_entry_reason_set_then_take() {
    let bot = make_test_bot();
    bot._set_pending_entry_reason("MOMENTUM_BREAKOUT");
    assert_eq!(
        bot._take_pending_entry_reason(),
        Some("MOMENTUM_BREAKOUT".to_string())
    );
}

#[test]
fn pending_entry_reason_take_clears() {
    let bot = make_test_bot();
    bot._set_pending_entry_reason("SIGNAL_ENTRY");
    let _ = bot._take_pending_entry_reason();
    assert!(bot._take_pending_entry_reason().is_none());
}

#[test]
fn pending_entry_reason_overwrite() {
    let bot = make_test_bot();
    bot._set_pending_entry_reason("FIRST");
    bot._set_pending_entry_reason("SECOND");
    assert_eq!(
        bot._take_pending_entry_reason(),
        Some("SECOND".to_string())
    );
}

// ═══════════════════════════════════════════════════════════════════
// 7. _default_entry_reason (based on exec_mode)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn default_entry_reason_maker() {
    let bot = make_test_bot_with_mode("MAKER");
    assert_eq!(bot._default_entry_reason(), "MAKER_ENTRY");
}

#[test]
fn default_entry_reason_sniper() {
    let bot = make_test_bot_with_mode("SNIPER");
    assert_eq!(bot._default_entry_reason(), "SNIPER_ENTRY");
}

#[test]
fn default_entry_reason_prob_sniper() {
    let bot = make_test_bot_with_mode("PROB_SNIPER");
    assert_eq!(bot._default_entry_reason(), "SNIPER_ENTRY");
}

#[test]
fn default_entry_reason_high_prob() {
    let bot = make_test_bot_with_mode("HIGH_PROB");
    assert_eq!(bot._default_entry_reason(), "SNIPER_ENTRY");
}

#[test]
fn default_entry_reason_high_prob_sniper() {
    let bot = make_test_bot_with_mode("HIGH_PROB_SNIPER");
    assert_eq!(bot._default_entry_reason(), "SNIPER_ENTRY");
}

#[test]
fn default_entry_reason_fixed_profit() {
    let bot = make_test_bot_with_mode("FIXED_PROFIT");
    assert_eq!(bot._default_entry_reason(), "SNIPER_ENTRY");
}

#[test]
fn default_entry_reason_signal_snipper() {
    let bot = make_test_bot_with_mode("SIGNAL_SNIPPER");
    assert_eq!(bot._default_entry_reason(), "SIGNAL_ENTRY");
}

#[test]
fn default_entry_reason_signal_sniper() {
    let bot = make_test_bot_with_mode("SIGNAL_SNIPER");
    assert_eq!(bot._default_entry_reason(), "SIGNAL_ENTRY");
}

#[test]
fn default_entry_reason_signal_snipe() {
    let bot = make_test_bot_with_mode("SIGNAL_SNIPE");
    assert_eq!(bot._default_entry_reason(), "SIGNAL_ENTRY");
}

#[test]
fn default_entry_reason_signal() {
    let bot = make_test_bot_with_mode("SIGNAL");
    assert_eq!(bot._default_entry_reason(), "SIGNAL_ENTRY");
}

#[test]
fn default_entry_reason_unknown_mode() {
    let bot = make_test_bot_with_mode("SOMETHING_ELSE");
    assert_eq!(bot._default_entry_reason(), "MAKER_ENTRY");
}

// ═══════════════════════════════════════════════════════════════════
// 8. _active_entry_reason_or_default
// ═══════════════════════════════════════════════════════════════════

#[test]
fn active_entry_reason_defaults_when_no_reason_set() {
    let bot = make_test_bot_with_mode("SNIPER");
    assert_eq!(bot._active_entry_reason_or_default(), "SNIPER_ENTRY");
}

#[test]
fn active_entry_reason_uses_active_reason() {
    let bot = make_test_bot_with_mode("SNIPER");
    if let Ok(mut reason) = bot.active_entry_reason.lock() {
        *reason = Some("MOMENTUM_BREAKOUT".to_string());
    }
    assert_eq!(bot._active_entry_reason_or_default(), "MOMENTUM_BREAKOUT");
}

#[test]
fn active_entry_reason_falls_back_to_first_entry_reason() {
    let bot = make_test_bot_with_mode("SNIPER");
    // active_entry_reason is None, first_entry_reason is set
    if let Ok(mut reason) = bot.first_entry_reason.lock() {
        *reason = Some("RTDS_DIFF_ENTRY".to_string());
    }
    assert_eq!(bot._active_entry_reason_or_default(), "RTDS_DIFF_ENTRY");
}

#[test]
fn active_entry_reason_prefers_active_over_first() {
    let bot = make_test_bot_with_mode("SNIPER");
    if let Ok(mut reason) = bot.active_entry_reason.lock() {
        *reason = Some("ACTIVE_REASON".to_string());
    }
    if let Ok(mut reason) = bot.first_entry_reason.lock() {
        *reason = Some("FIRST_REASON".to_string());
    }
    assert_eq!(bot._active_entry_reason_or_default(), "ACTIVE_REASON");
}

// ═══════════════════════════════════════════════════════════════════
// 9. _sniper_is_flat
// ═══════════════════════════════════════════════════════════════════

#[test]
fn sniper_is_flat_when_no_position() {
    let bot = make_test_bot();
    // BotState::default() has q_yes=0.0, q_no=0.0
    assert!(bot._sniper_is_flat());
}

#[test]
fn sniper_is_flat_with_yes_position() {
    let bot = make_test_bot();
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 10.0;
    }
    // min_shares default is 5.0, so 10.0 > 5.0 - eps -> not flat
    assert!(!bot._sniper_is_flat());
}

#[test]
fn sniper_is_flat_with_no_position() {
    let bot = make_test_bot();
    if let Ok(mut s) = bot.state.lock() {
        s.q_no = 10.0;
    }
    assert!(!bot._sniper_is_flat());
}

#[test]
fn sniper_is_flat_with_sub_min_shares() {
    let bot = make_test_bot();
    // min_shares default is 5.0, so 3.0 < 5.0 - eps -> flat
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 3.0;
        s.q_no = 2.0;
    }
    assert!(bot._sniper_is_flat());
}

#[test]
fn sniper_is_flat_at_min_shares_boundary() {
    let mut cfg = BotConfig::default();
    cfg.min_shares = 5.0;
    let bot = make_test_bot_with_cfg(cfg, "SNIPER");
    // exactly at min_shares - not flat (>= min_sh where min_sh = 5.0 - eps)
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 5.0;
    }
    assert!(!bot._sniper_is_flat());
}

// ═══════════════════════════════════════════════════════════════════
// 10. _sniper_is_paired_hedged
// ═══════════════════════════════════════════════════════════════════

#[test]
fn sniper_paired_hedged_when_equal_positions() {
    let bot = make_test_bot();
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 10.0;
        s.q_no = 10.0;
    }
    assert!(bot._sniper_is_paired_hedged());
}

#[test]
fn sniper_paired_hedged_when_close_positions() {
    let bot = make_test_bot();
    // min_shares default is 5.0, so gap of 4.0 < 5.0 -> hedged
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 10.0;
        s.q_no = 7.0;
    }
    assert!(bot._sniper_is_paired_hedged());
}

#[test]
fn sniper_not_paired_hedged_when_gap_too_large() {
    let bot = make_test_bot();
    // min_shares default is 5.0, gap of 6.0 >= 5.0 -> not hedged
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 11.0;
        s.q_no = 5.0;
    }
    assert!(!bot._sniper_is_paired_hedged());
}

#[test]
fn sniper_not_paired_hedged_when_flat() {
    let bot = make_test_bot();
    // both zero -> not hedged (below min_shares threshold)
    assert!(!bot._sniper_is_paired_hedged());
}

#[test]
fn sniper_not_paired_hedged_when_one_side_below_min() {
    let bot = make_test_bot();
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 10.0;
        s.q_no = 2.0; // below min_shares (5.0)
    }
    assert!(!bot._sniper_is_paired_hedged());
}

// ═══════════════════════════════════════════════════════════════════
// 11. _sniper_is_hedge_order / _sniper_mark_hedge_order /
//     _sniper_clear_hedge_order
// ═══════════════════════════════════════════════════════════════════

#[test]
fn sniper_hedge_order_initially_not_tracked() {
    let bot = make_test_bot();
    assert!(!bot._sniper_is_hedge_order("order_1"));
}

#[test]
fn sniper_mark_then_check_hedge_order() {
    let bot = make_test_bot();
    bot._sniper_mark_hedge_order("order_2");
    assert!(bot._sniper_is_hedge_order("order_2"));
}

#[test]
fn sniper_clear_hedge_order() {
    let bot = make_test_bot();
    bot._sniper_mark_hedge_order("order_3");
    bot._sniper_clear_hedge_order("order_3");
    assert!(!bot._sniper_is_hedge_order("order_3"));
}

#[test]
fn sniper_hedge_order_empty_id_returns_false() {
    let bot = make_test_bot();
    assert!(!bot._sniper_is_hedge_order(""));
    assert!(!bot._sniper_is_hedge_order("   "));
}

#[test]
fn sniper_mark_hedge_order_empty_id_is_noop() {
    let bot = make_test_bot();
    bot._sniper_mark_hedge_order("");
    // Should not crash or set anything
    assert!(!bot._sniper_is_hedge_order(""));
}

#[test]
fn sniper_hedge_order_independent_orders() {
    let bot = make_test_bot();
    bot._sniper_mark_hedge_order("order_a");
    bot._sniper_mark_hedge_order("order_b");
    bot._sniper_clear_hedge_order("order_a");
    assert!(!bot._sniper_is_hedge_order("order_a"));
    assert!(bot._sniper_is_hedge_order("order_b"));
}

// ═══════════════════════════════════════════════════════════════════
// 12. _sniper_stop_loss_reset_failures
// ═══════════════════════════════════════════════════════════════════

#[test]
fn stop_loss_reset_failures_clears_counter() {
    let bot = make_test_bot();
    let key = MakerHedgeCapBot::_sniper_stop_loss_fail_key("asset_x");
    bot._runtime_ts_set(&key, 5.0);
    bot._sniper_stop_loss_reset_failures("asset_x");
    assert!((bot._runtime_ts_get(&key) - 0.0).abs() < 1e-12);
}

#[test]
fn stop_loss_reset_failures_empty_asset_is_noop() {
    let bot = make_test_bot();
    // Should not crash
    bot._sniper_stop_loss_reset_failures("");
    bot._sniper_stop_loss_reset_failures("   ");
}

// ═══════════════════════════════════════════════════════════════════
// 13. _sniper_stop_loss_mode (reads env, tests default behavior)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn stop_loss_mode_returns_normalized_string() {
    let bot = make_test_bot();
    let mode = bot._sniper_stop_loss_mode();
    // Should be one of the normalized values
    assert!(
        mode == "MARKET" || mode == "LIMIT" || mode == "HEDGE",
        "Unexpected mode: {mode}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 14. _sniper_stop_loss_fallback_mode
// ═══════════════════════════════════════════════════════════════════

#[test]
fn stop_loss_fallback_mode_returns_string() {
    let bot = make_test_bot();
    let mode = bot._sniper_stop_loss_fallback_mode();
    // Could be empty or a normalized value
    assert!(
        mode.is_empty() || mode == "MARKET" || mode == "LIMIT" || mode == "HEDGE",
        "Unexpected fallback mode: {mode}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 15. _sniper_stop_loss_fallback_fails
// ═══════════════════════════════════════════════════════════════════

#[test]
fn stop_loss_fallback_fails_default_is_positive() {
    let bot = make_test_bot();
    let fails = bot._sniper_stop_loss_fallback_fails();
    assert!(fails >= 1.0, "Expected >= 1.0, got {fails}");
}

// ═══════════════════════════════════════════════════════════════════
// 16. Maker TTL / timing config methods (defaults)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn maker_single_inflight_returns_bool() {
    let bot = make_test_bot();
    // Just verify it returns without panic; default is true
    let _ = bot._maker_single_inflight_enabled();
}

#[test]
fn maker_submit_pending_ttl_at_least_half_second() {
    let bot = make_test_bot();
    assert!(bot._maker_submit_pending_ttl_seconds() >= 0.5);
}

#[test]
fn maker_cancel_pending_ttl_at_least_half_second() {
    let bot = make_test_bot();
    assert!(bot._maker_cancel_pending_ttl_seconds() >= 0.5);
}

#[test]
fn maker_working_missing_ttl_at_least_one_second() {
    let bot = make_test_bot();
    assert!(bot._maker_working_missing_ttl_seconds() >= 1.0);
}

#[test]
fn maker_replace_min_interval_non_negative() {
    let bot = make_test_bot();
    assert!(bot._maker_replace_min_interval_seconds() >= 0.0);
}

#[test]
fn maker_submit_reject_cooldown_non_negative() {
    let bot = make_test_bot();
    assert!(bot._maker_submit_reject_cooldown_seconds() >= 0.0);
}

// ═══════════════════════════════════════════════════════════════════
// 17. _pair_arb_imbalance_enter_shares
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pair_arb_imbalance_enter_shares_non_negative() {
    let bot = make_test_bot();
    assert!(bot._pair_arb_imbalance_enter_shares() >= 0.0);
}

#[test]
fn pair_arb_imbalance_enter_shares_at_least_min_shares() {
    let bot = make_test_bot();
    // Default: env not set -> uses cfg.min_shares.max(1.0) = 5.0
    let enter = bot._pair_arb_imbalance_enter_shares();
    assert!(
        enter >= bot.cfg.min_shares.max(1.0),
        "Expected >= {}, got {enter}",
        bot.cfg.min_shares.max(1.0)
    );
}

// ═══════════════════════════════════════════════════════════════════
// 18. _pair_arb_imbalance_release_shares
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pair_arb_imbalance_release_shares_non_negative() {
    let bot = make_test_bot();
    assert!(bot._pair_arb_imbalance_release_shares() >= 0.0);
}

#[test]
fn pair_arb_imbalance_release_capped_by_enter() {
    let bot = make_test_bot();
    let release = bot._pair_arb_imbalance_release_shares();
    let enter = bot._pair_arb_imbalance_enter_shares();
    assert!(
        release <= enter + 1e-12,
        "release={release} should be <= enter={enter}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 19. _pair_base_mode_enabled
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pair_base_mode_disabled_by_default() {
    let bot = make_test_bot();
    // Default: PAIR_BASE_ENABLED=false, so should be disabled
    // (also MAKER_SKEW_ENABLED defaults to true, which blocks it)
    assert!(!bot._pair_base_mode_enabled());
}

// ═══════════════════════════════════════════════════════════════════
// 20. _pair_recovery_enabled
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pair_recovery_enabled_default() {
    let bot = make_test_bot();
    // Default: PAIR_RECOVERY_ENABLED=true
    let _ = bot._pair_recovery_enabled();
}

// ═══════════════════════════════════════════════════════════════════
// 21. _pair_base_window_budget
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pair_base_window_budget_positive() {
    let bot = make_test_bot();
    assert!(bot._pair_base_window_budget() >= 1.0);
}

#[test]
fn pair_base_window_budget_capped_by_max_total_cost() {
    let bot = make_test_bot();
    let budget = bot._pair_base_window_budget();
    assert!(
        budget <= bot.cfg.max_total_cost.max(1.0) + 1e-12,
        "budget={budget} should be <= max_total_cost={}",
        bot.cfg.max_total_cost
    );
}

// ═══════════════════════════════════════════════════════════════════
// 22. _pair_base_merge_budget
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pair_base_merge_budget_positive() {
    let bot = make_test_bot();
    assert!(bot._pair_base_merge_budget() >= 1.0);
}

#[test]
fn pair_base_merge_budget_capped_by_max_total_cost() {
    let bot = make_test_bot();
    let budget = bot._pair_base_merge_budget();
    assert!(
        budget <= bot.cfg.max_total_cost.max(1.0) + 1e-12,
        "budget={budget} should be <= max_total_cost={}",
        bot.cfg.max_total_cost
    );
}

// ═══════════════════════════════════════════════════════════════════
// 23. _pair_base_hard_reserve
// ═══════════════════════════════════════════════════════════════════

#[test]
fn pair_base_hard_reserve_non_negative() {
    let bot = make_test_bot();
    assert!(bot._pair_base_hard_reserve() >= 0.0);
}

#[test]
fn pair_base_hard_reserve_capped_by_window_budget() {
    let bot = make_test_bot();
    let reserve = bot._pair_base_hard_reserve();
    let budget = bot._pair_base_window_budget();
    assert!(
        reserve <= budget + 1e-12,
        "reserve={reserve} should be <= budget={budget}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 24. Integration: test bot with custom config
// ═══════════════════════════════════════════════════════════════════

#[test]
fn custom_cfg_min_shares_affects_is_flat() {
    let mut cfg = BotConfig::default();
    cfg.min_shares = 1.0;
    let bot = make_test_bot_with_cfg(cfg, "SNIPER");
    // q_yes=0, q_no=0 -> flat (both < 1.0 - eps)
    assert!(bot._sniper_is_flat());
    // set q_yes to 1.5, which is > 1.0 - eps
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 1.5;
    }
    assert!(!bot._sniper_is_flat());
}

#[test]
fn custom_cfg_min_shares_affects_paired_hedged() {
    let mut cfg = BotConfig::default();
    cfg.min_shares = 2.0;
    let bot = make_test_bot_with_cfg(cfg, "SNIPER");
    // Both sides >= min_shares, gap < min_shares -> hedged
    if let Ok(mut s) = bot.state.lock() {
        s.q_yes = 5.0;
        s.q_no = 4.0; // gap = 1.0 < 2.0
    }
    assert!(bot._sniper_is_paired_hedged());
    // gap = 3.0 >= 2.0 -> not hedged
    if let Ok(mut s) = bot.state.lock() {
        s.q_no = 2.0; // gap = 3.0
    }
    assert!(!bot._sniper_is_paired_hedged());
}

#[test]
fn custom_cfg_max_total_cost_affects_budgets() {
    let mut cfg = BotConfig::default();
    cfg.max_total_cost = 100.0;
    cfg.reserve_usd = 10.0;
    let bot = make_test_bot_with_cfg(cfg, "MAKER");
    let window = bot._pair_base_window_budget();
    let merge = bot._pair_base_merge_budget();
    let reserve = bot._pair_base_hard_reserve();
    assert!(window <= 100.0 + 1e-12);
    assert!(merge <= 100.0 + 1e-12);
    assert!(reserve <= window + 1e-12);
}
