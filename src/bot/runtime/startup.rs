use super::*;
impl MakerHedgeCapBot {
    /// Implements config for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_cfg(&self) -> &BotRuntimeConfigSnapshot {
        &self.bot_runtime_cfg
    }
    /// Logs BOT runtime config for diagnostics and operator visibility.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _log_bot_runtime_cfg(&self) {
        let cfg = self._bot_runtime_cfg();
        let effective_order_mode = self._bot_runtime_effective_order_mode();
        self.logger.info(&format!(
            "[BOT][CFG] mode={} configured_order_mode={} effective_order_mode={} phase_controller={} prearm_lead={:.0}s open_seed_deadline={:.1}s open_submit_delta_max={:.1}s late_seed_once={} phase_budgets=open:{:.0}-{:.0}% early:{:.0}-{:.0}% main:{:.0}-{:.0}% late:{:.0}-{:.0}% taper:{:.0}-{:.0}% clip_ladder={:.0}/{:.0}/{:.0}/{:.0} await_second_fill_target={:.0}s await_second_fill_deadline={:.0}s await_second_fill_rescue_once=true late_reduce_start={:.0}s late_balance_only_start={:.0}s late_stop_new_orders_start={:.0}s buy_only_normal_flow={} tail_caps={}s:{:.1}%/{}s:{:.1}%/late:{:.1}% bad_regime_window={:.0}s bad_regime_expensive_fraction={:.2}",
            self.exec_mode,
            self.configured_order_mode,
            effective_order_mode.as_str(),
            cfg.phase_controller,
            cfg.prearm_lead_seconds,
            cfg.open_both_seed_deadline_seconds,
            cfg.open_both_submit_delta_max_seconds,
            cfg.open_both_allow_single_late_seed,
            cfg.seed_budget_min_fraction * 100.0,
            cfg.seed_budget_max_fraction * 100.0,
            cfg.early_budget_min_fraction * 100.0,
            cfg.early_budget_max_fraction * 100.0,
            cfg.main_budget_min_fraction * 100.0,
            cfg.main_budget_max_fraction * 100.0,
            cfg.late_budget_min_fraction * 100.0,
            cfg.late_budget_max_fraction * 100.0,
            cfg.taper_budget_min_fraction * 100.0,
            cfg.taper_budget_max_fraction * 100.0,
            cfg.clip_ladder[0],
            cfg.clip_ladder[1],
            cfg.clip_ladder[2],
            cfg.clip_ladder[3],
            bot_runtime_await_second_fill_target_seconds(),
            bot_runtime_await_second_fill_deadline_seconds(),
            cfg.late_reduce_start_seconds,
            cfg.late_balance_only_start_seconds,
            cfg.late_stop_new_orders_start_seconds,
            cfg.buy_only_normal_flow,
            cfg.tail_cap_mid_start_seconds,
            cfg.tail_cap_early_fraction * 100.0,
            cfg.tail_cap_late_start_seconds,
            cfg.tail_cap_mid_fraction * 100.0,
            cfg.tail_cap_late_fraction * 100.0,
            cfg.bad_regime_window_seconds,
            cfg.bad_regime_expensive_fraction
        ));
        if (cfg.mean_reversion_tilt_fraction - 0.50).abs() > 1e-9 {
            self.logger.info(&format!(
                "[BOT][CFG] mean_reversion_tilt_fraction={:.2} (underdog side gets {:.0}% of clip shares)",
                cfg.mean_reversion_tilt_fraction,
                cfg.mean_reversion_tilt_fraction * 100.0
            ));
        }
    }
    /// Implements quote input status for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_quote_input_status(&self) -> (bool, String) {
        let (Some(yes_asset), Some(no_asset)) = (&self.yes_asset, &self.no_asset) else {
            return (false, "asset_ids_missing".to_string());
        };
        let stale_s = self.cfg.market_data_stale_add_block_seconds.max(1) as f64;
        let now = now_ts_f64();
        for (label, asset_id) in [("YES", yes_asset.as_str()), ("NO", no_asset.as_str())] {
            let (ready, reason) = bot_runtime_quote_snapshot_status(
                label,
                self._best_bid_ask_with_ts(asset_id),
                now,
                stale_s,
            );
            if !ready {
                return (false, reason);
            }
        }
        (true, "ok".to_string())
    }
    /// Implements startup pair quote status for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_startup_pair_quote_status(&self) -> (bool, String) {
        let (Some(yes_asset), Some(no_asset)) = (&self.yes_asset, &self.no_asset) else {
            return (false, "asset_ids_missing".to_string());
        };
        let stale_s = self.cfg.market_data_stale_add_block_seconds.max(1) as f64;
        bot_runtime_startup_pair_quote_status(
            self._best_bid_ask_with_ts(yes_asset),
            self._best_bid_ask_with_ts(no_asset),
            now_ts_f64(),
            stale_s,
        )
    }

    /// Implements post-open pair quote status for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_post_open_pair_quote_status(
        &self,
        now: f64,
    ) -> (bool, String) {
        let (Some(yes_asset), Some(no_asset)) = (&self.yes_asset, &self.no_asset) else {
            return (false, "asset_ids_missing".to_string());
        };
        let open_confirmed_ts = self
            .bot_runtime_state
            .lock()
            .map(|st| st.open_confirmed_ts)
            .unwrap_or(0.0);
        let stale_s = self.cfg.market_data_stale_add_block_seconds.max(1) as f64;
        bot_runtime_post_open_pair_quote_status(
            self._best_bid_ask_with_ts(yes_asset),
            self._best_bid_ask_with_ts(no_asset),
            open_confirmed_ts,
            now,
            stale_s,
        )
    }
    /// Implements prearm status for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_prearm_status(
        &self,
        t_into_s: f64,
    ) -> BotRuntimePreArmStatus {
        let cfg = *self._bot_runtime_cfg();
        let market_selected = !self.market_slug.trim().is_empty()
            && self.start_ts > 0
            && self.expiry_ts > self.start_ts;
        let asset_ids_ready =
            self.condition_id.is_some() && self.yes_asset.is_some() && self.no_asset.is_some();
        let market_ws_ready = self.market_connected.load(Ordering::SeqCst);
        let user_ws_required = self._bot_runtime_user_ws_required();
        let user_ws_ready = !user_ws_required || self.user_connected.load(Ordering::SeqCst);
        let (quotes_ready, quote_input_reason) = if asset_ids_ready {
            self._bot_runtime_quote_input_status()
        } else {
            (false, "asset_ids_missing".to_string())
        };
        let (paired_quotes_ready, paired_quote_reason) = if asset_ids_ready {
            self._bot_runtime_startup_pair_quote_status()
        } else {
            (false, "asset_ids_missing".to_string())
        };
        let mut status = bot_runtime_prearm_status_from_snapshot(
            t_into_s,
            market_selected,
            asset_ids_ready,
            market_ws_ready,
            user_ws_ready,
            quotes_ready,
            &quote_input_reason,
            paired_quotes_ready,
            &paired_quote_reason,
        );
        if t_into_s < 0.0 && !bot_runtime_prearm_window_active(t_into_s, &cfg) {
            status.ready = false;
            status.hold_reason = "before_prearm_window".to_string();
        }
        status
    }

    /// Records that startup prerequisites became ready before official open.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_prearm_ready_before_open(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.prearm_ready_before_open = true;
        }
    }

    /// Records the first runtime cycle observed at or after official open.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_open_confirmed(&self, now: f64) -> bool {
        let pair_id = self.pair_identity().pair_id;
        let should_log = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.open_confirmed_ts > 0.0 {
                    false
                } else {
                    st.open_confirmed_ts = now;
                    st.open_both_seed_anchor_ts = bot_runtime_open_both_seed_anchor_ts(
                        st.open_confirmed_ts,
                        st.open_both_first_tradable_post_open_ts,
                    );
                    true
                }
            })
            .unwrap_or(false);
        if should_log {
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] pair_id={} open_confirmed t_into={:.1}s",
                pair_id,
                (now - self.start_ts as f64).max(0.0)
            ));
        }
        should_log
    }

    /// Records the first post-open moment where both sides are tradable from fresh quotes.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_first_tradable_post_open(&self, now: f64) -> bool {
        let (ready, _reason) = self._bot_runtime_post_open_pair_quote_status(now);
        if !ready {
            return false;
        }
        let pair_id = self.pair_identity().pair_id;
        let should_log = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.open_both_first_tradable_post_open_ts > 0.0 {
                    false
                } else {
                    st.open_both_first_tradable_post_open_ts = now;
                    st.open_both_seed_anchor_ts = bot_runtime_open_both_seed_anchor_ts(
                        st.open_confirmed_ts,
                        st.open_both_first_tradable_post_open_ts,
                    );
                    true
                }
            })
            .unwrap_or(false);
        if should_log {
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] pair_id={} first_tradable_post_open t_into={:.1}s",
                pair_id,
                (now - self.start_ts as f64).max(0.0)
            ));
        }
        should_log
    }

    /// Records the first time startup seeding missed the initial post-open deadline.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_open_both_deadline_miss(
        &self,
        now: f64,
        seed_deadline_ts: f64,
    ) {
        let pair_id = self.pair_identity().pair_id;
        let should_log = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.open_both_seed_deadline_missed_ts > 0.0 {
                    false
                } else {
                    st.open_both_seed_deadline_missed_ts = now;
                    true
                }
            })
            .unwrap_or(false);
        if should_log {
            self.logger.warning(&format!(
                "[BOT][OPEN_BOTH] pair_id={} seed_deadline_missed t_into={:.1}s deadline_t_into={:.1}s",
                pair_id,
                (now - self.start_ts as f64).max(0.0),
                if seed_deadline_ts > 0.0 {
                    (seed_deadline_ts - self.start_ts as f64).max(0.0)
                } else {
                    0.0
                }
            ));
        }
    }

    /// Unlocks a single post-deadline late seed attempt once readiness is clean.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_unlock_late_seed_once(
        &self,
        now: f64,
        seed_deadline_ts: f64,
    ) -> bool {
        let pair_id = self.pair_identity().pair_id;
        let should_log = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.open_both_late_seed_unlock_used || st.open_both_late_seed_exhausted {
                    false
                } else {
                    st.open_both_late_seed_unlock_used = true;
                    true
                }
            })
            .unwrap_or(false);
        if should_log {
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] pair_id={} late_seed_unlock t_into={:.1}s deadline_t_into={:.1}s",
                pair_id,
                (now - self.start_ts as f64).max(0.0),
                if seed_deadline_ts > 0.0 {
                    (seed_deadline_ts - self.start_ts as f64).max(0.0)
                } else {
                    0.0
                }
            ));
        }
        should_log
    }

    /// Marks the single post-deadline late seed unlock as exhausted after a no-op submit.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_mark_late_seed_exhausted(&self, now: f64) {
        let pair_id = self.pair_identity().pair_id;
        let should_log = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.open_both_late_seed_exhausted {
                    false
                } else {
                    st.open_both_late_seed_exhausted = true;
                    true
                }
            })
            .unwrap_or(false);
        if should_log {
            self.logger.warning(&format!(
                "[BOT][OPEN_BOTH] pair_id={} late_seed_exhausted t_into={:.1}s",
                pair_id,
                (now - self.start_ts as f64).max(0.0)
            ));
        }
    }
    /// Implements open both hold changed for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_open_both_hold_changed(&self, reason: &str) -> bool {
        self.bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.open_both_last_hold_reason == reason {
                    false
                } else {
                    st.open_both_last_hold_reason = reason.to_string();
                    true
                }
            })
            .unwrap_or(true)
    }
    /// Implements clear open both hold for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_clear_open_both_hold(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.open_both_last_hold_reason.clear();
        }
    }
    /// Implements note open both submit for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_open_both_submit(
        &self,
        now: f64,
        yes_new: bool,
        no_new: bool,
        seed_deadline_ts: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) -> (u32, bool) {
        let pair_id = self.pair_identity().pair_id;
        let mut attempt_count = 0;
        let mut first_submit = false;
        let mut first_yes_submit = false;
        let mut first_no_submit = false;
        let mut delta_logged = false;
        let mut submit_delta_ms = 0.0;
        let mut seed_by_deadline_met = false;
        let mut submit_delta_met = false;
        self.bot_runtime_state
            .lock()
            .map(|mut st| {
                st.open_both_last_hold_reason.clear();
                st.open_both_attempt_count = st.open_both_attempt_count.saturating_add(1);
                attempt_count = st.open_both_attempt_count;
                if st.open_both_first_submit_ts <= 0.0 {
                    st.open_both_first_submit_ts = now;
                    first_submit = true;
                }
                if yes_new && st.open_both_first_yes_submit_ts <= 0.0 {
                    st.open_both_first_yes_submit_ts = now;
                    first_yes_submit = true;
                }
                if no_new && st.open_both_first_no_submit_ts <= 0.0 {
                    st.open_both_first_no_submit_ts = now;
                    first_no_submit = true;
                }
                if let Some(delta_ms) = bot_runtime_open_both_submit_delta_ms(
                    st.open_both_first_yes_submit_ts,
                    st.open_both_first_no_submit_ts,
                ) {
                    st.open_both_first_submit_delta_ms = delta_ms;
                    let latest_submit_ts = st
                        .open_both_first_yes_submit_ts
                        .max(st.open_both_first_no_submit_ts);
                    st.open_both_seed_by_deadline_met =
                        seed_deadline_ts > 0.0 && latest_submit_ts <= seed_deadline_ts + 1e-9;
                    st.open_both_submit_delta_met =
                        delta_ms <= (cfg.open_both_submit_delta_max_seconds * 1000.0) + 1e-9;
                    if first_yes_submit || first_no_submit {
                        delta_logged = true;
                        submit_delta_ms = delta_ms;
                        seed_by_deadline_met = st.open_both_seed_by_deadline_met;
                        submit_delta_met = st.open_both_submit_delta_met;
                    }
                }
                (attempt_count, first_submit)
            })
            .unwrap_or((0, false));
        let t_into_s = (now - self.start_ts as f64).max(0.0);
        if first_yes_submit {
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] pair_id={} first_yes_seed_submit t_into={:.1}s",
                pair_id, t_into_s
            ));
        }
        if first_no_submit {
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] pair_id={} first_no_seed_submit t_into={:.1}s",
                pair_id, t_into_s
            ));
        }
        if delta_logged {
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] pair_id={} first_seed_submit_delta_ms={:.0} seed_by_deadline_met={} submit_delta_met={}",
                pair_id,
                submit_delta_ms,
                seed_by_deadline_met,
                submit_delta_met
            ));
        }
        (attempt_count, first_submit)
    }
    /// Implements note first fill for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_first_fill(
        &self,
        now: f64,
        q_yes: f64,
        q_no: f64,
        cost_yes: f64,
        cost_no: f64,
    ) {
        let phase = bot_runtime_phase_from_t_into_s(
            (now - self.start_ts as f64).max(0.0),
            self._bot_runtime_cfg(),
        );
        let pair_snapshot = self._pair_snapshot_from_inputs(
            phase,
            (now - self.start_ts as f64).max(0.0),
            q_yes,
            q_no,
            cost_yes,
            cost_no,
        );
        let has_fill =
            has_side_participation(q_yes, cost_yes) || has_side_participation(q_no, cost_no);
        if !has_fill {
            return;
        }
        let first_submit_ts = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.open_both_first_fill_ts > 0.0 {
                    None
                } else {
                    st.open_both_first_fill_ts = now;
                    Some(st.open_both_first_submit_ts)
                }
            })
            .unwrap_or(None);
        let Some(first_submit_ts) = first_submit_ts else {
            return;
        };
        self.logger.info(&format!(
            "[BOT][OPEN_BOTH] pair_id={} first_fill t_into={:.1}s submit_to_fill_ms={:.0} qYES={:.2} qNO={:.2} costYES={:.2} costNO={:.2}",
            pair_snapshot.identity.pair_id,
            (now - self.start_ts as f64).max(0.0),
            if first_submit_ts > 0.0 {
                ((now - first_submit_ts).max(0.0)) * 1000.0
            } else {
                0.0
            },
            q_yes,
            q_no,
            cost_yes.max(0.0),
            cost_no.max(0.0)
        ));
    }
    /// Implements log open both hold for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_log_open_both_hold(
        &self,
        reason: &str,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
    ) {
        if !self._bot_runtime_open_both_hold_changed(reason) {
            return;
        }
        let pair_id = self.pair_identity().pair_id;
        let _ = self._audit_insert_runtime_event(
            "risk_block",
            None,
            None,
            None,
            None,
            Some(reason),
            json!({
                "scope": "open_both",
                "pair_id": pair_id,
                "reason_code": reason,
                "t_into_seconds": t_into_s.max(0.0),
                "q_yes": q_yes.max(0.0),
                "q_no": q_no.max(0.0),
                "total_cost": total_cost.max(0.0),
            }),
        );
        self.logger.info(&format!(
            "[BOT][OPEN_BOTH] pair_id={} hold reason={} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
            pair_id,
            reason,
            t_into_s.max(0.0),
            q_yes,
            q_no,
            total_cost.max(0.0)
        ));
    }
    /// Implements pair order asymmetry for the maker-side BOT workflow.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _maker_pair_order_asymmetry(
        &self,
        now: f64,
        yes_asset: &str,
        no_asset: &str,
        origin_prefix: &str,
    ) -> Option<MakerPairOrderAsymmetry> {
        let yes_slot = self._maker_order_slot_get(&MakerOrderKey::buy(yes_asset));
        let no_slot = self._maker_order_slot_get(&MakerOrderKey::buy(no_asset));
        maker_pair_order_asymmetry(&yes_slot, &no_slot, origin_prefix, now)
    }
    /// Implements open both handler for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_open_both_handler(
        &self,
        now: f64,
        t_into_s: f64,
        _total_cost: f64,
        _q_yes: f64,
        _q_no: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
        let pair_snapshot = self
            ._pair_snapshot_from_state(bot_runtime_phase_from_t_into_s(t_into_s, cfg), t_into_s);
        let pair_id = pair_snapshot.identity.pair_id.clone();
        let total_cost = pair_snapshot.total_cost;
        let q_yes = pair_snapshot.position.q_yes;
        let q_no = pair_snapshot.position.q_no;
        self._bot_runtime_note_open_confirmed(now);
        let (
            seed_anchor_ts,
            seed_deadline_ts,
            no_first_submit_yet,
            late_seed_unlock_used,
            late_seed_exhausted,
        ) = self
            .bot_runtime_state
            .lock()
            .map(|st| {
                let anchor_ts = bot_runtime_open_both_seed_anchor_ts(
                    st.open_confirmed_ts,
                    st.open_both_first_tradable_post_open_ts,
                );
                (
                    anchor_ts,
                    bot_runtime_open_both_seed_deadline_ts(anchor_ts, cfg),
                    st.open_both_first_yes_submit_ts <= 0.0
                        && st.open_both_first_no_submit_ts <= 0.0,
                    st.open_both_late_seed_unlock_used,
                    st.open_both_late_seed_exhausted,
                )
            })
            .unwrap_or((0.0, 0.0, true, false, false));
        let post_deadline_without_entry =
            no_first_submit_yet && seed_deadline_ts > 0.0 && now > seed_deadline_ts + 1e-9;
        if post_deadline_without_entry {
            self._bot_runtime_note_open_both_deadline_miss(now, seed_deadline_ts);
        }
        let (yes_asset, no_asset) = match (&self.yes_asset, &self.no_asset) {
            (Some(yes_asset), Some(no_asset)) => (yes_asset.as_str(), no_asset.as_str()),
            _ => {
                self._bot_runtime_log_open_both_hold(
                    "missing_assets",
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };
        if !self.market_connected.load(Ordering::SeqCst) {
            self._bot_runtime_log_open_both_hold(
                "market_ws_disconnected",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if self._bot_runtime_user_ws_required() && !self.user_connected.load(Ordering::SeqCst) {
            self._bot_runtime_log_open_both_hold(
                "user_ws_disconnected",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let (quotes_ready, quote_reason) = self._bot_runtime_quote_input_status();
        if !quotes_ready {
            self._bot_runtime_log_open_both_hold(
                &format!("quote_inputs_unready:{quote_reason}"),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let (paired_quotes_ready, paired_quote_reason) =
            self._bot_runtime_startup_pair_quote_status();
        if !paired_quotes_ready {
            self._bot_runtime_log_open_both_hold(
                &format!("paired_quotes_unready:{paired_quote_reason}"),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let (post_open_quotes_ready, post_open_quote_reason) =
            self._bot_runtime_post_open_pair_quote_status(now);
        if !post_open_quotes_ready {
            self._bot_runtime_log_open_both_hold(
                &format!("post_open_quotes_unready:{post_open_quote_reason}"),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        self._bot_runtime_note_first_tradable_post_open(now);
        let Some((y_bid, _y_ask)) = self._best_bid_ask(yes_asset) else {
            self._bot_runtime_log_open_both_hold(
                "missing_yes_quotes",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        let Some((n_bid, _n_ask)) = self._best_bid_ask(no_asset) else {
            self._bot_runtime_log_open_both_hold(
                "missing_no_quotes",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        let pair_sum = y_bid + n_bid;
        if y_bid <= 0.0 || n_bid <= 0.0 || pair_sum <= 0.0 {
            self._bot_runtime_log_open_both_hold(
                "zero_bid_pair",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if pair_sum >= 1.0 {
            self._bot_runtime_log_open_both_hold(
                &format!("pair_sum_too_high({pair_sum:.3})"),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let total_usable_budget =
            usable_budget_after_reserve(self.cfg.max_total_cost, self.cfg.reserve_usd);
        let budget_snapshot =
            bot_runtime_budget_snapshot(t_into_s, total_usable_budget, total_cost, cfg);
        if budget_snapshot.remaining_to_max_cost <= 1e-9 {
            self._bot_runtime_log_open_both_hold(
                "phase_budget_exhausted",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let Some(size_int) = bot_runtime_open_both_seed_size(
            cfg.clip_ladder[0],
            self.cfg.min_shares,
            pair_sum,
            total_cost + budget_snapshot.remaining_to_max_cost,
            total_cost,
        ) else {
            self._bot_runtime_log_open_both_hold(
                "budget_too_small",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        let yes_key = MakerOrderKey::buy(yes_asset);
        let no_key = MakerOrderKey::buy(no_asset);
        let prev_yes_slot = self._maker_order_slot_get(&yes_key);
        let prev_no_slot = self._maker_order_slot_get(&no_key);
        let yes_live = maker_slot_family_live(&prev_yes_slot, "BOT_OPEN_BOTH");
        let no_live = maker_slot_family_live(&prev_no_slot, "BOT_OPEN_BOTH");
        if yes_live && no_live {
            self._bot_runtime_log_open_both_hold(
                &format!(
                    "awaiting_open_both_live_orders:{}:{}",
                    maker_order_lifecycle_label(prev_yes_slot.state),
                    maker_order_lifecycle_label(prev_no_slot.state)
                ),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let asymmetry_timeout_s = self.cfg.stale_seconds.max(1) as f64;
        if let Some(asymmetry) =
            maker_pair_order_asymmetry(&prev_yes_slot, &prev_no_slot, "BOT_OPEN_BOTH", now)
        {
            let live_asset = match asymmetry.live_side {
                OutcomeSide::Yes => yes_asset,
                OutcomeSide::No => no_asset,
            };
            let live_key = MakerOrderKey::buy(live_asset);
            if asymmetry.state != MakerOrderLifecycle::CancelPending
                && asymmetry.age_s >= asymmetry_timeout_s
            {
                let cancel_reason = format!(
                    "BOT open-both asymmetric submit stale live_side={} age_s={:.1}",
                    asymmetry.live_side.as_str(),
                    asymmetry.age_s
                );
                if let Err(reason) =
                    self._maker_order_request_refresh_cancel(&live_key, cancel_reason.as_str())
                {
                    self._bot_runtime_log_open_both_hold(
                        &reason, t_into_s, total_cost, q_yes, q_no,
                    );
                    return;
                }
                self._bot_runtime_log_open_both_hold(
                    &format!(
                        "asymmetric_submit_stale_cancel:{}:{:.1}",
                        asymmetry.live_side.as_str(),
                        asymmetry.age_s
                    ),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            } else {
                self._bot_runtime_log_open_both_hold(
                    &format!(
                        "awaiting_asymmetric_submit_resolution:{}:{}",
                        asymmetry.live_side.as_str(),
                        maker_order_lifecycle_label(asymmetry.state)
                    ),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            return;
        }
        let mut late_seed_attempt_active = false;
        if post_deadline_without_entry {
            if late_seed_exhausted
                || (late_seed_unlock_used && !cfg.open_both_allow_single_late_seed)
            {
                self._bot_runtime_log_open_both_hold(
                    "late_seed_exhausted",
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            if cfg.open_both_allow_single_late_seed {
                if late_seed_unlock_used {
                    self._bot_runtime_log_open_both_hold(
                        "late_seed_exhausted",
                        t_into_s,
                        total_cost,
                        q_yes,
                        q_no,
                    );
                    return;
                }
                late_seed_attempt_active =
                    self._bot_runtime_unlock_late_seed_once(now, seed_deadline_ts);
                if !late_seed_attempt_active {
                    self._bot_runtime_log_open_both_hold(
                        "late_seed_exhausted",
                        t_into_s,
                        total_cost,
                        q_yes,
                        q_no,
                    );
                    return;
                }
            } else {
                self._bot_runtime_log_open_both_hold(
                    "seed_deadline_expired",
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        }
        self._set_pending_entry_reason("BOT_OPEN_BOTH");
        let submit_started = now_ts_f64();
        let tick_size = self.cfg.tick.max(0.0001);
        let (_, seed_underdog_side) =
            bot_runtime_favorite_underdog_sides(y_bid, n_bid, tick_size);
        let (y_seed, n_seed) = bot_runtime_mean_reversion_clip_pair(
            size_int,
            cfg.mean_reversion_tilt_fraction,
            seed_underdog_side,
        );
        let (y_oid, n_oid) = self._maker_submit_pair_orders_asymmetric(
            y_seed,
            n_seed,
            y_bid,
            n_bid,
            "GTC",
            Some(true),
            "BOT_OPEN_BOTH",
        );
        let submit_elapsed_ms = ((now_ts_f64() - submit_started).max(0.0)) * 1000.0;
        let yes_live = self
            ._maker_order_slot_get(&yes_key)
            .order_id
            .or(y_oid.clone());
        let no_live = self
            ._maker_order_slot_get(&no_key)
            .order_id
            .or(n_oid.clone());
        let yes_noop = yes_live
            .as_deref()
            .map(|order_id| self._consume_refresh_cadence_noop_marker(order_id))
            .unwrap_or(false);
        let no_noop = no_live
            .as_deref()
            .map(|order_id| self._consume_refresh_cadence_noop_marker(order_id))
            .unwrap_or(false);
        let yes_new =
            !yes_noop && maker_pair_submit_leg_is_new(yes_live.as_deref(), &prev_yes_slot);
        let no_new = !no_noop && maker_pair_submit_leg_is_new(no_live.as_deref(), &prev_no_slot);
        if yes_new || no_new {
            let decision_event_id = self._audit_insert_decision_event(
                "open_both",
                None,
                true,
                "open_both_submit",
                Some("BOT_OPEN_BOTH"),
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            for (is_new, live_oid, side) in [
                (yes_new, yes_live.as_deref(), OutcomeSide::Yes),
                (no_new, no_live.as_deref(), OutcomeSide::No),
            ] {
                if !is_new {
                    continue;
                }
                if let (Some(order_id), Some(decision_event_id)) =
                    (live_oid, decision_event_id.as_deref())
                {
                    self._audit_attach_decision_context(
                        order_id,
                        decision_event_id,
                        "open_both_submit",
                    );
                    self._merge_order_execution_context_fields(
                        order_id,
                        &json!({
                            "submit_origin": "BOT_OPEN_BOTH",
                            "submit_side": side.as_str(),
                        }),
                    );
                }
            }
            let (attempt_count, first_submit) = self._bot_runtime_note_open_both_submit(
                submit_started,
                yes_new,
                no_new,
                seed_deadline_ts,
                cfg,
            );
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] pair_id={} submit attempt={} t_into={:.1}s seed_anchor_t_into={} y_bid={:.3} n_bid={:.3} pair_sum={:.3} clip={} post_only=true neutral=true favorite_gating=false elapsed_ms={:.0} first_submit={}",
                pair_id,
                attempt_count,
                t_into_s.max(0.0),
                if seed_anchor_ts > 0.0 {
                    format!("{:.1}s", (seed_anchor_ts - self.start_ts as f64).max(0.0))
                } else {
                    "NA".to_string()
                },
                y_bid,
                n_bid,
                pair_sum,
                size_int,
                submit_elapsed_ms,
                first_submit
            ));
        }
        if late_seed_attempt_active && !yes_new && !no_new {
            self._bot_runtime_mark_late_seed_exhausted(now);
        }
        if let Some(asymmetry) =
            self._maker_pair_order_asymmetry(now_ts_f64(), yes_asset, no_asset, "BOT_OPEN_BOTH")
        {
            self._bot_runtime_log_open_both_hold(
                &format!(
                    "asymmetric_submit:{}:{:.0}ms",
                    asymmetry.live_side.as_str(),
                    submit_elapsed_ms
                ),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if yes_live.is_none() && no_live.is_none() {
            self._bot_runtime_log_open_both_hold(
                "no_pair_orders_live",
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if !yes_new && !no_new {
            self._bot_runtime_clear_open_both_hold();
        }
    }
    /// Implements await second fill hold changed for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_await_second_fill_hold_changed(&self, reason: &str) -> bool {
        self.bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.await_second_fill_last_hold_reason == reason {
                    false
                } else {
                    st.await_second_fill_last_hold_reason = reason.to_string();
                    true
                }
            })
            .unwrap_or(true)
    }
    /// Implements clear await second fill hold for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_clear_await_second_fill_hold(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.await_second_fill_last_hold_reason.clear();
        }
    }
    /// Implements fill clears pair build repost for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_fill_clears_pair_build_repost(
        &self,
        asset_id: &str,
        origin: Option<&str>,
    ) -> bool {
        let trimmed = origin.unwrap_or("").trim();
        if bot_runtime_origin_is_pair_build(trimmed) {
            return true;
        }
        if trimmed != "RECONCILE" {
            return false;
        }
        let slot = self._maker_order_slot_get(&MakerOrderKey::buy(asset_id));
        if bot_runtime_origin_is_pair_build(&slot.origin) {
            return true;
        }
        if slot
            .replace_target
            .as_ref()
            .map(|target| bot_runtime_origin_is_pair_build(&target.origin))
            .unwrap_or(false)
        {
            return true;
        }
        false
    }
    /// Implements note observed fill for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_observed_fill(
        &self,
        asset_id: &str,
        filled: f64,
        is_maker: bool,
        side: &str,
        order_id: Option<&str>,
        origin: Option<&str>,
    ) {
        if self.exec_mode != "BOT" {
            return;
        }
        if filled <= 1e-9 {
            return;
        }
        let side_u = side.trim().to_ascii_uppercase();
        if !matches!(side_u.as_str(), "BUY" | "SELL") {
            return;
        }
        let is_wallet_asset = self.yes_asset.as_deref() == Some(asset_id)
            || self.no_asset.as_deref() == Some(asset_id);
        if !is_wallet_asset || self.start_ts <= 0 {
            return;
        }
        let cfg = *self._bot_runtime_cfg();
        let t_into_s = now_ts_f64() - self.start_ts as f64;
        if t_into_s < 0.0 {
            return;
        }
        let clears_pair_build_repost =
            self._bot_runtime_fill_clears_pair_build_repost(asset_id, origin);
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            if side_u == "BUY" && clears_pair_build_repost {
                if self.yes_asset.as_deref() == Some(asset_id) {
                    st.pair_build_yes_repost = BotRuntimePairBuildSideRepostState::default();
                } else if self.no_asset.as_deref() == Some(asset_id) {
                    st.pair_build_no_repost = BotRuntimePairBuildSideRepostState::default();
                }
                // Do NOT clear pair_build_last_paired_growth_{yes,no}_bid here.
                // The lighter-side repair cap needs the anchor prices from the
                // paired growth submit that produced this fill.  They are
                // naturally overwritten by the next paired growth submit.
            }
            bot_runtime_note_fill_event(&mut st, t_into_s, filled, is_maker, &cfg);
        }
        let order_ctx = order_id.and_then(|oid| self._get_order_execution_context(oid));
        if order_ctx
            .as_ref()
            .and_then(|ctx| ctx.get("bot_runtime_below_snapshot_optional"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            self._bot_runtime_note_below_snapshot_optional_fill(filled);
        }
    }
    /// Implements log await second fill hold for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_log_await_second_fill_hold(
        &self,
        reason: &str,
        missing_side: Option<OutcomeSide>,
        t_into_s: f64,
        time_since_first_side_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
    ) {
        if !self._bot_runtime_await_second_fill_hold_changed(reason) {
            return;
        }
        self._bot_runtime_note_startup_completion_blocked();
        let pair_id = self.pair_identity().pair_id;
        let _ = self._audit_insert_runtime_event(
            "risk_block",
            None,
            None,
            None,
            missing_side.map(|side| side.as_str()),
            Some(reason),
            json!({
                "scope": "await_second_fill",
                "pair_id": pair_id,
                "reason_code": reason,
                "missing_side": missing_side.map(|side| side.as_str()),
                "t_into_seconds": t_into_s.max(0.0),
                "time_since_first_side_seconds": time_since_first_side_s.max(0.0),
                "q_yes": q_yes.max(0.0),
                "q_no": q_no.max(0.0),
                "total_cost": total_cost.max(0.0),
            }),
        );
        let yes_bid = self
            .yes_asset
            .as_deref()
            .and_then(|asset| self._best_bid_ask(asset).map(|(bid, _)| bid))
            .unwrap_or(0.0);
        let no_bid = self
            .no_asset
            .as_deref()
            .and_then(|asset| self._best_bid_ask(asset).map(|(bid, _)| bid))
            .unwrap_or(0.0);
        let (favorite_side, underdog_side) =
            bot_runtime_favorite_underdog_sides(yes_bid, no_bid, self.cfg.tick.max(0.0001));
        let residual_side = bot_runtime_residual_side(q_yes, q_no);
        let residual_kind = bot_runtime_residual_kind(favorite_side, underdog_side, residual_side);
        self.logger.info(&format!(
            "[BOT][AWAIT_SECOND_FILL] pair_id={} hold reason={} missing_side={} favorite_side={} underdog_side={} residual_side={} residual_kind={} one_side_exception_kind={} t_into={:.1}s since_first_side={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
            pair_id,
            reason,
            missing_side
                .map(|side| side.as_str().to_string())
                .unwrap_or_else(|| "NA".to_string()),
            favorite_side
                .map(|side| side.as_str().to_string())
                .unwrap_or_else(|| "NA".to_string()),
            underdog_side
                .map(|side| side.as_str().to_string())
                .unwrap_or_else(|| "NA".to_string()),
            residual_side
                .map(|side| side.as_str().to_string())
                .unwrap_or_else(|| "NA".to_string()),
            residual_kind.as_str(),
            BotRuntimeOneSideExceptionKind::SecondSideCompletion.as_str(),
            t_into_s.max(0.0),
            time_since_first_side_s.max(0.0),
            q_yes,
            q_no,
            total_cost.max(0.0)
        ));
    }

    /// Marks the AwaitSecondFill 15-second target as missed.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_mark_await_second_fill_target_missed(
        &self,
        started_ts: f64,
        t_into_s: f64,
        missing_side: OutcomeSide,
        q_yes: f64,
        q_no: f64,
    ) {
        let pair_id = self.pair_identity().pair_id;
        let should_log = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.await_second_fill_target_missed_ts > 0.0 {
                    false
                } else {
                    st.await_second_fill_target_missed_ts =
                        started_ts + bot_runtime_await_second_fill_target_seconds();
                    st.second_side_by_15s = false;
                    true
                }
            })
            .unwrap_or(false);
        if should_log {
            self.logger.warning(&format!(
                "[BOT][AWAIT_SECOND_FILL] pair_id={} target_missed missing_side={} t_into={:.1}s since_first_side={:.1}s qYES={:.2} qNO={:.2}",
                pair_id,
                missing_side.as_str(),
                t_into_s.max(0.0),
                bot_runtime_await_second_fill_target_seconds(),
                q_yes,
                q_no
            ));
        }
    }

    /// Marks AwaitSecondFill as permanently hard-paused for the rest of the market.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_mark_await_second_fill_hard_paused(
        &self,
        now: f64,
        reason: &str,
        missing_side: Option<OutcomeSide>,
        t_into_s: f64,
        time_since_first_side_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
    ) {
        let pair_id = self.pair_identity().pair_id;
        let should_log = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.await_second_fill_hard_paused {
                    false
                } else {
                    st.await_second_fill_hard_paused = true;
                    st.await_second_fill_rescue_attempted_ts =
                        st.await_second_fill_rescue_attempted_ts.max(now);
                    true
                }
            })
            .unwrap_or(false);
        if should_log {
            self.logger.warning(&format!(
                "[BOT][AWAIT_SECOND_FILL] pair_id={} hard_pause reason={} missing_side={} t_into={:.1}s since_first_side={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
                pair_id,
                reason,
                missing_side
                    .map(|side| side.as_str().to_string())
                    .unwrap_or_else(|| "NA".to_string()),
                t_into_s.max(0.0),
                time_since_first_side_s.max(0.0),
                q_yes,
                q_no,
                total_cost.max(0.0)
            ));
        }
    }

    /// Implements note await second fill progress for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_await_second_fill_progress(
        &self,
        now: f64,
        t_into_s: f64,
        q_yes: f64,
        q_no: f64,
        cost_yes: f64,
        cost_no: f64,
    ) {
        let phase = bot_runtime_phase_from_t_into_s(t_into_s, self._bot_runtime_cfg());
        let pair_snapshot =
            self._pair_snapshot_from_inputs(phase, t_into_s, q_yes, q_no, cost_yes, cost_no);
        let pair_id = pair_snapshot.identity.pair_id;
        let first_fill_ts = self
            .bot_runtime_state
            .lock()
            .map(|st| st.open_both_first_fill_ts)
            .unwrap_or(0.0);
        if first_fill_ts <= 0.0 {
            return;
        }
        let yes_live = has_side_participation(q_yes, cost_yes);
        let no_live = has_side_participation(q_no, cost_no);
        let missing_side =
            bot_runtime_await_second_fill_missing_side(q_yes, cost_yes, q_no, cost_no);
        if yes_live && no_live {
            let _ = self._bot_runtime_cancel_await_second_fill_orders(
                None,
                "bot_runtime_await_second_fill_restored",
            );
            let transition = self
                .bot_runtime_state
                .lock()
                .map(|mut st| {
                    if st.await_second_fill_second_fill_ts > 0.0 {
                        None
                    } else {
                        let started_ts = if st.await_second_fill_started_ts > 0.0 {
                            st.await_second_fill_started_ts
                        } else {
                            first_fill_ts
                        };
                        let second_fill_ts = if st.await_second_fill_started_ts > 0.0 {
                            now
                        } else {
                            first_fill_ts
                        };
                        let latency_s = (second_fill_ts - started_ts).max(0.0);
                        st.await_second_fill_started_ts = started_ts;
                        st.await_second_fill_second_fill_ts = second_fill_ts;
                        st.await_second_fill_missing_side = None;
                        st.await_second_fill_last_hold_reason.clear();
                        st.first_fill_to_second_fill_ms = latency_s * 1000.0;
                        st.second_side_by_15s =
                            latency_s <= bot_runtime_await_second_fill_target_seconds() + 1e-9;
                        st.second_side_by_30s =
                            latency_s <= bot_runtime_await_second_fill_deadline_seconds() + 1e-9;
                        Some((latency_s, st.second_side_by_15s, st.second_side_by_30s))
                    }
                })
                .unwrap_or(None);
            if let Some((latency_s, by_15s, by_30s)) = transition {
                self.logger.info(&format!(
                    "[BOT][AWAIT_SECOND_FILL] pair_id={} success reason=missing_side_restored t_into={:.1}s since_first_side={:.1}s second_side_by_15s={} second_side_by_30s={} qYES={:.2} qNO={:.2}",
                    pair_id,
                    t_into_s.max(0.0),
                    latency_s,
                    by_15s,
                    by_30s,
                    q_yes,
                    q_no
                ));
            }
            return;
        }
        let Some(missing_side) = missing_side else {
            return;
        };
        let should_log_start = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.await_second_fill_started_ts > 0.0 {
                    st.await_second_fill_missing_side = Some(missing_side);
                    false
                } else {
                    st.await_second_fill_started_ts = first_fill_ts;
                    st.await_second_fill_missing_side = Some(missing_side);
                    true
                }
            })
            .unwrap_or(false);
        let started_ts = self
            .bot_runtime_state
            .lock()
            .map(|st| st.await_second_fill_started_ts)
            .unwrap_or(first_fill_ts);
        let time_since_first_side_s = (now - started_ts).max(0.0);
        if should_log_start {
            self.logger.info(&format!(
                "[BOT][AWAIT_SECOND_FILL] pair_id={} start reason=startup_asymmetry missing_side={} t_into={:.1}s since_first_side={:.1}s qYES={:.2} qNO={:.2}",
                pair_id,
                missing_side.as_str(),
                t_into_s.max(0.0),
                time_since_first_side_s,
                q_yes,
                q_no
            ));
        }
        if time_since_first_side_s >= bot_runtime_await_second_fill_target_seconds() - 1e-9 {
            self._bot_runtime_mark_await_second_fill_target_missed(
                started_ts,
                t_into_s,
                missing_side,
                q_yes,
                q_no,
            );
        }
    }
    /// Implements seed completion handler for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_await_second_fill_handler(
        &self,
        now: f64,
        t_into_s: f64,
        _total_cost: f64,
        _q_yes: f64,
        _q_no: f64,
        _cost_yes: f64,
        _cost_no: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
        let (_total_cost, q_yes, q_no, cost_yes, cost_no) = self
            .state
            .lock()
            .map(|s| (s.c_yes + s.c_no, s.q_yes, s.q_no, s.c_yes, s.c_no))
            .unwrap_or((_total_cost, _q_yes, _q_no, _cost_yes, _cost_no));
        let pair_snapshot = self._pair_snapshot_from_inputs(
            bot_runtime_phase_from_t_into_s(t_into_s, cfg),
            t_into_s,
            q_yes,
            q_no,
            cost_yes,
            cost_no,
        );
        let pair_id = pair_snapshot.identity.pair_id.clone();
        let total_cost = pair_snapshot.total_cost;
        let q_yes = pair_snapshot.position.q_yes;
        let q_no = pair_snapshot.position.q_no;
        let cost_yes = pair_snapshot.position.c_yes;
        let cost_no = pair_snapshot.position.c_no;
        let first_fill_ts = self
            .bot_runtime_state
            .lock()
            .map(|st| st.open_both_first_fill_ts)
            .unwrap_or(0.0);
        let (started_ts, rescue_used, hard_paused, latched_missing_side) = self
            .bot_runtime_state
            .lock()
            .map(|st| {
                (
                    if st.await_second_fill_started_ts > 0.0 {
                        st.await_second_fill_started_ts
                    } else {
                        first_fill_ts
                    },
                    st.await_second_fill_rescue_used,
                    st.await_second_fill_hard_paused,
                    st.await_second_fill_missing_side,
                )
            })
            .unwrap_or((first_fill_ts, false, false, None));
        let time_since_first_side_s = if started_ts > 0.0 {
            (now - started_ts).max(0.0)
        } else {
            0.0
        };
        let missing_side = latched_missing_side
            .or_else(|| bot_runtime_await_second_fill_missing_side(q_yes, cost_yes, q_no, cost_no));
        let Some(missing_side) = missing_side else {
            let _ = self._bot_runtime_cancel_await_second_fill_orders(
                None,
                "bot_runtime_await_second_fill_restored",
            );
            self._bot_runtime_clear_await_second_fill_hold();
            return;
        };
        let (yes_asset, no_asset) = match (&self.yes_asset, &self.no_asset) {
            (Some(yes_asset), Some(no_asset)) => (yes_asset.as_str(), no_asset.as_str()),
            _ => {
                self._bot_runtime_log_await_second_fill_hold(
                    "missing_assets",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };
        let _ = self._bot_runtime_cancel_order_family(
            "BOT_OPEN_BOTH",
            Some(missing_side),
            "bot_runtime_await_second_fill_filled_side_cancel",
        );
        let _ = self._bot_runtime_cancel_pair_build_orders(
            Some(missing_side),
            "bot_runtime_await_second_fill_filled_side_cancel",
        );
        let _ = self._bot_runtime_cancel_taper_orders(
            Some(missing_side),
            "bot_runtime_await_second_fill_filled_side_cancel",
        );
        let _ = self._bot_runtime_cancel_await_second_fill_orders(
            Some(missing_side),
            "bot_runtime_await_second_fill_filled_side_cancel",
        );
        if hard_paused {
            let _ = self._bot_runtime_cancel_order_family(
                "BOT_OPEN_BOTH",
                None,
                "bot_runtime_await_second_fill_hard_paused",
            );
            let _ = self._bot_runtime_cancel_pair_build_orders(
                None,
                "bot_runtime_await_second_fill_hard_paused",
            );
            let _ = self._bot_runtime_cancel_taper_orders(
                None,
                "bot_runtime_await_second_fill_hard_paused",
            );
            let _ = self._bot_runtime_cancel_await_second_fill_orders(
                None,
                "bot_runtime_await_second_fill_hard_paused",
            );
            self._bot_runtime_log_await_second_fill_hold(
                "hard_paused",
                Some(missing_side),
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if !self.market_connected.load(Ordering::SeqCst) {
            self._bot_runtime_log_await_second_fill_hold(
                "market_ws_disconnected",
                Some(missing_side),
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if self._bot_runtime_user_ws_required() && !self.user_connected.load(Ordering::SeqCst) {
            self._bot_runtime_log_await_second_fill_hold(
                "user_ws_disconnected",
                Some(missing_side),
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let missing_asset = match missing_side {
            OutcomeSide::Yes => yes_asset,
            OutcomeSide::No => no_asset,
        };
        let missing_label = match missing_side {
            OutcomeSide::Yes => "YES",
            OutcomeSide::No => "NO",
        };
        let stale_s = self.cfg.market_data_stale_add_block_seconds.max(1) as f64;
        let deadline_elapsed =
            time_since_first_side_s >= bot_runtime_await_second_fill_deadline_seconds() - 1e-9;
        let missing_quote = self._best_bid_ask_with_ts(missing_asset);
        let (missing_quote_ready, missing_quote_reason) = if deadline_elapsed {
            bot_runtime_ask_snapshot_status(missing_label, missing_quote, now, stale_s)
        } else {
            bot_runtime_quote_snapshot_status(missing_label, missing_quote, now, stale_s)
        };
        if !missing_quote_ready {
            self._bot_runtime_log_await_second_fill_hold(
                &format!("missing_side_quote_unready:{missing_quote_reason}"),
                Some(missing_side),
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let total_usable_budget =
            usable_budget_after_reserve(self.cfg.max_total_cost, self.cfg.reserve_usd);
        let (missing_bid, missing_ask) = missing_quote
            .map(|(bid, ask, _)| (bid, ask))
            .unwrap_or((0.0, 0.0));
        let size_int = if deadline_elapsed {
            None
        } else {
            if missing_bid <= 0.0 {
                self._bot_runtime_log_await_second_fill_hold(
                    "zero_missing_bid",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            let Some(size_int) = bot_runtime_await_second_fill_repair_size(
                cfg.clip_ladder[0],
                self.cfg.min_shares,
                missing_bid,
                total_usable_budget,
                total_cost,
            ) else {
                self._bot_runtime_log_await_second_fill_hold(
                    "budget_too_small",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            };
            Some(size_int)
        };
        let key = MakerOrderKey::buy(missing_asset);
        let prev_slot = self._maker_order_slot_get(&key);
        let live_order_timeout_s = bot_runtime_await_second_fill_live_order_timeout_seconds(
            self.cfg.stale_seconds.max(1) as f64,
        );
        if time_since_first_side_s >= bot_runtime_await_second_fill_target_seconds() - 1e-9 {
            self._bot_runtime_mark_await_second_fill_target_missed(
                started_ts,
                t_into_s,
                missing_side,
                q_yes,
                q_no,
            );
        }
        if time_since_first_side_s >= bot_runtime_await_second_fill_deadline_seconds() - 1e-9 {
            if maker_slot_family_live(&prev_slot, "BOT_AWAIT_SECOND_FILL")
                && prev_slot.state != MakerOrderLifecycle::CancelPending
            {
                let _ = self._maker_order_request_cancel_unthrottled(
                    &key,
                    "bot_runtime_await_second_fill_deadline_cancel",
                );
                self._bot_runtime_log_await_second_fill_hold(
                    "deadline_cancel_missing_side_maker",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            if prev_slot.order_id.is_some()
                && matches!(
                    prev_slot.state,
                    MakerOrderLifecycle::Working
                        | MakerOrderLifecycle::SubmitPending
                        | MakerOrderLifecycle::CancelPending
                )
            {
                if prev_slot.state != MakerOrderLifecycle::CancelPending {
                    let _ = self._maker_order_request_cancel_unthrottled(
                        &key,
                        "bot_runtime_await_second_fill_deadline_handoff",
                    );
                }
                self._bot_runtime_log_await_second_fill_hold(
                    &format!(
                        "awaiting_missing_side_deadline_handoff:{}:{}",
                        prev_slot.origin,
                        maker_order_lifecycle_label(prev_slot.state)
                    ),
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            if rescue_used {
                self._bot_runtime_mark_await_second_fill_hard_paused(
                    now,
                    "still_one_sided_after_rescue",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                self._bot_runtime_log_await_second_fill_hold(
                    "hard_paused",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            let rescue_budget_clip = bot_runtime_await_second_fill_repair_size(
                cfg.clip_ladder[0],
                self.cfg.min_shares,
                missing_ask,
                total_usable_budget,
                total_cost,
            );
            let visible_ask_size =
                self._cum_depth(missing_asset, "asks", missing_ask, Some(1), Some(stale_s));
            let rescue_size = rescue_budget_clip.and_then(|repair_clip| {
                bot_runtime_await_second_fill_rescue_size(
                    repair_clip,
                    bot_runtime_await_second_fill_unmatched_size(q_yes, q_no),
                    visible_ask_size,
                    self.cfg.min_shares,
                )
            });
            let marginal_pair_sum = bot_runtime_await_second_fill_marginal_pair_sum(
                missing_side,
                q_yes,
                q_no,
                cost_yes,
                cost_no,
                missing_ask,
            );
            let Some(rescue_size) = rescue_size else {
                self._bot_runtime_mark_await_second_fill_hard_paused(
                    now,
                    "rescue_clip_unavailable",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                self._bot_runtime_log_await_second_fill_hold(
                    "rescue_clip_unavailable",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            };
            let Some(marginal_pair_sum) = marginal_pair_sum else {
                self._bot_runtime_mark_await_second_fill_hard_paused(
                    now,
                    "rescue_pair_sum_unavailable",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                self._bot_runtime_log_await_second_fill_hold(
                    "rescue_pair_sum_unavailable",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            };
            let (yes_bid_for_residual, no_bid_for_residual) = match missing_side {
                OutcomeSide::Yes => (
                    missing_bid,
                    self._best_bid_ask(no_asset)
                        .map(|(bid, _)| bid)
                        .unwrap_or(0.0),
                ),
                OutcomeSide::No => (
                    self._best_bid_ask(yes_asset)
                        .map(|(bid, _)| bid)
                        .unwrap_or(0.0),
                    missing_bid,
                ),
            };
            let (favorite_side, underdog_side) = bot_runtime_favorite_underdog_sides(
                yes_bid_for_residual,
                no_bid_for_residual,
                self.cfg.tick.max(0.0001),
            );
            let residual_side = bot_runtime_residual_side(q_yes, q_no);
            let projected_residual_side = bot_runtime_projected_residual_side_and_magnitude(
                BotRuntimePairBuildMode::LighterSideFirst,
                Some(missing_side),
                rescue_size as f64,
                q_yes,
                q_no,
            )
            .0;
            let residual_kind =
                bot_runtime_residual_kind(favorite_side, underdog_side, residual_side);
            let increases_underdog_residual = bot_runtime_would_increase_underdog_residual_for_side(
                BotRuntimePairBuildMode::LighterSideFirst,
                Some(missing_side),
                rescue_size as f64,
                q_yes,
                q_no,
                underdog_side,
            );
            if increases_underdog_residual {
                let reason = format!(
                    "underdog_residual_increase_block:{}:{}:{}",
                    missing_side.as_str(),
                    residual_side.map(|side| side.as_str()).unwrap_or("NONE"),
                    underdog_side.map(|side| side.as_str()).unwrap_or("NONE")
                );
                let _ = self._bot_runtime_cancel_bot_orders_on_side(
                    missing_side,
                    "bot_runtime_await_second_fill_residual_hold",
                );
                self._bot_runtime_mark_await_second_fill_hard_paused(
                    now,
                    reason.as_str(),
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                self._bot_runtime_log_await_second_fill_hold(
                    reason.as_str(),
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            if marginal_pair_sum >= 1.0 - 1e-9 {
                self._bot_runtime_mark_await_second_fill_hard_paused(
                    now,
                    "rescue_pair_sum_too_high",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                self._bot_runtime_log_await_second_fill_hold(
                    &format!("rescue_pair_sum_too_high:{marginal_pair_sum:.3}"),
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            if let Err(reason) = self._evaluate_taker_submit_gate(
                "BUY",
                missing_asset,
                rescue_size as f64,
                Some(TakerExceptionReason::AwaitSecondFillRescue),
                TakerCapPolicy::EnforceCap,
            ) {
                self._bot_runtime_mark_await_second_fill_hard_paused(
                    now,
                    reason.as_str(),
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                self._bot_runtime_log_await_second_fill_hold(
                    reason.as_str(),
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            self._set_pending_entry_reason("BOT_AWAIT_SECOND_FILL_RESCUE");
            let oid = self._place_taker_bid_fak(
                missing_asset,
                missing_ask,
                rescue_size as f64,
                Some("FAK"),
                Some(TakerExceptionReason::AwaitSecondFillRescue),
                TakerCapPolicy::EnforceCap,
            );
            if let Some(order_id) = oid.as_deref() {
                let decision_event_id = self._audit_insert_decision_event(
                    "await_second_fill",
                    None,
                    true,
                    "await_second_fill_rescue_submit",
                    Some("BOT_AWAIT_SECOND_FILL_RESCUE"),
                    Some(missing_side.as_str()),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                if let Ok(mut st) = self.bot_runtime_state.lock() {
                    st.await_second_fill_rescue_used = true;
                    st.await_second_fill_rescue_attempted_ts = now;
                    st.second_side_by_30s = false;
                    st.await_second_fill_last_hold_reason.clear();
                }
                if let Some(decision_event_id) = decision_event_id.as_deref() {
                    self._audit_attach_decision_context(
                        order_id,
                        decision_event_id,
                        "await_second_fill_rescue_submit",
                    );
                }
                self._merge_order_execution_context_fields(
                    order_id,
                    &json!({
                        "origin": "BOT_AWAIT_SECOND_FILL_RESCUE",
                        "bot_runtime_await_second_fill_rescue": true,
                        "missing_side": missing_side.as_str(),
                    }),
                );
                self.logger.warning(&format!(
                    "[BOT][AWAIT_SECOND_FILL] pair_id={} rescue_attempted missing_side={} ask={:.3} clip={} visible_ask={:.2} marginal_pair_sum={:.3} favorite_side={} underdog_side={} residual_side={} projected_residual_side={} residual_kind={} one_side_exception_kind={} increases_underdog_residual={} t_into={:.1}s since_first_side={:.1}s",
                    pair_id,
                    missing_side.as_str(),
                    missing_ask,
                    rescue_size,
                    visible_ask_size.max(0.0),
                    marginal_pair_sum,
                    favorite_side
                        .map(|side| side.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    underdog_side
                        .map(|side| side.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    residual_side
                        .map(|side| side.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    projected_residual_side
                        .map(|side| side.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    residual_kind.as_str(),
                    BotRuntimeOneSideExceptionKind::SecondSideCompletion.as_str(),
                    increases_underdog_residual,
                    t_into_s.max(0.0),
                    time_since_first_side_s
                ));
                self._bot_runtime_clear_await_second_fill_hold();
            } else {
                self._bot_runtime_mark_await_second_fill_hard_paused(
                    now,
                    "rescue_submit_failed",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                self._bot_runtime_log_await_second_fill_hold(
                    "rescue_submit_failed",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            return;
        }
        if maker_slot_family_live(&prev_slot, "BOT_AWAIT_SECOND_FILL") {
            let age_s = (now - prev_slot.last_submit_ts).max(0.0);
            if age_s >= live_order_timeout_s
                && prev_slot.state != MakerOrderLifecycle::CancelPending
            {
                if let Err(reason) = self._maker_order_request_refresh_cancel(
                    &key,
                    "bot_runtime_await_second_fill_stale",
                ) {
                    self._bot_runtime_log_await_second_fill_hold(
                        &reason,
                        Some(missing_side),
                        t_into_s,
                        time_since_first_side_s,
                        total_cost,
                        q_yes,
                        q_no,
                    );
                    return;
                }
                self._bot_runtime_log_await_second_fill_hold(
                    "missing_side_live_order_stale_cancel",
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            } else {
                self._bot_runtime_log_await_second_fill_hold(
                    &format!(
                        "awaiting_missing_side_live_order:{}",
                        maker_order_lifecycle_label(prev_slot.state)
                    ),
                    Some(missing_side),
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            return;
        }
        if prev_slot.order_id.is_some()
            && matches!(
                prev_slot.state,
                MakerOrderLifecycle::Working
                    | MakerOrderLifecycle::SubmitPending
                    | MakerOrderLifecycle::CancelPending
            )
        {
            if prev_slot.state != MakerOrderLifecycle::CancelPending {
                let _ = self._maker_order_request_cancel_unthrottled(
                    &key,
                    "bot_runtime_await_second_fill_handoff",
                );
            }
            self._bot_runtime_log_await_second_fill_hold(
                &format!(
                    "awaiting_missing_side_handoff:{}:{}",
                    prev_slot.origin,
                    maker_order_lifecycle_label(prev_slot.state)
                ),
                Some(missing_side),
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let Some(size_int) = size_int else {
            self._bot_runtime_log_await_second_fill_hold(
                "maker_size_unavailable",
                Some(missing_side),
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        // Cap the missing side bid to preserve structural edge (combined VWAP < 0.97).
        // If the filled side has a known VWAP, cap missing_bid so that
        // filled_side_vwap + missing_bid <= 0.97 (3% structural edge target).
        let capped_missing_bid = {
            let (filled_qty, filled_cost) = match missing_side {
                OutcomeSide::Yes => (q_no, cost_no),
                OutcomeSide::No => (q_yes, cost_yes),
            };
            if filled_qty > 1e-9 {
                let filled_vwap = filled_cost / filled_qty;
                let max_missing_price = (0.97 - filled_vwap).max(0.01);
                if missing_bid > max_missing_price {
                    self.logger.info(&format!(
                        "[BOT][AWAIT_SECOND_FILL] pair_id={} capping missing_bid from {:.3} to {:.3} (filled_vwap={:.3} edge_cap=0.97)",
                        pair_id, missing_bid, max_missing_price, filled_vwap,
                    ));
                }
                missing_bid.min(max_missing_price)
            } else {
                missing_bid
            }
        };
        let oid = self._maker_order_upsert_gtc(
            &key,
            capped_missing_bid,
            size_int as f64,
            "BOT_AWAIT_SECOND_FILL",
        );
        if let Some(order_id) = oid.as_deref() {
            let refresh_noop = self._consume_refresh_cadence_noop_marker(order_id);
            let is_new_submit = !refresh_noop
                && (prev_slot.order_id.as_deref() != Some(order_id)
                    || prev_slot.state != MakerOrderLifecycle::Working);
            if is_new_submit {
                if let Some(decision_event_id) = self._audit_insert_decision_event(
                    "await_second_fill",
                    None,
                    true,
                    "await_second_fill_submit",
                    Some("BOT_AWAIT_SECOND_FILL"),
                    Some(missing_side.as_str()),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                ) {
                    self._audit_attach_decision_context(
                        order_id,
                        decision_event_id.as_str(),
                        "await_second_fill_submit",
                    );
                    self._merge_order_execution_context_fields(
                        order_id,
                        &json!({
                            "submit_origin": "BOT_AWAIT_SECOND_FILL",
                            "submit_side": missing_side.as_str(),
                        }),
                    );
                }
                self._bot_runtime_clear_await_second_fill_hold();
                self.logger.info(&format!(
                    "[BOT][AWAIT_SECOND_FILL] pair_id={} submit missing_side={} bid={:.3} clip={} favorite_side={} underdog_side={} residual_side={} residual_kind={} one_side_exception_kind={} t_into={:.1}s since_first_side={:.1}s budget_scope=whole_window maker_first=true",
                    pair_id,
                    missing_side.as_str(),
                    missing_bid,
                    size_int,
                    bot_runtime_favorite_underdog_sides(
                        self._best_bid_ask(yes_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                        self._best_bid_ask(no_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                        self.cfg.tick.max(0.0001),
                    )
                    .0
                    .map(|side| side.as_str().to_string())
                    .unwrap_or_else(|| "NA".to_string()),
                    bot_runtime_favorite_underdog_sides(
                        self._best_bid_ask(yes_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                        self._best_bid_ask(no_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                        self.cfg.tick.max(0.0001),
                    )
                    .1
                    .map(|side| side.as_str().to_string())
                    .unwrap_or_else(|| "NA".to_string()),
                    bot_runtime_residual_side(q_yes, q_no)
                        .map(|side| side.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    bot_runtime_residual_kind(
                        bot_runtime_favorite_underdog_sides(
                            self._best_bid_ask(yes_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                            self._best_bid_ask(no_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                            self.cfg.tick.max(0.0001),
                        )
                        .0,
                        bot_runtime_favorite_underdog_sides(
                            self._best_bid_ask(yes_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                            self._best_bid_ask(no_asset).map(|(bid, _)| bid).unwrap_or(0.0),
                            self.cfg.tick.max(0.0001),
                        )
                        .1,
                        bot_runtime_residual_side(q_yes, q_no),
                    )
                    .as_str(),
                    BotRuntimeOneSideExceptionKind::SecondSideCompletion.as_str(),
                    t_into_s.max(0.0),
                    time_since_first_side_s
                ));
            } else {
                self._bot_runtime_clear_await_second_fill_hold();
            }
        } else {
            self._bot_runtime_log_await_second_fill_hold(
                "no_missing_side_order_live",
                Some(missing_side),
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
        }
    }
}
