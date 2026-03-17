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
            if t_into_s >= cfg.taper_start_seconds {
                st.taper_new_orders_after_240 += 1;
            }
            if t_into_s >= (300.0 - cfg.final_quiet_seconds) {
                st.taper_new_orders_after_270 += 1;
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
            || reason == "final_quiet_rest"
            || reason.starts_with("late_repair_first_suppress:")
            || reason.starts_with("late_no_optional_adds_suppress:")
            || reason.starts_with("late_floor_tail_priority:")
        {
            self._bot_runtime_note_optional_add_skipped();
        }
        if reason.starts_with("late_floor_tail_priority:") {
            self._bot_runtime_note_floor_tail_blocked();
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
        self.logger.info(&format!(
            "[BOT][TAPER] {} reason={} taper_mode={} mode={} side={} clip={} clip_bucket={} cpp_hint={} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2}",
            state_kind,
            reason,
            taper_mode.as_str(),
            mode,
            side,
            clip,
            clip_bucket,
            cpp_hint,
            t_into_s.max(0.0),
            q_yes,
            q_no,
            total_cost.max(0.0)
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
}
