use super::*;
use serde_json::json;

// ───────────────────────────────────────────────────────────────────
// 1. PairBasePhaseState::as_str
// ───────────────────────────────────────────────────────────────────

#[test]
fn phase_state_as_str_flat() {
    assert_eq!(PairBasePhaseState::Flat.as_str(), "Flat");
}

#[test]
fn phase_state_as_str_pair_resting() {
    assert_eq!(PairBasePhaseState::PairResting.as_str(), "PairResting");
}

#[test]
fn phase_state_as_str_merge_pending() {
    assert_eq!(PairBasePhaseState::MergePending.as_str(), "MergePending");
}

#[test]
fn phase_state_as_str_balanced() {
    assert_eq!(PairBasePhaseState::Balanced.as_str(), "Balanced");
}

#[test]
fn phase_state_as_str_risk_exit_only() {
    assert_eq!(PairBasePhaseState::RiskExitOnly.as_str(), "RiskExitOnly");
}

#[test]
fn phase_state_default_is_flat() {
    assert_eq!(PairBasePhaseState::default(), PairBasePhaseState::Flat);
}

// ───────────────────────────────────────────────────────────────────
// 2. SniperPostHedgePolicy::as_str
// ───────────────────────────────────────────────────────────────────

#[test]
fn post_hedge_policy_as_str_hybrid() {
    assert_eq!(SniperPostHedgePolicy::HybridTimed.as_str(), "HYBRID_TIMED");
}

#[test]
fn post_hedge_policy_as_str_hold() {
    assert_eq!(
        SniperPostHedgePolicy::HoldToResolution.as_str(),
        "HOLD_TO_RESOLUTION"
    );
}

#[test]
fn post_hedge_policy_as_str_immediate() {
    assert_eq!(
        SniperPostHedgePolicy::ImmediateUnwind.as_str(),
        "IMMEDIATE_UNWIND"
    );
}

// ───────────────────────────────────────────────────────────────────
// 3. MakerOrderKey::buy
// ───────────────────────────────────────────────────────────────────

#[test]
fn maker_order_key_buy_trims_asset_id() {
    let key = MakerOrderKey::buy("  abc123  ");
    assert_eq!(key.asset_id, "abc123");
    assert_eq!(key.side, "BUY");
}

#[test]
fn maker_order_key_buy_equality() {
    let a = MakerOrderKey::buy("token_a");
    let b = MakerOrderKey::buy("token_a");
    assert_eq!(a, b);
}

#[test]
fn maker_order_key_buy_hash_consistency() {
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(MakerOrderKey::buy("token_x"));
    assert!(set.contains(&MakerOrderKey::buy("token_x")));
    assert!(!set.contains(&MakerOrderKey::buy("token_y")));
}

// ───────────────────────────────────────────────────────────────────
// 4. _clob_order_type
// ───────────────────────────────────────────────────────────────────

#[test]
fn clob_order_type_fak() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_order_type("FAK"),
        ClobOrderType::Fak
    ));
}

#[test]
fn clob_order_type_fok() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_order_type("fok"),
        ClobOrderType::Fok
    ));
}

#[test]
fn clob_order_type_gtd() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_order_type("Gtd"),
        ClobOrderType::Gtd
    ));
}

#[test]
fn clob_order_type_gtc_default() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_order_type("GTC"),
        ClobOrderType::Gtc
    ));
}

#[test]
fn clob_order_type_unknown_defaults_to_gtc() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_order_type("INVALID"),
        ClobOrderType::Gtc
    ));
}

#[test]
fn clob_order_type_handles_whitespace() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_order_type("  fak  "),
        ClobOrderType::Fak
    ));
}

// ───────────────────────────────────────────────────────────────────
// 5. _clob_side
// ───────────────────────────────────────────────────────────────────

#[test]
fn clob_side_buy() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_side("BUY"),
        Some(ClobSide::Buy)
    ));
}

#[test]
fn clob_side_sell() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_side("sell"),
        Some(ClobSide::Sell)
    ));
}

#[test]
fn clob_side_case_insensitive() {
    assert!(matches!(
        MakerHedgeCapBot::_clob_side("  Buy  "),
        Some(ClobSide::Buy)
    ));
}

#[test]
fn clob_side_invalid_returns_none() {
    assert!(MakerHedgeCapBot::_clob_side("HOLD").is_none());
}

#[test]
fn clob_side_empty_returns_none() {
    assert!(MakerHedgeCapBot::_clob_side("").is_none());
}

// ───────────────────────────────────────────────────────────────────
// 6. _tick_size_from_f64
// ───────────────────────────────────────────────────────────────────

#[test]
fn tick_size_0_1() {
    assert!(matches!(
        MakerHedgeCapBot::_tick_size_from_f64(0.1),
        TickSize::ZeroPointOne
    ));
}

#[test]
fn tick_size_0_01() {
    assert!(matches!(
        MakerHedgeCapBot::_tick_size_from_f64(0.01),
        TickSize::ZeroPointZeroOne
    ));
}

#[test]
fn tick_size_0_001() {
    assert!(matches!(
        MakerHedgeCapBot::_tick_size_from_f64(0.001),
        TickSize::ZeroPointZeroZeroOne
    ));
}

#[test]
fn tick_size_0_0001_fallback() {
    assert!(matches!(
        MakerHedgeCapBot::_tick_size_from_f64(0.0001),
        TickSize::ZeroPointZeroZeroZeroOne
    ));
}

#[test]
fn tick_size_unusual_value_falls_to_smallest() {
    assert!(matches!(
        MakerHedgeCapBot::_tick_size_from_f64(0.05),
        TickSize::ZeroPointZeroZeroZeroOne
    ));
}

// ───────────────────────────────────────────────────────────────────
// 7. _value_f64
// ───────────────────────────────────────────────────────────────────

#[test]
fn value_f64_from_number() {
    let v = json!(42.5);
    assert_eq!(MakerHedgeCapBot::_value_f64(Some(&v)), Some(42.5));
}

#[test]
fn value_f64_from_string() {
    let v = json!("3.14");
    assert_eq!(MakerHedgeCapBot::_value_f64(Some(&v)), Some(3.14));
}

#[test]
fn value_f64_from_integer() {
    let v = json!(100);
    assert_eq!(MakerHedgeCapBot::_value_f64(Some(&v)), Some(100.0));
}

#[test]
fn value_f64_none_input() {
    assert_eq!(MakerHedgeCapBot::_value_f64(None), None);
}

#[test]
fn value_f64_non_numeric_string() {
    let v = json!("not_a_number");
    assert_eq!(MakerHedgeCapBot::_value_f64(Some(&v)), None);
}

#[test]
fn value_f64_bool_returns_none() {
    let v = json!(true);
    assert_eq!(MakerHedgeCapBot::_value_f64(Some(&v)), None);
}

#[test]
fn value_f64_null_returns_none() {
    let v = json!(null);
    assert_eq!(MakerHedgeCapBot::_value_f64(Some(&v)), None);
}

// ───────────────────────────────────────────────────────────────────
// 8. _max_numeric_in_value
// ───────────────────────────────────────────────────────────────────

#[test]
fn max_numeric_single_number() {
    let v = json!(7.0);
    assert_eq!(MakerHedgeCapBot::_max_numeric_in_value(Some(&v)), Some(7.0));
}

#[test]
fn max_numeric_array() {
    let v = json!([1, 5, 3, 9, 2]);
    assert_eq!(MakerHedgeCapBot::_max_numeric_in_value(Some(&v)), Some(9.0));
}

#[test]
fn max_numeric_nested_object() {
    let v = json!({"a": 10, "b": {"c": 20, "d": [5, 30]}});
    assert_eq!(
        MakerHedgeCapBot::_max_numeric_in_value(Some(&v)),
        Some(30.0)
    );
}

#[test]
fn max_numeric_string_numbers() {
    let v = json!(["100", "200", "50"]);
    assert_eq!(
        MakerHedgeCapBot::_max_numeric_in_value(Some(&v)),
        Some(200.0)
    );
}

#[test]
fn max_numeric_mixed_types() {
    let v = json!({"x": "99.5", "y": 50, "z": true});
    assert_eq!(
        MakerHedgeCapBot::_max_numeric_in_value(Some(&v)),
        Some(99.5)
    );
}

#[test]
fn max_numeric_none_input() {
    assert_eq!(MakerHedgeCapBot::_max_numeric_in_value(None), None);
}

#[test]
fn max_numeric_empty_array() {
    let v = json!([]);
    assert_eq!(MakerHedgeCapBot::_max_numeric_in_value(Some(&v)), None);
}

#[test]
fn max_numeric_no_numbers() {
    let v = json!({"a": "hello", "b": true, "c": null});
    assert_eq!(MakerHedgeCapBot::_max_numeric_in_value(Some(&v)), None);
}

// ───────────────────────────────────────────────────────────────────
// 9. _extract_posted_order_id
// ───────────────────────────────────────────────────────────────────

#[test]
fn extract_order_id_from_order_id_field() {
    let v = json!({"orderID": "ord_123"});
    assert_eq!(
        MakerHedgeCapBot::_extract_posted_order_id(&v),
        Some("ord_123".to_string())
    );
}

#[test]
fn extract_order_id_from_snake_case() {
    let v = json!({"order_id": "ord_456"});
    assert_eq!(
        MakerHedgeCapBot::_extract_posted_order_id(&v),
        Some("ord_456".to_string())
    );
}

#[test]
fn extract_order_id_from_id() {
    let v = json!({"id": "ord_789"});
    assert_eq!(
        MakerHedgeCapBot::_extract_posted_order_id(&v),
        Some("ord_789".to_string())
    );
}

#[test]
fn extract_order_id_from_nested_order() {
    let v = json!({"order": {"id": "nested_id"}});
    assert_eq!(
        MakerHedgeCapBot::_extract_posted_order_id(&v),
        Some("nested_id".to_string())
    );
}

#[test]
fn extract_order_id_from_nested_order_order_id() {
    let v = json!({"order": {"order_id": "nested_oid"}});
    assert_eq!(
        MakerHedgeCapBot::_extract_posted_order_id(&v),
        Some("nested_oid".to_string())
    );
}

#[test]
fn extract_order_id_priority_orderid_first() {
    let v = json!({"orderID": "first", "order_id": "second", "id": "third"});
    assert_eq!(
        MakerHedgeCapBot::_extract_posted_order_id(&v),
        Some("first".to_string())
    );
}

#[test]
fn extract_order_id_missing() {
    let v = json!({"status": "ok"});
    assert_eq!(MakerHedgeCapBot::_extract_posted_order_id(&v), None);
}

#[test]
fn extract_order_id_non_string_returns_none() {
    let v = json!({"orderID": 12345});
    assert_eq!(MakerHedgeCapBot::_extract_posted_order_id(&v), None);
}

// ───────────────────────────────────────────────────────────────────
// 10. _normalize_open_orders_payload
// ───────────────────────────────────────────────────────────────────

#[test]
fn normalize_orders_from_array() {
    let payload = json!([{"id": "o1", "asset_id": "a1", "price": 0.5, "size": 10.0, "side": "buy"}]);
    let result = MakerHedgeCapBot::_normalize_open_orders_payload(&payload);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["id"], "o1");
    assert_eq!(result[0]["asset_id"], "a1");
    assert_eq!(result[0]["side"], "BUY");
}

#[test]
fn normalize_orders_from_data_wrapper() {
    let payload = json!({"data": [{"id": "o2", "asset_id": "a2", "price": 0.3, "side": "sell"}]});
    let result = MakerHedgeCapBot::_normalize_open_orders_payload(&payload);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["id"], "o2");
    assert_eq!(result[0]["side"], "SELL");
}

#[test]
fn normalize_orders_from_orders_wrapper() {
    let payload = json!({"orders": [{"order_id": "o3", "token_id": "t3", "price": "0.6", "side": "BUY"}]});
    let result = MakerHedgeCapBot::_normalize_open_orders_payload(&payload);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["order_id"], "o3");
    assert_eq!(result[0]["token_id"], "t3");
}

#[test]
fn normalize_orders_from_results_wrapper() {
    let payload = json!({"results": [{"orderID": "o4", "assetId": "a4", "price": 0.7, "side": "sell"}]});
    let result = MakerHedgeCapBot::_normalize_open_orders_payload(&payload);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["id"], "o4");
}

#[test]
fn normalize_orders_skips_empty_id() {
    let payload = json!([{"price": 0.5, "side": "buy"}]);
    let result = MakerHedgeCapBot::_normalize_open_orders_payload(&payload);
    assert_eq!(result.len(), 0);
}

#[test]
fn normalize_orders_empty_payload() {
    let payload = json!({});
    let result = MakerHedgeCapBot::_normalize_open_orders_payload(&payload);
    assert_eq!(result.len(), 0);
}

#[test]
fn normalize_orders_calculates_remaining_from_original_minus_matched() {
    let payload = json!([{
        "id": "o5",
        "asset_id": "a5",
        "price": 0.5,
        "original_size": 100.0,
        "size_matched": 30.0,
        "side": "buy"
    }]);
    let result = MakerHedgeCapBot::_normalize_open_orders_payload(&payload);
    assert_eq!(result.len(), 1);
    let remaining = result[0]["remaining_size"].as_f64().unwrap();
    assert!((remaining - 70.0).abs() < 1e-9);
}

// ───────────────────────────────────────────────────────────────────
// 11. _is_sniper_like_mode
// ───────────────────────────────────────────────────────────────────

#[test]
fn is_sniper_like_mode_sniper() {
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("SNIPER"));
}

#[test]
fn is_sniper_like_mode_prob_sniper() {
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("PROB_SNIPER"));
}

#[test]
fn is_sniper_like_mode_high_prob() {
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("HIGH_PROB"));
}

#[test]
fn is_sniper_like_mode_high_prob_sniper() {
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("HIGH_PROB_SNIPER"));
}

#[test]
fn is_sniper_like_mode_fixed_profit() {
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("FIXED_PROFIT"));
}

#[test]
fn is_sniper_like_mode_signal_variants() {
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("SIGNAL_SNIPPER"));
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("SIGNAL_SNIPER"));
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("SIGNAL_SNIPE"));
    assert!(MakerHedgeCapBot::_is_sniper_like_mode("SIGNAL"));
}

#[test]
fn is_sniper_like_mode_non_sniper() {
    assert!(!MakerHedgeCapBot::_is_sniper_like_mode("MAKER"));
    assert!(!MakerHedgeCapBot::_is_sniper_like_mode("TAKER"));
    assert!(!MakerHedgeCapBot::_is_sniper_like_mode("PAIR_ARB"));
    assert!(!MakerHedgeCapBot::_is_sniper_like_mode(""));
}

// ───────────────────────────────────────────────────────────────────
// 12. _sniper_submit_order_type_from_origin
// ───────────────────────────────────────────────────────────────────

#[test]
fn sniper_order_type_fak_origin() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_submit_order_type_from_origin("TAKER_FAK_ENTRY"),
        "FAK"
    );
}

#[test]
fn sniper_order_type_fok_origin() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_submit_order_type_from_origin("fok_order"),
        "FOK"
    );
}

#[test]
fn sniper_order_type_gtc_origin() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_submit_order_type_from_origin("GTC_LIMIT_ENTRY"),
        "GTC"
    );
}

#[test]
fn sniper_order_type_limit_origin() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_submit_order_type_from_origin("LIMIT_ORDER"),
        "GTC"
    );
}

#[test]
fn sniper_order_type_unknown_origin() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_submit_order_type_from_origin("SOME_OTHER"),
        ""
    );
}

#[test]
fn sniper_order_type_whitespace_handling() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_submit_order_type_from_origin("  fak_entry  "),
        "FAK"
    );
}

// ───────────────────────────────────────────────────────────────────
// 13. _sniper_order_kind_from_origin
// ───────────────────────────────────────────────────────────────────

#[test]
fn sniper_order_kind_taker() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_order_kind_from_origin("TAKER_ENTRY"),
        "taker"
    );
}

#[test]
fn sniper_order_kind_maker_from_limit() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_order_kind_from_origin("LIMIT_ORDER"),
        "maker"
    );
}

#[test]
fn sniper_order_kind_maker_from_postonly() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_order_kind_from_origin("POSTONLY_BID"),
        "maker"
    );
}

#[test]
fn sniper_order_kind_maker_from_maker() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_order_kind_from_origin("MAKER_ENTRY"),
        "maker"
    );
}

#[test]
fn sniper_order_kind_unknown() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_order_kind_from_origin("SOMETHING_ELSE"),
        ""
    );
}

// ───────────────────────────────────────────────────────────────────
// 14. Key builder functions
// ───────────────────────────────────────────────────────────────────

#[test]
fn sniper_hedge_oid_key_format() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_hedge_oid_key("order_123"),
        "__sniper_hedge_oid_order_123"
    );
}

#[test]
fn sniper_hedge_last_remaining_key_format() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_hedge_last_remaining_key("order_456"),
        "__sniper_hedge_last_remaining_order_456"
    );
}

#[test]
fn sniper_stop_loss_fail_key_format() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_stop_loss_fail_key("asset_789"),
        "__sniper_stop_loss_sell_failures_asset_789"
    );
}

#[test]
fn sniper_entry_pending_key_format() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_entry_pending_key("asset_abc"),
        "__sniper_entry_pending_asset_abc"
    );
}

#[test]
fn sniper_entry_confirmed_key_format() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_entry_confirmed_key("asset_def"),
        "__sniper_entry_confirmed_asset_def"
    );
}

// ───────────────────────────────────────────────────────────────────
// 15. _sniper_normalize_stop_loss_mode
// ───────────────────────────────────────────────────────────────────

#[test]
fn normalize_stop_loss_mode_stop_limit_to_limit() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("STOP_LIMIT"),
        "LIMIT"
    );
}

#[test]
fn normalize_stop_loss_mode_stoplimit_to_limit() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("stoplimit"),
        "LIMIT"
    );
}

#[test]
fn normalize_stop_loss_mode_stop_hedge_to_hedge() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("STOP_HEDGE"),
        "HEDGE"
    );
}

#[test]
fn normalize_stop_loss_mode_hedge_stop_to_hedge() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("hedge_stop"),
        "HEDGE"
    );
}

#[test]
fn normalize_stop_loss_mode_stop_market_to_market() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("STOP_MARKET"),
        "MARKET"
    );
}

#[test]
fn normalize_stop_loss_mode_taker_to_market() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("taker"),
        "MARKET"
    );
}

#[test]
fn normalize_stop_loss_mode_aggressive_to_market() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("aggressive"),
        "MARKET"
    );
}

#[test]
fn normalize_stop_loss_mode_passthrough() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("LIMIT"),
        "LIMIT"
    );
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("hedge"),
        "HEDGE"
    );
}

#[test]
fn normalize_stop_loss_mode_trims_whitespace() {
    assert_eq!(
        MakerHedgeCapBot::_sniper_normalize_stop_loss_mode("  stop_limit  "),
        "LIMIT"
    );
}

// ───────────────────────────────────────────────────────────────────
// 16. _force_diff_entry_reason
// ───────────────────────────────────────────────────────────────────

#[test]
fn force_diff_entry_reason_positive() {
    assert!(MakerHedgeCapBot::_force_diff_entry_reason(
        "SNIPER_FORCE_DIFF_ENTRY"
    ));
    assert!(MakerHedgeCapBot::_force_diff_entry_reason(
        "RTDS_DIFF_TIME_OVERRIDE"
    ));
}

#[test]
fn force_diff_entry_reason_case_insensitive() {
    assert!(MakerHedgeCapBot::_force_diff_entry_reason(
        "sniper_force_diff_entry"
    ));
    assert!(MakerHedgeCapBot::_force_diff_entry_reason(
        "  Rtds_Diff_Time_Override  "
    ));
}

#[test]
fn force_diff_entry_reason_negative() {
    assert!(!MakerHedgeCapBot::_force_diff_entry_reason("SNIPER_ENTRY"));
    assert!(!MakerHedgeCapBot::_force_diff_entry_reason(""));
    assert!(!MakerHedgeCapBot::_force_diff_entry_reason("SOMETHING"));
}

// ───────────────────────────────────────────────────────────────────
// 17. _maker_compute_rsi
// ───────────────────────────────────────────────────────────────────

#[test]
fn rsi_not_enough_data() {
    assert_eq!(MakerHedgeCapBot::_maker_compute_rsi(&[10.0, 11.0], 5), None);
}

#[test]
fn rsi_period_too_small() {
    assert_eq!(
        MakerHedgeCapBot::_maker_compute_rsi(&[10.0, 11.0, 12.0], 1),
        None
    );
}

#[test]
fn rsi_all_gains_is_100() {
    let closes = vec![10.0, 11.0, 12.0, 13.0, 14.0, 15.0];
    let rsi = MakerHedgeCapBot::_maker_compute_rsi(&closes, 5).unwrap();
    assert!((rsi - 100.0).abs() < 1e-9);
}

#[test]
fn rsi_all_losses_is_0() {
    let closes = vec![15.0, 14.0, 13.0, 12.0, 11.0, 10.0];
    let rsi = MakerHedgeCapBot::_maker_compute_rsi(&closes, 5).unwrap();
    assert!(rsi.abs() < 1e-9);
}

#[test]
fn rsi_equal_gains_losses_is_50() {
    // alternating +1, -1 -> equal avg gain and avg loss
    let closes = vec![10.0, 11.0, 10.0, 11.0, 10.0, 11.0, 10.0];
    let rsi = MakerHedgeCapBot::_maker_compute_rsi(&closes, 6).unwrap();
    assert!((rsi - 50.0).abs() < 1e-9);
}

#[test]
fn rsi_typical_range() {
    let closes = vec![44.0, 44.3, 44.1, 43.6, 44.3, 44.8, 45.1, 45.4, 45.1, 45.3, 44.9];
    let rsi = MakerHedgeCapBot::_maker_compute_rsi(&closes, 10).unwrap();
    assert!(rsi > 0.0 && rsi < 100.0);
}

// ───────────────────────────────────────────────────────────────────
// 18. _maker_exec_record_matches
// ───────────────────────────────────────────────────────────────────

fn sample_candidate() -> MakerExecCandidate {
    MakerExecCandidate {
        order_id: "o1".to_string(),
        asset_id: "a1".to_string(),
        side: "BUY".to_string(),
        qty: 10.0,
        price: 0.55,
        tx_hash: Some("tx1".to_string()),
        trade_id: Some("tr1".to_string()),
        taker_order_id: Some("taker1".to_string()),
        match_time: Some("2024-01-01T00:00:00Z".to_string()),
    }
}

fn sample_record() -> MakerExecRecord {
    MakerExecRecord {
        canonical_id: "c1".to_string(),
        order_id: "o1".to_string(),
        qty: 10.0,
        price: 0.55,
        asset_id: "a1".to_string(),
        side: "BUY".to_string(),
        aliases: vec![],
        applied_ts: 0.0,
    }
}

#[test]
fn exec_record_matches_identical() {
    assert!(MakerHedgeCapBot::_maker_exec_record_matches(
        &sample_record(),
        &sample_candidate()
    ));
}

#[test]
fn exec_record_mismatch_order_id() {
    let mut rec = sample_record();
    rec.order_id = "different".to_string();
    assert!(!MakerHedgeCapBot::_maker_exec_record_matches(
        &rec,
        &sample_candidate()
    ));
}

#[test]
fn exec_record_mismatch_side() {
    let mut rec = sample_record();
    rec.side = "SELL".to_string();
    assert!(!MakerHedgeCapBot::_maker_exec_record_matches(
        &rec,
        &sample_candidate()
    ));
}

#[test]
fn exec_record_mismatch_qty() {
    let mut rec = sample_record();
    rec.qty = 20.0;
    assert!(!MakerHedgeCapBot::_maker_exec_record_matches(
        &rec,
        &sample_candidate()
    ));
}

#[test]
fn exec_record_mismatch_price() {
    let mut rec = sample_record();
    rec.price = 0.99;
    assert!(!MakerHedgeCapBot::_maker_exec_record_matches(
        &rec,
        &sample_candidate()
    ));
}

#[test]
fn exec_record_matches_within_epsilon() {
    let mut rec = sample_record();
    rec.qty = 10.0 + 1e-10; // within EPS of 1e-9
    assert!(MakerHedgeCapBot::_maker_exec_record_matches(
        &rec,
        &sample_candidate()
    ));
}

// ───────────────────────────────────────────────────────────────────
// 19. _maker_exec_order_sum
// ───────────────────────────────────────────────────────────────────

#[test]
fn exec_order_sum_empty_ledger() {
    let ledger = MakerExecLedger::default();
    assert!((MakerHedgeCapBot::_maker_exec_order_sum(&ledger, "o1")).abs() < 1e-9);
}

#[test]
fn exec_order_sum_empty_order_id() {
    let ledger = MakerExecLedger::default();
    assert!((MakerHedgeCapBot::_maker_exec_order_sum(&ledger, "")).abs() < 1e-9);
}

#[test]
fn exec_order_sum_single_record() {
    let mut ledger = MakerExecLedger::default();
    ledger.records.insert(
        "c1".to_string(),
        MakerExecRecord {
            canonical_id: "c1".to_string(),
            order_id: "o1".to_string(),
            qty: 15.0,
            price: 0.5,
            asset_id: "a1".to_string(),
            side: "BUY".to_string(),
            aliases: vec![],
            applied_ts: 0.0,
        },
    );
    let sum = MakerHedgeCapBot::_maker_exec_order_sum(&ledger, "o1");
    assert!((sum - 15.0).abs() < 1e-9);
}

#[test]
fn exec_order_sum_multiple_records() {
    let mut ledger = MakerExecLedger::default();
    for (i, qty) in [10.0, 20.0, 5.0].iter().enumerate() {
        ledger.records.insert(
            format!("c{}", i),
            MakerExecRecord {
                canonical_id: format!("c{}", i),
                order_id: "o1".to_string(),
                qty: *qty,
                price: 0.5,
                asset_id: "a1".to_string(),
                side: "BUY".to_string(),
                aliases: vec![],
                applied_ts: 0.0,
            },
        );
    }
    let sum = MakerHedgeCapBot::_maker_exec_order_sum(&ledger, "o1");
    assert!((sum - 35.0).abs() < 1e-9);
}

#[test]
fn exec_order_sum_filters_by_order_id() {
    let mut ledger = MakerExecLedger::default();
    ledger.records.insert(
        "c1".to_string(),
        MakerExecRecord {
            canonical_id: "c1".to_string(),
            order_id: "o1".to_string(),
            qty: 10.0,
            price: 0.5,
            asset_id: "a1".to_string(),
            side: "BUY".to_string(),
            aliases: vec![],
            applied_ts: 0.0,
        },
    );
    ledger.records.insert(
        "c2".to_string(),
        MakerExecRecord {
            canonical_id: "c2".to_string(),
            order_id: "o2".to_string(),
            qty: 99.0,
            price: 0.5,
            asset_id: "a1".to_string(),
            side: "BUY".to_string(),
            aliases: vec![],
            applied_ts: 0.0,
        },
    );
    let sum = MakerHedgeCapBot::_maker_exec_order_sum(&ledger, "o1");
    assert!((sum - 10.0).abs() < 1e-9);
}

// ───────────────────────────────────────────────────────────────────
// 20. _maker_trade_exec_aliases
// ───────────────────────────────────────────────────────────────────

#[test]
fn exec_aliases_all_fields() {
    let c = sample_candidate();
    let aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&c);
    assert_eq!(aliases.len(), 3);
    assert!(aliases[0].starts_with("maker_tx:"));
    assert!(aliases[1].starts_with("maker_trade:"));
    assert!(aliases[2].starts_with("maker_match:"));
}

#[test]
fn exec_aliases_only_tx_hash() {
    let c = MakerExecCandidate {
        order_id: "o1".to_string(),
        asset_id: "a1".to_string(),
        side: "BUY".to_string(),
        qty: 5.0,
        price: 0.5,
        tx_hash: Some("tx_abc".to_string()),
        trade_id: None,
        taker_order_id: None,
        match_time: None,
    };
    let aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&c);
    assert_eq!(aliases.len(), 1);
    assert!(aliases[0].starts_with("maker_tx:"));
}

#[test]
fn exec_aliases_no_optional_fields() {
    let c = MakerExecCandidate {
        order_id: "o1".to_string(),
        asset_id: "a1".to_string(),
        side: "BUY".to_string(),
        qty: 5.0,
        price: 0.5,
        tx_hash: None,
        trade_id: None,
        taker_order_id: None,
        match_time: None,
    };
    let aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&c);
    assert!(aliases.is_empty());
}

#[test]
fn exec_aliases_match_requires_both_taker_and_match_time() {
    let c = MakerExecCandidate {
        order_id: "o1".to_string(),
        asset_id: "a1".to_string(),
        side: "BUY".to_string(),
        qty: 5.0,
        price: 0.5,
        tx_hash: None,
        trade_id: None,
        taker_order_id: Some("taker1".to_string()),
        match_time: None, // missing match_time
    };
    let aliases = MakerHedgeCapBot::_maker_trade_exec_aliases(&c);
    assert!(aliases.is_empty());
}

// ───────────────────────────────────────────────────────────────────
// 21. _maker_exec_alias_kind
// ───────────────────────────────────────────────────────────────────

#[test]
fn exec_alias_kind_tx() {
    assert_eq!(MakerHedgeCapBot::_maker_exec_alias_kind("maker_tx:o1:hash:1.0:0.5"), "tx");
}

#[test]
fn exec_alias_kind_trade() {
    assert_eq!(MakerHedgeCapBot::_maker_exec_alias_kind("maker_trade:o1:tr1"), "trade");
}

#[test]
fn exec_alias_kind_match() {
    assert_eq!(
        MakerHedgeCapBot::_maker_exec_alias_kind("maker_match:o1:taker1:time:1.0:0.5"),
        "match"
    );
}

#[test]
fn exec_alias_kind_unknown() {
    assert_eq!(MakerHedgeCapBot::_maker_exec_alias_kind("something_else"), "unknown");
}

// ───────────────────────────────────────────────────────────────────
// 22. _env_first (reads env vars — isolated tests)
// ───────────────────────────────────────────────────────────────────

#[test]
fn env_first_returns_first_set() {
    let key = "__TEST_ENV_FIRST_A_8372";
    std::env::set_var(key, "hello");
    let result = MakerHedgeCapBot::_env_first(&[key]);
    assert_eq!(result, "hello");
    std::env::remove_var(key);
}

#[test]
fn env_first_skips_empty() {
    let key1 = "__TEST_ENV_FIRST_EMPTY_1";
    let key2 = "__TEST_ENV_FIRST_FULL_1";
    std::env::set_var(key1, "  ");
    std::env::set_var(key2, "found");
    let result = MakerHedgeCapBot::_env_first(&[key1, key2]);
    assert_eq!(result, "found");
    std::env::remove_var(key1);
    std::env::remove_var(key2);
}

#[test]
fn env_first_returns_empty_when_none_set() {
    let result = MakerHedgeCapBot::_env_first(&[
        "__TEST_ENV_FIRST_MISSING_X1",
        "__TEST_ENV_FIRST_MISSING_X2",
    ]);
    assert_eq!(result, "");
}

// ───────────────────────────────────────────────────────────────────
// 23. _env_positive_float_if_set
// ───────────────────────────────────────────────────────────────────

#[test]
fn env_positive_float_positive_value() {
    let key = "__TEST_POS_FLOAT_1";
    std::env::set_var(key, "3.5");
    assert_eq!(MakerHedgeCapBot::_env_positive_float_if_set(key), Some(3.5));
    std::env::remove_var(key);
}

#[test]
fn env_positive_float_zero_returns_none() {
    let key = "__TEST_POS_FLOAT_ZERO";
    std::env::set_var(key, "0");
    assert_eq!(MakerHedgeCapBot::_env_positive_float_if_set(key), None);
    std::env::remove_var(key);
}

#[test]
fn env_positive_float_negative_returns_none() {
    let key = "__TEST_POS_FLOAT_NEG";
    std::env::set_var(key, "-5.0");
    assert_eq!(MakerHedgeCapBot::_env_positive_float_if_set(key), None);
    std::env::remove_var(key);
}

#[test]
fn env_positive_float_unset_returns_none() {
    assert_eq!(
        MakerHedgeCapBot::_env_positive_float_if_set("__TEST_POS_FLOAT_MISSING_999"),
        None
    );
}

// ───────────────────────────────────────────────────────────────────
// 24. MakerExecLedger default
// ───────────────────────────────────────────────────────────────────

#[test]
fn exec_ledger_default_is_empty() {
    let ledger = MakerExecLedger::default();
    assert!(ledger.alias_to_canonical.is_empty());
    assert!(ledger.records.is_empty());
    assert!(ledger.per_order_applied.is_empty());
}

// ───────────────────────────────────────────────────────────────────
// 25. MakerOrderLifecycle variants
// ───────────────────────────────────────────────────────────────────

#[test]
fn maker_order_lifecycle_default_is_idle() {
    assert_eq!(MakerOrderLifecycle::default(), MakerOrderLifecycle::Idle);
}

// ───────────────────────────────────────────────────────────────────
// 26. MakerExecApplyResult variants
// ───────────────────────────────────────────────────────────────────

#[test]
fn exec_apply_result_applied_contains_canonical_id() {
    let r = MakerExecApplyResult::Applied {
        canonical_id: "c1".to_string(),
    };
    if let MakerExecApplyResult::Applied { canonical_id } = r {
        assert_eq!(canonical_id, "c1");
    } else {
        panic!("expected Applied variant");
    }
}

#[test]
fn exec_apply_result_duplicate() {
    let r = MakerExecApplyResult::Duplicate {
        canonical_id: "c2".to_string(),
    };
    assert!(matches!(r, MakerExecApplyResult::Duplicate { .. }));
}

#[test]
fn exec_apply_result_conflict() {
    let r = MakerExecApplyResult::Conflict {
        canonical_id: "c3".to_string(),
        reason: "mismatch".to_string(),
    };
    assert!(matches!(r, MakerExecApplyResult::Conflict { .. }));
}

#[test]
fn exec_apply_result_dropped_weak_id() {
    let r = MakerExecApplyResult::DroppedWeakId {
        reason: "no tx hash".to_string(),
    };
    assert!(matches!(r, MakerExecApplyResult::DroppedWeakId { .. }));
}
