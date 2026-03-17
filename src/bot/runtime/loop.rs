use super::*;
impl MakerHedgeCapBot {
    /// Implements log final metrics for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_log_final_metrics(&self, exit_reason: &str) {
        let cfg = *self._bot_runtime_cfg();
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
        let metrics = bot_runtime_metrics_snapshot(&state, q_yes, q_no, cost_yes, cost_no, total_cost);
        let combined_avg_paid = if metrics.inventory_vwap_sum.is_finite() {
            format!("{:.3}", metrics.inventory_vwap_sum)
        } else {
            "NA".to_string()
        };
        let both_by_30s_achieved = state.seed_completion_both_sides_ts > 0.0
            && (state.seed_completion_both_sides_ts - self.start_ts as f64) <= 30.0 + 1e-9;
        let both_by_60s_achieved = state.seed_completion_both_sides_ts > 0.0
            && (state.seed_completion_both_sides_ts - self.start_ts as f64) <= 60.0 + 1e-9;
        let both_by_30s_target_met =
            (if both_by_30s_achieved { 1.0 } else { 0.0 }) + 1e-9 >= cfg.target_both_sides_by_30s;
        let both_by_60s_target_met =
            (if both_by_60s_achieved { 1.0 } else { 0.0 }) + 1e-9 >= cfg.target_both_sides_by_60s;
        let paired_cost_band_occupancy =
            bot_runtime_paired_cost_band_summary_u32(&metrics.paired_cost_band_observations);
        let paired_cost_band_occupancy_rate =
            bot_runtime_paired_cost_band_summary_fraction(&metrics.paired_cost_band_observations);
        let paired_size_delta_by_state =
            bot_runtime_paired_cost_band_summary_f64(&metrics.paired_size_delta_by_state);
        let canary_success = bot_runtime_canary_success(&metrics);
        let canary_failure_summary = bot_runtime_canary_failure_summary(&metrics);
        self.logger.info(&format!(
            "[BOT][METRICS] exit_reason={} market_participated={} market_participation={:.3} fills_per_market={} total_fill_shares={:.2} maker_fill_share={:.3} fill_events_by_segment={} fill_shares_by_segment={} paired_size={:.2} unmatched_size={:.2} pair_coverage={:.3} share_skew={:.3} combined_avg_paid={} paired_cost_band_occupancy={} paired_cost_band_occupancy_rate={} paired_size_delta_by_state={} below_snapshot_optional_submit_count={} below_snapshot_optional_submit_shares={:.2} below_snapshot_optional_fill_count={} below_snapshot_optional_fill_shares={:.2} below_snapshot_optional_fill_rate={:.3} tail_at_expiry={:.2} worst_case_settlement_floor={:+.2} bad_regime_expensive_ratio={:.3} bad_regime_shutdown={} canary_success={} canary_failure_summary={} fills_after_taper_start={} fills_after_final_quiet={} new_orders_after_taper_start={} new_orders_after_final_quiet={} both_by_30s={} both_by_60s={} target_both_by_30s={:.2} target_both_by_60s={:.2} target_both_by_30s_met={} target_both_by_60s_met={} skipped_optional_adds={} repair_reserve_blocks={} floor_tail_blocks={} startup_completion_blocked={}",
            exit_reason,
            metrics.market_participated,
            if metrics.market_participated { 1.0 } else { 0.0 },
            metrics.fills_per_market,
            metrics.total_fill_shares,
            metrics.maker_fill_share,
            bot_runtime_fill_distribution_summary_u32(&metrics.fill_events_by_segment),
            bot_runtime_fill_distribution_summary_f64(&metrics.fill_shares_by_segment),
            metrics.paired_size,
            metrics.unmatched_size,
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
            both_by_30s_achieved,
            both_by_60s_achieved,
            cfg.target_both_sides_by_30s,
            cfg.target_both_sides_by_60s,
            both_by_30s_target_met,
            both_by_60s_target_met,
            metrics.skipped_optional_add_count,
            metrics.repair_reserve_blocked_count,
            metrics.floor_tail_blocked_count,
            metrics.startup_completion_blocked_count
        ));
    }
    /// Returns or derives run BOT runtime loop for the active BOT execution path.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _run_bot_runtime_loop(&self) -> String {
        let mut last_log = 0.0;
        let mut stale_logged = false;
        self.logger.info(
            "[BOT] Phase 2 open-both active; runtime path is isolated from settlement-shaper target-gap planning and opening now posts neutral paired BUY seeds",
        );
        while !self.stop_flag.load(Ordering::SeqCst) {
            let wait_s = self.loop_wait_seconds_maker.max(0.01);
            thread::sleep(Duration::from_secs_f64(wait_s.min(0.5)));
            let now = now_ts_f64();
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
                phase = BotRuntimePhase::HoldSettleRollover;
            }
            let (owner, owner_reason) = bot_runtime_owner_for_snapshot(phase, qy, qn);
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
                        "[BOT] armed phase={} owner={} reason={} t_into={:.1}s t_left={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
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
                            "[BOT] phase {} -> {} t_into={:.1}s t_left={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}{}",
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
                            "[BOT] owner {} -> {} reason={} phase={} t_into={:.1}s t_left={:.1}s",
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
                        if !st.prearm_ready_once {
                            st.prearm_ready_once = true;
                            st.prearm_ready_ts = now;
                            self.logger.info(&format!(
                                "[BOT][PREARM] ready t_to_open={:.1}s market_slug={} start_ts={} discovery_preloaded={} asset_ids_ready={} market_ws_ready={} user_ws_ready={} quote_inputs={} paired_quotes={} prearm_lead={:.0}s",
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
                            "[BOT][PREARM] hold reason={} t_to_open={:.1}s market_slug={} market_selected={} asset_ids_ready={} market_ws_ready={} user_ws_ready={} quotes_ready={} paired_quotes_ready={} quote_inputs={} paired_quotes={}",
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
            self._bot_runtime_note_seed_completion_progress(
                now, t_into_s, qy, qn, cost_yes, cost_no,
            );
            if bot_runtime_should_run_open_both_handler(owner) {
                self._bot_runtime_open_both_handler(now, t_into_s, total_cost, qy, qn, &cfg);
            } else if matches!(owner, BotRuntimeControlOwner::SeedCompletion) {
                self._bot_runtime_seed_completion_handler(
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
            } else {
                let _ = self._bot_runtime_cancel_pair_build_orders(
                    None,
                    "bot_runtime_pair_build_owner_inactive",
                );
                let _ = self._bot_runtime_cancel_taper_orders(
                    None,
                    "bot_runtime_taper_owner_inactive",
                );
            }
            if matches!(phase, BotRuntimePhase::PreArm) {
                stale_logged = false;
            } else if !self._market_data_fresh() {
                if !stale_logged {
                    self.logger.info("[BOT] hold reason=market_data_stale");
                    stale_logged = true;
                }
            } else if stale_logged {
                self.logger
                    .info("[BOT] market data fresh -> bot phase controller active");
                stale_logged = false;
            }
            if now - last_log >= (self.cfg.log_every as f64).max(0.5) {
                let (
                    phase,
                    owner,
                    owner_reason,
                    armed_once,
                    prearm_ready_once,
                    prearm_hold_reason,
                    open_both_attempt_count,
                    open_both_first_submit_ts,
                    open_both_first_fill_ts,
                    seed_completion_started_ts,
                    seed_completion_both_sides_ts,
                    seed_completion_failure_logged,
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
                            if st.prearm_hold_reason.is_empty() {
                                "NA".to_string()
                            } else {
                                st.prearm_hold_reason.clone()
                            },
                            st.open_both_attempt_count,
                            st.open_both_first_submit_ts,
                            st.open_both_first_fill_ts,
                            st.seed_completion_started_ts,
                            st.seed_completion_both_sides_ts,
                            st.seed_completion_failure_logged,
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
                        "state_unavailable".to_string(),
                        0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        false,
                        0,
                        0,
                        0,
                        0,
                    ));
                self.logger.info(&format!(
                    "[BOT] hold phase={} owner={} owner_reason={} armed={} prearm_ready={} prearm_hold_reason={} open_attempts={} first_seed_submit_t_into={} first_fill_t_into={} second_side_t_into={} second_side_latency={} both_by_30s={} both_by_60s={} seed_completion_failed={} taper_fill_events_240={} taper_fill_events_270={} taper_new_orders_240={} taper_new_orders_270={} t_left={:.1}s prearm_lead={:.0}s qYES={:.2} qNO={:.2} total_cost={:.2} market_data_fresh={} market_connected={} user_connected={}",
                    phase.as_str(),
                    owner.as_str(),
                    owner_reason,
                    armed_once,
                    prearm_ready_once,
                    prearm_hold_reason,
                    open_both_attempt_count,
                    if open_both_first_submit_ts > 0.0 {
                        format!("{:.1}s", (open_both_first_submit_ts - self.start_ts as f64).max(0.0))
                    } else {
                        "NA".to_string()
                    },
                    if open_both_first_fill_ts > 0.0 {
                        format!("{:.1}s", (open_both_first_fill_ts - self.start_ts as f64).max(0.0))
                    } else {
                        "NA".to_string()
                    },
                    if seed_completion_both_sides_ts > 0.0 {
                        format!("{:.1}s", (seed_completion_both_sides_ts - self.start_ts as f64).max(0.0))
                    } else {
                        "NA".to_string()
                    },
                    if open_both_first_fill_ts > 0.0 && seed_completion_both_sides_ts > 0.0 {
                        format!("{:.1}s", (seed_completion_both_sides_ts - open_both_first_fill_ts).max(0.0))
                    } else {
                        "NA".to_string()
                    },
                    if seed_completion_both_sides_ts > 0.0 {
                        ((seed_completion_both_sides_ts - self.start_ts as f64) <= 30.0 + 1e-9).to_string()
                    } else if t_into_s >= 30.0 && seed_completion_started_ts > 0.0 {
                        "false".to_string()
                    } else {
                        "pending".to_string()
                    },
                    if seed_completion_both_sides_ts > 0.0 {
                        ((seed_completion_both_sides_ts - self.start_ts as f64) <= 60.0 + 1e-9).to_string()
                    } else if t_into_s >= 60.0 && seed_completion_started_ts > 0.0 {
                        "false".to_string()
                    } else {
                        "pending".to_string()
                    },
                    seed_completion_failure_logged,
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
            if matches!(phase, BotRuntimePhase::HoldSettleRollover) {
                self.logger.info(&format!(
                    "[BOT] Expiring in {:.0}s -> stopping for rollover.",
                    (seconds_left - 10.0).max(0.0)
                ));
                self.cancel_all_orders_exchange("expiry");
                self._set_exit_reason("ROLLOVER");
                break;
            }
        }
        let exit_reason = self._get_exit_reason();
        self._bot_runtime_log_final_metrics(&exit_reason);
        exit_reason
    }
}

