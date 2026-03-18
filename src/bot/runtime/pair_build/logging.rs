use super::super::*;

impl MakerHedgeCapBot {
    /// Implements log pair build state for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_log_pair_build_state(
        &self,
        state_kind: &str,
        reason: &str,
        decision: Option<BotRuntimePairBuildDecision>,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
    ) {
        if !self._bot_runtime_pair_build_hold_changed(state_kind, reason) {
            return;
        }
        if state_kind == "hold" {
            self._bot_runtime_note_optional_add_skipped();
            if reason.starts_with("repair_reserve_block:") {
                self._bot_runtime_note_repair_reserve_blocked();
            }
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
        let pair_sum = decision.map(|value| value.pair_sum).unwrap_or(0.0);
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
        let reduces_imbalance = decision
            .map(|value| value.reduces_imbalance)
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
        let green_conditions_met = decision
            .map(|value| value.green_conditions_met)
            .unwrap_or(false);
        let pair_coverage = decision.map(|value| value.pair_coverage).unwrap_or(0.0);
        let skew_ratio = decision.map(|value| value.skew_ratio).unwrap_or(0.0);
        let current_base = decision.map(|value| value.current_base).unwrap_or(0.0);
        let inventory_vwap_sum = decision
            .map(|value| value.inventory_vwap_sum)
            .unwrap_or(f64::INFINITY);
        let pair_id = self.pair_identity().pair_id;
        self.logger.info(&format!(
            "[BOT][PAIR_BUILD] pair_id={} {} reason={} mode={} side={} clip={} clip_bucket={} selected_rung={} requested_rung={} requested_large_clip={} cpp_hint={} price_zone={} marginal_cost_mode={} effective_marginal_pair_cost={:.3} pair_sum={:.3} residual_unit_cost={} lagging_side_quote={} heavier_side={} current_base={:.2} green_conditions_met={} green_both_sides_filled={} green_price_ok={} green_imbalance_ok={} green_time_ok={} green_budget_ok={} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2} unmatched_fraction={:.3} projected_unmatched_fraction={:.3} match_ratio={:.3} imbalance_state={} reduces_imbalance={} pair_coverage={:.3} skew={:.3} inventory_vwap_sum={:.3}",
            pair_id,
            state_kind,
            reason,
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
            pair_sum,
            residual_unit_cost,
            lagging_side_quote,
            heavier_side,
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
            imbalance_state,
            reduces_imbalance,
            pair_coverage,
            skew_ratio,
            inventory_vwap_sum
        ));
    }

    /// Implements log lighter repair ownership for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_log_lighter_repair_ownership(
        &self,
        context: &str,
        active_side: OutcomeSide,
        inactive_side: OutcomeSide,
        age_s: f64,
        policy: &BotRuntimeLighterOppositeOrderPolicy,
    ) {
        self.logger.info(&format!(
            "[BOT][{context}] ownership={} active_side={} inactive_side={} reason={} age_s={:.1} remaining={:.2} compatible_remaining={:.2} live_price={:.3} target_price={:.3}",
            if policy.preserve {
                "preserve_opposite_side"
            } else {
                "cancel_opposite_side"
            },
            active_side.as_str(),
            inactive_side.as_str(),
            policy.reason,
            age_s.max(0.0),
            policy.remaining,
            policy.compatible_remaining,
            policy.live_price,
            policy.target_price
        ));
    }
}
