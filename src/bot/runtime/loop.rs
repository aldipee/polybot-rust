use super::*;
impl MakerHedgeCapBot {
    /// Tracks the runtime imbalance state for the active pair.
    pub(in crate::bot) fn _bot_runtime_note_imbalance_state(
        &self,
        now: f64,
        q_yes: f64,
        q_no: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) -> BotRuntimeImbalanceState {
        let current_fraction = unmatched_fraction(q_yes, q_no);
        let computed_state = bot_runtime_imbalance_state_from_fraction(current_fraction, cfg);
        let match_ratio = match_ratio(q_yes, q_no);
        let pair_id = self.pair_identity().pair_id;
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            let previous_state = st.imbalance_state;
            let pair_completed =
                st.await_second_fill_second_fill_ts > 0.0 || (q_yes > 1e-9 && q_no > 1e-9);
            let next_state = if !pair_completed {
                previous_state
            } else if matches!(previous_state, BotRuntimeImbalanceState::HardDisable) {
                BotRuntimeImbalanceState::HardDisable
            } else {
                computed_state
            };
            if previous_state != next_state {
                st.imbalance_state = next_state;
                st.imbalance_state_enter_ts = now;
                self.logger.info(&format!(
                    "[BOT] pair_id={} imbalance_state {} -> {} unmatched_fraction={:.3} match_ratio={:.3} qYES={:.2} qNO={:.2}",
                    pair_id,
                    previous_state.as_str(),
                    next_state.as_str(),
                    current_fraction,
                    match_ratio,
                    q_yes.max(0.0),
                    q_no.max(0.0)
                ));
            }
            if st.imbalance_state_enter_ts <= 0.0 {
                st.imbalance_state_enter_ts = now;
            }
            return st.imbalance_state;
        }
        computed_state
    }

    /// Implements log final metrics for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_log_final_metrics(&self, exit_reason: &str) {
        let pair_id = self.pair_identity().pair_id;
        self._bot_runtime_refresh_daily_liquidity_counters();
        let (q_yes, q_no, cost_yes, cost_no, total_cost) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no, s.c_yes, s.c_no, s.c_yes + s.c_no))
            .unwrap_or((0.0, 0.0, 0.0, 0.0, 0.0));
        let state = self
            .bot_runtime_state
            .lock()
            .map(|st| st.clone())
            .unwrap_or_default();
        let metrics =
            bot_runtime_metrics_snapshot(&state, q_yes, q_no, cost_yes, cost_no, total_cost);
        let combined_avg_paid = if metrics.inventory_vwap_sum.is_finite() {
            format!("{:.3}", metrics.inventory_vwap_sum)
        } else {
            "NA".to_string()
        };
        let paired_cost_band_occupancy =
            bot_runtime_paired_cost_band_summary_u32(&metrics.paired_cost_band_observations);
        let paired_cost_band_occupancy_rate =
            bot_runtime_paired_cost_band_summary_fraction(&metrics.paired_cost_band_observations);
        let paired_size_delta_by_state =
            bot_runtime_paired_cost_band_summary_f64(&metrics.paired_size_delta_by_state);
        let canary_success = bot_runtime_canary_success(&metrics);
        let canary_failure_summary = bot_runtime_canary_failure_summary(&metrics);
        let seed_anchor_t_into = if state.open_both_seed_anchor_ts > 0.0 {
            format!(
                "{:.1}s",
                (state.open_both_seed_anchor_ts - self.start_ts as f64).max(0.0)
            )
        } else {
            "NA".to_string()
        };
        let first_yes_seed_submit_t_into = if state.open_both_first_yes_submit_ts > 0.0 {
            format!(
                "{:.1}s",
                (state.open_both_first_yes_submit_ts - self.start_ts as f64).max(0.0)
            )
        } else {
            "NA".to_string()
        };
        let first_no_seed_submit_t_into = if state.open_both_first_no_submit_ts > 0.0 {
            format!(
                "{:.1}s",
                (state.open_both_first_no_submit_ts - self.start_ts as f64).max(0.0)
            )
        } else {
            "NA".to_string()
        };
        let seed_submit_delta_ms = if state.open_both_first_yes_submit_ts > 0.0
            && state.open_both_first_no_submit_ts > 0.0
        {
            format!("{:.0}", metrics.open_both_first_submit_delta_ms)
        } else {
            "NA".to_string()
        };
        self.logger.info(&format!(
            "[BOT][METRICS] pair_id={} exit_reason={} market_participated={} market_participation={:.3} fills_per_market={} total_fill_shares={:.2} maker_fill_share={:.3} taker_fill_events={} taker_fill_shares={:.2} pair_taker_share={:.3} daily_maker_fill_shares={:.2} daily_taker_fill_shares={:.2} daily_taker_share={:.3} fill_events_by_segment={} fill_shares_by_segment={} paired_size={:.2} unmatched_size={:.2} unmatched_fraction={:.3} match_ratio={:.3} imbalance_state={} pair_coverage={:.3} share_skew={:.3} combined_avg_paid={} paired_cost_band_occupancy={} paired_cost_band_occupancy_rate={} paired_size_delta_by_state={} below_snapshot_optional_submit_count={} below_snapshot_optional_submit_shares={:.2} below_snapshot_optional_fill_count={} below_snapshot_optional_fill_shares={:.2} below_snapshot_optional_fill_rate={:.3} tail_at_expiry={:.2} worst_case_settlement_floor={:+.2} bad_regime_expensive_ratio={:.3} bad_regime_shutdown={} canary_success={} canary_failure_summary={} fills_after_taper_start={} fills_after_final_quiet={} new_orders_after_taper_start={} new_orders_after_final_quiet={} prearm_ready_before_open={} seed_anchor_t_into={} first_yes_seed_submit_t_into={} first_no_seed_submit_t_into={} seed_by_5s_met={} late_seed_used={} seed_submit_delta_ms={} seed_submit_delta_met={} second_side_by_15s={} second_side_by_30s={} first_fill_to_second_fill_ms={} await_second_fill_rescue_used={} await_second_fill_hard_paused={} skipped_optional_adds={} repair_reserve_blocks={} floor_tail_blocks={} startup_completion_blocked={}",
            pair_id,
            exit_reason,
            metrics.market_participated,
            if metrics.market_participated { 1.0 } else { 0.0 },
            metrics.fills_per_market,
            metrics.total_fill_shares,
            metrics.maker_fill_share,
            metrics.taker_fill_events,
            metrics.taker_fill_shares,
            metrics.pair_taker_share,
            metrics.daily_maker_fill_shares,
            metrics.daily_taker_fill_shares,
            metrics.daily_taker_share,
            bot_runtime_fill_distribution_summary_u32(&metrics.fill_events_by_segment),
            bot_runtime_fill_distribution_summary_f64(&metrics.fill_shares_by_segment),
            metrics.paired_size,
            metrics.unmatched_size,
            metrics.unmatched_fraction,
            metrics.match_ratio,
            metrics.imbalance_state.as_str(),
            metrics.pair_coverage,
            metrics.share_skew_ratio,
            combined_avg_paid,
            paired_cost_band_occupancy,
            paired_cost_band_occupancy_rate,
            paired_size_delta_by_state,
            metrics.below_snapshot_optional_submit_count,
            metrics.below_snapshot_optional_submit_shares,
            metrics.below_snapshot_optional_fill_count,
            metrics.below_snapshot_optional_fill_shares,
            metrics.below_snapshot_optional_fill_rate,
            metrics.tail_at_expiry,
            metrics.worst_case_settlement_floor,
            metrics.bad_regime_expensive_ratio,
            metrics.bad_regime_shutdown,
            canary_success,
            canary_failure_summary,
            metrics.taper_fill_events_after_240,
            metrics.taper_fill_events_after_270,
            metrics.taper_new_orders_after_240,
            metrics.taper_new_orders_after_270,
            metrics.prearm_ready_before_open,
            seed_anchor_t_into,
            first_yes_seed_submit_t_into,
            first_no_seed_submit_t_into,
            metrics.open_both_seed_by_deadline_met,
            metrics.open_both_late_seed_used,
            seed_submit_delta_ms,
            metrics.open_both_submit_delta_met,
            metrics.second_side_by_15s,
            metrics.second_side_by_30s,
            format!("{:.0}", metrics.first_fill_to_second_fill_ms),
            metrics.await_second_fill_rescue_used,
            metrics.await_second_fill_hard_paused,
            metrics.skipped_optional_add_count,
            metrics.repair_reserve_blocked_count,
            metrics.floor_tail_blocked_count,
            metrics.startup_completion_blocked_count
        ));
    }

    /// Returns pair asset ids tracked by the BOT runtime for the active market.
    pub(in crate::bot) fn _bot_runtime_pair_asset_ids(&self) -> Vec<String> {
        [self.yes_asset.clone(), self.no_asset.clone()]
            .into_iter()
            .flatten()
            .filter(|asset_id| !asset_id.trim().is_empty())
            .collect()
    }

    /// Counts locally tracked BOT strategy orders still considered live by the runtime.
    pub(in crate::bot) fn _bot_runtime_active_strategy_order_count(&self) -> usize {
        let slot_count = self
            .maker_order_slots
            .lock()
            .map(|slots| {
                slots
                    .values()
                    .filter(|slot| {
                        slot.order_id.is_some()
                            && slot.origin.starts_with("BOT_")
                            && matches!(
                                slot.state,
                                MakerOrderLifecycle::Working
                                    | MakerOrderLifecycle::SubmitPending
                                    | MakerOrderLifecycle::CancelPending
                            )
                    })
                    .count()
            })
            .unwrap_or(0);
        let pair_assets: HashSet<String> = self._bot_runtime_pair_asset_ids().into_iter().collect();
        let open_order_count = self
            .state
            .lock()
            .map(|state| {
                state
                    .open_orders
                    .iter()
                    .filter(|(asset_id, row)| {
                        pair_assets.contains(asset_id.as_str())
                            && row
                                .order_id
                                .as_deref()
                                .map(|order_id| !order_id.trim().is_empty())
                                .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0);
        slot_count + open_order_count
    }

    /// Drives terminal order drain before handing control to settlement finalization.
    pub(in crate::bot) fn _bot_runtime_await_settlement_handler(
        &self,
        now: f64,
        seconds_left: f64,
    ) -> bool {
        let timeout_s = env_float(
            "BOT_AWAIT_SETTLEMENT_CANCEL_TIMEOUT_SECONDS",
            self.cfg.stale_seconds.max(1) as f64,
        )
        .clamp(0.5, 30.0);
        let pair_id = self.pair_identity().pair_id;
        let pair_assets = self._bot_runtime_pair_asset_ids();
        let mut started_ts = now;
        let mut cancel_requested = false;
        let mut first_entry = false;
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            if st.await_settlement_started_ts <= 0.0 {
                st.await_settlement_started_ts = now;
                st.await_settlement_orders_cleared_ts = 0.0;
                st.await_settlement_cancel_requested = false;
                first_entry = true;
            }
            started_ts = st.await_settlement_started_ts.max(0.0);
            cancel_requested = st.await_settlement_cancel_requested;
        }

        if !cancel_requested {
            let reason = "bot_runtime_await_settlement";
            let _ = self._bot_runtime_cancel_order_family("BOT_OPEN_BOTH", None, reason);
            let _ = self._bot_runtime_cancel_await_second_fill_orders(None, reason);
            let _ = self._bot_runtime_cancel_pair_build_orders(None, reason);
            let _ = self._bot_runtime_cancel_taper_orders(None, reason);
            self.cancel_all_open_orders_local(reason);
            if !pair_assets.is_empty() {
                self._cancel_exchange_orders_for_assets(&pair_assets, reason);
            }
            if let Ok(mut st) = self.bot_runtime_state.lock() {
                st.await_settlement_cancel_requested = true;
            }
        }

        let active_orders = self._bot_runtime_active_strategy_order_count();
        if active_orders == 0 {
            let mut first_clear = false;
            if let Ok(mut st) = self.bot_runtime_state.lock() {
                if st.await_settlement_orders_cleared_ts <= 0.0 {
                    st.await_settlement_orders_cleared_ts = now;
                    first_clear = true;
                }
            }
            if first_clear || first_entry {
                self.logger.info(&format!(
                    "[BOT] pair_id={} AwaitSettlement orders_cleared=true t_left={:.1}s active_orders=0",
                    pair_id,
                    seconds_left.max(0.0)
                ));
            }
            self._set_exit_reason("AWAIT_SETTLEMENT");
            return true;
        }

        let elapsed_s = (now - started_ts).max(0.0);
        if first_entry {
            self.logger.info(&format!(
                "[BOT] pair_id={} AwaitSettlement start t_left={:.1}s cancel_requested=true active_orders={} timeout_s={:.1}",
                pair_id,
                seconds_left.max(0.0),
                active_orders,
                timeout_s
            ));
        }
        if elapsed_s >= timeout_s {
            self.logger.warning(&format!(
                "[BOT] pair_id={} AwaitSettlement timeout t_left={:.1}s active_orders={} waited_s={:.1}",
                pair_id,
                seconds_left.max(0.0),
                active_orders,
                elapsed_s
            ));
            self._set_exit_reason("AWAIT_SETTLEMENT");
            return true;
        }

        false
    }

    /// Returns or derives run BOT runtime loop for the active BOT execution path.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _run_bot_runtime_loop(&self) -> String {
        let mut last_log = 0.0;
        let mut stale_logged = false;
        let pair_id = self.pair_identity().pair_id;
        self.logger.info(&format!(
            "[BOT] pair_id={} Phase 2 open-both active; runtime path is isolated from settlement-shaper target-gap planning and opening now posts neutral paired BUY seeds",
            pair_id
        ));
        while !self.stop_flag.load(Ordering::SeqCst) {
            let wait_s = self.loop_wait_seconds_maker.max(0.01);
            thread::sleep(Duration::from_secs_f64(wait_s.min(0.5)));
            let now = now_ts_f64();
            self._bot_runtime_refresh_daily_liquidity_counters();
            let t_into_s = now - self.start_ts as f64;
            let seconds_left = self.expiry_ts as f64 - now;
            let (total_cost, qy, qn, cost_yes, cost_no) = self
                .state
                .lock()
                .map(|s| (s.c_yes + s.c_no, s.q_yes, s.q_no, s.c_yes, s.c_no))
                .unwrap_or((0.0, 0.0, 0.0, 0.0, 0.0));
            let cfg = *self._bot_runtime_cfg();
            if let Err(reason) = bot_runtime_validate_config(&cfg) {
                self.logger.warning(&format!(
                    "[BOT] invalid config reason={} -> stopping bot loop",
                    reason
                ));
                self._set_exit_reason(&format!("BOT_INVALID_CONFIG:{reason}"));
                break;
            }
            let mut phase = bot_runtime_phase_from_t_into_s(t_into_s, &cfg);
            if bot_runtime_should_stop_for_rollover(seconds_left, self.cfg.stop_buffer_seconds) {
                phase = BotRuntimePhase::AwaitSettlement;
            }
            if matches!(phase, BotRuntimePhase::OpenBoth) {
                self._bot_runtime_note_open_confirmed(now);
            }
            let pair_snapshot =
                self._pair_snapshot_from_inputs(phase, t_into_s, qy, qn, cost_yes, cost_no);
            let pair_id = pair_snapshot.identity.pair_id.clone();
            let await_second_fill_hard_paused = self
                .bot_runtime_state
                .lock()
                .map(|st| st.await_second_fill_hard_paused)
                .unwrap_or(false);
            let (owner, owner_reason) =
                bot_runtime_owner_for_snapshot(phase, qy, qn, await_second_fill_hard_paused);
            let prearm_status = matches!(phase, BotRuntimePhase::PreArm)
                .then(|| self._bot_runtime_prearm_status(t_into_s));
            if let Ok(mut st) = self.bot_runtime_state.lock() {
                if !st.armed_once {
                    st.armed_once = true;
                    st.phase = phase;
                    st.state_enter_ts = now;
                    st.owner = owner;
                    st.owner_enter_ts = now;
                    st.owner_reason = owner_reason;
                    self.logger.info(&format!(
                        "[BOT] pair_id={} armed phase={} owner={} reason={} t_into={:.1}s t_left={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
                        pair_id,
                        phase.as_str(),
                        owner.as_str(),
                        owner_reason,
                        t_into_s.max(0.0),
                        seconds_left.max(0.0),
                        qy,
                        qn,
                        total_cost
                    ));
                } else {
                    if st.phase != phase {
                        let prearm_summary = if st.phase == BotRuntimePhase::PreArm
                            && phase == BotRuntimePhase::OpenBoth
                        {
                            format!(
                                " prearm_ready={} prearm_hold_reason={}",
                                st.prearm_ready_once,
                                if st.prearm_hold_reason.is_empty() {
                                    "NA"
                                } else {
                                    st.prearm_hold_reason.as_str()
                                }
                            )
                        } else {
                            String::new()
                        };
                        self.logger.info(&format!(
                            "[BOT] pair_id={} phase {} -> {} t_into={:.1}s t_left={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}{}",
                            pair_id,
                            st.phase.as_str(),
                            phase.as_str(),
                            t_into_s.max(0.0),
                            seconds_left.max(0.0),
                            qy,
                            qn,
                            total_cost,
                            prearm_summary
                        ));
                        st.phase = phase;
                        st.state_enter_ts = now;
                    }
                    if st.owner != owner || st.owner_reason != owner_reason {
                        self.logger.info(&format!(
                            "[BOT] pair_id={} owner {} -> {} reason={} phase={} t_into={:.1}s t_left={:.1}s",
                            pair_id,
                            st.owner.as_str(),
                            owner.as_str(),
                            owner_reason,
                            phase.as_str(),
                            t_into_s.max(0.0),
                            seconds_left.max(0.0)
                        ));
                        st.owner = owner;
                        st.owner_enter_ts = now;
                        st.owner_reason = owner_reason;
                    }
                }
                if let Some(prearm) = prearm_status.as_ref() {
                    if prearm.ready {
                        st.prearm_hold_reason = prearm.hold_reason.clone();
                        st.prearm_ready_before_open = true;
                        if !st.prearm_ready_once {
                            st.prearm_ready_once = true;
                            st.prearm_ready_ts = now;
                            self.logger.info(&format!(
                                "[BOT][PREARM] pair_id={} ready t_to_open={:.1}s market_slug={} start_ts={} discovery_preloaded={} asset_ids_ready={} market_ws_ready={} user_ws_ready={} quote_inputs={} paired_quotes={} prearm_lead={:.0}s",
                                pair_id,
                                (-t_into_s).max(0.0),
                                self.market_slug,
                                self.start_ts,
                                prearm.market_selected,
                                prearm.asset_ids_ready,
                                prearm.market_ws_ready,
                                prearm.user_ws_ready,
                                prearm.quote_input_reason,
                                prearm.paired_quote_reason,
                                cfg.prearm_lead_seconds
                            ));
                        }
                    } else if st.prearm_hold_reason != prearm.hold_reason {
                        st.prearm_hold_reason = prearm.hold_reason.clone();
                        self.logger.info(&format!(
                            "[BOT][PREARM] pair_id={} hold reason={} t_to_open={:.1}s market_slug={} market_selected={} asset_ids_ready={} market_ws_ready={} user_ws_ready={} quotes_ready={} paired_quotes_ready={} quote_inputs={} paired_quotes={}",
                            pair_id,
                            prearm.hold_reason,
                            (-t_into_s).max(0.0),
                            self.market_slug,
                            prearm.market_selected,
                            prearm.asset_ids_ready,
                            prearm.market_ws_ready,
                            prearm.user_ws_ready,
                            prearm.quotes_ready,
                            prearm.paired_quotes_ready,
                            prearm.quote_input_reason,
                            prearm.paired_quote_reason
                        ));
                    }
                }
            }
            self._bot_runtime_note_first_fill(now, qy, qn, cost_yes, cost_no);
            self._bot_runtime_note_await_second_fill_progress(
                now, t_into_s, qy, qn, cost_yes, cost_no,
            );
            self._bot_runtime_note_imbalance_state(now, qy, qn, &cfg);
            if bot_runtime_should_run_open_both_handler(owner) {
                self._bot_runtime_open_both_handler(now, t_into_s, total_cost, qy, qn, &cfg);
            } else if matches!(owner, BotRuntimeControlOwner::AwaitSecondFill) {
                self._bot_runtime_await_second_fill_handler(
                    now, t_into_s, total_cost, qy, qn, cost_yes, cost_no, &cfg,
                );
            } else if matches!(owner, BotRuntimeControlOwner::PairBuild) {
                self._bot_runtime_pair_build_handler(
                    now, t_into_s, total_cost, qy, qn, cost_yes, cost_no, &cfg,
                );
            } else if matches!(owner, BotRuntimeControlOwner::Taper) {
                self._bot_runtime_taper_handler(
                    now, t_into_s, total_cost, qy, qn, cost_yes, cost_no, &cfg,
                );
            } else if matches!(owner, BotRuntimeControlOwner::AwaitSettlement) {
                if self._bot_runtime_await_settlement_handler(now, seconds_left) {
                    break;
                }
            } else {
                let _ = self._bot_runtime_cancel_pair_build_orders(
                    None,
                    "bot_runtime_pair_build_owner_inactive",
                );
                let _ =
                    self._bot_runtime_cancel_taper_orders(None, "bot_runtime_taper_owner_inactive");
            }
            if matches!(phase, BotRuntimePhase::PreArm) {
                stale_logged = false;
            } else if !self._market_data_fresh() {
                if !stale_logged {
                    self.logger.info(&format!(
                        "[BOT] pair_id={} hold reason=market_data_stale",
                        pair_id
                    ));
                    stale_logged = true;
                }
            } else if stale_logged {
                self.logger.info(&format!(
                    "[BOT] pair_id={} market data fresh -> bot phase controller active",
                    pair_id
                ));
                stale_logged = false;
            }
            if now - last_log >= (self.cfg.log_every as f64).max(0.5) {
                let (
                    phase,
                    owner,
                    owner_reason,
                    armed_once,
                    prearm_ready_once,
                    prearm_ready_before_open,
                    prearm_hold_reason,
                    open_both_attempt_count,
                    open_both_seed_anchor_ts,
                    open_both_first_submit_ts,
                    open_both_first_yes_submit_ts,
                    open_both_first_no_submit_ts,
                    open_both_first_submit_delta_ms,
                    open_both_seed_by_deadline_met,
                    open_both_late_seed_unlock_used,
                    open_both_submit_delta_met,
                    open_both_first_fill_ts,
                    await_second_fill_started_ts,
                    await_second_fill_second_fill_ts,
                    second_side_by_15s,
                    second_side_by_30s,
                    first_fill_to_second_fill_ms,
                    await_second_fill_rescue_used,
                    await_second_fill_hard_paused,
                    imbalance_state,
                    taper_fill_events_after_240,
                    taper_fill_events_after_270,
                    taper_new_orders_after_240,
                    taper_new_orders_after_270,
                ) = self
                    .bot_runtime_state
                    .lock()
                    .map(|st| {
                        (
                            st.phase,
                            st.owner,
                            st.owner_reason,
                            st.armed_once,
                            st.prearm_ready_once,
                            st.prearm_ready_before_open,
                            if st.prearm_hold_reason.is_empty() {
                                "NA".to_string()
                            } else {
                                st.prearm_hold_reason.clone()
                            },
                            st.open_both_attempt_count,
                            st.open_both_seed_anchor_ts,
                            st.open_both_first_submit_ts,
                            st.open_both_first_yes_submit_ts,
                            st.open_both_first_no_submit_ts,
                            st.open_both_first_submit_delta_ms,
                            st.open_both_seed_by_deadline_met,
                            st.open_both_late_seed_unlock_used,
                            st.open_both_submit_delta_met,
                            st.open_both_first_fill_ts,
                            st.await_second_fill_started_ts,
                            st.await_second_fill_second_fill_ts,
                            st.second_side_by_15s,
                            st.second_side_by_30s,
                            st.first_fill_to_second_fill_ms,
                            st.await_second_fill_rescue_used,
                            st.await_second_fill_hard_paused,
                            st.imbalance_state,
                            st.taper_fill_events_after_240,
                            st.taper_fill_events_after_270,
                            st.taper_new_orders_after_240,
                            st.taper_new_orders_after_270,
                        )
                    })
                    .unwrap_or((
                        BotRuntimePhase::default(),
                        BotRuntimeControlOwner::default(),
                        "state_unavailable",
                        false,
                        false,
                        false,
                        "state_unavailable".to_string(),
                        0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        false,
                        false,
                        false,
                        0.0,
                        0.0,
                        0.0,
                        false,
                        false,
                        0.0,
                        false,
                        false,
                        BotRuntimeImbalanceState::Normal,
                        0,
                        0,
                        0,
                        0,
                    ));
                self.logger.info(&format!(
                    "[BOT] pair_id={} hold phase={} owner={} owner_reason={} armed={} prearm_ready={} prearm_ready_before_open={} prearm_hold_reason={} open_attempts={} seed_anchor_t_into={} first_seed_submit_t_into={} first_yes_seed_submit_t_into={} first_no_seed_submit_t_into={} seed_by_5s_met={} late_seed_used={} seed_submit_delta_ms={} seed_submit_delta_met={} first_fill_t_into={} second_side_t_into={} first_fill_to_second_fill_ms={} second_side_by_15s={} second_side_by_30s={} await_second_fill_rescue_used={} await_second_fill_hard_paused={} imbalance_state={} unmatched_fraction={:.3} match_ratio={:.3} pair_taker_share={:.3} daily_taker_share={:.3} taper_fill_events_240={} taper_fill_events_270={} taper_new_orders_240={} taper_new_orders_270={} t_left={:.1}s prearm_lead={:.0}s qYES={:.2} qNO={:.2} total_cost={:.2} market_data_fresh={} market_connected={} user_connected={}",
                    pair_id,
                    phase.as_str(),
                    owner.as_str(),
                    owner_reason,
                    armed_once,
                    prearm_ready_once,
                    prearm_ready_before_open,
                    prearm_hold_reason,
                    open_both_attempt_count,
                    if open_both_seed_anchor_ts > 0.0 {
                        format!(
                            "{:.1}s",
                            (open_both_seed_anchor_ts - self.start_ts as f64).max(0.0)
                        )
                    } else {
                        "NA".to_string()
                    },
                    if open_both_first_submit_ts > 0.0 {
                        format!(
                            "{:.1}s",
                            (open_both_first_submit_ts - self.start_ts as f64).max(0.0)
                        )
                    } else {
                        "NA".to_string()
                    },
                    if open_both_first_yes_submit_ts > 0.0 {
                        format!(
                            "{:.1}s",
                            (open_both_first_yes_submit_ts - self.start_ts as f64).max(0.0)
                        )
                    } else {
                        "NA".to_string()
                    },
                    if open_both_first_no_submit_ts > 0.0 {
                        format!(
                            "{:.1}s",
                            (open_both_first_no_submit_ts - self.start_ts as f64).max(0.0)
                        )
                    } else {
                        "NA".to_string()
                    },
                    if open_both_first_yes_submit_ts > 0.0 && open_both_first_no_submit_ts > 0.0 {
                        open_both_seed_by_deadline_met.to_string()
                    } else if t_into_s >= cfg.open_both_seed_deadline_seconds + 1e-9 {
                        "false".to_string()
                    } else {
                        "pending".to_string()
                    },
                    open_both_late_seed_unlock_used,
                    if open_both_first_yes_submit_ts > 0.0 && open_both_first_no_submit_ts > 0.0 {
                        format!("{:.0}", open_both_first_submit_delta_ms)
                    } else {
                        "NA".to_string()
                    },
                    if open_both_first_yes_submit_ts > 0.0 && open_both_first_no_submit_ts > 0.0 {
                        open_both_submit_delta_met.to_string()
                    } else {
                        "pending".to_string()
                    },
                    if open_both_first_fill_ts > 0.0 {
                        format!("{:.1}s", (open_both_first_fill_ts - self.start_ts as f64).max(0.0))
                    } else {
                        "NA".to_string()
                    },
                    if await_second_fill_second_fill_ts > 0.0 {
                        format!(
                            "{:.1}s",
                            (await_second_fill_second_fill_ts - self.start_ts as f64).max(0.0)
                        )
                    } else {
                        "NA".to_string()
                    },
                    if await_second_fill_second_fill_ts > 0.0 {
                        format!("{:.0}", first_fill_to_second_fill_ms.max(0.0))
                    } else {
                        "NA".to_string()
                    },
                    if await_second_fill_second_fill_ts > 0.0 {
                        second_side_by_15s.to_string()
                    } else if await_second_fill_started_ts > 0.0
                        && t_into_s >= bot_runtime_await_second_fill_target_seconds() - 1e-9
                    {
                        "false".to_string()
                    } else {
                        "pending".to_string()
                    },
                    if await_second_fill_second_fill_ts > 0.0 {
                        second_side_by_30s.to_string()
                    } else if await_second_fill_started_ts > 0.0
                        && t_into_s >= bot_runtime_await_second_fill_deadline_seconds() - 1e-9
                    {
                        "false".to_string()
                    } else {
                        "pending".to_string()
                    },
                    await_second_fill_rescue_used,
                    await_second_fill_hard_paused,
                    imbalance_state.as_str(),
                    unmatched_fraction(qy, qn),
                    match_ratio(qy, qn),
                    bot_runtime_taker_share(
                        self.bot_runtime_state
                            .lock()
                            .map(|st| st.maker_fill_shares)
                            .unwrap_or(0.0),
                        self.bot_runtime_state
                            .lock()
                            .map(|st| st.taker_fill_shares)
                            .unwrap_or(0.0)
                    ),
                    bot_runtime_taker_share(
                        self.bot_runtime_state
                            .lock()
                            .map(|st| st.daily_maker_fill_shares)
                            .unwrap_or(0.0),
                        self.bot_runtime_state
                            .lock()
                            .map(|st| st.daily_taker_fill_shares)
                            .unwrap_or(0.0)
                    ),
                    taper_fill_events_after_240,
                    taper_fill_events_after_270,
                    taper_new_orders_after_240,
                    taper_new_orders_after_270,
                    seconds_left.max(0.0),
                    cfg.prearm_lead_seconds,
                    qy,
                    qn,
                    total_cost,
                    self._market_data_fresh(),
                    self.market_connected.load(Ordering::SeqCst),
                    self.user_connected.load(Ordering::SeqCst)
                ));
                last_log = now;
            }
        }
        let exit_reason = self._get_exit_reason();
        self._bot_runtime_log_final_metrics(&exit_reason);
        exit_reason
    }
}
