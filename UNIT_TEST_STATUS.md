# UNIT_TEST_STATUS — bot.rs

> Auto-generated code map & test coverage tracker for `src/bot.rs` (16,540 lines).
> Use this file to plan and track unit test execution.
> Last updated: 2026-03-07

---

## File Statistics

| Metric | Count |
|--------|-------|
| Total lines | 16,540 |
| Structs | 22 |
| Enums | 5 |
| Impl blocks | 5 |
| Standalone functions | 16 |
| Methods (MakerHedgeCapBot) | ~228 |
| Tests (existing in `mod tests`) | 26 |
| Tests (Priority 1 in `bot_priority1_tests.rs`) | 105 |
| **Total tests** | **131** |

---

## Test Files

| File | Module | Tests | Status |
|------|--------|-------|--------|
| `src/bot.rs` (inline `mod tests`) | `bot::tests` | 26 | ALL PASS |
| `src/bot_priority1_tests.rs` | `bot::bot_priority1_tests` | 105 | ALL PASS |

---

## 1. Structs

| Line | Name | Visibility | Fields |
|------|------|------------|--------|
| 67 | `TradeMetrics` | pub | lp, total_cost, q_yes, q_no, cpp, entry_time_iso, entry_reason, stop_loss_category, exit_reason, fill_count |
| 81 | `SniperOrderFillAgg` | private | qty, notional |
| 87 | `SniperTradeDecisionRuntime` | private | order_id, data |
| 93 | `TakerOrderRecord` | private | order_id, asset_id, size, applied, px_limit, side, ts |
| 104 | `MakerSkewArbState` | private | window_start_ts, cost_total, shares_up, shares_down, downside, upside, skew_ratio, cpp, last_decision_ts, unhedged_since, stretch_* fields |
| 126 | `MakerSkewLoopCtx` | private | now, t_into_s, peak_window, total_cost, budget_usable, yes/no_asset, y/n_bid/ask, q_yes/no_eff, downside, upside, skew_ratio |
| 146 | `MakerSkewRecoveryState` | private | mode, side |
| 152 | `PairBaseRecoveryState` | private | mode, gap, heavy_side, light_side, light_asset_id |
| 161 | `PairBaseFeeNetSnapshot` | private | fees_enabled, fee_source, maker_rebate_bps, estimated_fees, fee_net_pair_cost, fee_net_worst/best_case_pnl, pair_coverage |
| 304 | `PairBaseRuntimeState` | private | phase, active_pair_id, yes/no_oid, target_qty, filled_yes/no, state_enter_ts, risk_exit_latched |
| 317 | `LadderOrderState` | private | key, asset_id, role, level, order_id, price, size, ts |
| 329 | `MakerOrderKey` | private | asset_id, side |
| 353 | `MakerOrderReplaceTarget` | private | price, size, origin |
| 360 | `MakerOrderSlot` | private | state, order_id, price, size, remaining, last_submit/cancel/reject_ts, consecutive_rejects, origin, replace_target |
| 375 | `MakerExecProgress` | private | applied_qty, last_update_ts |
| 381 | `MakerExecCandidate` | private | order_id, asset_id, side, qty, price, tx_hash, trade_id, taker_order_id, match_time |
| 394 | `MakerExecRecord` | private | canonical_id, order_id, qty, price, asset_id, side, aliases, applied_ts |
| 406 | `MakerExecLedger` | private | alias_to_canonical, records, per_order_applied |
| 421 | `ApplyFillMutationMeta` | private | opened_position, closed_position, mark_first_entry_fill |
| 428 | `PairArbPendingImbalance` | private | yes/no_oid, heavy/light_side, gap_shares, created_ts |
| 468 | `SniperStopCertaintyConfig` | private | enabled, sell_budget_ms, sell_max_submits, sell_post_wait_ms, no_derisk_eps_shares, hedge_* fields, post_hedge_* fields, stop_loss_* fields, hedged_block_new_entries |
| 532 | `MakerHedgeCapBot` | pub | ~55 fields (cfg, logger, market_slug, signal_hub, …) |

## 2. Enums

| Line | Name | Variants |
|------|------|----------|
| 173 | `PairBasePhaseState` | Flat, PairResting, MergePending, Balanced, RiskExitOnly |
| 253 | `PairBaseSubMinGapPolicy` | Hold, TakerImmediate |
| 344 | `MakerOrderLifecycle` | Idle, SubmitPending, Working, CancelPending |
| 413 | `MakerExecApplyResult` | Applied, Duplicate, Conflict, DroppedWeakId |
| 438 | `SniperPostHedgePolicy` | HybridTimed, HoldToResolution, ImmediateUnwind |

## 3. Standalone Functions

| Line | Name | Testable | Test Status |
|------|------|----------|-------------|
| 45 | `now_ts()` | no (system clock) | N/A |
| 52 | `now_ts_f64()` | no (system clock) | N/A |
| 59 | `now_ns()` | no (system clock) | N/A |
| 194 | `pair_base_remaining_gap` | **yes** | COVERED (`mod tests`) |
| 198 | `pair_base_phase_without_recovery` | **yes** | COVERED (`mod tests`) |
| 216 | `pair_base_should_force_recovery` | **yes** | COVERED (`mod tests`) |
| 232 | `pair_base_early_risk_exit_lead_seconds` | **yes** | COVERED (`mod tests`) |
| 240 | `pair_base_should_latch_risk_exit` | **yes** | COVERED (`mod tests`) |
| 244 | `pair_base_should_continue_risk_exit` | **yes** | COVERED (`mod tests`, via latch tests) |
| 258 | `pair_base_sub_min_gap_policy` | **yes** | COVERED (`mod tests`) |
| 270 | `pair_base_near_expiry_taker_override_active` | **yes** | COVERED (`mod tests`) |
| 283 | `pair_base_effective_taker_cap` | **yes** | COVERED (`mod tests`) |
| 287 | `pair_base_allows_merge_requote` | **yes** | COVERED (`mod tests`) |
| 291 | `pair_base_blocks_maker_submit` | **yes** | COVERED (`mod tests`) |
| 295 | `pair_base_recovery_uses_exact_order` | **yes** | COVERED (`mod tests`) |
| 299 | `pair_submit_tracks_taker_fallback` | **yes** | COVERED (`mod tests`) |

## 4. impl PairBasePhaseState (line 182)

| Line | Method | Test Status |
|------|--------|-------------|
| 183 | `as_str` | COVERED (P1: 5 tests + default) |

## 5. impl MakerOrderKey (line 334)

| Line | Method | Test Status |
|------|--------|-------------|
| 335 | `buy` | COVERED (P1: 3 tests) |

## 6. impl SniperPostHedgePolicy (line 444)

| Line | Method | Test Status |
|------|--------|-------------|
| 445 | `from_env` | NOT TESTED |
| 458 | `as_str` | COVERED (P1: 3 tests) |

## 7. impl SniperStopCertaintyConfig (line 487)

| Line | Method | Test Status |
|------|--------|-------------|
| 488 | `from_env` | NOT TESTED |

## 8. impl MakerHedgeCapBot — Methods & Test Status

### 8.1 Core / Initialization

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 614 | `new` | pub | NOT TESTED |
| 920 | `_warm_clob_order_meta_cache` | priv | NOT TESTED |
| 958 | `_init_native_clob_client` | priv | NOT TESTED |
| 1288 | `_init_binance_feed_if_needed` | priv | NOT TESTED |
| 4151 | `_apply_cfg_overrides_from_env` | priv | NOT TESTED |

### 8.2 CLOB / Exchange Utilities

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 1047 | `_clob_order_type` | priv static | COVERED (P1: 6 tests) |
| 1056 | `_clob_side` | priv static | COVERED (P1: 5 tests) |
| 1064 | `_tick_size_from_f64` | priv static | COVERED (P1: 5 tests) |
| 1077 | `_value_f64` | priv static | COVERED (P1: 7 tests) |
| 1085 | `_max_numeric_in_value` | priv static | COVERED (P1: 8 tests) |
| 1119 | `_extract_posted_order_id` | priv static | COVERED (P1: 8 tests) |
| 1129 | `_build_l2_headers` | priv | NOT TESTED |
| 1160 | `_normalize_open_orders_payload` | priv static | COVERED (P1: 7 tests) |
| 1236 | `_list_open_orders_exchange_raw` | priv | NOT TESTED |
| 4178 | `_parse_clip_set_from_env` | priv | NOT TESTED |

### 8.3 Runtime State

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 1259 | `_runtime_ts_get` | priv | NOT TESTED |
| 1267 | `_runtime_ts_set` | priv | NOT TESTED |

### 8.4 Sniper Filters

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 1273 | `_is_sniper_like_mode` | priv static | COVERED (P1: 4 tests) |
| 1320 | `_sniper_filters_load_state` | priv | NOT TESTED |
| 1351 | `_sniper_filters_save_state` | priv | NOT TESTED |
| 1393 | `_sniper_filters_ingest_latest_tick` | priv | NOT TESTED |
| 1418 | `_sniper_filter_log` | priv | NOT TESTED |
| 1431 | `_sniper_filters_eval_entry` | priv | NOT TESTED |
| 1513 | `_sniper_filters_allow_entry` | priv | NOT TESTED |
| 1519 | `_sniper_build_breakout_entry_anchor` | priv | NOT TESTED |
| 1553 | `_sniper_set_pending_breakout_entry_anchor` | priv | NOT TESTED |
| 1563 | `_sniper_clear_breakout_entry_anchor_state` | priv | NOT TESTED |
| 1578 | `_sniper_activate_breakout_entry_anchor` | priv | NOT TESTED |
| 1608 | `_sniper_filters_arm_breakout_invalidation_stop_from_anchor` | priv | NOT TESTED |
| 1711 | `_sniper_filters_clear_breakout_invalidation_stop` | priv | NOT TESTED |
| 1718 | `_sniper_arm_breakout_invalidation_stop_for_position` | priv | NOT TESTED |
| 1748 | `_sniper_filters_eval_breakout_invalidation_stop` | priv | NOT TESTED |

### 8.5 Sniper Order / Trade

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 1816 | `_sniper_submit_order_type_from_origin` | priv static | COVERED (P1: 6 tests) |
| 1829 | `_sniper_order_kind_from_origin` | priv static | COVERED (P1: 5 tests) |
| 1840 | `_sniper_apply_fill_stats_to_decision` | priv | NOT TESTED |
| 1860 | `_sniper_trade_decision_record_submit` | priv | NOT TESTED |
| 1993 | `_sniper_record_order_fill` | priv | NOT TESTED |
| 2013 | `_sniper_hedge_oid_key` | priv static | COVERED (P1: 1 test) |
| 2017 | `_sniper_hedge_last_remaining_key` | priv static | COVERED (P1: 1 test) |
| 2021 | `_sniper_is_hedge_order` | priv | NOT TESTED |
| 2028 | `_sniper_mark_hedge_order` | priv | NOT TESTED |
| 2036 | `_sniper_clear_hedge_order` | priv | NOT TESTED |
| 2044 | `_sniper_log_hedge_order_progress` | priv | NOT TESTED |

### 8.6 Sniper Stop Loss

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 2090 | `_sniper_stop_loss_fail_key` | priv static | COVERED (P1: 1 test) |
| 2094 | `_sniper_normalize_stop_loss_mode` | priv static | COVERED (P1: 9 tests) |
| 2111 | `_sniper_stop_loss_mode` | priv | NOT TESTED |
| 2119 | `_sniper_stop_loss_fallback_mode` | priv | NOT TESTED |
| 2129 | `_sniper_stop_loss_fallback_fails` | priv | NOT TESTED |
| 2133 | `_sniper_stop_loss_reset_failures` | priv | NOT TESTED |
| 2141 | `_sniper_stop_loss_record_sell_failure` | priv | NOT TESTED |

### 8.7 RTDS (Chainlink) Gate

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 2186 | `_rtds_gate_log` | priv | NOT TESTED |
| 2200 | `_rtds_gate_load_payload` | priv | NOT TESTED |
| 2235 | `_rtds_gate_diff_price` | priv static | NOT TESTED |
| 2247 | `_rtds_gate_snapshot` | priv | NOT TESTED |
| 2291 | `_rtds_entry_gate_min_diff_price_for_context` | priv | NOT TESTED |
| 2337 | `_rtds_entry_gate_eval_side` | priv | NOT TESTED |
| 2405 | `_rtds_entry_gate_allows_side` | priv | NOT TESTED |
| 2601 | `_rtds_hold_till_resolution_active` | priv | NOT TESTED |

### 8.8 Sniper Entry Reason / TP-SL

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 2410 | `_sniper_force_entry_diff_signal` | priv | NOT TESTED |
| 2459 | `_sniper_endgame_side_from_rtds` | priv | NOT TESTED |
| 2538 | `_sniper_endgame_resolution_tick_ready` | priv | NOT TESTED |
| 2700 | `_sniper_entry_pending_key` | priv static | COVERED (P1: 1 test) |
| 2704 | `_sniper_entry_confirmed_key` | priv static | COVERED (P1: 1 test) |
| 2708 | `_set_exit_reason` | priv | NOT TESTED |
| 2714 | `_get_exit_reason` | priv | NOT TESTED |
| 2721 | `_default_entry_reason` | priv | NOT TESTED |
| 2733 | `_active_entry_reason_or_default` | priv | NOT TESTED |
| 2742 | `_env_positive_float_if_set` | priv static | COVERED (P1: 4 tests) |
| 2751 | `_sniper_tp_sl_overrides_for_entry_reason` | priv static | NOT TESTED (env-dependent) |
| 2770 | `_sniper_tp_sl_for_entry_reason` | priv | NOT TESTED |
| 2781 | `_force_diff_entry_reason` | priv static | COVERED (P1: 3 tests) |
| 2788 | `_should_bypass_rtds_hold_for_take_profit` | priv | NOT TESTED |
| 2802 | `_entry_reason_from_candidate` | priv | NOT TESTED |
| 2827 | `_set_pending_entry_reason` | priv | NOT TESTED |
| 2833 | `_take_pending_entry_reason` | priv | NOT TESTED |
| 2840 | `_mark_sniper_entry_state` | priv | NOT TESTED |
| 2849 | `_mark_sniper_exit_state` | priv | NOT TESTED |
| 2856 | `_clear_local_position_for_asset` | priv | NOT TESTED |

### 8.9 WebSocket / Connection

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 2907 | `_env_first` | priv static | COVERED (P1: 3 tests) |
| 2919 | `_user_ws_auth` | priv | NOT TESTED |
| 2956 | `_set_ws_stream_timeouts` | priv | NOT TESTED |
| 2975 | `run` | pub | NOT TESTED |
| 3249 | `_init_clob_client` | pub | NOT TESTED |
| 3265 | `_mk_ws` | pub | NOT TESTED |
| 3297 | `_on_open` | pub | NOT TESTED |
| 3306 | `_on_error` | pub | NOT TESTED |
| 3315 | `_on_close` | pub | NOT TESTED |
| 3325 | `_ping_loop` | pub | NOT TESTED |
| 3333 | `_ws_runner` | pub | NOT TESTED |

### 8.10 Market Data

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 3472 | `_handle_market_event` | pub | NOT TESTED |
| 3536 | `on_market_message` | pub | NOT TESTED |
| 3550 | `_market_data_fresh` | pub | NOT TESTED |
| 3580 | `_best_bid_ask` | pub | NOT TESTED |
| 3587 | `_dbg` | pub | NOT TESTED |
| 3600 | `_dbg_maker` | pub | NOT TESTED |
| 3604 | `_maker_dbg_idle` | priv | NOT TESTED |
| 3608 | `_book_url` | pub | NOT TESTED |
| 3612 | `_extract_float_any` | pub | NOT TESTED |
| 3627 | `_fetch_book_summary_http` | pub | NOT TESTED |
| 3643 | `_get_book_cached` | pub | NOT TESTED |
| 3667 | `_iter_book_levels` | pub | NOT TESTED |
| 3691 | `_book_side_levels` | pub | NOT TESTED |
| 3708 | `_cum_depth` | pub | NOT TESTED |
| 3740 | `_apply_tick_dependent_params` | priv | NOT TESTED |
| 3748 | `_sync_market_params_from_book` | priv | NOT TESTED |
| 3755 | `_depth_gate_accumulate` | pub | NOT TESTED |
| 3778 | `_reconcile_state_from_positions` | pub | NOT TESTED |

### 8.11 Maker Price / Inventory

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 4022 | `_chunked_unwind_heavy_leg` | pub | NOT TESTED |
| 4142 | `_fsm_set_state` | pub | NOT TESTED |
| 4197 | `_maker_price_bucket` | priv static | COVERED (`mod tests`) |
| 4211 | `_maker_clip_bucket` | priv static | COVERED (`mod tests`) |
| 4223 | `_maker_pick_clip_size_for_price` | priv | NOT TESTED |
| 4259 | `_maker_skew_update_state` | priv | NOT TESTED |
| 4313 | `_maker_poly_fee_estimate` | priv | NOT TESTED |
| 4334 | `_maker_pair_edge_after_fees` | priv | NOT TESTED |
| 4342 | `_maker_single_inflight_enabled` | priv | NOT TESTED |
| 4346 | `_maker_submit_pending_ttl_seconds` | priv | NOT TESTED |
| 4350 | `_maker_cancel_pending_ttl_seconds` | priv | NOT TESTED |
| 4354 | `_maker_working_missing_ttl_seconds` | priv | NOT TESTED |
| 4358 | `_maker_replace_min_interval_seconds` | priv | NOT TESTED |
| 4362 | `_maker_submit_reject_cooldown_seconds` | priv | NOT TESTED |
| 4366 | `_pair_arb_imbalance_enter_shares` | priv | NOT TESTED |
| 4374 | `_pair_arb_imbalance_release_shares` | priv | NOT TESTED |
| 4380 | `_maker_actual_inventory` | priv | NOT TESTED |
| 4387 | `_maker_projected_gap_from_inventory` | priv | COVERED (`mod tests`) |
| 4402 | `_maker_projected_gap_after_buy` | priv | COVERED (`mod tests`) |
| 4439 | `_maker_effective_inventory` | priv | NOT TESTED |

### 8.12 Maker Execution Ledger

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 4457 | `_maker_trade_exec_candidate` | priv | NOT TESTED |
| 4546 | `_maker_trade_exec_aliases` | priv static | COVERED (`mod tests` + P1: 4 tests) |
| 4569 | `_maker_exec_alias_kind` | priv static | COVERED (`mod tests` + P1: 4 tests) |
| 4581 | `_maker_exec_record_matches` | priv static | COVERED (P1: 6 tests) |
| 4590 | `_maker_exec_order_sum` | priv static | COVERED (P1: 5 tests) |
| 4602 | `_maker_exec_attach_aliases` | priv | COVERED (`mod tests`) |
| 4627 | `_maker_exec_applied_qty` | priv | NOT TESTED |
| 4639 | `_maker_commit_exec_fill` | priv | NOT TESTED |

### 8.13 Pair Arbitrage State

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 4784 | `_pair_arb_set_pending_imbalance` | priv | NOT TESTED |
| 4814 | `_pair_arb_clear_pending_if_resolved` | priv | NOT TESTED |
| 4864 | `_pair_arb_pending_active` | priv | NOT TESTED |

### 8.14 Maker Recovery

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 4872 | `_maker_recovery_unsettled_buy_risk` | priv | COVERED (`mod tests`, indirect) |
| 4925 | `_maker_recovery_unsettled_buy_risks` | priv | NOT TESTED |
| 4939 | `_maker_recovery_light_requote_ready` | priv | NOT TESTED |
| 4959 | `_maker_recovery_light_refresh_reason` | priv | NOT TESTED |
| 4997 | `_maker_recovery_mode_snapshot` | priv | NOT TESTED |

### 8.15 Maker Order Management

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 5063 | `_maker_order_slot_get` | priv | NOT TESTED |
| 5071 | `_maker_order_is_live` | priv | NOT TESTED |
| 5103 | `_maker_order_clear_index_for_key` | priv | NOT TESTED |
| 5109 | `_maker_order_open_buy_remaining` | priv | NOT TESTED |
| 5131 | `_maker_order_on_cancel_ack_by_order_id` | priv | NOT TESTED |
| 5157 | `_maker_order_on_submit_ack` | priv | NOT TESTED |
| 5207 | `_maker_order_on_submit_reject` | priv | NOT TESTED |
| 5231 | `_maker_order_request_cancel` | priv | NOT TESTED |
| 5270 | `_maker_order_cancel_all_except_asset` | priv | NOT TESTED |
| 5297 | `_maker_cancel_strategy_orders` | priv | NOT TESTED |
| 5309 | `_maker_order_reconcile_asset` | priv | NOT TESTED |
| 5459 | `_maker_order_on_user_event` | priv | NOT TESTED |
| 5627 | `_maker_order_upsert_gtc` | priv | NOT TESTED |

### 8.16 Maker Payoff / Fees / Ladder

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 5851 | `_maker_payoff_envelope` | priv | COVERED (`mod tests`) |
| 5867 | `_maker_poly_fee_formula` | priv | COVERED (`mod tests`) |
| 5890 | `_maker_ladder_cancel_all` | priv | NOT TESTED |
| 5913 | `_maker_ladder_reserved_notional` | priv | NOT TESTED |
| 5924 | `_maker_ladder_place_or_replace` | priv | NOT TESTED |
| 5984 | `_maker_ladder_sync_role` | priv | NOT TESTED |
| 6111 | `_maker_ladder_cancel_except_role_asset` | priv | NOT TESTED |
| 6138 | `_maker_compute_rsi` | priv static | COVERED (P1: 6 tests) |
| 6165 | `_maker_stretch_bias_side` | priv | NOT TESTED |
| 6268 | `_maker_submit_pair_orders` | priv | NOT TESTED |

### 8.17 Pair Base Configuration

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 6362 | `_pair_base_mode_enabled` | priv | NOT TESTED |
| 6369 | `_pair_recovery_enabled` | priv | NOT TESTED |
| 6373 | `_pair_base_window_budget` | priv | NOT TESTED |
| 6379 | `_pair_base_merge_budget` | priv | NOT TESTED |
| 6385 | `_pair_base_hard_reserve` | priv | NOT TESTED |
| 6391 | `_pair_base_fee_net_snapshot` | priv | NOT TESTED |
| 6440 | `_pair_base_log_fee_net` | priv | NOT TESTED |
| 6470 | `_pair_base_live_order_id` | priv | NOT TESTED |
| 6489 | `_pair_base_cancel_orders` | priv | NOT TESTED |
| 6497 | `_pair_base_set_phase` | priv | NOT TESTED |

### 8.18 Pair Base Recovery / Risk Exit

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 6539 | `_maker_pair_base_recovery_phase` | priv | NOT TESTED |
| 6684 | `_maker_pair_base_risk_exit_step` | priv | NOT TESTED |
| 6789 | `_maker_pair_base_recovery_step` | priv | NOT TESTED |
| 6952 | `_maker_pair_base_step` | priv | NOT TESTED |

### 8.19 Maker Trade Decision

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 7321 | `_maker_record_trade_decision` | priv | NOT TESTED |

### 8.20 Maker Trading Loops

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 7366 | `_maker_skew_try_arb` | priv | NOT TESTED |
| 7638 | `_maker_quote_only_step` | priv | NOT TESTED |
| 7872 | `_maker_skew_arb_step` | priv | NOT TESTED |
| 8064 | `_maker_skew_handle_base_seed_phase` | priv | NOT TESTED |
| 8190 | `_maker_skew_handle_shared_gate_phase` | priv | NOT TESTED |
| 8241 | `_maker_skew_handle_recovery_phase` | priv | NOT TESTED |
| 8343 | `_maker_skew_handle_directional_phase` | priv | NOT TESTED |

### 8.21 Accumulate / Entry Decision

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 8556 | `_accumulate_allowed` | pub | NOT TESTED |
| 8596 | `_maker_quote_only_allowed` | priv | NOT TESTED |
| 8629 | `_paired_quotes_active` | pub | NOT TESTED |
| 8640 | `_quotes_invalidated` | pub | NOT TESTED |
| 8703 | `_oco_after_maker_fill` | pub | NOT TESTED |

### 8.22 Fill / Apply

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 8711 | `_apply_fill` | pub | NOT TESTED |
| 8794 | `_apply_fill_locked_nodedupe` | priv | NOT TESTED |
| 8831 | `_apply_fill_finalize` | priv | NOT TESTED |

### 8.23 Latency / Context

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 8876 | `_lat_ms` | pub | NOT TESTED |
| 8883 | `_lat_us` | pub | NOT TESTED |
| 8890 | `_set_active_signal_context` | pub | NOT TESTED |
| 8900 | `_clear_active_signal_context` | pub | NOT TESTED |
| 8906 | `_get_active_signal_context` | pub | NOT TESTED |
| 8913 | `_utc_iso` | pub | NOT TESTED |
| 8922 | `_should_file_log_submit_event` | pub | NOT TESTED |
| 8932 | `_latency_file_append` | pub | NOT TESTED |
| 8938 | `_prune_order_exec_context_locked` | pub | NOT TESTED |
| 8961 | `_track_order_execution_context` | pub | NOT TESTED |
| 9370 | `_get_order_execution_context` | pub | NOT TESTED |
| 9377 | `_log_execution_latency_on_fill` | pub | NOT TESTED |

### 8.24 Taker Order Tracking

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 9418 | `_remember_taker_order` | pub | NOT TESTED |
| 9443 | `_forget_taker_order` | pub | NOT TESTED |
| 9452 | `_is_recent_taker_order` | pub | NOT TESTED |
| 9462 | `_has_pending_taker_order` | pub | NOT TESTED |
| 9480 | `_pending_taker_notional_usd` | pub | NOT TESTED |
| 9501 | `_has_pending_taker_order_recent` | pub | NOT TESTED |
| 9524 | `_get_position_size_data_api` | pub | NOT TESTED |
| 9650 | `_get_balance_allowance_conditional_cached` | pub | NOT TESTED |

### 8.25 Order Event Handling

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 9799 | `_taker_order_fallback_on_order_event` | pub | NOT TESTED |
| 9911 | `_handle_user_trade_event` | pub | NOT TESTED |
| 10363 | `_handle_user_order_event` | pub | NOT TESTED |
| 10512 | `_handle_user_event` | pub | NOT TESTED |
| 10527 | `on_user_message` | pub | NOT TESTED |

### 8.26 Order Placement / Cancellation

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 10541 | `_cancel` | pub | NOT TESTED |
| 10574 | `_cancel_open_order_local` | pub | NOT TESTED |
| 10602 | `cancel_all_open_orders_local` | pub | NOT TESTED |
| 10626 | `cancel_all_open_orders_local_except` | pub | NOT TESTED |
| 10677 | `_extract_order_id` | pub | NOT TESTED |
| 10686 | `_extract_order_token_id` | pub | NOT TESTED |
| 10695 | `_extract_order_side` | pub | NOT TESTED |
| 10702 | `_extract_order_price` | pub | NOT TESTED |
| 10708 | `_extract_order_remaining_size` | pub | NOT TESTED |
| 10718 | `_list_open_orders_exchange` | pub | NOT TESTED |
| 10789 | `_cancel_exchange_orders_for_assets` | pub | NOT TESTED |
| 10831 | `_reconcile_exchange_orders_for_asset` | pub | NOT TESTED |
| 10982 | `_post_order_compat` | pub | NOT TESTED |
| 11153 | `_post_orders_compat` | pub | NOT TESTED |
| 11168 | `_place_postonly_bid` | pub | NOT TESTED |
| 11242 | `_place_limit_bid_gtc` | pub | NOT TESTED |
| 11257 | `_place_limit_bid_gtc_with_origin` | priv | NOT TESTED |
| 11351 | `_place_limit_bid_gtc_exact_with_origin` | priv | NOT TESTED |
| 11438 | `_resolve_order_type` | pub | NOT TESTED |
| 11459 | `_place_taker_bid_fak` | pub | NOT TESTED |
| 11527 | `_place_taker_bid_fak_exact` | pub | NOT TESTED |
| 11593 | `_place_taker_ask_fak` | pub | NOT TESTED |
| 11689 | `_place_taker_ask_fak_exact` | pub | NOT TESTED |

### 8.27 Pair Arbitrage Trading

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 11764 | `_pair_arb_required_total` | pub | NOT TESTED |
| 11771 | `_taker_pair_submit` | pub | NOT TESTED |
| 11863 | `_wait_for_pair_fills` | pub | NOT TESTED |
| 11888 | `_wait_for_pair_order_fills` | pub | NOT TESTED |
| 11923 | `_handle_exposure_mismatch` | pub | NOT TESTED |
| 12012 | `_normalize_exposure_policy` | pub | NOT TESTED |
| 12020 | `_unwind_heavy_leg` | pub | NOT TESTED |
| 12064 | `_maker_exposure_step` | pub | NOT TESTED |
| 12198 | `_taker_pair_arb_step` | pub | NOT TESTED |

### 8.28 Hedge / Price

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 12468 | `_desired_maker_bid` | pub | NOT TESTED |
| 12473 | `_maker_max_price` | pub | NOT TESTED |
| 12478 | `_maker_bid_cross_ask_safe` | pub | NOT TESTED |
| 12490 | `_maybe_replace` | pub | NOT TESTED |
| 12584 | `_hedge_price_cap` | pub | NOT TESTED |
| 12602 | `_cancel_heavy_side_orders` | pub | NOT TESTED |
| 12639 | `_log_status` | pub | NOT TESTED |
| 12708 | `_flatten_now_best` | pub | NOT TESTED |
| 12761 | `_maybe_trigger_max_loss` | pub | NOT TESTED |
| 12817 | `_force_flatten_and_stop` | pub | NOT TESTED |
| 12916 | `_emergency_taker_hedge_step` | pub | NOT TESTED |
| 13077 | `_pair_base_exact_taker_hedge_step` | priv | NOT TESTED |

### 8.29 Sniper Position

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 13187 | `_sniper_best_snapshot` | pub | NOT TESTED |
| 13201 | `_sniper_mark_to_market_pnl` | pub | NOT TESTED |
| 13206 | `_sniper_position` | pub | NOT TESTED |
| 13252 | `_sniper_has_resting_entry_order` | priv | NOT TESTED |
| 13272 | `_sniper_est_entry_price` | pub | NOT TESTED |
| 13280 | `_sniper_est_exit_price` | pub | NOT TESTED |
| 13289 | `_sniper_maybe_endgame_blind_post` | pub | NOT TESTED |
| 13568 | `_sniper_entry_candidate_for_side` | priv | NOT TESTED |
| 13681 | `_sniper_entry_candidate` | pub | NOT TESTED |
| 13689 | `_sniper_entry_confirmed` | pub | NOT TESTED |
| 13723 | `_sniper_calc_entry_size` | pub | NOT TESTED |
| 13734 | `_log_status_sniper` | pub | NOT TESTED |
| 13813 | `_sniper_try_enter` | pub | NOT TESTED |
| 14215 | `_sniper_is_flat` | priv | NOT TESTED |
| 14223 | `_sniper_is_paired_hedged` | priv | NOT TESTED |
| 14235 | `_sniper_post_hedge_active` | priv | NOT TESTED |
| 14239 | `_sniper_clear_post_hedge_state` | priv | NOT TESTED |
| 14246 | `_sniper_mark_post_hedge_state` | priv | NOT TESTED |
| 14261 | `_sniper_should_block_new_entries` | priv | NOT TESTED |
| 14265 | `_sniper_bounded_pause_seconds` | priv | NOT TESTED |
| 14293 | `_sniper_set_fail_pause` | priv | NOT TESTED |
| 14301 | `_sniper_stop_certainty_hedge_phase` | priv | NOT TESTED |
| 14341 | `_sniper_handle_post_hedge_policy` | priv | NOT TESTED |
| 14395 | `_sniper_maybe_exit_hedge` | priv | NOT TESTED |
| 14399 | `_sniper_maybe_exit_hedge_with_opts` | priv | NOT TESTED |
| 14604 | `_sniper_try_exit` | pub | NOT TESTED |

### 8.30 Signal

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 15036 | `_signal_direction_to_side` | pub | NOT TESTED |
| 15044 | `_signal_seen` | pub | NOT TESTED |
| 15055 | `_signal_mark_seen` | pub | NOT TESTED |
| 15074 | `_ensure_signal_hub` | pub | NOT TESTED |
| 15093 | `_signal_entry_candidate_from_signal` | pub | NOT TESTED |
| 15190 | `_log_status_signal` | pub | NOT TESTED |
| 15219 | `_run_signal_sniper_loop` | pub | NOT TESTED |
| 15610 | `_run_sniper_loop` | pub | NOT TESTED |

### 8.31 Lifecycle

| Line | Method | Vis | Test Status |
|------|--------|-----|-------------|
| 16078 | `stop` | pub | NOT TESTED |
| 16083 | `trade_metrics_snapshot` | pub | NOT TESTED |
| 16121 | `trade_decision_snapshot` | pub | NOT TESTED |
| 16129 | `persist_state` | pub | NOT TESTED |
| 16135 | `cancel_all_orders_exchange` | pub | NOT TESTED |

---

## 9. Existing Tests

### 9a. `mod tests` (26 tests, inline in bot.rs)

| Line | Test Name | Category | Status |
|------|-----------|----------|--------|
| 16164 | `maker_payoff_envelope_math` | Maker Payoff | PASS |
| 16172 | `maker_fee_formula_peaks_near_mid` | Maker Fees | PASS |
| 16182 | `maker_fee_formula_maker_path_is_zero_cost` | Maker Fees | PASS |
| 16189 | `maker_bucket_helpers` | Maker Utils | PASS |
| 16202 | `maker_exec_aliases_prefer_tx_then_trade_then_match` | Exec Ledger | PASS |
| 16226 | `maker_exec_alias_enrichment_resolves_to_existing_trade_canonical` | Exec Ledger | PASS |
| 16275 | `maker_exec_alias_enrichment_resolves_to_existing_match_canonical` | Exec Ledger | PASS |
| 16323 | `maker_projected_gap_allows_light_side_buy_that_stays_below_enter` | Inventory | PASS |
| 16336 | `maker_projected_gap_blocks_same_side_buy_that_would_reopen_recovery` | Inventory | PASS |
| 16349 | `maker_projected_gap_includes_unsettled_buy_risk` | Inventory | PASS |
| 16362 | `pair_base_remaining_gap_respects_live_light_risk` | Pair Base | PASS |
| 16370 | `pair_base_phase_stays_resting_while_pair_orders_are_live` | Pair Base | PASS |
| 16382 | `pair_base_phase_only_balances_or_flats_when_no_pair_orders_are_live` | Pair Base | PASS |
| 16394 | `pair_submit_does_not_track_gtc_orders_as_taker_fallback` | Pair Submit | PASS |
| 16401 | `pair_submit_tracks_non_gtc_orders_as_taker_fallback` | Pair Submit | PASS |
| 16407 | `pair_base_forces_recovery_when_merge_pending_gap_remains` | Pair Recovery | PASS |
| 16417 | `pair_base_forces_recovery_when_pair_resting_light_leg_is_untrusted` | Pair Recovery | PASS |
| 16433 | `pair_base_early_risk_exit_lead_is_ahead_of_stop_buffer` | Risk Exit | PASS |
| 16440 | `pair_base_early_risk_exit_lead_honors_env_override` | Risk Exit | PASS |
| 16446 | `pair_base_near_expiry_taker_override_is_reason_and_time_gated` | Near Expiry | PASS |
| 16474 | `pair_base_near_expiry_taker_override_raises_cap` | Near Expiry | PASS |
| 16480 | `pair_base_near_expiry_risk_exit_latches_terminal_mode` | Near Expiry | PASS |
| 16487 | `pair_base_merge_requote_requires_positive_worst_case_pnl` | Merge | PASS |
| 16494 | `pair_base_risk_exit_blocks_maker_submits` | Risk Exit | PASS |
| 16504 | `pair_base_risk_exit_keeps_terminal_ownership_until_flat` | Risk Exit | PASS |
| 16523 | `pair_base_sub_min_recovery_uses_exact_orders` | Recovery | PASS |

### 9b. `bot_priority1_tests` (105 tests, in `src/bot_priority1_tests.rs`)

| # | Test Name | Category | Status |
|---|-----------|----------|--------|
| 1-6 | `phase_state_as_str_*`, `phase_state_default_is_flat` | Enum: PairBasePhaseState | PASS |
| 7-9 | `post_hedge_policy_as_str_*` | Enum: SniperPostHedgePolicy | PASS |
| 10-12 | `maker_order_key_buy_*` | Struct: MakerOrderKey | PASS |
| 13-18 | `clob_order_type_*` | CLOB: _clob_order_type | PASS |
| 19-23 | `clob_side_*` | CLOB: _clob_side | PASS |
| 24-28 | `tick_size_*` | CLOB: _tick_size_from_f64 | PASS |
| 29-35 | `value_f64_*` | JSON: _value_f64 | PASS |
| 36-43 | `max_numeric_*` | JSON: _max_numeric_in_value | PASS |
| 44-51 | `extract_order_id_*` | JSON: _extract_posted_order_id | PASS |
| 52-58 | `normalize_orders_*` | JSON: _normalize_open_orders_payload | PASS |
| 59-62 | `is_sniper_like_mode_*` | Sniper: _is_sniper_like_mode | PASS |
| 63-68 | `sniper_order_type_*` | Sniper: _sniper_submit_order_type_from_origin | PASS |
| 69-73 | `sniper_order_kind_*` | Sniper: _sniper_order_kind_from_origin | PASS |
| 74-78 | `sniper_hedge_oid_key_*`, `sniper_hedge_last_remaining_key_*`, `sniper_stop_loss_fail_key_*`, `sniper_entry_*_key_*` | Key builders | PASS |
| 79-87 | `normalize_stop_loss_mode_*` | Sniper: _sniper_normalize_stop_loss_mode | PASS |
| 88-90 | `force_diff_entry_reason_*` | Entry: _force_diff_entry_reason | PASS |
| 91-96 | `rsi_*` | Maker: _maker_compute_rsi | PASS |
| 97-102 | `exec_record_matches_*` | Exec: _maker_exec_record_matches | PASS |
| 103-107 | `exec_order_sum_*` | Exec: _maker_exec_order_sum | PASS |
| 108-111 | `exec_aliases_*` | Exec: _maker_trade_exec_aliases | PASS |
| 112-115 | `exec_alias_kind_*` | Exec: _maker_exec_alias_kind | PASS |
| 116-118 | `env_first_*` | Env: _env_first | PASS |
| 119-122 | `env_positive_float_*` | Env: _env_positive_float_if_set | PASS |
| 123 | `exec_ledger_default_is_empty` | Struct: MakerExecLedger | PASS |
| 124 | `maker_order_lifecycle_default_is_idle` | Enum: MakerOrderLifecycle | PASS |
| 125-128 | `exec_apply_result_*` | Enum: MakerExecApplyResult | PASS |

---

## 10. Coverage Summary

| Category | Total Fns | Tested | NOT Tested | Coverage |
|----------|-----------|--------|------------|----------|
| Standalone functions | 16 | 13 | 3 (clock) | 81% |
| PairBasePhaseState | 1 | 1 | 0 | **100%** |
| MakerOrderKey | 1 | 1 | 0 | **100%** |
| SniperPostHedgePolicy | 2 | 1 | 1 | 50% |
| SniperStopCertaintyConfig | 1 | 0 | 1 | 0% |
| 8.1 Core/Init | 5 | 0 | 5 | 0% |
| 8.2 CLOB Utils | 10 | 7 | 3 | **70%** |
| 8.3 Runtime State | 2 | 0 | 2 | 0% |
| 8.4 Sniper Filters | 15 | 1 | 14 | 7% |
| 8.5 Sniper Order/Trade | 11 | 4 | 7 | 36% |
| 8.6 Sniper Stop Loss | 7 | 2 | 5 | 29% |
| 8.7 RTDS Gate | 8 | 0 | 8 | 0% |
| 8.8 Entry Reason / TP-SL | 19 | 4 | 15 | 21% |
| 8.9 WebSocket/Connection | 11 | 1 | 10 | 9% |
| 8.10 Market Data | 18 | 0 | 18 | 0% |
| 8.11 Maker Price/Inventory | 20 | 4 | 16 | 20% |
| 8.12 Exec Ledger | 8 | 6 | 2 | **75%** |
| 8.13 Pair Arb State | 3 | 0 | 3 | 0% |
| 8.14 Maker Recovery | 5 | 1 | 4 | 20% |
| 8.15 Maker Order Mgmt | 13 | 0 | 13 | 0% |
| 8.16 Payoff/Fees/Ladder | 10 | 3 | 7 | 30% |
| 8.17 Pair Base Config | 10 | 0 | 10 | 0% |
| 8.18 Pair Base Recovery | 4 | 0 | 4 | 0% |
| 8.19 Trade Decision | 1 | 0 | 1 | 0% |
| 8.20 Maker Loops | 7 | 0 | 7 | 0% |
| 8.21 Accumulate/Entry | 5 | 0 | 5 | 0% |
| 8.22 Fill/Apply | 3 | 0 | 3 | 0% |
| 8.23 Latency/Context | 12 | 0 | 12 | 0% |
| 8.24 Taker Order Track | 8 | 0 | 8 | 0% |
| 8.25 Order Events | 5 | 0 | 5 | 0% |
| 8.26 Order Placement | 23 | 0 | 23 | 0% |
| 8.27 Pair Arb Trading | 9 | 0 | 9 | 0% |
| 8.28 Hedge/Price | 12 | 0 | 12 | 0% |
| 8.29 Sniper Position | 26 | 0 | 26 | 0% |
| 8.30 Signal | 8 | 0 | 8 | 0% |
| 8.31 Lifecycle | 5 | 0 | 5 | 0% |
| **TOTAL** | **~352** | **~48** | **~304** | **~13.6%** |

---

## 11. Recommended Test Priority (remaining)

### Priority 1 — Static/Pure helpers (no `&self`) — DONE
All Priority 1 functions are now covered in `src/bot_priority1_tests.rs`.

### Priority 2 — Lightweight `&self` methods (read config/state only)
- `_maker_single_inflight_enabled`, `_maker_submit_pending_ttl_seconds`
- `_maker_cancel_pending_ttl_seconds`, `_maker_working_missing_ttl_seconds`
- `_maker_replace_min_interval_seconds`, `_maker_submit_reject_cooldown_seconds`
- `_pair_arb_imbalance_enter_shares`, `_pair_arb_imbalance_release_shares`
- `_pair_base_mode_enabled`, `_pair_recovery_enabled`
- `_pair_base_window_budget`, `_pair_base_merge_budget`, `_pair_base_hard_reserve`
- `_sniper_stop_loss_mode`, `_sniper_stop_loss_fallback_mode`
- `_default_entry_reason`, `_active_entry_reason_or_default`
- `_sniper_is_flat`, `_sniper_is_paired_hedged`
- `_lat_ms`, `_lat_us`, `_utc_iso`

### Priority 3 — Complex logic requiring mock/state setup
- `_maker_skew_update_state`, `_maker_projected_gap_from_inventory`
- `_sniper_filters_eval_entry`, `_sniper_filters_eval_breakout_invalidation_stop`
- `_rtds_gate_snapshot`, `_rtds_entry_gate_eval_side`
- `_maker_pair_edge_after_fees`, `_maker_poly_fee_estimate`
- `_maker_recovery_mode_snapshot`
- `_pair_base_fee_net_snapshot`

---

## 12. How to Run Tests

```bash
# Run ALL bot.rs tests (131 tests)
cargo test --bin polybot bot::

# Run only Priority 1 tests (105 tests)
cargo test --bin polybot bot_priority1_tests

# Run only original inline tests (26 tests)
cargo test --bin polybot bot::tests::

# Run a specific test
cargo test --bin polybot bot_priority1_tests::rsi_all_gains_is_100

# Run with output
cargo test --bin polybot bot:: -- --nocapture
```
