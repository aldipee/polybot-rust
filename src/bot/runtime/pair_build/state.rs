use super::super::*;

#[derive(Debug, Clone)]
pub(in crate::bot) struct BotRuntimePairBuildMarketContext {
    pub(in crate::bot) yes_asset: String,
    pub(in crate::bot) no_asset: String,
    pub(in crate::bot) yes_key: MakerOrderKey,
    pub(in crate::bot) no_key: MakerOrderKey,
    pub(in crate::bot) yes_slot: MakerOrderSlot,
    pub(in crate::bot) no_slot: MakerOrderSlot,
    pub(in crate::bot) y_bid: f64,
    pub(in crate::bot) y_ask: f64,
    pub(in crate::bot) n_bid: f64,
    pub(in crate::bot) n_ask: f64,
}

#[derive(Debug, Clone)]
pub(in crate::bot) struct BotRuntimePairBuildPlan {
    pub(in crate::bot) decision: BotRuntimePairBuildDecision,
    pub(in crate::bot) budget_snapshot: BotRuntimeBudgetSnapshot,
    pub(in crate::bot) lighter_repair_policy: Option<BotRuntimeLighterRepairPolicy>,
    pub(in crate::bot) repair_reserve_policy: Option<BotRuntimeRepairReservePolicy>,
    pub(in crate::bot) optional_growth_policy: Option<BotRuntimePairedGrowthPolicy>,
    pub(in crate::bot) optional_buy_policy: Option<BotRuntimeOptionalBuyPolicy>,
    pub(in crate::bot) paired_cost_observation: Option<(f64, BotRuntimePairedCostBand)>,
    pub(in crate::bot) bad_regime_shutdown: (bool, f64, u32, u32),
}

impl MakerHedgeCapBot {
    /// Implements note optional add skipped for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_optional_add_skipped(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.skipped_optional_add_count = st.skipped_optional_add_count.saturating_add(1);
        }
    }

    /// Implements note repair reserve blocked for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_repair_reserve_blocked(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.repair_reserve_blocked_count = st.repair_reserve_blocked_count.saturating_add(1);
        }
    }

    /// Implements note floor tail blocked for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_floor_tail_blocked(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.floor_tail_blocked_count = st.floor_tail_blocked_count.saturating_add(1);
        }
    }

    /// Implements note startup completion blocked for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_startup_completion_blocked(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.startup_completion_blocked_count =
                st.startup_completion_blocked_count.saturating_add(1);
        }
    }

    /// Implements note paired cost band observation for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_paired_cost_band_observation(
        &self,
        band: BotRuntimePairedCostBand,
        t_into_s: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            let idx = bot_runtime_paired_cost_band_index(band);
            st.paired_cost_band_observations[idx] =
                st.paired_cost_band_observations[idx].saturating_add(1);
            if t_into_s <= cfg.bad_regime_window_seconds + 1e-9 {
                st.bad_regime_early_observations =
                    st.bad_regime_early_observations.saturating_add(1);
                if matches!(
                    band,
                    BotRuntimePairedCostBand::StopAdd | BotRuntimePairedCostBand::Danger
                ) {
                    st.bad_regime_expensive_observations =
                        st.bad_regime_expensive_observations.saturating_add(1);
                }
            }
            if !st.bad_regime_shutdown
                && t_into_s >= cfg.bad_regime_window_seconds
                && st.bad_regime_early_observations >= 12
            {
                let expensive_ratio = st.bad_regime_expensive_observations as f64
                    / st.bad_regime_early_observations.max(1) as f64;
                if expensive_ratio + 1e-9 >= cfg.bad_regime_expensive_fraction {
                    st.bad_regime_shutdown = true;
                }
            }
        }
    }

    /// Implements bad regime shutdown status for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_bad_regime_shutdown_status(&self) -> (bool, f64, u32, u32) {
        self.bot_runtime_state
            .lock()
            .map(|st| {
                let ratio = if st.bad_regime_early_observations > 0 {
                    st.bad_regime_expensive_observations as f64
                        / st.bad_regime_early_observations as f64
                } else {
                    0.0
                };
                (
                    st.bad_regime_shutdown,
                    ratio,
                    st.bad_regime_expensive_observations,
                    st.bad_regime_early_observations,
                )
            })
            .unwrap_or((false, 0.0, 0, 0))
    }

    /// Implements note below snapshot optional submit for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_below_snapshot_optional_submit(&self, size: f64) {
        if size <= 1e-9 {
            return;
        }
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.below_snapshot_optional_submit_count =
                st.below_snapshot_optional_submit_count.saturating_add(1);
            st.below_snapshot_optional_submit_shares += size.max(0.0);
        }
    }

    /// Implements note below snapshot optional fill for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_below_snapshot_optional_fill(&self, filled: f64) {
        if filled <= 1e-9 {
            return;
        }
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.below_snapshot_optional_fill_count =
                st.below_snapshot_optional_fill_count.saturating_add(1);
            st.below_snapshot_optional_fill_shares += filled.max(0.0);
        }
    }

    /// Implements pair build hold changed for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_hold_changed(
        &self,
        state_kind: &str,
        reason: &str,
    ) -> bool {
        let combined = format!("{state_kind}:{reason}");
        self.bot_runtime_state
            .lock()
            .map(|mut st| {
                if st.pair_build_last_hold_reason == combined {
                    false
                } else {
                    st.pair_build_last_hold_reason = combined;
                    true
                }
            })
            .unwrap_or(true)
    }

    /// Implements clear pair build hold for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_clear_pair_build_hold(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.pair_build_last_hold_reason.clear();
        }
    }

    /// Implements last optional growth submit timestamp for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_last_optional_growth_submit_ts(&self) -> f64 {
        self.bot_runtime_state
            .lock()
            .map(|st| st.pair_build_last_optional_growth_submit_ts)
            .unwrap_or(0.0)
    }

    /// Implements note pair build submit for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_note_pair_build_submit(
        &self,
        now: f64,
        decision: &BotRuntimePairBuildDecision,
        band: Option<BotRuntimePairedCostBand>,
    ) {
        if decision.mode != BotRuntimePairBuildMode::PairedGrowth {
            return;
        }
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.pair_build_last_optional_growth_submit_ts = now;
            if let Some(band) = band {
                let idx = bot_runtime_paired_cost_band_index(band);
                st.paired_size_delta_by_state[idx] += decision.clip.max(0) as f64;
            }
        }
    }
}
