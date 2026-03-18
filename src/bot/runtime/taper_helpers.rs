use super::*;

impl MakerHedgeCapBot {
    /// Implements taper hold changed for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_taper_hold_changed(
        &self,
        state_kind: &str,
        reason: &str,
    ) -> bool {
        let combined = format!("{state_kind}:{reason}");
        self.bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.taper_last_hold_reason == combined {
                    false
                } else {
                    st.taper_last_hold_reason = combined;
                    true
                }
            })
            .unwrap_or(true)
    }

    /// Implements clear taper hold for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_clear_taper_hold(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.taper_last_hold_reason.clear();
        }
    }

    /// Implements note taper submit for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_note_taper_submit(
        &self,
        t_into_s: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            if t_into_s >= cfg.late_balance_only_start_seconds {
                st.late_new_orders_after_225 += 1;
            }
            if t_into_s >= cfg.late_stop_new_orders_start_seconds {
                st.late_new_orders_after_240 += 1;
            }
        }
    }

    /// Implements log taper state for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_log_taper_state(
        &self,
        state_kind: &str,
        reason: &str,
        taper_mode: BotRuntimeTaperMode,
        decision: Option<BotRuntimePairBuildDecision>,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
    ) {
        if !self._bot_runtime_taper_hold_changed(state_kind, reason) {
            return;
        }
        if state_kind == "hold"
            || reason.starts_with("late_reduce_clips_repair_first_suppress:")
            || reason.starts_with("late_balance_only_suppress:")
            || reason.starts_with("late_floor_tail_priority:")
        {
            self._bot_runtime_note_optional_add_skipped();
        }
        if reason.starts_with("late_floor_tail_priority:") {
            self._bot_runtime_note_floor_tail_blocked();
        }
        if reason.starts_with("imbalance_")
            || reason.starts_with("projected_hard_imbalance_block")
            || reason.starts_with("hard_imbalance_disable")
            || reason.starts_with("repair_does_not_reduce_imbalance")
        {
            if let Ok(mut st) = self.bot_runtime_state.lock() {
                st.imbalance_last_hold_reason = reason.to_string();
            }
        }
        let mode = decision
            .map(|value| value.mode.as_str().to_string())
            .unwrap_or_else(|| "NA".to_string());
        let side = decision
            .and_then(|value| value.side.map(|side| side.as_str().to_string()))
            .unwrap_or_else(|| "NA".to_string());
        let clip = decision.map(|value| value.clip).unwrap_or(0);
        let clip_bucket = decision
            .map(|value| value.clip_bucket.to_string())
            .unwrap_or_else(|| "NA".to_string());
        let selected_rung = decision
            .map(|value| value.selected_rung.as_str().to_string())
            .unwrap_or_else(|| "NA".to_string());
        let requested_rung = decision
            .map(|value| value.requested_rung.as_str().to_string())
            .unwrap_or_else(|| "NA".to_string());
        let requested_large_clip = decision
            .map(|value| value.requested_large_clip)
            .unwrap_or(false);
        let cpp_hint = decision
            .map(|value| value.cpp_hint.as_str().to_string())
            .unwrap_or_else(|| "NA".to_string());
        let price_zone = decision
            .map(|value| value.price_zone.as_str().to_string())
            .unwrap_or_else(|| "NA".to_string());
        let marginal_cost_mode = decision
            .map(|value| value.marginal_cost_mode.as_str().to_string())
            .unwrap_or_else(|| "NA".to_string());
        let effective_marginal_pair_cost = decision
            .map(|value| value.effective_marginal_pair_cost)
            .unwrap_or(f64::INFINITY);
        let residual_unit_cost = decision
            .and_then(|value| value.residual_unit_cost)
            .map(|value| format!("{value:.3}"))
            .unwrap_or_else(|| "NA".to_string());
        let lagging_side_quote = decision
            .and_then(|value| value.lagging_side_quote)
            .map(|value| format!("{value:.3}"))
            .unwrap_or_else(|| "NA".to_string());
        let heavier_side = decision
            .and_then(|value| {
                if value.marginal_cost_mode == BotRuntimeMarginalCostMode::RebalanceAdd {
                    value.side.map(|side| side.opposite().as_str().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "NA".to_string());
        let unmatched_fraction = decision
            .map(|value| value.current_unmatched_fraction)
            .unwrap_or_else(|| unmatched_fraction(q_yes, q_no));
        let projected_unmatched_fraction = decision
            .map(|value| value.projected_unmatched_fraction)
            .unwrap_or(unmatched_fraction);
        let match_ratio_value = decision
            .map(|value| value.match_ratio)
            .unwrap_or_else(|| match_ratio(q_yes, q_no));
        let imbalance_state = decision
            .map(|value| value.imbalance_state.as_str().to_string())
            .unwrap_or_else(|| {
                self.bot_runtime_state
                    .lock()
                    .map(|st| st.imbalance_state.as_str().to_string())
                    .unwrap_or_else(|_| BotRuntimeImbalanceState::Normal.as_str().to_string())
            });
        let current_base = decision.map(|value| value.current_base).unwrap_or(0.0);
        let green_conditions_met = decision
            .map(|value| value.green_conditions_met)
            .unwrap_or(false);
        let green_both_sides_filled = decision
            .map(|value| value.green_both_sides_filled)
            .unwrap_or(false);
        let green_price_ok = decision.map(|value| value.green_price_ok).unwrap_or(false);
        let green_imbalance_ok = decision
            .map(|value| value.green_imbalance_ok)
            .unwrap_or(false);
        let green_time_ok = decision.map(|value| value.green_time_ok).unwrap_or(false);
        let green_budget_ok = decision.map(|value| value.green_budget_ok).unwrap_or(false);
        let favorite_side = decision
            .and_then(|value| value.favorite_side.map(|side| side.as_str().to_string()))
            .unwrap_or_else(|| "NA".to_string());
        let underdog_side = decision
            .and_then(|value| value.underdog_side.map(|side| side.as_str().to_string()))
            .unwrap_or_else(|| "NA".to_string());
        let residual_side = decision
            .and_then(|value| value.residual_side.map(|side| side.as_str().to_string()))
            .unwrap_or_else(|| "NA".to_string());
        let projected_residual_side = decision
            .and_then(|value| {
                value
                    .projected_residual_side
                    .map(|side| side.as_str().to_string())
            })
            .unwrap_or_else(|| "NA".to_string());
        let residual_kind = decision
            .map(|value| value.residual_kind.as_str().to_string())
            .unwrap_or_else(|| BotRuntimeResidualKind::None.as_str().to_string());
        let increases_underdog_residual = decision
            .map(|value| value.increases_underdog_residual)
            .unwrap_or(false);
        let one_side_exception_kind = decision
            .map(|value| value.one_side_exception_kind.as_str().to_string())
            .unwrap_or_else(|| BotRuntimeOneSideExceptionKind::None.as_str().to_string());
        let pair_id = self.pair_identity().pair_id;
        self.logger.info(&format!(
            "[BOT][TAPER] pair_id={} {} reason={} taper_mode={} mode={} side={} clip={} clip_bucket={} selected_rung={} requested_rung={} requested_large_clip={} cpp_hint={} price_zone={} marginal_cost_mode={} effective_marginal_pair_cost={:.3} residual_unit_cost={} lagging_side_quote={} heavier_side={} favorite_side={} underdog_side={} residual_side={} projected_residual_side={} residual_kind={} one_side_exception_kind={} increases_underdog_residual={} current_base={:.2} green_conditions_met={} green_both_sides_filled={} green_price_ok={} green_imbalance_ok={} green_time_ok={} green_budget_ok={} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2} unmatched_fraction={:.3} projected_unmatched_fraction={:.3} match_ratio={:.3} imbalance_state={}",
            pair_id,
            state_kind,
            reason,
            taper_mode.as_str(),
            mode,
            side,
            clip,
            clip_bucket,
            selected_rung,
            requested_rung,
            requested_large_clip,
            cpp_hint,
            price_zone,
            marginal_cost_mode,
            effective_marginal_pair_cost,
            residual_unit_cost,
            lagging_side_quote,
            heavier_side,
            favorite_side,
            underdog_side,
            residual_side,
            projected_residual_side,
            residual_kind,
            one_side_exception_kind,
            increases_underdog_residual,
            current_base,
            green_conditions_met,
            green_both_sides_filled,
            green_price_ok,
            green_imbalance_ok,
            green_time_ok,
            green_budget_ok,
            t_into_s.max(0.0),
            q_yes,
            q_no,
            total_cost.max(0.0),
            unmatched_fraction,
            projected_unmatched_fraction,
            match_ratio_value,
            imbalance_state
        ));
    }

    /// Implements cancel taper orders for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_cancel_taper_orders(
        &self,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        self._bot_runtime_cancel_order_family("BOT_TAPER", active_side, reason)
    }

    /// Implements cancel paired-growth taper orders while preserving lighter-side repair orders
    /// for the BOT runtime.
    /// This helper coordinates BOT phase routing, runtime state transitions, or metrics for the
    /// active market.

    pub(in crate::bot) fn _bot_runtime_cancel_taper_growth_orders(
        &self,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        self._bot_runtime_cancel_order_family_excluding(
            "BOT_TAPER",
            "BOT_TAPER_LIGHTER",
            active_side,
            reason,
        )
    }
}
