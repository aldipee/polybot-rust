# Native Rust Port Status

Auto-generated from `main.py` symbols vs `src/*.rs` function names.

## Top-level functions

| Function | Status |
|---|---|
| `_segment` | Ported |
| `_iso_to_epoch` | Ported |
| `_infer_year_et` | Ported |
| `_parse_1h_slug_et` | Ported |
| `_format_1h_slug_et` | Ported |
| `_parse_1d_slug_et` | Ported |
| `_format_1d_slug_et` | Ported |
| `_increment_human_slug` | Ported |
| `load_state` | Ported |
| `save_state` | Ported |
| `locked_profit` | Ported |
| `cost_per_pair` | Ported |
| `round_down` | Ported |
| `round_up` | Ported |
| `clamp` | Ported |
| `_D` | Ported |
| `q_down` | Ported |
| `q_up` | Ported |
| `env_bool` | Ported |
| `env_float` | Ported |
| `env_int` | Ported |
| `fetch_market_by_slug` | Ported |
| `get_next_slug` | Ported |
| `_maybe_json_list` | Ported |
| `parse_tokens_and_condition` | Ported |
| `print_pnl_metrics` | Ported |
| `main` | Ported |

## Class methods

### `SignalTrade`

| Method | Status |
|---|---|
| `to_dict` | Ported |

### `JsonlFileService`

| Method | Status |
|---|---|
| `append` | Ported |

### `CsvFileService`

| Method | Status |
|---|---|
| `append_row` | Ported |

### `LatencyLogService`

| Method | Status |
|---|---|
| `append` | Ported |

### `SignalInbox`

| Method | Status |
|---|---|
| `put` | Ported |
| `peek` | Ported |
| `get` | Ported |
| `get_for_slug` | Ported |

### `SignalHub`

| Method | Status |
|---|---|
| `start` | Ported |
| `close` | Ported |
| `is_connected` | Ported |
| `last_message_age_s` | Ported |
| `_log` | Ported |
| `_log_warn` | Ported |
| `_log_err` | Ported |
| `_sslopt` | Ported |
| `_dedup_ok` | Ported |
| `_extract_signal` | Ported |
| `_on_open` | Ported |
| `_on_close` | Ported |
| `_on_error` | Ported |
| `_on_message` | Ported |
| `_mk_ws` | Ported |
| `_run_loop` | Ported |

### `BotConfig`

| Method | Status |
|---|---|
| *(no methods)* | - |

### `MakerHedgeCapBot`

| Method | Status |
|---|---|
| `_init_clob_client` | Ported |
| `_mk_ws` | Ported |
| `_on_open` | Ported |
| `_on_error` | Ported |
| `_on_close` | Ported |
| `_ping_loop` | Ported |
| `_ws_runner` | Ported |
| `_handle_market_event` | Ported |
| `on_market_message` | Ported |
| `_market_data_fresh` | Ported |
| `_best_bid_ask` | Ported |
| `_dbg` | Ported |
| `_dbg_maker` | Ported |
| `_book_url` | Ported |
| `_extract_float_any` | Ported |
| `_fetch_book_summary_http` | Ported |
| `_get_book_cached` | Ported |
| `_iter_book_levels` | Ported |
| `_book_side_levels` | Ported |
| `_cum_depth` | Ported |
| `_apply_tick_dependent_params` | Ported |
| `_sync_market_params_from_book` | Ported |
| `_depth_gate_accumulate` | Ported |
| `_reconcile_state_from_balances` | Ported |
| `_chunked_unwind_heavy_leg` | Ported |
| `_fsm_set_state` | Ported |
| `_apply_cfg_overrides_from_env` | Ported |
| `_accumulate_allowed` | Ported |
| `_paired_quotes_active` | Ported |
| `_quotes_invalidated` | Ported |
| `_oco_after_maker_fill` | Ported |
| `_apply_fill` | Ported |
| `_lat_ms` | Ported |
| `_set_active_signal_context` | Ported |
| `_clear_active_signal_context` | Ported |
| `_get_active_signal_context` | Ported |
| `_utc_iso` | Ported |
| `_should_file_log_submit_event` | Ported |
| `_latency_file_append` | Ported |
| `_prune_order_exec_context_locked` | Ported |
| `_track_order_execution_context` | Ported |
| `_get_order_execution_context` | Ported |
| `_log_execution_latency_on_fill` | Ported |
| `_remember_taker_order` | Ported |
| `_is_recent_taker_order` | Ported |
| `_has_pending_taker_order` | Ported |
| `_pending_taker_notional_usd` | Ported |
| `_has_pending_taker_order_recent` | Ported |
| `_get_balance_allowance_conditional_cached` | Ported |
| `_taker_order_fallback_on_order_event` | Ported |
| `_handle_user_trade_event` | Ported |
| `_handle_user_order_event` | Ported |
| `_handle_user_event` | Ported |
| `on_user_message` | Ported |
| `_cancel` | Ported |
| `_cancel_open_order_local` | Ported |
| `cancel_all_open_orders_local` | Ported |
| `cancel_all_open_orders_local_except` | Ported |
| `cancel_all_orders_exchange` | Ported |
| `_extract_order_id` | Ported |
| `_extract_order_token_id` | Ported |
| `_extract_order_side` | Ported |
| `_extract_order_price` | Ported |
| `_extract_order_remaining_size` | Ported |
| `_list_open_orders_exchange` | Ported |
| `_cancel_exchange_orders_for_assets` | Ported |
| `_reconcile_exchange_orders_for_asset` | Ported |
| `_post_order_compat` | Ported |
| `_post_orders_compat` | Ported |
| `_place_postonly_bid` | Ported |
| `_place_limit_bid_gtc` | Ported |
| `_resolve_order_type` | Ported |
| `_place_taker_bid_fak` | Ported |
| `_place_taker_ask_fak` | Ported |
| `_pair_arb_required_total` | Ported |
| `_taker_pair_submit` | Ported |
| `_wait_for_pair_fills` | Ported |
| `_handle_exposure_mismatch` | Ported |
| `_normalize_exposure_policy` | Ported |
| `_unwind_heavy_leg` | Ported |
| `_maker_exposure_step` | Ported |
| `_taker_pair_arb_step` | Ported |
| `_desired_maker_bid` | Ported |
| `_maker_max_price` | Ported |
| `_maker_bid_cross_ask_safe` | Ported |
| `_maybe_replace` | Ported |
| `_hedge_price_cap` | Ported |
| `_cancel_heavy_side_orders` | Ported |
| `_log_status` | Ported |
| `trade_metrics_snapshot` | Ported |
| `_flatten_now_best` | Ported |
| `_maybe_trigger_max_loss` | Ported |
| `_force_flatten_and_stop` | Ported |
| `_emergency_taker_hedge_step` | Ported |
| `_sniper_best_snapshot` | Ported |
| `_sniper_mark_to_market_pnl` | Ported |
| `_sniper_position` | Ported |
| `_sniper_est_entry_price` | Ported |
| `_sniper_est_exit_price` | Ported |
| `_sniper_maybe_endgame_blind_post` | Ported |
| `_sniper_entry_candidate` | Ported |
| `_sniper_entry_confirmed` | Ported |
| `_sniper_calc_entry_size` | Ported |
| `_log_status_sniper` | Ported |
| `_sniper_try_enter` | Ported |
| `_sniper_try_exit` | Ported |
| `_signal_direction_to_side` | Ported |
| `_signal_seen` | Ported |
| `_signal_mark_seen` | Ported |
| `_ensure_signal_hub` | Ported |
| `_signal_entry_candidate_from_signal` | Ported |
| `_log_status_signal` | Ported |
| `_run_signal_sniper_loop` | Ported |
| `_run_sniper_loop` | Ported |
| `run` | Ported |
| `stop` | Ported |

