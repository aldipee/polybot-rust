use super::*;
impl MakerHedgeCapBot {
    /// Implements taper handler for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_taper_handler(
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
        let taper_mode = bot_runtime_taper_mode(t_into_s, cfg);
        if q_yes <= 1e-9 && q_no <= 1e-9 {
            let cancelled = self._bot_runtime_cancel_pair_build_orders(
                None,
                "bot_runtime_taper_no_inventory",
            ) || self._bot_runtime_cancel_taper_orders(None, "bot_runtime_taper_no_inventory");
            self._bot_runtime_log_taper_state(
                if cancelled { "rest" } else { "hold" },
                "late_taper_no_inventory",
                taper_mode,
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let (yes_asset, no_asset) = match (&self.yes_asset, &self.no_asset) {
            (Some(yes_asset), Some(no_asset)) => (yes_asset.as_str(), no_asset.as_str()),
            _ => {
                self._bot_runtime_log_taper_state(
                    "hold",
                    "missing_assets",
                    taper_mode,
                    None,
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };
        if !self.market_connected.load(Ordering::SeqCst) {
            self._bot_runtime_log_taper_state(
                "hold",
                "market_ws_disconnected",
                taper_mode,
                None,
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
            self._bot_runtime_log_taper_state(
                "hold",
                "user_ws_disconnected",
                taper_mode,
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let (quotes_ready, quote_reason) = self._bot_runtime_quote_input_status();
        if !quotes_ready {
            self._bot_runtime_log_taper_state(
                "hold",
                &format!("quote_inputs_unready:{quote_reason}"),
                taper_mode,
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let Some((y_bid, y_ask)) = self._best_bid_ask(yes_asset) else {
            self._bot_runtime_log_taper_state(
                "hold",
                "missing_yes_quotes",
                taper_mode,
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        let Some((n_bid, n_ask)) = self._best_bid_ask(no_asset) else {
            self._bot_runtime_log_taper_state(
                "hold",
                "missing_no_quotes",
                taper_mode,
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        let yes_key = MakerOrderKey::buy(yes_asset);
        let no_key = MakerOrderKey::buy(no_asset);
        let yes_slot = self._maker_order_slot_get(&yes_key);
        let no_slot = self._maker_order_slot_get(&no_key);
        for (side, key, slot) in [
            (OutcomeSide::Yes, &yes_key, &yes_slot),
            (OutcomeSide::No, &no_key, &no_slot),
        ] {
            if slot.order_id.is_none()
                || !slot.origin.starts_with("BOT_")
                || slot.origin.starts_with("BOT_TAPER")
                || !matches!(
                    slot.state,
                    MakerOrderLifecycle::Working
                        | MakerOrderLifecycle::SubmitPending
                        | MakerOrderLifecycle::CancelPending
                )
            {
                continue;
            }
            if slot.state != MakerOrderLifecycle::CancelPending {
                let _ = self._maker_order_request_cancel(key, "bot_runtime_taper_order_handoff");
            }
            self._bot_runtime_log_taper_state(
                "rest",
                &format!(
                    "awaiting_handoff:{}:{}:{}",
                    side.as_str(),
                    slot.origin,
                    maker_order_lifecycle_label(slot.state)
                ),
                taper_mode,
                None,
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
            self._bot_runtime_log_taper_state(
                "hold",
                "phase_budget_exhausted",
                taper_mode,
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let decision = match bot_runtime_pair_build_decision(
            q_yes,
            q_no,
            cost_yes,
            cost_no,
            y_bid,
            y_ask,
            n_bid,
            n_ask,
            total_cost + budget_snapshot.remaining_to_max_cost,
            total_cost,
            self.cfg.min_shares,
            self.min_maker_notional,
            cfg,
            false,
        ) {
            Ok(decision) => bot_runtime_taper_maintenance_decision(decision, self.cfg.min_shares),
            Err(reason) => {
                self._bot_runtime_log_taper_state(
                    "hold",
                    &reason,
                    taper_mode,
                    None,
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };
        if decision.clip <= 0 {
            self._bot_runtime_log_taper_state(
                "hold",
                "no_legal_taper_clip",
                taper_mode,
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let late_action_policy = match bot_runtime_taper_late_action_policy(
            taper_mode,
            &decision,
            q_yes,
            q_no,
            total_cost,
            y_bid,
            n_bid,
        ) {
            Some(policy) => policy,
            None => {
                self._bot_runtime_log_taper_state(
                    "hold",
                    "late_floor_tail_policy_unavailable",
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };
        if let Some(reason) = late_action_policy.hold_reason.as_deref() {
            let cancelled = self._bot_runtime_cancel_taper_orders(None, reason)
                || self._bot_runtime_cancel_pair_build_orders(None, reason);
            self._bot_runtime_log_taper_state(
                if cancelled { "rest" } else { "hold" },
                reason,
                taper_mode,
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let asymmetry_timeout_s = self.cfg.stale_seconds.max(1) as f64;
        if decision.mode == BotRuntimePairBuildMode::LighterSideFirst {
            let active_side = decision.side.unwrap_or(OutcomeSide::Yes);
            let inactive_side = match active_side {
                OutcomeSide::Yes => OutcomeSide::No,
                OutcomeSide::No => OutcomeSide::Yes,
            };
            let inactive_asset = match inactive_side {
                OutcomeSide::Yes => yes_asset,
                OutcomeSide::No => no_asset,
            };
            let inactive_bid = match inactive_side {
                OutcomeSide::Yes => y_bid,
                OutcomeSide::No => n_bid,
            };
            let inactive_key = MakerOrderKey::buy(inactive_asset);
            let inactive_slot = self._maker_order_slot_get(&inactive_key);
            if maker_slot_family_live(&inactive_slot, "BOT_TAPER") {
                let inactive_age_s = (now - inactive_slot.last_submit_ts).max(0.0);
                let ownership_policy = bot_runtime_lighter_repair_opposite_order_policy(
                    &decision,
                    &inactive_slot,
                    inactive_bid,
                    self.cfg.tick.max(0.0001),
                );
                if ownership_policy
                    .as_ref()
                    .map(|policy| policy.preserve)
                    .unwrap_or(false)
                {
                    if let Some(policy) = ownership_policy.as_ref() {
                        self._bot_runtime_log_lighter_repair_ownership(
                            "TAPER",
                            active_side,
                            inactive_side,
                            inactive_age_s,
                            policy,
                        );
                    }
                } else {
                    if let Some(policy) = ownership_policy.as_ref() {
                        self._bot_runtime_log_lighter_repair_ownership(
                            "TAPER",
                            active_side,
                            inactive_side,
                            inactive_age_s,
                            policy,
                        );
                    }
                    if inactive_slot.state != MakerOrderLifecycle::CancelPending {
                        let _ = self._maker_order_request_cancel(
                            &inactive_key,
                            "bot_runtime_taper_lighter_side_owner",
                        );
                    }
                    self._bot_runtime_log_taper_state(
                        "rest",
                        &format!("lighter_side_handoff:{}", active_side.as_str()),
                        taper_mode,
                        Some(decision),
                        t_into_s,
                        total_cost,
                        q_yes,
                        q_no,
                    );
                    return;
                }
            } else if self._bot_runtime_cancel_taper_orders(
                Some(active_side),
                "bot_runtime_taper_lighter_side_owner",
            ) {
                self._bot_runtime_log_taper_state(
                    "rest",
                    &format!("lighter_side_handoff:{}", active_side.as_str()),
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
            let active_asset = match active_side {
                OutcomeSide::Yes => yes_asset,
                OutcomeSide::No => no_asset,
            };
            let active_bid = match active_side {
                OutcomeSide::Yes => y_bid,
                OutcomeSide::No => n_bid,
            };
            let key = MakerOrderKey::buy(active_asset);
            let prev_slot = self._maker_order_slot_get(&key);
            if maker_slot_family_live(&prev_slot, "BOT_TAPER") {
                let age_s = (now - prev_slot.last_submit_ts).max(0.0);
                if age_s >= asymmetry_timeout_s
                    && prev_slot.state != MakerOrderLifecycle::CancelPending
                {
                    let _ = self._maker_order_request_cancel(
                        &key,
                        "bot_runtime_taper_lighter_side_stale",
                    );
                    self._bot_runtime_log_taper_state(
                        "rest",
                        &format!("lighter_side_live_order_stale_cancel:{:.1}", age_s),
                        taper_mode,
                        Some(decision),
                        t_into_s,
                        total_cost,
                        q_yes,
                        q_no,
                    );
                } else {
                    self._bot_runtime_log_taper_state(
                        "rest",
                        &format!(
                            "awaiting_lighter_side_live_order:{}:{:.1}",
                            maker_order_lifecycle_label(prev_slot.state),
                            age_s
                        ),
                        taper_mode,
                        Some(decision),
                        t_into_s,
                        total_cost,
                        q_yes,
                        q_no,
                    );
                }
                return;
            }
            if decision.cpp_hint != BotRuntimePairBuildCppHint::Normal {
                self._bot_runtime_log_taper_state(
                    "suppress",
                    &format!("cpp_hint_{}", decision.cpp_hint.as_str()),
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            self._set_pending_entry_reason("BOT_TAPER");
            let oid = self._maker_order_upsert_gtc(
                &key,
                active_bid,
                decision.clip as f64,
                "BOT_TAPER_LIGHTER",
            );
            if let Some(order_id) = oid.as_deref() {
                let is_new_submit = prev_slot.order_id.as_deref() != Some(order_id)
                    || prev_slot.state != MakerOrderLifecycle::Working;
                self._bot_runtime_clear_taper_hold();
                if is_new_submit {
                    self._bot_runtime_note_taper_submit(t_into_s, cfg);
                    self.logger.info(&format!(
                        "[BOT][TAPER] submit taper_mode={} mode={} side={} clip={} clip_bucket={} cpp_hint={} current_tail={:.2} projected_tail={:.2} current_floor={:+.2} projected_floor={:+.2} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
                        taper_mode.as_str(),
                        decision.mode.as_str(),
                        active_side.as_str(),
                        decision.clip,
                        decision.clip_bucket,
                        decision.cpp_hint.as_str(),
                        late_action_policy.current_tail_size,
                        late_action_policy.projected_tail_size,
                        late_action_policy.current_floor,
                        late_action_policy.projected_floor,
                        t_into_s.max(0.0),
                        q_yes,
                        q_no,
                        total_cost.max(0.0)
                    ));
                }
            } else {
                self._bot_runtime_log_taper_state(
                    "hold",
                    "no_lighter_side_order_live",
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            return;
        }
        let yes_live = maker_slot_family_live(&yes_slot, "BOT_TAPER");
        let no_live = maker_slot_family_live(&no_slot, "BOT_TAPER");
        if yes_live && no_live {
            let yes_age_s = (now - yes_slot.last_submit_ts).max(0.0);
            let no_age_s = (now - no_slot.last_submit_ts).max(0.0);
            let max_age_s = yes_age_s.max(no_age_s);
            if max_age_s >= asymmetry_timeout_s
                && yes_slot.state != MakerOrderLifecycle::CancelPending
                && no_slot.state != MakerOrderLifecycle::CancelPending
            {
                let _ = self._maker_order_request_cancel(&yes_key, "bot_runtime_taper_stale_both_live");
                let _ = self._maker_order_request_cancel(&no_key, "bot_runtime_taper_stale_both_live");
                self._bot_runtime_log_taper_state(
                    "rest",
                    &format!("maintenance_live_orders_stale_cancel:{yes_age_s:.1}:{no_age_s:.1}"),
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            } else {
                self._bot_runtime_log_taper_state(
                    "rest",
                    &format!(
                        "awaiting_taper_live_orders:{}:{}:{yes_age_s:.1}:{no_age_s:.1}",
                        maker_order_lifecycle_label(yes_slot.state),
                        maker_order_lifecycle_label(no_slot.state)
                    ),
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            return;
        }
        if let Some(asymmetry) = self._maker_pair_order_asymmetry(
            now,
            yes_asset,
            no_asset,
            "BOT_TAPER",
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
                    "bot_runtime_taper_asymmetric_submit_stale",
                );
                self._bot_runtime_log_taper_state(
                    "rest",
                    &format!(
                        "asymmetric_submit_stale_cancel:{}:{:.1}",
                        asymmetry.live_side.as_str(),
                        asymmetry.age_s
                    ),
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            } else {
                self._bot_runtime_log_taper_state(
                    "rest",
                    &format!(
                        "awaiting_asymmetric_submit_resolution:{}:{}:{:.1}",
                        asymmetry.live_side.as_str(),
                        maker_order_lifecycle_label(asymmetry.state),
                        asymmetry.age_s
                    ),
                    taper_mode,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            return;
        }
        if let Some(reason) = bot_runtime_taper_paired_growth_submin_notional_reason(
            &decision,
            y_bid,
            n_bid,
            self.min_maker_notional,
        ) {
            self._bot_runtime_log_taper_state(
                "hold",
                &reason,
                taper_mode,
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if decision.cpp_hint != BotRuntimePairBuildCppHint::Normal {
            self._bot_runtime_log_taper_state(
                "suppress",
                &format!("cpp_hint_{}", decision.cpp_hint.as_str()),
                taper_mode,
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let prev_yes_slot = yes_slot;
        let prev_no_slot = no_slot;
        self._set_pending_entry_reason("BOT_TAPER");
        let submit_started = now_ts_f64();
        let (y_oid, n_oid) = self._maker_submit_pair_orders(
            decision.clip,
            y_bid,
            n_bid,
            "GTC",
            Some(true),
            "BOT_TAPER",
        );
        let submit_elapsed_ms = ((now_ts_f64() - submit_started).max(0.0)) * 1000.0;
        if let Some(asymmetry) = self._maker_pair_order_asymmetry(
            now_ts_f64(),
            yes_asset,
            no_asset,
            "BOT_TAPER",
        ) {
            self._bot_runtime_log_taper_state(
                "rest",
                &format!(
                    "asymmetric_submit:{}:{:.0}ms",
                    asymmetry.live_side.as_str(),
                    submit_elapsed_ms
                ),
                taper_mode,
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let yes_live_oid = self._maker_order_slot_get(&yes_key).order_id.or(y_oid);
        let no_live_oid = self._maker_order_slot_get(&no_key).order_id.or(n_oid);
        if yes_live_oid.is_none() && no_live_oid.is_none() {
            self._bot_runtime_log_taper_state(
                "hold",
                "no_taper_orders_live",
                taper_mode,
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let yes_new =
            maker_pair_submit_leg_is_new(yes_live_oid.as_deref(), &prev_yes_slot);
        let no_new =
            maker_pair_submit_leg_is_new(no_live_oid.as_deref(), &prev_no_slot);
        self._bot_runtime_clear_taper_hold();
        if yes_new || no_new {
            self._bot_runtime_note_taper_submit(t_into_s, cfg);
            self.logger.info(&format!(
                "[BOT][TAPER] submit taper_mode={} mode={} clip={} clip_bucket={} cpp_hint={} current_tail={:.2} projected_tail={:.2} current_floor={:+.2} projected_floor={:+.2} t_into={:.1}s elapsed_ms={:.0} qYES={:.2} qNO={:.2} total_cost={:.2}",
                taper_mode.as_str(),
                decision.mode.as_str(),
                decision.clip,
                decision.clip_bucket,
                decision.cpp_hint.as_str(),
                late_action_policy.current_tail_size,
                late_action_policy.projected_tail_size,
                late_action_policy.current_floor,
                late_action_policy.projected_floor,
                t_into_s.max(0.0),
                submit_elapsed_ms,
                q_yes,
                q_no,
                total_cost.max(0.0)
            ));
        }
    }
}

