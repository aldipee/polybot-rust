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
        self.logger.info(&format!(
            "[BOT][CFG] mode={} phase_controller={} prearm_lead={:.0}s phase_budgets=open:{:.0}-{:.0}% early:{:.0}-{:.0}% main:{:.0}-{:.0}% late:{:.0}-{:.0}% taper:{:.0}-{:.0}% seed_clip={:.0} repair_clip={:.0} large_clips={:.0}/{:.0} startup_targets=both_by_30s:{:.0}% both_by_60s:{:.0}% taper_start={:.0}s final_quiet={:.0}s buy_only_normal_flow={} tail_caps={}s:{:.1}%/{}s:{:.1}%/late:{:.1}% bad_regime_window={:.0}s bad_regime_expensive_fraction={:.2}",
            self.exec_mode,
            cfg.phase_controller,
            cfg.prearm_lead_seconds,
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
            cfg.seed_clip_small,
            cfg.repair_clip_small,
            cfg.large_clip_ladder[0],
            cfg.large_clip_ladder[1],
            cfg.target_both_sides_by_30s * 100.0,
            cfg.target_both_sides_by_60s * 100.0,
            cfg.taper_start_seconds,
            cfg.final_quiet_seconds,
            cfg.buy_only_normal_flow,
            cfg.tail_cap_mid_start_seconds,
            cfg.tail_cap_early_fraction * 100.0,
            cfg.tail_cap_late_start_seconds,
            cfg.tail_cap_mid_fraction * 100.0,
            cfg.tail_cap_late_fraction * 100.0,
            cfg.bad_regime_window_seconds,
            cfg.bad_regime_expensive_fraction
        ));
    }
    /// Implements quote input status for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_quote_input_status(&self) -> (bool, String) {
        let (Some(yes_asset), Some(no_asset)) = (&self.yes_asset, &self.no_asset) else {
            return (false, "asset_ids_missing".to_string());
        };
        let stale_s = self.cfg.market_data_stale_seconds.max(1) as f64;
        let now = now_ts_f64();
        for (label, asset_id) in [("YES", yes_asset.as_str()), ("NO", no_asset.as_str())] {
            let (ready, reason) =
                bot_runtime_quote_snapshot_status(label, self._best_bid_ask_with_ts(asset_id), now, stale_s);
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
        let stale_s = self.cfg.market_data_stale_seconds.max(1) as f64;
        bot_runtime_startup_pair_quote_status(
            self._best_bid_ask_with_ts(yes_asset),
            self._best_bid_ask_with_ts(no_asset),
            now_ts_f64(),
            stale_s,
        )
    }
    /// Implements prearm status for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_prearm_status(&self, t_into_s: f64) -> BotRuntimePreArmStatus {
        let cfg = *self._bot_runtime_cfg();
        let market_selected =
            !self.market_slug.trim().is_empty() && self.start_ts > 0 && self.expiry_ts > self.start_ts;
        let asset_ids_ready =
            self.condition_id.is_some() && self.yes_asset.is_some() && self.no_asset.is_some();
        let market_ws_ready = self.market_connected.load(Ordering::SeqCst);
        let user_ws_required = env_bool("REQUIRE_USER_WS_CONNECTED", true);
        let user_ws_ready =
            !user_ws_required || self.user_connected.load(Ordering::SeqCst);
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

    pub(in crate::bot) fn _bot_runtime_note_open_both_submit(&self, now: f64) -> (u32, bool) {
        self.bot_runtime_state
            .lock()
            .map(|mut st| {
                st.open_both_last_hold_reason.clear();
                st.open_both_attempt_count = st.open_both_attempt_count.saturating_add(1);
                let first_submit = st.open_both_first_submit_ts <= 0.0;
                if first_submit {
                    st.open_both_first_submit_ts = now;
                }
                (st.open_both_attempt_count, first_submit)
            })
            .unwrap_or((0, false))
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
        let has_fill = has_side_participation(q_yes, cost_yes)
            || has_side_participation(q_no, cost_no);
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
            "[BOT][OPEN_BOTH] first_fill t_into={:.1}s submit_to_fill_ms={:.0} qYES={:.2} qNO={:.2} costYES={:.2} costNO={:.2}",
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
        self.logger.info(&format!(
            "[BOT][OPEN_BOTH] hold reason={} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
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
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
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
        if env_bool("REQUIRE_USER_WS_CONNECTED", true)
            && !self.user_connected.load(Ordering::SeqCst)
        {
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
            cfg.seed_clip_small,
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
        let yes_live =
            maker_slot_family_live(&prev_yes_slot, "BOT_OPEN_BOTH");
        let no_live =
            maker_slot_family_live(&prev_no_slot, "BOT_OPEN_BOTH");
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
        if let Some(asymmetry) = maker_pair_order_asymmetry(
            &prev_yes_slot,
            &prev_no_slot,
            "BOT_OPEN_BOTH",
            now,
        ) {
            let live_asset = match asymmetry.live_side {
                OutcomeSide::Yes => yes_asset,
                OutcomeSide::No => no_asset,
            };
            let live_key = MakerOrderKey::buy(live_asset);
            if asymmetry.state != MakerOrderLifecycle::CancelPending
                && asymmetry.age_s >= asymmetry_timeout_s
            {
                let _ = self._maker_order_request_cancel(
                    &live_key,
                    &format!(
                        "BOT open-both asymmetric submit stale live_side={} age_s={:.1}",
                        asymmetry.live_side.as_str(),
                        asymmetry.age_s
                    ),
                );
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
        self._set_pending_entry_reason("BOT_OPEN_BOTH");
        let submit_started = now_ts_f64();
        let (y_oid, n_oid) = self._maker_submit_pair_orders(
            size_int,
            y_bid,
            n_bid,
            "GTC",
            Some(true),
            "BOT_OPEN_BOTH",
        );
        let submit_elapsed_ms = ((now_ts_f64() - submit_started).max(0.0)) * 1000.0;
        let yes_live = self._maker_order_slot_get(&yes_key).order_id.or(y_oid.clone());
        let no_live = self._maker_order_slot_get(&no_key).order_id.or(n_oid.clone());
        let yes_new =
            maker_pair_submit_leg_is_new(yes_live.as_deref(), &prev_yes_slot);
        let no_new =
            maker_pair_submit_leg_is_new(no_live.as_deref(), &prev_no_slot);
        if yes_new || no_new {
            let (attempt_count, first_submit) =
                self._bot_runtime_note_open_both_submit(now_ts_f64());
            self.logger.info(&format!(
                "[BOT][OPEN_BOTH] submit attempt={} t_into={:.1}s y_bid={:.3} n_bid={:.3} pair_sum={:.3} clip={} post_only=true neutral=true favorite_gating=false elapsed_ms={:.0} first_submit={}",
                attempt_count,
                t_into_s.max(0.0),
                y_bid,
                n_bid,
                pair_sum,
                size_int,
                submit_elapsed_ms,
                first_submit
            ));
        }
        if let Some(asymmetry) = self._maker_pair_order_asymmetry(
            now_ts_f64(),
            yes_asset,
            no_asset,
            "BOT_OPEN_BOTH",
        ) {
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
    /// Implements seed completion hold changed for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_seed_completion_hold_changed(&self, reason: &str) -> bool {
        self.bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.seed_completion_last_hold_reason == reason {
                    false
                } else {
                    st.seed_completion_last_hold_reason = reason.to_string();
                    true
                }
            })
            .unwrap_or(true)
    }
    /// Implements clear seed completion hold for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_clear_seed_completion_hold(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.seed_completion_last_hold_reason.clear();
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
        let is_wallet_asset =
            self.yes_asset.as_deref() == Some(asset_id) || self.no_asset.as_deref() == Some(asset_id);
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
    /// Implements log seed completion hold for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_log_seed_completion_hold(
        &self,
        reason: &str,
        missing_side: OutcomeSide,
        t_into_s: f64,
        time_since_first_side_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
    ) {
        if !self._bot_runtime_seed_completion_hold_changed(reason) {
            return;
        }
        self._bot_runtime_note_startup_completion_blocked();
        self.logger.info(&format!(
            "[BOT][SEED_COMPLETION] hold reason={} missing_side={} t_into={:.1}s since_first_side={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
            reason,
            missing_side.as_str(),
            t_into_s.max(0.0),
            time_since_first_side_s.max(0.0),
            q_yes,
            q_no,
            total_cost.max(0.0)
        ));
    }
    /// Implements note seed completion progress for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_seed_completion_progress(
        &self,
        now: f64,
        t_into_s: f64,
        q_yes: f64,
        q_no: f64,
        cost_yes: f64,
        cost_no: f64,
    ) {
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
        let missing_side = bot_runtime_seed_completion_missing_side(q_yes, cost_yes, q_no, cost_no);
        if yes_live && no_live {
            let _ = self._bot_runtime_cancel_seed_completion_orders(
                None,
                "bot_runtime_seed_completion_restored",
            );
            let should_log = self
                .bot_runtime_state
                .lock()
                .map(|mut st| {
                    if st.seed_completion_both_sides_ts > 0.0 {
                        false
                    } else {
                        st.seed_completion_both_sides_ts = now;
                        st.seed_completion_last_hold_reason.clear();
                        true
                    }
                })
                .unwrap_or(false);
            if should_log {
                let second_side_latency_s = (now - first_fill_ts).max(0.0);
                let both_by_30s = (now - self.start_ts as f64) <= 30.0 + 1e-9;
                let both_by_60s = (now - self.start_ts as f64) <= 60.0 + 1e-9;
                self.logger.info(&format!(
                    "[BOT][SEED_COMPLETION] success reason=missing_side_restored t_into={:.1}s since_first_side={:.1}s both_by_30s={} both_by_60s={} qYES={:.2} qNO={:.2}",
                    t_into_s.max(0.0),
                    second_side_latency_s,
                    both_by_30s,
                    both_by_60s,
                    q_yes,
                    q_no
                ));
            }
            return;
        }
        let Some(missing_side) = missing_side else {
            return;
        };
        let time_since_first_side_s = (now - first_fill_ts).max(0.0);
        let should_log_start = self
            .bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.seed_completion_started_ts > 0.0 {
                    false
                } else {
                    st.seed_completion_started_ts = now;
                    true
                }
            })
            .unwrap_or(false);
        if should_log_start {
            self.logger.info(&format!(
                "[BOT][SEED_COMPLETION] start reason=startup_asymmetry missing_side={} t_into={:.1}s since_first_side={:.1}s qYES={:.2} qNO={:.2}",
                missing_side.as_str(),
                t_into_s.max(0.0),
                time_since_first_side_s,
                q_yes,
                q_no
            ));
        }
        if t_into_s >= 60.0 {
            let should_log_failure = self
                .bot_runtime_state
                .lock()
                .map(|mut st| {
                    if st.seed_completion_failure_logged {
                        false
                    } else {
                        st.seed_completion_failure_logged = true;
                        true
                    }
                })
                .unwrap_or(false);
            if should_log_failure {
                self.logger.info(&format!(
                    "[BOT][SEED_COMPLETION] failure reason=still_one_sided_by_60s missing_side={} t_into={:.1}s since_first_side={:.1}s qYES={:.2} qNO={:.2}",
                    missing_side.as_str(),
                    t_into_s.max(0.0),
                    time_since_first_side_s,
                    q_yes,
                    q_no
                ));
            }
        }
    }
    /// Implements seed completion handler for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_seed_completion_handler(
        &self,
        now: f64,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        cost_yes: f64,
        cost_no: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
        let (total_cost, q_yes, q_no, cost_yes, cost_no) = self
            .state
            .lock()
            .map(|s| (s.c_yes + s.c_no, s.q_yes, s.q_no, s.c_yes, s.c_no))
            .unwrap_or((total_cost, q_yes, q_no, cost_yes, cost_no));
        let Some(missing_side) =
            bot_runtime_seed_completion_missing_side(q_yes, cost_yes, q_no, cost_no)
        else {
            let _ = self._bot_runtime_cancel_seed_completion_orders(
                None,
                "bot_runtime_seed_completion_restored",
            );
            self._bot_runtime_clear_seed_completion_hold();
            return;
        };
        let first_fill_ts = self
            .bot_runtime_state
            .lock()
            .map(|st| st.open_both_first_fill_ts)
            .unwrap_or(0.0);
        let time_since_first_side_s = if first_fill_ts > 0.0 {
            (now_ts_f64() - first_fill_ts).max(0.0)
        } else {
            0.0
        };
        let (yes_asset, no_asset) = match (&self.yes_asset, &self.no_asset) {
            (Some(yes_asset), Some(no_asset)) => (yes_asset.as_str(), no_asset.as_str()),
            _ => {
                self._bot_runtime_log_seed_completion_hold(
                    "missing_assets",
                    missing_side,
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };
        if !self.market_connected.load(Ordering::SeqCst) {
            self._bot_runtime_log_seed_completion_hold(
                "market_ws_disconnected",
                missing_side,
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if env_bool("REQUIRE_USER_WS_CONNECTED", true)
            && !self.user_connected.load(Ordering::SeqCst)
        {
            self._bot_runtime_log_seed_completion_hold(
                "user_ws_disconnected",
                missing_side,
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
        let stale_s = self.cfg.market_data_stale_seconds.max(1) as f64;
        let (missing_quote_ready, missing_quote_reason) = bot_runtime_quote_snapshot_status(
            missing_label,
            self._best_bid_ask_with_ts(missing_asset),
            now_ts_f64(),
            stale_s,
        );
        if !missing_quote_ready {
            self._bot_runtime_log_seed_completion_hold(
                &format!("missing_side_quote_unready:{missing_quote_reason}"),
                missing_side,
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let missing_bid = match missing_side {
            OutcomeSide::Yes => self._best_bid_ask(yes_asset).map(|quote| quote.0),
            OutcomeSide::No => self._best_bid_ask(no_asset).map(|quote| quote.0),
        }
        .unwrap_or(0.0);
        if missing_bid <= 0.0 {
            self._bot_runtime_log_seed_completion_hold(
                "zero_missing_bid",
                missing_side,
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
        let Some(size_int) = bot_runtime_seed_completion_repair_size(
            cfg.repair_clip_small,
            self.cfg.min_shares,
            missing_bid,
            total_usable_budget,
            total_cost,
        ) else {
            self._bot_runtime_log_seed_completion_hold(
                "budget_too_small",
                missing_side,
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        let key = MakerOrderKey::buy(missing_asset);
        let prev_slot = self._maker_order_slot_get(&key);
        let live_order_timeout_s = bot_runtime_seed_completion_live_order_timeout_seconds(
            self.cfg.stale_seconds.max(1) as f64,
        );
        if maker_slot_family_live(&prev_slot, "BOT_SEED_COMPLETION") {
            let age_s = (now - prev_slot.last_submit_ts).max(0.0);
            if age_s >= live_order_timeout_s && prev_slot.state != MakerOrderLifecycle::CancelPending {
                let _ =
                    self._maker_order_request_cancel(&key, "bot_runtime_seed_completion_stale");
                self._bot_runtime_log_seed_completion_hold(
                    "missing_side_live_order_stale_cancel",
                    missing_side,
                    t_into_s,
                    time_since_first_side_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            } else {
                self._bot_runtime_log_seed_completion_hold(
                    &format!(
                        "awaiting_missing_side_live_order:{}",
                        maker_order_lifecycle_label(prev_slot.state)
                    ),
                    missing_side,
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
                let _ =
                    self._maker_order_request_cancel(&key, "bot_runtime_seed_completion_handoff");
            }
            self._bot_runtime_log_seed_completion_hold(
                &format!(
                    "awaiting_missing_side_handoff:{}:{}",
                    prev_slot.origin,
                    maker_order_lifecycle_label(prev_slot.state)
                ),
                missing_side,
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let oid = self._maker_order_upsert_gtc(
            &key,
            missing_bid,
            size_int as f64,
            "BOT_SEED_COMPLETION",
        );
        if let Some(order_id) = oid.as_deref() {
            let is_new_submit = prev_slot.order_id.as_deref() != Some(order_id)
                || prev_slot.state != MakerOrderLifecycle::Working;
            if is_new_submit {
                self._bot_runtime_clear_seed_completion_hold();
                self.logger.info(&format!(
                    "[BOT][SEED_COMPLETION] submit missing_side={} bid={:.3} clip={} t_into={:.1}s since_first_side={:.1}s budget_scope=whole_window gates_bypassed=hard_skew,shape_target,cpp",
                    missing_side.as_str(),
                    missing_bid,
                    size_int,
                    t_into_s.max(0.0),
                    time_since_first_side_s
                ));
            } else {
                self._bot_runtime_clear_seed_completion_hold();
            }
        } else {
            self._bot_runtime_log_seed_completion_hold(
                "no_missing_side_order_live",
                missing_side,
                t_into_s,
                time_since_first_side_s,
                total_cost,
                q_yes,
                q_no,
            );
        }
    }
}

