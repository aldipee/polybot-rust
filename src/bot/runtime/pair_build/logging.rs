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
        let cpp_hint = decision
            .map(|value| value.cpp_hint.as_str().to_string())
            .unwrap_or_else(|| "NA".to_string());
        let pair_sum = decision.map(|value| value.pair_sum).unwrap_or(0.0);
        let pair_coverage = decision.map(|value| value.pair_coverage).unwrap_or(0.0);
        let skew_ratio = decision.map(|value| value.skew_ratio).unwrap_or(0.0);
        let inventory_vwap_sum = decision
            .map(|value| value.inventory_vwap_sum)
            .unwrap_or(f64::INFINITY);
        let pair_id = self.pair_identity().pair_id;
        self.logger.info(&format!(
            "[BOT][PAIR_BUILD] pair_id={} {} reason={} mode={} side={} clip={} clip_bucket={} cpp_hint={} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2} pair_sum={:.3} pair_coverage={:.3} skew={:.3} inventory_vwap_sum={:.3}",
            pair_id,
            state_kind,
            reason,
            mode,
            side,
            clip,
            clip_bucket,
            cpp_hint,
            t_into_s.max(0.0),
            q_yes,
            q_no,
            total_cost.max(0.0),
            pair_sum,
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
