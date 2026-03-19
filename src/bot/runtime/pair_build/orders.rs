use super::super::*;
use super::decision::{
    bot_runtime_pair_build_asymmetry_timeout_seconds, bot_runtime_pair_build_broken_asymmetry,
    bot_runtime_pair_build_buy_order_is_economically_invalid,
    bot_runtime_pair_build_pair_orders_are_economically_invalid,
    bot_runtime_pair_build_paired_live_order_timeout_seconds,
    bot_runtime_pair_build_price_moved_meaningfully,
    bot_runtime_pair_build_repost_cooldown_seconds,
};
use super::state::BotRuntimePairBuildMarketContext;

impl MakerHedgeCapBot {
    fn _bot_runtime_side_for_asset_id(&self, asset_id: &str) -> Option<OutcomeSide> {
        let trimmed = asset_id.trim();
        if trimmed.is_empty() {
            return None;
        }
        if self.yes_asset.as_deref() == Some(trimmed) {
            return Some(OutcomeSide::Yes);
        }
        if self.no_asset.as_deref() == Some(trimmed) {
            return Some(OutcomeSide::No);
        }
        None
    }

    fn _bot_runtime_cancel_direct_order_family(
        &self,
        family_prefix: &str,
        exclude_prefix: Option<&str>,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        let tracked_order_ids: HashSet<String> = self
            .maker_order_slots
            .lock()
            .map(|slots| {
                slots
                    .values()
                    .filter_map(|slot| slot.order_id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let targets: Vec<(String, String)> = self
            .order_exec_context
            .lock()
            .map(|ctx_map| {
                ctx_map
                    .iter()
                    .filter_map(|(order_id, ctx)| {
                        let origin = ctx.get("origin").and_then(|value| value.as_str())?.trim();
                        if !origin.starts_with(family_prefix) {
                            return None;
                        }
                        if exclude_prefix
                            .map(|prefix| origin.starts_with(prefix))
                            .unwrap_or(false)
                        {
                            return None;
                        }
                        if ctx
                            .get("direct_cancel_requested")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false)
                        {
                            return None;
                        }
                        if tracked_order_ids.contains(order_id) {
                            return None;
                        }
                        let asset_id = ctx
                            .get("asset_id")
                            .or_else(|| ctx.get("token_id"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        let side = ctx
                            .get("side")
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_ascii_uppercase();
                        if side != "BUY" {
                            return None;
                        }
                        if active_side == self._bot_runtime_side_for_asset_id(asset_id.as_str()) {
                            return None;
                        }
                        if asset_id.is_empty() {
                            return None;
                        }
                        Some((order_id.clone(), asset_id))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if targets.is_empty() {
            return false;
        }
        let mut cancelled: Vec<(String, String)> = Vec::new();
        for (order_id, asset_id) in &targets {
            if self._cancel(order_id) {
                cancelled.push((order_id.clone(), asset_id.clone()));
            }
        }
        if cancelled.is_empty() {
            return true;
        }
        let cancel_ts = now_ts_f64();
        if let Ok(mut ctx_map) = self.order_exec_context.lock() {
            for (order_id, _) in &cancelled {
                if let Some(existing) = ctx_map
                    .get_mut(order_id)
                    .and_then(|value| value.as_object_mut())
                {
                    existing.insert("direct_cancel_requested".to_string(), json!(true));
                    existing.insert("direct_cancel_reason".to_string(), json!(reason));
                    existing.insert("direct_cancel_ts".to_string(), json!(cancel_ts));
                }
            }
        }
        let cancelled_ids: HashSet<String> = cancelled
            .iter()
            .map(|(order_id, _)| order_id.clone())
            .collect();
        if let Ok(mut state) = self.state.lock() {
            let mut removed_any = false;
            state.open_orders.retain(|_, row| {
                let keep = row
                    .order_id
                    .as_ref()
                    .map(|order_id| !cancelled_ids.contains(order_id))
                    .unwrap_or(true);
                if !keep {
                    removed_any = true;
                }
                keep
            });
            if removed_any {
                let _ = self._bot_runtime_save_state_or_dependency_pause(
                    &mut state,
                    "cancel_direct_order_family",
                );
            }
        }
        true
    }

    /// Implements pair build note side cancel for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_note_side_cancel(
        &self,
        side: OutcomeSide,
        price: f64,
        now: f64,
    ) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            let target = match side {
                OutcomeSide::Yes => &mut st.pair_build_yes_repost,
                OutcomeSide::No => &mut st.pair_build_no_repost,
            };
            target.last_cancel_ts = now;
            target.last_cancel_price = price.max(0.0);
        }
    }

    /// Implements pair build note side submit for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_note_side_submit(
        &self,
        side: OutcomeSide,
        price: f64,
        now: f64,
    ) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            let target = match side {
                OutcomeSide::Yes => &mut st.pair_build_yes_repost,
                OutcomeSide::No => &mut st.pair_build_no_repost,
            };
            target.last_submit_ts = now;
            target.last_submit_price = price.max(0.0);
            target.last_cancel_ts = 0.0;
            target.last_cancel_price = 0.0;
        }
    }

    /// Implements pair build repost block reason for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_repost_block_reason(
        &self,
        side: OutcomeSide,
        target_price: f64,
        now: f64,
        tick_size: f64,
        decision: &BotRuntimePairBuildDecision,
    ) -> Option<String> {
        let (repost_state, cooldown_s) = self
            .bot_runtime_state
            .lock()
            .map(|st| {
                let state = match side {
                    OutcomeSide::Yes => st.pair_build_yes_repost.clone(),
                    OutcomeSide::No => st.pair_build_no_repost.clone(),
                };
                (
                    state,
                    bot_runtime_pair_build_repost_cooldown_seconds(
                        self._maker_replace_min_interval_seconds(),
                        decision,
                    ),
                )
            })
            .unwrap_or_default();
        if repost_state.last_cancel_ts > 0.0 {
            let elapsed = (now - repost_state.last_cancel_ts).max(0.0);
            if elapsed < cooldown_s
                && !bot_runtime_pair_build_price_moved_meaningfully(
                    repost_state.last_cancel_price,
                    target_price,
                    tick_size,
                )
            {
                return Some(format!(
                    "repost_hysteresis_cancel:{}:{:.1}",
                    side.as_str(),
                    (cooldown_s - elapsed).max(0.0)
                ));
            }
        }
        let submit_dedup_window_s = self._maker_replace_min_interval_seconds().max(0.25);
        if repost_state.last_submit_ts > 0.0 {
            let elapsed = (now - repost_state.last_submit_ts).max(0.0);
            if elapsed < submit_dedup_window_s
                && !bot_runtime_pair_build_price_moved_meaningfully(
                    repost_state.last_submit_price,
                    target_price,
                    tick_size,
                )
            {
                return Some(format!(
                    "repost_hysteresis_dedup:{}:{elapsed:.1}",
                    side.as_str()
                ));
            }
        }
        None
    }

    /// Implements cancel order family for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_cancel_order_family(
        &self,
        family_prefix: &str,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        let mut touched = false;
        for side in [OutcomeSide::Yes, OutcomeSide::No] {
            if active_side == Some(side) {
                continue;
            }
            let Some(asset_id) = (match side {
                OutcomeSide::Yes => self.yes_asset.as_deref(),
                OutcomeSide::No => self.no_asset.as_deref(),
            }) else {
                continue;
            };
            let key = MakerOrderKey::buy(asset_id);
            let slot = self._maker_order_slot_get(&key);
            if !slot.origin.starts_with(family_prefix) || slot.order_id.is_none() {
                continue;
            }
            if matches!(
                slot.state,
                MakerOrderLifecycle::Working
                    | MakerOrderLifecycle::SubmitPending
                    | MakerOrderLifecycle::CancelPending
            ) {
                touched = true;
                if slot.state != MakerOrderLifecycle::CancelPending {
                    let _ = self._maker_order_request_cancel(&key, reason);
                }
            }
        }
        let cancelled_direct =
            self._bot_runtime_cancel_direct_order_family(family_prefix, None, active_side, reason);
        touched || cancelled_direct
    }

    /// Implements cancel order family on a single side for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_cancel_order_family_on_side(
        &self,
        family_prefix: &str,
        side: OutcomeSide,
        reason: &str,
    ) -> bool {
        self._bot_runtime_cancel_order_family(family_prefix, Some(side.opposite()), reason)
    }

    /// Implements cancel order family excluding a narrower preserve prefix for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_cancel_order_family_excluding(
        &self,
        family_prefix: &str,
        exclude_prefix: &str,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        let mut touched = false;
        for side in [OutcomeSide::Yes, OutcomeSide::No] {
            if active_side == Some(side) {
                continue;
            }
            let Some(asset_id) = (match side {
                OutcomeSide::Yes => self.yes_asset.as_deref(),
                OutcomeSide::No => self.no_asset.as_deref(),
            }) else {
                continue;
            };
            let key = MakerOrderKey::buy(asset_id);
            let slot = self._maker_order_slot_get(&key);
            if !slot.origin.starts_with(family_prefix)
                || slot.origin.starts_with(exclude_prefix)
                || slot.order_id.is_none()
            {
                continue;
            }
            if matches!(
                slot.state,
                MakerOrderLifecycle::Working
                    | MakerOrderLifecycle::SubmitPending
                    | MakerOrderLifecycle::CancelPending
            ) {
                touched = true;
                if slot.state != MakerOrderLifecycle::CancelPending {
                    let _ = self._maker_order_request_cancel(&key, reason);
                }
            }
        }
        let cancelled_direct = self._bot_runtime_cancel_direct_order_family(
            family_prefix,
            Some(exclude_prefix),
            active_side,
            reason,
        );
        touched || cancelled_direct
    }

    /// Implements cancel pair build orders for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_cancel_pair_build_orders(
        &self,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        self._bot_runtime_cancel_order_family("BOT_PAIR_BUILD", active_side, reason)
    }

    /// Implements cancel paired-growth pair-build orders while preserving lighter-side repair
    /// orders for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_cancel_pair_build_growth_orders(
        &self,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        self._bot_runtime_cancel_order_family_excluding(
            "BOT_PAIR_BUILD",
            "BOT_PAIR_BUILD_LIGHTER",
            active_side,
            reason,
        )
    }

    /// Implements cancel await second fill orders for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_cancel_await_second_fill_orders(
        &self,
        active_side: Option<OutcomeSide>,
        reason: &str,
    ) -> bool {
        self._bot_runtime_cancel_order_family("BOT_AWAIT_SECOND_FILL", active_side, reason)
    }

    /// Implements cancel BOT-owned working orders on one side for the BOT runtime.
    /// This helper supports residual-direction cleanup while preserving the opposite side.

    pub(in crate::bot) fn _bot_runtime_cancel_bot_orders_on_side(
        &self,
        side: OutcomeSide,
        reason: &str,
    ) -> bool {
        let cancelled_open_both =
            self._bot_runtime_cancel_order_family_on_side("BOT_OPEN_BOTH", side, reason);
        let cancelled_await_second_fill =
            self._bot_runtime_cancel_order_family_on_side("BOT_AWAIT_SECOND_FILL", side, reason);
        let cancelled_pair_build =
            self._bot_runtime_cancel_order_family_on_side("BOT_PAIR_BUILD", side, reason);
        let cancelled_taper =
            self._bot_runtime_cancel_order_family_on_side("BOT_TAPER", side, reason);
        cancelled_open_both
            || cancelled_await_second_fill
            || cancelled_pair_build
            || cancelled_taper
    }

    /// Implements pair build await second fill handoff for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_await_second_fill_handoff(
        &self,
        now: f64,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        yes_slot: &MakerOrderSlot,
        no_slot: &MakerOrderSlot,
    ) -> bool {
        for (side, slot) in [(OutcomeSide::Yes, yes_slot), (OutcomeSide::No, no_slot)] {
            if !maker_slot_family_live(slot, "BOT_AWAIT_SECOND_FILL") {
                continue;
            }
            let age_s = (now - slot.last_submit_ts).max(0.0);
            let _ = self._bot_runtime_cancel_await_second_fill_orders(
                None,
                "bot_runtime_pair_build_await_second_fill_handoff",
            );
            self._bot_runtime_log_pair_build_state(
                "rest",
                &format!(
                    "awaiting_await_second_fill_handoff:{}:{}:{:.1}",
                    side.as_str(),
                    maker_order_lifecycle_label(slot.state),
                    age_s
                ),
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return true;
        }
        false
    }

    /// Implements pair build foreign order handoff for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_foreign_order_handoff(
        &self,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        decision: BotRuntimePairBuildDecision,
        context: &BotRuntimePairBuildMarketContext,
    ) -> bool {
        for (side, key, slot) in [
            (OutcomeSide::Yes, &context.yes_key, &context.yes_slot),
            (OutcomeSide::No, &context.no_key, &context.no_slot),
        ] {
            if slot.order_id.is_none()
                || !slot.origin.starts_with("BOT_")
                || slot.origin.starts_with("BOT_PAIR_BUILD")
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
                let _ =
                    self._maker_order_request_cancel(key, "bot_runtime_pair_build_order_handoff");
            }
            self._bot_runtime_log_pair_build_state(
                "rest",
                &format!(
                    "awaiting_handoff:{}:{}:{}",
                    side.as_str(),
                    slot.origin,
                    maker_order_lifecycle_label(slot.state)
                ),
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return true;
        }
        false
    }

    /// Implements pair build handle live paired orders for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_handle_live_paired_orders(
        &self,
        now: f64,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        decision: BotRuntimePairBuildDecision,
        context: &BotRuntimePairBuildMarketContext,
    ) -> bool {
        let yes_live = maker_slot_family_live(&context.yes_slot, "BOT_PAIR_BUILD");
        let no_live = maker_slot_family_live(&context.no_slot, "BOT_PAIR_BUILD");
        if !(yes_live && no_live) {
            return false;
        }

        let paired_live_timeout_s = bot_runtime_pair_build_paired_live_order_timeout_seconds(
            self.cfg.stale_seconds as f64,
            &decision,
        );
        let price_tick = self.cfg.tick.max(0.0001);
        let yes_age_s = (now - context.yes_slot.last_submit_ts).max(0.0);
        let no_age_s = (now - context.no_slot.last_submit_ts).max(0.0);
        let max_age_s = yes_age_s.max(no_age_s);
        let economically_invalid = bot_runtime_pair_build_pair_orders_are_economically_invalid(
            context.yes_slot.price,
            context.no_slot.price,
            context.y_bid,
            context.n_bid,
            price_tick,
        );

        if max_age_s >= paired_live_timeout_s
            && economically_invalid
            && context.yes_slot.state != MakerOrderLifecycle::CancelPending
            && context.no_slot.state != MakerOrderLifecycle::CancelPending
        {
            let _ = self._maker_order_request_cancel(
                &context.yes_key,
                "bot_runtime_pair_build_invalid_both_live",
            );
            self._bot_runtime_pair_build_note_side_cancel(
                OutcomeSide::Yes,
                context.yes_slot.price,
                now,
            );
            let _ = self._maker_order_request_cancel(
                &context.no_key,
                "bot_runtime_pair_build_invalid_both_live",
            );
            self._bot_runtime_pair_build_note_side_cancel(
                OutcomeSide::No,
                context.no_slot.price,
                now,
            );
            self._bot_runtime_log_pair_build_state(
                "rest",
                &format!("paired_growth_live_orders_invalid_cancel:{yes_age_s:.1}:{no_age_s:.1}"),
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
        } else {
            self._bot_runtime_log_pair_build_state(
                "rest",
                &format!(
                    "awaiting_pair_build_live_orders:{}:{}:{yes_age_s:.1}:{no_age_s:.1}",
                    maker_order_lifecycle_label(context.yes_slot.state),
                    maker_order_lifecycle_label(context.no_slot.state)
                ),
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
        }
        true
    }

    /// Implements pair build handle asymmetry for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_handle_asymmetry(
        &self,
        now: f64,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        decision: BotRuntimePairBuildDecision,
        context: &BotRuntimePairBuildMarketContext,
    ) -> bool {
        let Some(asymmetry) = self._maker_pair_order_asymmetry(
            now,
            &context.yes_asset,
            &context.no_asset,
            "BOT_PAIR_BUILD",
        ) else {
            return false;
        };

        let reject_cooldown = self._maker_submit_reject_cooldown_seconds();
        let max_reject_cooldown =
            env_float("MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS", 60.0).max(reject_cooldown);
        let broken_submit = bot_runtime_pair_build_broken_asymmetry(
            asymmetry.live_side,
            &context.yes_slot,
            &context.no_slot,
            now,
            reject_cooldown,
            max_reject_cooldown,
        );
        let (live_key, live_price, live_target_price) = match asymmetry.live_side {
            OutcomeSide::Yes => (&context.yes_key, context.yes_slot.price, context.y_bid),
            OutcomeSide::No => (&context.no_key, context.no_slot.price, context.n_bid),
        };
        let economically_invalid = bot_runtime_pair_build_buy_order_is_economically_invalid(
            live_price,
            live_target_price,
            self.cfg.tick.max(0.0001),
        );
        let asymmetry_timeout_s = bot_runtime_pair_build_asymmetry_timeout_seconds(
            self.cfg.stale_seconds as f64,
            &decision,
            broken_submit,
        );

        if asymmetry.state != MakerOrderLifecycle::CancelPending
            && (economically_invalid || broken_submit)
            && asymmetry.age_s >= asymmetry_timeout_s
        {
            let _ = self._maker_order_request_cancel(
                live_key,
                if broken_submit {
                    "bot_runtime_pair_build_asymmetric_submit_broken"
                } else {
                    "bot_runtime_pair_build_asymmetric_submit_invalid"
                },
            );
            self._bot_runtime_pair_build_note_side_cancel(asymmetry.live_side, live_price, now);
            self._bot_runtime_log_pair_build_state(
                "rest",
                &format!(
                    "{}:{}:{:.1}",
                    if broken_submit {
                        "asymmetric_submit_broken_cancel"
                    } else {
                        "asymmetric_submit_invalid_cancel"
                    },
                    asymmetry.live_side.as_str(),
                    asymmetry.age_s
                ),
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
        } else {
            self._bot_runtime_log_pair_build_state(
                "rest",
                &format!(
                    "awaiting_asymmetric_submit_resolution:{}:{}:{:.1}",
                    asymmetry.live_side.as_str(),
                    maker_order_lifecycle_label(asymmetry.state),
                    asymmetry.age_s
                ),
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
        }

        true
    }
}
