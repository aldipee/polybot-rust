use super::*;

#[derive(Debug, Clone)]
pub(in crate::bot) enum MakerDirectRefreshDecision {
    Initial,
    Started(OutcomeSide),
    Blocked {
        existing_order_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
struct MakerPairGrossPreview {
    requested_gross_usd: f64,
    replace_order_ids: Vec<String>,
}

fn maker_refresh_family(origin: &str) -> Option<&'static str> {
    let trimmed = origin.trim();
    if trimmed.starts_with("BOT_OPEN_BOTH") {
        Some("BOT_OPEN_BOTH")
    } else if trimmed.starts_with("BOT_AWAIT_SECOND_FILL") {
        Some("BOT_AWAIT_SECOND_FILL")
    } else if trimmed.starts_with("BOT_PAIR_BUILD") {
        Some("BOT_PAIR_BUILD")
    } else if trimmed.starts_with("BOT_TAPER") {
        Some("BOT_TAPER")
    } else {
        None
    }
}

fn maker_refresh_families_match(left: &str, right: &str) -> bool {
    match (maker_refresh_family(left), maker_refresh_family(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

impl MakerHedgeCapBot {
    fn _order_is_known_taker_buy(&self, order_id: &str, asset_id: &str) -> bool {
        let order_id = order_id.trim();
        let asset_id = asset_id.trim();
        if order_id.is_empty() || asset_id.is_empty() {
            return false;
        }
        self._get_order_execution_context(order_id)
            .as_ref()
            .and_then(|ctx| ctx.get("liquidity_intent").and_then(|value| value.as_str()))
            .map(|value| value.eq_ignore_ascii_case("taker_exception"))
            .unwrap_or(false)
            || self
                .state
                .lock()
                .ok()
                .and_then(|state| state.open_orders.get(asset_id).cloned())
                .filter(|entry| entry.order_id.as_deref() == Some(order_id))
                .and_then(|entry| entry.kind)
                .map(|kind| kind.eq_ignore_ascii_case("taker"))
                .unwrap_or(false)
            || self._shared_pending_taker_order_exists(order_id)
            || self
                ._shared_gross_order_reservation_snapshot(order_id)
                .map(|reservation| reservation.kind.eq_ignore_ascii_case("taker"))
                .unwrap_or(false)
    }

    fn _maker_pair_gross_preview_gtc_leg(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        origin: &str,
        now: f64,
    ) -> MakerPairGrossPreview {
        if asset_id.trim().is_empty()
            || !price.is_finite()
            || price <= 0.0
            || !size.is_finite()
            || size <= 0.0
        {
            return MakerPairGrossPreview::default();
        }

        if !self._maker_single_inflight_enabled() {
            let direct_refresh_decision =
                self._bot_runtime_direct_refresh_preview(asset_id, origin, now);
            return match direct_refresh_decision {
                MakerDirectRefreshDecision::Blocked { .. } => MakerPairGrossPreview::default(),
                MakerDirectRefreshDecision::Started(_) => MakerPairGrossPreview {
                    requested_gross_usd: price * size,
                    replace_order_ids: Vec::new(),
                },
                MakerDirectRefreshDecision::Initial => MakerPairGrossPreview {
                    requested_gross_usd: price * size,
                    replace_order_ids: Vec::new(),
                },
            };
        }

        self._maker_order_reconcile_asset(asset_id, Some(price));
        let key = MakerOrderKey::buy(asset_id);
        let mut slot = self._maker_order_slot_get(&key);
        let submit_ttl = self._maker_submit_pending_ttl_seconds();
        let cancel_ttl = self._maker_cancel_pending_ttl_seconds();
        let reject_cooldown = self._maker_submit_reject_cooldown_seconds();
        let replace_min = self._maker_replace_min_interval_seconds();
        let stale = env_int("STALE_SECONDS", self.cfg.stale_seconds).max(1) as f64;
        let replace_ticks = env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            self.cfg.replace_if_price_moves_ticks,
        ) as f64;

        if slot.state == MakerOrderLifecycle::SubmitPending
            && now - slot.last_submit_ts < submit_ttl
        {
            return MakerPairGrossPreview::default();
        }
        if slot.state == MakerOrderLifecycle::CancelPending
            && now - slot.last_cancel_ts < cancel_ttl
        {
            return MakerPairGrossPreview::default();
        }
        if slot.order_id.is_none()
            && slot.state == MakerOrderLifecycle::Idle
            && reject_cooldown > 0.0
            && slot.last_reject_ts > 0.0
        {
            let max_reject_cooldown =
                env_float("MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS", 60.0).max(reject_cooldown);
            let effective_cooldown = maker_order_effective_reject_cooldown_seconds(
                origin,
                &slot,
                reject_cooldown,
                max_reject_cooldown,
            );
            if now - slot.last_reject_ts < effective_cooldown {
                return MakerPairGrossPreview::default();
            }
        }
        if slot.state != MakerOrderLifecycle::Working
            && slot.state != MakerOrderLifecycle::Idle
            && now - slot.last_submit_ts >= submit_ttl
            && now - slot.last_cancel_ts >= cancel_ttl
        {
            slot.state = MakerOrderLifecycle::Idle;
            slot.order_id = None;
            slot.replace_target = None;
        }

        if slot.state == MakerOrderLifecycle::Working {
            let Some(_) = slot.order_id.clone() else {
                return MakerPairGrossPreview::default();
            };
            let old_price = slot.price.max(0.0);
            let old_size = slot.remaining.max(slot.size).max(0.0);
            let age = (now - slot.last_submit_ts).max(0.0);
            let moved_ticks = (price - old_price).abs() / self.cfg.tick.max(0.0001);
            let size_changed = old_size <= 0.0
                || (size - old_size).abs() >= (0.25 * old_size).max(self.cfg.min_shares);
            if age < stale && moved_ticks < replace_ticks && !size_changed {
                return MakerPairGrossPreview::default();
            }
            let same_refresh_family = maker_refresh_families_match(slot.origin.as_str(), origin);
            let refresh_capped_handoff = self._bot_runtime_origin_is_refresh_capped(&slot.origin)
                && self._bot_runtime_origin_is_refresh_capped(origin);
            if refresh_capped_handoff {
                if let Some(side) = self._maker_side_for_asset_id(asset_id) {
                    if self
                        ._bot_runtime_refresh_cycle_block_reason_peek(side, origin, now)
                        .is_some()
                    {
                        return MakerPairGrossPreview::default();
                    }
                }
            } else if !same_refresh_family && now - slot.last_cancel_ts < replace_min {
                return MakerPairGrossPreview::default();
            }
            return MakerPairGrossPreview::default();
        }

        MakerPairGrossPreview {
            requested_gross_usd: price * size,
            replace_order_ids: Vec::new(),
        }
    }

    fn _bot_runtime_refresh_cancel_is_protective(reason: &str) -> bool {
        let lower = reason.trim().to_ascii_lowercase();
        lower.contains("stale") || lower.contains("invalid") || lower.contains("broken")
    }

    fn _maker_side_for_asset_id(&self, asset_id: &str) -> Option<OutcomeSide> {
        let trimmed = asset_id.trim();
        if trimmed.is_empty() {
            None
        } else if self.yes_asset.as_deref() == Some(trimmed) {
            Some(OutcomeSide::Yes)
        } else if self.no_asset.as_deref() == Some(trimmed) {
            Some(OutcomeSide::No)
        } else {
            None
        }
    }

    /// Implements order slot get for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_slot_get(&self, key: &MakerOrderKey) -> MakerOrderSlot {
        self.maker_order_slots
            .lock()
            .ok()
            .and_then(|m| m.get(key).cloned())
            .unwrap_or_default()
    }

    /// Implements order origin by order id for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_origin_by_order_id(&self, order_id: &str) -> Option<String> {
        let trimmed = order_id.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(origin) = self
            .maker_exec_ledger
            .lock()
            .ok()
            .and_then(|ledger| ledger.per_order_origin.get(trimmed).cloned())
        {
            let stable = origin.trim();
            if !stable.is_empty() {
                return Some(stable.to_string());
            }
        }
        let key = self
            .maker_order_index
            .lock()
            .ok()
            .and_then(|idx| idx.get(trimmed).cloned())?;
        let slot = self
            .maker_order_slots
            .lock()
            .ok()
            .and_then(|slots| slots.get(&key).cloned())?;
        if slot.order_id.as_deref() != Some(trimmed) {
            return None;
        }
        let origin = slot.origin.trim();
        if origin.is_empty() {
            None
        } else {
            Some(origin.to_string())
        }
    }

    fn _bot_runtime_refresh_cycle_state_mut<'a>(
        state: &'a mut BotRuntimeState,
        side: OutcomeSide,
    ) -> &'a mut BotRuntimeSideRefreshCycleState {
        match side {
            OutcomeSide::Yes => &mut state.yes_refresh_cycle,
            OutcomeSide::No => &mut state.no_refresh_cycle,
        }
    }

    pub(super) fn _bot_runtime_origin_is_refresh_capped(&self, origin: &str) -> bool {
        maker_refresh_family(origin).is_some()
    }

    fn _bot_runtime_refresh_cycle_block_reason_peek(
        &self,
        side: OutcomeSide,
        origin: &str,
        now: f64,
    ) -> Option<String> {
        let cap_seconds = self._maker_replace_min_interval_seconds();
        if cap_seconds <= 0.0 || !self._bot_runtime_origin_is_refresh_capped(origin) {
            return None;
        }
        let (last_cycle_started_ts, last_cycle_origin) = self
            .bot_runtime_state
            .lock()
            .map(|state| {
                let refresh_state = match side {
                    OutcomeSide::Yes => &state.yes_refresh_cycle,
                    OutcomeSide::No => &state.no_refresh_cycle,
                };
                (
                    refresh_state.last_cycle_started_ts,
                    refresh_state.last_origin.clone(),
                )
            })
            .unwrap_or((0.0, String::new()));
        if last_cycle_started_ts <= 0.0 || last_cycle_origin.trim().is_empty() {
            return None;
        }
        let elapsed = (now - last_cycle_started_ts).max(0.0);
        if elapsed + 1e-9 >= cap_seconds {
            return None;
        }
        Some(format!(
            "refresh_cadence_cap:{}:{:.2}",
            side.as_str(),
            (cap_seconds - elapsed).max(0.0)
        ))
    }

    fn _bot_runtime_refresh_cycle_block_reason(
        &self,
        side: OutcomeSide,
        origin: &str,
        reason: &str,
        now: f64,
    ) -> Option<String> {
        let block_reason = self._bot_runtime_refresh_cycle_block_reason_peek(side, origin, now)?;
        if let Ok(mut state) = self.bot_runtime_state.lock() {
            let refresh_state = Self::_bot_runtime_refresh_cycle_state_mut(&mut state, side);
            refresh_state.last_origin = origin.trim().to_string();
            refresh_state.last_reason = reason.trim().to_string();
            match side {
                OutcomeSide::Yes => {
                    state.yes_refresh_cap_block_count =
                        state.yes_refresh_cap_block_count.saturating_add(1);
                }
                OutcomeSide::No => {
                    state.no_refresh_cap_block_count =
                        state.no_refresh_cap_block_count.saturating_add(1);
                }
            }
        }
        self.logger.info(&format!(
            "[BOT][REFRESH_CAP] side={} origin={} reason={} hold_reason={}",
            side.as_str(),
            origin.trim(),
            reason.trim(),
            block_reason
        ));
        Some(block_reason)
    }

    pub(in crate::bot) fn _bot_runtime_note_refresh_cycle_started(
        &self,
        side: OutcomeSide,
        origin: &str,
        reason: &str,
        now: f64,
    ) {
        if let Ok(mut state) = self.bot_runtime_state.lock() {
            let refresh_state = Self::_bot_runtime_refresh_cycle_state_mut(&mut state, side);
            refresh_state.last_cycle_started_ts = now;
            refresh_state.awaiting_repost = true;
            refresh_state.last_origin = origin.trim().to_string();
            refresh_state.last_reason = reason.trim().to_string();
            match side {
                OutcomeSide::Yes => {
                    state.yes_refresh_cycles_started =
                        state.yes_refresh_cycles_started.saturating_add(1);
                }
                OutcomeSide::No => {
                    state.no_refresh_cycles_started =
                        state.no_refresh_cycles_started.saturating_add(1);
                }
            }
        }
    }

    pub(in crate::bot) fn _bot_runtime_note_refresh_cycle_submit(
        &self,
        side: OutcomeSide,
        origin: &str,
        reason: &str,
    ) {
        if let Ok(mut state) = self.bot_runtime_state.lock() {
            let refresh_state = Self::_bot_runtime_refresh_cycle_state_mut(&mut state, side);
            refresh_state.awaiting_repost = false;
            if !origin.trim().is_empty() {
                refresh_state.last_origin = origin.trim().to_string();
            }
            if !reason.trim().is_empty() {
                refresh_state.last_reason = reason.trim().to_string();
            }
        }
    }

    fn _maker_order_request_cancel_with_policy(
        &self,
        key: &MakerOrderKey,
        reason: &str,
        enforce_refresh_interval: bool,
    ) -> bool {
        if !self._maker_single_inflight_enabled() {
            return false;
        }
        let now = now_ts_f64();
        let slot = self._maker_order_slot_get(key);
        let Some(oid) = slot.order_id.clone() else {
            return false;
        };
        if slot.state == MakerOrderLifecycle::CancelPending
            && now - slot.last_cancel_ts < self._maker_cancel_pending_ttl_seconds()
        {
            return false;
        }
        if enforce_refresh_interval
            && now - slot.last_cancel_ts < self._maker_replace_min_interval_seconds()
        {
            return false;
        }
        if !self._cancel(&oid) {
            return false;
        }
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            if let Some(s) = slots.get_mut(key) {
                if s.order_id.as_deref() == Some(oid.as_str()) {
                    s.state = MakerOrderLifecycle::CancelPending;
                    s.last_cancel_ts = now;
                }
            }
        }
        if !reason.trim().is_empty() {
            self.logger.info(&format!(
                "[MAKER_ORD] cancel requested asset={} side={} oid={}.. ({reason})",
                key.asset_id,
                key.side,
                oid.chars().take(10).collect::<String>()
            ));
        }
        true
    }

    pub(super) fn _maker_order_request_cancel_unthrottled(
        &self,
        key: &MakerOrderKey,
        reason: &str,
    ) -> bool {
        self._maker_order_request_cancel_with_policy(key, reason, false)
    }

    pub(super) fn _maker_order_request_refresh_cancel(
        &self,
        key: &MakerOrderKey,
        reason: &str,
    ) -> Result<bool, String> {
        if !self._maker_single_inflight_enabled() {
            return Ok(false);
        }
        let slot = self._maker_order_slot_get(key);
        let Some(_) = slot.order_id.as_ref() else {
            return Ok(false);
        };
        let Some(side) = self._maker_side_for_asset_id(key.asset_id.as_str()) else {
            return Ok(false);
        };
        let now = now_ts_f64();
        let protective_cancel = Self::_bot_runtime_refresh_cancel_is_protective(reason);
        if !protective_cancel {
            if let Some(block_reason) =
                self._bot_runtime_refresh_cycle_block_reason(side, &slot.origin, reason, now)
            {
                return Err(block_reason);
            }
        }
        if self._maker_order_request_cancel_with_policy(key, reason, false) {
            self._bot_runtime_note_refresh_cycle_started(side, &slot.origin, reason, now);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(in crate::bot) fn _bot_runtime_direct_refresh_decision(
        &self,
        asset_id: &str,
        origin: &str,
        now: f64,
    ) -> MakerDirectRefreshDecision {
        if self._maker_single_inflight_enabled()
            || !self._bot_runtime_origin_is_refresh_capped(origin)
        {
            return MakerDirectRefreshDecision::Initial;
        }
        let Some(side) = self._maker_side_for_asset_id(asset_id) else {
            return MakerDirectRefreshDecision::Initial;
        };
        let current = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get(asset_id).cloned());
        let Some(current_order) = current else {
            return MakerDirectRefreshDecision::Initial;
        };
        let Some(existing_order_id) = current_order.order_id.clone() else {
            return MakerDirectRefreshDecision::Initial;
        };
        let Some(existing_ctx) = self._get_order_execution_context(existing_order_id.as_str())
        else {
            let recent_submit_ts = current_order.submit_ts.unwrap_or(0.0);
            let cap_seconds = self._maker_replace_min_interval_seconds();
            if recent_submit_ts > 0.0 && cap_seconds > 0.0 {
                let elapsed = (now - recent_submit_ts).max(0.0);
                if elapsed + 1e-9 < cap_seconds {
                    let block_reason = format!(
                        "refresh_cadence_cap:{}:{:.2}",
                        side.as_str(),
                        (cap_seconds - elapsed).max(0.0)
                    );
                    if let Ok(mut state) = self.bot_runtime_state.lock() {
                        let refresh_state =
                            Self::_bot_runtime_refresh_cycle_state_mut(&mut state, side);
                        refresh_state.last_origin = origin.trim().to_string();
                        refresh_state.last_reason = "direct_refresh_missing_context".to_string();
                        match side {
                            OutcomeSide::Yes => {
                                state.yes_refresh_cap_block_count =
                                    state.yes_refresh_cap_block_count.saturating_add(1);
                            }
                            OutcomeSide::No => {
                                state.no_refresh_cap_block_count =
                                    state.no_refresh_cap_block_count.saturating_add(1);
                            }
                        }
                    }
                    self.logger.info(&format!(
                        "[BOT][REFRESH_CAP] side={} origin={} reason=direct_refresh_missing_context hold_reason={}",
                        side.as_str(),
                        origin.trim(),
                        block_reason
                    ));
                    return MakerDirectRefreshDecision::Blocked {
                        existing_order_id,
                        reason: block_reason,
                    };
                }
                return MakerDirectRefreshDecision::Started(side);
            }
            return MakerDirectRefreshDecision::Initial;
        };
        if existing_ctx
            .get("direct_cancel_requested")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return MakerDirectRefreshDecision::Initial;
        }
        let existing_origin = existing_ctx
            .get("origin")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let existing_side = existing_ctx
            .get("side")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        if existing_side != "BUY"
            || !self._bot_runtime_origin_is_refresh_capped(existing_origin.as_str())
        {
            return MakerDirectRefreshDecision::Initial;
        }
        if !maker_refresh_families_match(existing_origin.as_str(), origin) {
            let recent_submit_ts = existing_ctx
                .get("order_submit_ts")
                .and_then(|value| value.as_f64())
                .or_else(|| {
                    existing_ctx
                        .get("decision_ts")
                        .and_then(|value| value.as_f64())
                })
                .or_else(|| {
                    existing_ctx
                        .get("post_start_ts")
                        .and_then(|value| value.as_f64())
                })
                .unwrap_or(0.0);
            let cap_seconds = self._maker_replace_min_interval_seconds();
            if recent_submit_ts > 0.0 && cap_seconds > 0.0 {
                let elapsed = (now - recent_submit_ts).max(0.0);
                if elapsed + 1e-9 < cap_seconds {
                    let block_reason = format!(
                        "refresh_cadence_cap:{}:{:.2}",
                        side.as_str(),
                        (cap_seconds - elapsed).max(0.0)
                    );
                    if let Ok(mut state) = self.bot_runtime_state.lock() {
                        let refresh_state =
                            Self::_bot_runtime_refresh_cycle_state_mut(&mut state, side);
                        refresh_state.last_origin = origin.trim().to_string();
                        refresh_state.last_reason = "direct_refresh_handoff".to_string();
                        match side {
                            OutcomeSide::Yes => {
                                state.yes_refresh_cap_block_count =
                                    state.yes_refresh_cap_block_count.saturating_add(1);
                            }
                            OutcomeSide::No => {
                                state.no_refresh_cap_block_count =
                                    state.no_refresh_cap_block_count.saturating_add(1);
                            }
                        }
                    }
                    self.logger.info(&format!(
                        "[BOT][REFRESH_CAP] side={} origin={} reason=direct_refresh_handoff hold_reason={}",
                        side.as_str(),
                        origin.trim(),
                        block_reason
                    ));
                    return MakerDirectRefreshDecision::Blocked {
                        existing_order_id,
                        reason: block_reason,
                    };
                }
            }
            return MakerDirectRefreshDecision::Initial;
        }
        if let Some(block_reason) =
            self._bot_runtime_refresh_cycle_block_reason(side, origin, "direct_refresh_submit", now)
        {
            return MakerDirectRefreshDecision::Blocked {
                existing_order_id,
                reason: block_reason,
            };
        }
        MakerDirectRefreshDecision::Started(side)
    }

    fn _bot_runtime_direct_refresh_preview(
        &self,
        asset_id: &str,
        origin: &str,
        now: f64,
    ) -> MakerDirectRefreshDecision {
        if self._maker_single_inflight_enabled()
            || !self._bot_runtime_origin_is_refresh_capped(origin)
        {
            return MakerDirectRefreshDecision::Initial;
        }
        let Some(side) = self._maker_side_for_asset_id(asset_id) else {
            return MakerDirectRefreshDecision::Initial;
        };
        let current = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.open_orders.get(asset_id).cloned());
        let Some(current_order) = current else {
            return MakerDirectRefreshDecision::Initial;
        };
        let Some(existing_order_id) = current_order.order_id.clone() else {
            return MakerDirectRefreshDecision::Initial;
        };
        let Some(existing_ctx) = self._get_order_execution_context(existing_order_id.as_str())
        else {
            let recent_submit_ts = current_order.submit_ts.unwrap_or(0.0);
            let cap_seconds = self._maker_replace_min_interval_seconds();
            if recent_submit_ts > 0.0 && cap_seconds > 0.0 {
                let elapsed = (now - recent_submit_ts).max(0.0);
                if elapsed + 1e-9 < cap_seconds {
                    return MakerDirectRefreshDecision::Blocked {
                        existing_order_id,
                        reason: format!(
                            "refresh_cadence_cap:{}:{:.2}",
                            side.as_str(),
                            (cap_seconds - elapsed).max(0.0)
                        ),
                    };
                }
                return MakerDirectRefreshDecision::Started(side);
            }
            return MakerDirectRefreshDecision::Initial;
        };
        if existing_ctx
            .get("direct_cancel_requested")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return MakerDirectRefreshDecision::Initial;
        }
        let existing_origin = existing_ctx
            .get("origin")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let existing_side = existing_ctx
            .get("side")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        if existing_side != "BUY"
            || !self._bot_runtime_origin_is_refresh_capped(existing_origin.as_str())
        {
            return MakerDirectRefreshDecision::Initial;
        }
        if !maker_refresh_families_match(existing_origin.as_str(), origin) {
            let recent_submit_ts = existing_ctx
                .get("order_submit_ts")
                .and_then(|value| value.as_f64())
                .or_else(|| {
                    existing_ctx
                        .get("decision_ts")
                        .and_then(|value| value.as_f64())
                })
                .or_else(|| {
                    existing_ctx
                        .get("post_start_ts")
                        .and_then(|value| value.as_f64())
                })
                .unwrap_or(0.0);
            let cap_seconds = self._maker_replace_min_interval_seconds();
            if recent_submit_ts > 0.0 && cap_seconds > 0.0 {
                let elapsed = (now - recent_submit_ts).max(0.0);
                if elapsed + 1e-9 < cap_seconds {
                    return MakerDirectRefreshDecision::Blocked {
                        existing_order_id,
                        reason: format!(
                            "refresh_cadence_cap:{}:{:.2}",
                            side.as_str(),
                            (cap_seconds - elapsed).max(0.0)
                        ),
                    };
                }
            }
            return MakerDirectRefreshDecision::Initial;
        }
        if let Some(block_reason) =
            self._bot_runtime_refresh_cycle_block_reason_peek(side, origin, now)
        {
            return MakerDirectRefreshDecision::Blocked {
                existing_order_id,
                reason: block_reason,
            };
        }
        MakerDirectRefreshDecision::Started(side)
    }

    /// Implements reconcile inferred origin for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_reconcile_inferred_origin(&self, key: &MakerOrderKey) -> String {
        let slot = self._maker_order_slot_get(key);
        let slot_origin = slot.origin.trim();
        if !slot_origin.is_empty() && slot_origin != "RECONCILE" {
            return slot_origin.to_string();
        }
        if let Some(target) = slot.replace_target.as_ref() {
            let target_origin = target.origin.trim();
            if !target_origin.is_empty() && target_origin != "RECONCILE" {
                return target_origin.to_string();
            }
        }
        "RECONCILE".to_string()
    }

    /// Implements order is live for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_is_live(
        &self,
        asset_id: &str,
        expected_oid: Option<&str>,
        max_age_s: f64,
    ) -> bool {
        if !self._maker_single_inflight_enabled() {
            return false;
        }
        let key = MakerOrderKey::buy(asset_id);
        let slot = self._maker_order_slot_get(&key);
        if !matches!(
            slot.state,
            MakerOrderLifecycle::Working | MakerOrderLifecycle::SubmitPending
        ) {
            return false;
        }
        let Some(slot_oid) = &slot.order_id else {
            return false;
        };
        if let Some(expected) = expected_oid {
            if slot_oid != expected {
                return false;
            }
        }
        let age = now_ts_f64() - slot.last_submit_ts;
        if age > max_age_s || age < 0.0 {
            return false;
        }
        true
    }

    /// Implements order clear index for key for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_clear_index_for_key(&self, key: &MakerOrderKey) {
        if let Ok(mut idx) = self.maker_order_index.lock() {
            idx.retain(|_, v| v != key);
        }
    }

    /// Implements order open buy remaining for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_open_buy_remaining(&self, asset_id: &str) -> f64 {
        if !self._maker_single_inflight_enabled() {
            return 0.0;
        }
        let key = MakerOrderKey::buy(asset_id);
        let slot = self._maker_order_slot_get(&key);
        if matches!(
            slot.state,
            MakerOrderLifecycle::Working
                | MakerOrderLifecycle::SubmitPending
                | MakerOrderLifecycle::CancelPending
        ) {
            if slot.remaining > 0.0 {
                slot.remaining.max(0.0)
            } else {
                slot.size.max(0.0)
            }
        } else {
            0.0
        }
    }

    /// Implements order on cancel ack by order id for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_on_cancel_ack_by_order_id(&self, order_id: &str) {
        if !self._maker_single_inflight_enabled() || order_id.trim().is_empty() {
            return;
        }
        let key = self
            .maker_order_index
            .lock()
            .ok()
            .and_then(|idx| idx.get(order_id).cloned());
        let Some(key) = key else {
            return;
        };
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            if slot.order_id.as_deref() == Some(order_id) {
                slot.state = MakerOrderLifecycle::Idle;
                slot.order_id = None;
                slot.last_cancel_ts = now_ts_f64();
                slot.replace_target = None;
            }
        }
        if let Ok(mut idx) = self.maker_order_index.lock() {
            idx.remove(order_id);
        }
        self._forget_shared_gross_order_reservation(order_id);
    }

    /// Implements order on submit ack for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_on_submit_ack(
        &self,
        order_id: &str,
        key: &MakerOrderKey,
        price: f64,
        size: f64,
        origin: &str,
    ) -> bool {
        if !self._maker_single_inflight_enabled() || order_id.trim().is_empty() {
            return false;
        }
        let trimmed_oid = order_id.trim();
        let trimmed_origin = origin.trim();
        if let Ok(mut ledger) = self.maker_exec_ledger.lock() {
            let entry = ledger
                .per_order_origin
                .entry(trimmed_oid.to_string())
                .or_default();
            let existing = entry.trim();
            let can_upgrade_reconcile = existing == "RECONCILE"
                && !trimmed_origin.is_empty()
                && trimmed_origin != "RECONCILE";
            if (existing.is_empty() || can_upgrade_reconcile) && !trimmed_origin.is_empty() {
                *entry = trimmed_origin.to_string();
            }
        }
        let now = now_ts_f64();
        let mut prev_oid: Option<String> = None;
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            prev_oid = slot.order_id.clone();
            slot.state = MakerOrderLifecycle::Working;
            slot.order_id = Some(order_id.to_string());
            slot.price = price;
            slot.size = size.max(0.0);
            slot.remaining = size.max(0.0);
            slot.last_submit_ts = now;
            slot.origin = origin.to_string();
            slot.last_reject_origin.clear();
            slot.replace_target = None;
            slot.consecutive_rejects = 0;
        }
        if let Ok(mut idx) = self.maker_order_index.lock() {
            if let Some(prev) = prev_oid {
                if prev != order_id {
                    idx.remove(&prev);
                }
            }
            idx.insert(order_id.to_string(), key.clone());
        }
        if key.side == "BUY" && !key.asset_id.trim().is_empty() {
            if let Some(side) = self._maker_side_for_asset_id(key.asset_id.as_str()) {
                self._bot_runtime_note_refresh_cycle_submit(side, origin, "submit_ack");
            }
        }
        if key.side == "BUY" && !key.asset_id.trim().is_empty() {
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    key.asset_id.clone(),
                    OpenOrderState {
                        order_id: Some(order_id.to_string()),
                        price: Some(price),
                        size: Some(size.max(0.0)),
                        ts: Some(now),
                        submit_ts: Some(now),
                        kind: Some("maker".to_string()),
                    },
                );
                let _ = self._bot_runtime_save_state_or_dependency_pause(
                    &mut s,
                    "maker_order_track_submit",
                );
            }
            if !self._remember_shared_gross_order_reservation(
                order_id,
                key.asset_id.as_str(),
                key.side.as_str(),
                price,
                size.max(0.0),
                origin,
                "maker",
            ) {
                let _ = self._cancel(order_id);
                if let Ok(mut s) = self.state.lock() {
                    let should_remove = s
                        .open_orders
                        .get(&key.asset_id)
                        .and_then(|order| order.order_id.as_deref())
                        == Some(order_id);
                    if should_remove {
                        s.open_orders.remove(&key.asset_id);
                        let _ = self._bot_runtime_save_state_or_dependency_pause(
                            &mut s,
                            "maker_order_submit_ack_gross_order_remember_failed",
                        );
                    }
                }
                return false;
            }
        }
        true
    }

    /// Implements order on submit reject for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_on_submit_reject(
        &self,
        key: &MakerOrderKey,
        origin: &str,
        reason: &str,
    ) {
        if !self._maker_single_inflight_enabled() {
            return;
        }
        let now = now_ts_f64();
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            slot.last_reject_ts = now;
            slot.consecutive_rejects = slot.consecutive_rejects.saturating_add(1);
            slot.last_reject_origin = origin.to_string();
            if slot.order_id.is_some() {
                slot.state = MakerOrderLifecycle::Working;
            } else {
                slot.state = MakerOrderLifecycle::Idle;
                slot.replace_target = None;
            }
        }
        if !reason.trim().is_empty() {
            self.logger.warning(&format!(
                "[MAKER_ORD] submit reject asset={} side={} reason={reason}",
                key.asset_id, key.side
            ));
        }
    }

    /// Implements order request cancel for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_request_cancel(&self, key: &MakerOrderKey, reason: &str) -> bool {
        self._maker_order_request_cancel_with_policy(key, reason, true)
    }

    /// Implements order cancel all except asset for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_cancel_all_except_asset(
        &self,
        keep_asset_id: Option<&str>,
        reason: &str,
    ) {
        if !self._maker_single_inflight_enabled() {
            return;
        }
        let keep = keep_asset_id.unwrap_or("").trim().to_string();
        let keys: Vec<MakerOrderKey> = self
            .maker_order_slots
            .lock()
            .map(|m| {
                m.keys()
                    .filter_map(|k| {
                        if k.side != "BUY" {
                            return None;
                        }
                        if !keep.is_empty() && k.asset_id == keep {
                            return None;
                        }
                        Some(k.clone())
                    })
                    .collect::<Vec<MakerOrderKey>>()
            })
            .unwrap_or_default();
        for key in keys {
            let _ = self._maker_order_request_cancel_unthrottled(&key, reason);
        }
    }

    /// Implements cancel strategy orders for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_cancel_strategy_orders(&self, keep_asset_id: Option<&str>, reason: &str) {
        if self._maker_single_inflight_enabled() {
            self._maker_order_cancel_all_except_asset(keep_asset_id, reason);
            return;
        }
        if let Some(keep) = keep_asset_id {
            self.cancel_all_open_orders_local_except(keep, reason);
        } else {
            self.cancel_all_open_orders_local(reason);
        }
    }

    /// Implements order reconcile asset for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_reconcile_asset(&self, asset_id: &str, intended_price: Option<f64>) {
        if !self._maker_single_inflight_enabled() || !self._bot_runtime_venue_reads_allowed() {
            return;
        }
        let aid = asset_id.trim().to_string();
        if aid.is_empty() {
            return;
        }
        let key = MakerOrderKey::buy(&aid);
        let max_active = env_int("MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET", 1).max(1) as usize;
        let pick_keep_oid = |orders: &[Value], tracked_oid: Option<String>| -> Option<String> {
            if orders.is_empty() {
                return None;
            }
            if let Some(t) = tracked_oid {
                let has_tracked = orders
                    .iter()
                    .any(|o| self._extract_order_id(o).map(|id| id == t).unwrap_or(false));
                if has_tracked {
                    return Some(t);
                }
            }
            if let Some(ip) = intended_price.filter(|p| *p > 0.0) {
                return orders
                    .iter()
                    .filter_map(|o| {
                        let oid = self._extract_order_id(o)?;
                        let d = (self._extract_order_price(o) - ip).abs();
                        Some((oid, d))
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|x| x.0);
            }
            orders
                .iter()
                .filter_map(|o| {
                    self._extract_order_id(o)
                        .map(|oid| (oid, self._extract_order_price(o)))
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|x| x.0)
        };

        let mut buy_orders: Vec<Value> = self
            ._list_open_orders_exchange()
            .into_iter()
            .filter(|o| {
                self._extract_order_token_id(o).as_deref() == Some(aid.as_str())
                    && self._extract_order_side(o) == "BUY"
                    && self._extract_order_id(o).is_some()
                    && self._extract_order_remaining_size(o) > 1e-9
            })
            .collect();
        if buy_orders.len() > max_active {
            let tracked_oid = self._maker_order_slot_get(&key).order_id;
            let keep_oid = pick_keep_oid(&buy_orders, tracked_oid);
            for o in &buy_orders {
                let Some(oid) = self._extract_order_id(o) else {
                    continue;
                };
                if keep_oid.as_deref() == Some(oid.as_str()) {
                    continue;
                }
                let _ = self._cancel(&oid);
            }
            buy_orders = self
                ._list_open_orders_exchange()
                .into_iter()
                .filter(|o| {
                    self._extract_order_token_id(o).as_deref() == Some(aid.as_str())
                        && self._extract_order_side(o) == "BUY"
                        && self._extract_order_id(o).is_some()
                        && self._extract_order_remaining_size(o) > 1e-9
                })
                .collect();
        }

        let tracked_oid = self._maker_order_slot_get(&key).order_id;
        let keep_oid = pick_keep_oid(&buy_orders, tracked_oid);
        let keep_order = keep_oid.as_ref().and_then(|oid| {
            buy_orders.iter().find_map(|o| {
                self._extract_order_id(o)
                    .filter(|x| x == oid)
                    .map(|_| o.clone())
            })
        });

        if let Some(order) = keep_order {
            let oid = self._extract_order_id(&order).unwrap_or_default();
            if self._order_is_known_taker_buy(oid.as_str(), aid.as_str()) {
                return;
            }
            let price = self._extract_order_price(&order);
            let remaining = self._extract_order_remaining_size(&order).max(0.0);
            let size = remaining.max(0.0);
            let reconcile_origin = self._maker_reconcile_inferred_origin(&key);
            if !self._maker_order_on_submit_ack(&oid, &key, price, size, &reconcile_origin) {
                return;
            }
            if max_active == 1 {
                for o in buy_orders {
                    let Some(oid2) = self._extract_order_id(&o) else {
                        continue;
                    };
                    if oid2 == oid {
                        continue;
                    }
                    let _ = self._cancel(&oid2);
                }
            }
            return;
        }

        let now = now_ts_f64();
        let submit_ttl = self._maker_submit_pending_ttl_seconds();
        let cancel_ttl = self._maker_cancel_pending_ttl_seconds();
        let working_missing_ttl = self._maker_working_missing_ttl_seconds();
        let cur_slot = self._maker_order_slot_get(&key);
        if buy_orders.is_empty()
            && cur_slot.state == MakerOrderLifecycle::Working
            && cur_slot.order_id.is_some()
            && (now - cur_slot.last_submit_ts) < working_missing_ttl
        {
            // Exchange list can be transiently stale right after submit/cancel churn.
            // Keep local working slot conservative for a short grace period to avoid
            // duplicate same-side submits.
            return;
        }
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            let keep_pending = match slot.state {
                MakerOrderLifecycle::SubmitPending => now - slot.last_submit_ts < submit_ttl,
                MakerOrderLifecycle::CancelPending => now - slot.last_cancel_ts < cancel_ttl,
                _ => false,
            };
            if !keep_pending {
                slot.state = MakerOrderLifecycle::Idle;
                slot.order_id = None;
                slot.remaining = 0.0;
                slot.replace_target = None;
            }
        }
        self._maker_order_clear_index_for_key(&key);
        if let Ok(mut s) = self.state.lock() {
            let should_remove = s
                .open_orders
                .get(&aid)
                .and_then(|oo| oo.order_id.clone())
                .is_some();
            if should_remove {
                s.open_orders.remove(&aid);
                let _ = self
                    ._bot_runtime_save_state_or_dependency_pause(&mut s, "maker_order_clear_slot");
            }
        }
    }

    /// Implements order on user event for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_on_user_event(&self, msg: &Value) {
        if !self._maker_single_inflight_enabled() {
            return;
        }
        let oid = self._extract_order_id(msg).unwrap_or_default();
        if oid.trim().is_empty() {
            return;
        }
        let side = msg
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        let key_from_index = self
            .maker_order_index
            .lock()
            .ok()
            .and_then(|idx| idx.get(&oid).cloned());
        let msg_asset_id = msg
            .get("asset_id")
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("assetId"))
            .or_else(|| msg.get("tokenId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let known_taker_buy = key_from_index.is_none()
            && side == "BUY"
            && !msg_asset_id.is_empty()
            && self._order_is_known_taker_buy(oid.as_str(), msg_asset_id.as_str());
        if known_taker_buy {
            return;
        }
        let key = if let Some(k) = key_from_index {
            k
        } else {
            if side != "BUY" || msg_asset_id.is_empty() {
                return;
            }
            MakerOrderKey::buy(&msg_asset_id)
        };
        if key.side != "BUY" || key.asset_id.trim().is_empty() {
            return;
        }
        let asset_id = key.asset_id.clone();
        let typ = msg
            .get("type")
            .or_else(|| msg.get("event_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let status = msg
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let cancelish = matches!(
            typ.as_str(),
            "CANCELLATION" | "CANCELED" | "CANCELLED" | "REJECTION" | "REJECTED"
        ) || matches!(status.as_str(), "CANCELED" | "CANCELLED" | "REJECTED");
        let price = Self::_value_f64(msg.get("price")).unwrap_or(0.0);
        let original = Self::_value_f64(
            msg.get("original_size")
                .or_else(|| msg.get("originalSize"))
                .or_else(|| msg.get("size")),
        )
        .unwrap_or(0.0);
        let matched = Self::_value_f64(
            msg.get("size_matched")
                .or_else(|| msg.get("matched_size"))
                .or_else(|| msg.get("filled_size"))
                .or_else(|| msg.get("filled")),
        )
        .unwrap_or(0.0);
        let mut remaining = if original > 0.0 {
            (original - matched).max(0.0)
        } else {
            Self::_value_f64(
                msg.get("remaining_size")
                    .or_else(|| msg.get("remainingSize"))
                    .or_else(|| msg.get("size")),
            )
            .unwrap_or(0.0)
            .max(0.0)
        };
        if !remaining.is_finite() {
            remaining = 0.0;
        }
        if cancelish || remaining <= 1e-9 {
            if let Ok(mut slots) = self.maker_order_slots.lock() {
                let slot = slots.entry(key.clone()).or_default();
                if slot.order_id.as_deref() == Some(oid.as_str())
                    || slot.order_id.is_none()
                    || slot.state == MakerOrderLifecycle::CancelPending
                {
                    slot.state = MakerOrderLifecycle::Idle;
                    slot.order_id = None;
                    slot.remaining = 0.0;
                    slot.replace_target = None;
                }
            }
            if let Ok(mut idx) = self.maker_order_index.lock() {
                idx.remove(&oid);
            }
            if let Ok(mut s) = self.state.lock() {
                let should_remove = s
                    .open_orders
                    .get(&asset_id)
                    .and_then(|oo| oo.order_id.clone())
                    .map(|x| x == oid)
                    .unwrap_or(false);
                if should_remove {
                    s.open_orders.remove(&asset_id);
                    let _ = self._bot_runtime_save_state_or_dependency_pause(
                        &mut s,
                        "maker_order_cancel_ack",
                    );
                }
            }
            return;
        }

        let max_active = env_int("MAKER_MAX_ACTIVE_BUY_ORDERS_PER_ASSET", 1).max(1);
        let mut duplicate_oid: Option<String> = None;
        let mut should_adopt = true;
        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let slot = slots.entry(key.clone()).or_default();
            if let Some(cur_oid) = slot.order_id.clone() {
                if cur_oid != oid && slot.state == MakerOrderLifecycle::Working && max_active <= 1 {
                    duplicate_oid = Some(oid.clone());
                    should_adopt = false;
                }
            }
            if should_adopt {
                slot.state = MakerOrderLifecycle::Working;
                slot.order_id = Some(oid.clone());
                slot.price = if price > 0.0 { price } else { slot.price };
                slot.size = original.max(remaining).max(slot.size);
                slot.remaining = remaining;
                slot.origin = if slot.origin.trim().is_empty() {
                    "ORDER_EVENT".to_string()
                } else {
                    slot.origin.clone()
                };
                slot.replace_target = None;
            }
        }
        if let Some(dup) = duplicate_oid {
            self.logger.warning(&format!(
                "[MAKER_ORD] duplicate BUY order for asset={} tracked differs; canceling {}..",
                asset_id,
                dup.chars().take(10).collect::<String>()
            ));
            let _ = self._cancel(&dup);
            return;
        }
        if should_adopt {
            if let Ok(mut idx) = self.maker_order_index.lock() {
                idx.retain(|_, v| v != &key);
                idx.insert(oid.clone(), key);
            }
            if let Ok(mut s) = self.state.lock() {
                let existing = s.open_orders.get(&asset_id).cloned();
                let submit_ts = existing
                    .as_ref()
                    .filter(|entry| entry.order_id.as_deref() == Some(oid.as_str()))
                    .and_then(|entry| entry.submit_ts.or(entry.ts))
                    .or_else(|| {
                        self._get_order_execution_context(oid.as_str())
                            .as_ref()
                            .and_then(|ctx| {
                                ctx.get("order_submit_ts")
                                    .and_then(|value| value.as_f64())
                                    .or_else(|| {
                                        ctx.get("decision_ts").and_then(|value| value.as_f64())
                                    })
                            })
                    })
                    .or(Some(now_ts_f64()));
                s.open_orders.insert(
                    asset_id,
                    OpenOrderState {
                        order_id: Some(oid),
                        price: Some(price),
                        size: Some(remaining),
                        ts: Some(now_ts_f64()),
                        submit_ts,
                        kind: Some("maker".to_string()),
                    },
                );
                let _ = self._bot_runtime_save_state_or_dependency_pause(
                    &mut s,
                    "maker_order_reconcile_local",
                );
            }
        }
    }

    /// Implements order upsert GTC for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_upsert_gtc(
        &self,
        key: &MakerOrderKey,
        price: f64,
        size: f64,
        origin: &str,
    ) -> Option<String> {
        self._maker_order_upsert_gtc_internal(key, price, size, origin, false)
    }

    pub(super) fn _maker_order_upsert_gtc_internal(
        &self,
        key: &MakerOrderKey,
        price: f64,
        size: f64,
        origin: &str,
        gross_cap_preapproved: bool,
    ) -> Option<String> {
        if key.asset_id.trim().is_empty() || key.side != "BUY" {
            return None;
        }
        if !self._maker_single_inflight_enabled() {
            return self._place_limit_bid_gtc_with_origin_internal(
                &key.asset_id,
                price,
                size,
                Some(true),
                origin,
                gross_cap_preapproved,
                true,
            );
        }

        let now = now_ts_f64();
        let submit_ttl = self._maker_submit_pending_ttl_seconds();
        let cancel_ttl = self._maker_cancel_pending_ttl_seconds();
        let reject_cooldown = self._maker_submit_reject_cooldown_seconds();
        let replace_min = self._maker_replace_min_interval_seconds();
        let stale = env_int("STALE_SECONDS", self.cfg.stale_seconds).max(1) as f64;
        let replace_ticks = env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            self.cfg.replace_if_price_moves_ticks,
        ) as f64;

        self._maker_order_reconcile_asset(&key.asset_id, Some(price));
        let mut slot = self._maker_order_slot_get(key);
        let mut target_price = price;
        let mut target_size = size;
        let mut target_origin = origin.to_string();
        if slot.state == MakerOrderLifecycle::CancelPending {
            if let Some(tgt) = slot.replace_target.clone() {
                if tgt.price > 0.0 && tgt.size > 0.0 {
                    target_price = tgt.price;
                    target_size = tgt.size;
                    if !tgt.origin.trim().is_empty() {
                        target_origin = tgt.origin;
                    }
                }
            }
        }
        let tick_size = Self::_tick_size_from_f64(self.cfg.tick.max(0.0001));
        target_size = Self::_maker_limit_exchange_quantized_size(
            ClobSide::Buy,
            target_price,
            target_size,
            tick_size,
        );
        if target_size <= 0.0 {
            return None;
        }
        if slot.state == MakerOrderLifecycle::SubmitPending
            && now - slot.last_submit_ts < submit_ttl
        {
            return slot.order_id.clone();
        }
        if slot.state == MakerOrderLifecycle::CancelPending
            && now - slot.last_cancel_ts < cancel_ttl
        {
            return None;
        }
        if slot.order_id.is_none()
            && slot.state == MakerOrderLifecycle::Idle
            && reject_cooldown > 0.0
            && slot.last_reject_ts > 0.0
        {
            let max_reject_cooldown =
                env_float("MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS", 60.0).max(reject_cooldown);
            let effective_cooldown = maker_order_effective_reject_cooldown_seconds(
                origin,
                &slot,
                reject_cooldown,
                max_reject_cooldown,
            );
            if now - slot.last_reject_ts < effective_cooldown {
                return None;
            }
        }
        if slot.state != MakerOrderLifecycle::Working
            && slot.state != MakerOrderLifecycle::Idle
            && now - slot.last_submit_ts >= submit_ttl
            && now - slot.last_cancel_ts >= cancel_ttl
        {
            if let Ok(mut slots) = self.maker_order_slots.lock() {
                let s = slots.entry(key.clone()).or_default();
                s.state = MakerOrderLifecycle::Idle;
                s.order_id = None;
                s.remaining = 0.0;
                s.replace_target = None;
            }
            slot = self._maker_order_slot_get(key);
        }

        if slot.state == MakerOrderLifecycle::Working {
            if let Some(oid) = slot.order_id.clone() {
                let old_price = slot.price.max(0.0);
                let old_size = slot.remaining.max(slot.size).max(0.0);
                let age = (now - slot.last_submit_ts).max(0.0);
                let moved_ticks = (target_price - old_price).abs() / self.cfg.tick.max(0.0001);
                let size_changed = old_size <= 0.0
                    || (target_size - old_size).abs() >= (0.25 * old_size).max(self.cfg.min_shares);
                if age < stale && moved_ticks < replace_ticks && !size_changed {
                    return Some(oid);
                }
                let same_refresh_family =
                    maker_refresh_families_match(slot.origin.as_str(), target_origin.as_str());
                let refresh_capped_handoff = self
                    ._bot_runtime_origin_is_refresh_capped(&slot.origin)
                    && self._bot_runtime_origin_is_refresh_capped(target_origin.as_str());
                if refresh_capped_handoff {
                    if let Some(side) = self._maker_side_for_asset_id(key.asset_id.as_str()) {
                        let refresh_reason = if same_refresh_family {
                            "maker_order_replace"
                        } else {
                            "maker_order_handoff_replace"
                        };
                        if let Some(block_reason) = self._bot_runtime_refresh_cycle_block_reason(
                            side,
                            target_origin.as_str(),
                            refresh_reason,
                            now,
                        ) {
                            self.logger.info(&format!(
                                "[MAKER_ORD] refresh capped asset={} side={} origin={} hold_reason={}",
                                key.asset_id,
                                side.as_str(),
                                target_origin,
                                block_reason
                            ));
                            return Some(oid);
                        }
                    }
                } else if !same_refresh_family && now - slot.last_cancel_ts < replace_min {
                    return Some(oid);
                }
                let cancel_requested = if same_refresh_family {
                    self._maker_order_request_cancel_with_policy(key, "maker_order_replace", false)
                } else {
                    self._maker_order_request_cancel_unthrottled(key, "maker_order_handoff_replace")
                };
                if cancel_requested {
                    if refresh_capped_handoff {
                        if let Some(side) = self._maker_side_for_asset_id(key.asset_id.as_str()) {
                            self._bot_runtime_note_refresh_cycle_started(
                                side,
                                target_origin.as_str(),
                                if same_refresh_family {
                                    "maker_order_replace"
                                } else {
                                    "maker_order_handoff_replace"
                                },
                                now,
                            );
                        }
                    }
                    if let Ok(mut slots) = self.maker_order_slots.lock() {
                        if let Some(s) = slots.get_mut(key) {
                            s.replace_target = Some(MakerOrderReplaceTarget {
                                price: target_price,
                                size: target_size,
                                origin: target_origin.clone(),
                            });
                        }
                    }
                }
                return None;
            }
        }

        if slot.state == MakerOrderLifecycle::CancelPending
            && now - slot.last_cancel_ts < cancel_ttl
        {
            return None;
        }

        if let Ok(mut slots) = self.maker_order_slots.lock() {
            let s = slots.entry(key.clone()).or_default();
            s.state = MakerOrderLifecycle::SubmitPending;
            s.last_submit_ts = now;
            s.price = target_price;
            s.size = target_size.max(0.0);
            s.remaining = target_size.max(0.0);
            s.origin = target_origin.clone();
        }
        let gross_snapshot = if target_origin.starts_with("BOT_") && !gross_cap_preapproved {
            let replace_order_ids = slot.order_id.clone().into_iter().collect::<Vec<_>>();
            match self._gross_cap_snapshot(target_price * target_size, &replace_order_ids) {
                Ok(snapshot) => {
                    if let Some(reason) = snapshot.block_reason() {
                        self._gross_cap_reject_submit(
                            reason,
                            key.asset_id.as_str(),
                            "BUY",
                            target_origin.as_str(),
                            snapshot,
                        );
                        if let Ok(mut slots) = self.maker_order_slots.lock() {
                            if let Some(s) = slots.get_mut(key) {
                                s.state = slot.state;
                                s.order_id = slot.order_id.clone();
                                s.price = slot.price;
                                s.size = slot.size;
                                s.remaining = slot.remaining;
                                s.last_submit_ts = slot.last_submit_ts;
                                s.last_cancel_ts = slot.last_cancel_ts;
                                s.origin = slot.origin.clone();
                                s.replace_target = None;
                            }
                        }
                        return None;
                    }
                    Some(snapshot)
                }
                Err(err) => {
                    self._gross_cap_shared_state_error(
                        "maker_single_inflight_submit_gate",
                        err.as_str(),
                    );
                    if let Ok(mut slots) = self.maker_order_slots.lock() {
                        if let Some(s) = slots.get_mut(key) {
                            s.state = slot.state;
                            s.order_id = slot.order_id.clone();
                            s.price = slot.price;
                            s.size = slot.size;
                            s.remaining = slot.remaining;
                            s.last_submit_ts = slot.last_submit_ts;
                            s.last_cancel_ts = slot.last_cancel_ts;
                            s.origin = slot.origin.clone();
                            s.replace_target = None;
                        }
                    }
                    return None;
                }
            }
        } else {
            None
        };
        let oid = self._place_limit_bid_gtc_with_origin_internal(
            &key.asset_id,
            target_price,
            target_size,
            Some(true),
            &target_origin,
            gross_cap_preapproved,
            false,
        );
        if let Some(oid) = oid {
            if !self._maker_order_on_submit_ack(
                &oid,
                key,
                target_price,
                target_size,
                &target_origin,
            ) {
                return None;
            }
            if let Some(snapshot) = gross_snapshot {
                self._gross_cap_record_order_context(&oid, snapshot);
            }
            return Some(oid);
        }
        self._maker_order_on_submit_reject(key, &target_origin, "post_order returned no oid");
        self._maker_order_reconcile_asset(&key.asset_id, Some(price));
        None
    }

    /// Implements payoff envelope for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_payoff_envelope(
        shares_up: f64,
        shares_down: f64,
        cost_total: f64,
    ) -> (f64, f64, f64) {
        let up = shares_up.max(0.0);
        let down = shares_down.max(0.0);
        let cost = cost_total.max(0.0);
        let downside = up.min(down) - cost;
        let upside = up.max(down) - cost;
        let mn = up.min(down);
        let mx = up.max(down);
        let skew_ratio = if mn > 1e-12 { mx / mn } else { f64::INFINITY };
        (downside, upside, skew_ratio)
    }

    /// Implements poly fee formula for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_poly_fee_formula(
        qty: f64,
        price: f64,
        fee_rate: f64,
        exponent: f64,
        maker_rebate_bps: f64,
        is_maker: bool,
        model_enabled: bool,
    ) -> f64 {
        if qty <= 0.0 || price <= 0.0 || !model_enabled {
            return 0.0;
        }
        if is_maker {
            return 0.0;
        }
        let p = clamp(price, 1e-6, 0.999_999);
        let notional = qty * p;
        let taker_fee =
            notional * fee_rate.max(0.0) * (p * (1.0 - p)).powf(exponent.clamp(0.0, 8.0));
        let _ = maker_rebate_bps;
        taker_fee.max(0.0)
    }

    /// Implements ladder cancel all for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_ladder_cancel_all(&self, reason: &str) {
        let orders = self
            .maker_ladder_open_orders
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        if orders.is_empty() {
            return;
        }
        for rec in orders.values() {
            if !rec.order_id.trim().is_empty() {
                let _ = self._cancel(&rec.order_id);
            }
        }
        if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
            m.clear();
        }
        if !reason.trim().is_empty() {
            self.logger
                .info(&format!("[MAKER_SKEW] ladder cleared: {reason}"));
        }
    }

    /// Implements ladder reserved notional for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_ladder_reserved_notional(&self) -> f64 {
        self.maker_ladder_open_orders
            .lock()
            .map(|m| {
                m.values()
                    .map(|o| o.price.max(0.0) * o.size.max(0.0))
                    .sum::<f64>()
            })
            .unwrap_or(0.0)
    }

    /// Implements ladder place or replace for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_ladder_place_or_replace(
        &self,
        key: &str,
        asset_id: &str,
        role: &str,
        level: i64,
        target_price: f64,
        target_size: f64,
    ) {
        if key.trim().is_empty() || asset_id.trim().is_empty() || target_price <= 0.0 {
            return;
        }
        let now = now_ts_f64();
        let stale = env_int("STALE_SECONDS", self.cfg.stale_seconds).max(1) as f64;
        let replace_ticks = env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            self.cfg.replace_if_price_moves_ticks,
        ) as f64;

        let existing = self
            .maker_ladder_open_orders
            .lock()
            .ok()
            .and_then(|m| m.get(key).cloned());
        if let Some(prev) = existing {
            let age = (now - prev.ts).max(0.0);
            let moved_ticks = (target_price - prev.price).abs() / self.cfg.tick.max(0.0001);
            let size_changed =
                (target_size - prev.size).abs() >= (0.25 * prev.size).max(self.cfg.min_shares);
            if age < stale && moved_ticks < replace_ticks && !size_changed {
                return;
            }
            if !prev.order_id.trim().is_empty() {
                let _ = self._cancel(&prev.order_id);
            }
            if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
                m.remove(key);
            }
        }
        let oid = self._place_postonly_bid(asset_id, target_price, target_size);
        let Some(oid) = oid else {
            return;
        };
        if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
            m.insert(
                key.to_string(),
                LadderOrderState {
                    key: key.to_string(),
                    asset_id: asset_id.to_string(),
                    role: role.to_string(),
                    level,
                    order_id: oid,
                    price: target_price,
                    size: target_size,
                    ts: now,
                },
            );
        }
    }

    /// Implements ladder sync role for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_ladder_sync_role(
        &self,
        role: &str,
        asset_id: &str,
        base_bid: f64,
        clip_size: f64,
        levels: i64,
        tick_step: i64,
    ) {
        if levels <= 0 || base_bid <= 0.0 || clip_size <= 0.0 {
            return;
        }
        let tick = self.cfg.tick.max(0.0001);
        let lv = levels.max(1);
        let step = tick_step.max(1) as f64;
        let mut target_prices: Vec<f64> = Vec::new();

        if role.eq_ignore_ascii_case("underdog") {
            let floor = env_float("MAKER_UNDERDOG_FLOOR_PRICE", 0.20).clamp(tick, 0.99);
            for i in 0..lv {
                let mut px = base_bid - (i as f64) * step * tick;
                px = round_down(clamp(px, floor, 0.99), tick);
                if target_prices
                    .last()
                    .map(|p| (p - px).abs() > tick * 0.5)
                    .unwrap_or(true)
                {
                    target_prices.push(px);
                }
                if px <= floor + tick * 0.5 {
                    break;
                }
            }
            if target_prices.is_empty() {
                target_prices.push(round_down(clamp(floor, tick, 0.99), tick));
            }
        } else if role.eq_ignore_ascii_case("hedge") {
            let floor = env_float("MAKER_HEDGE_FLOOR_PRICE", 0.55).clamp(tick, 0.99);
            let span = ((lv - 1) as f64) * step * tick;
            let start = (base_bid.max(floor + span)).clamp(tick, 0.99);
            for i in 0..lv {
                let mut px = start - (i as f64) * step * tick;
                px = round_down(clamp(px, floor, 0.99), tick);
                if target_prices
                    .last()
                    .map(|p| (p - px).abs() > tick * 0.5)
                    .unwrap_or(true)
                {
                    target_prices.push(px);
                }
                if px <= floor + tick * 0.5 {
                    break;
                }
            }
            if target_prices.is_empty() {
                target_prices.push(round_down(clamp(floor, tick, 0.99), tick));
            }
        } else {
            for i in 0..lv {
                let mut px = base_bid - (i as f64) * step * tick;
                px = round_down(clamp(px, tick, 0.99), tick);
                if target_prices
                    .last()
                    .map(|p| (p - px).abs() > tick * 0.5)
                    .unwrap_or(true)
                {
                    target_prices.push(px);
                }
            }
        }

        if self._maker_single_inflight_enabled() {
            self._maker_ladder_cancel_all("single_inflight_guard");
            if let Some(px) = target_prices.first().copied() {
                let key = MakerOrderKey::buy(asset_id);
                let _ = self._maker_order_upsert_gtc(&key, px, clip_size, "MAKER_POSTONLY_GTC");
            }
            return;
        }

        let min_per_order = self.cfg.min_shares.max(1.0);
        let max_levels_by_clip = ((clip_size + 1e-12) / min_per_order).floor().max(1.0) as usize;
        if target_prices.len() > max_levels_by_clip {
            target_prices.truncate(max_levels_by_clip);
        }
        let level_count = target_prices.len().max(1) as f64;
        let per_level = (clip_size / level_count).max(min_per_order);
        let mut desired: HashSet<String> = HashSet::new();
        for (idx, px) in target_prices.into_iter().enumerate() {
            if px <= 0.0 {
                continue;
            }
            let i = idx as i64;
            let key = format!("{role}:{asset_id}:{i}");
            desired.insert(key.clone());
            self._maker_ladder_place_or_replace(&key, asset_id, role, i, px, per_level);
        }

        let stale_keys: Vec<String> = self
            .maker_ladder_open_orders
            .lock()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| {
                        if v.role == role && v.asset_id == asset_id && !desired.contains(k) {
                            Some(k.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        if stale_keys.is_empty() {
            return;
        }
        for key in stale_keys {
            let mut rec = None;
            if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
                rec = m.remove(&key);
            }
            if let Some(r) = rec {
                let _ = self._cancel(&r.order_id);
            }
        }
    }

    /// Implements ladder cancel except role asset for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_ladder_cancel_except_role_asset(
        &self,
        keep_role: &str,
        keep_asset_id: &str,
    ) {
        let stale_keys: Vec<String> = self
            .maker_ladder_open_orders
            .lock()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| {
                        if v.role == keep_role && v.asset_id == keep_asset_id {
                            None
                        } else {
                            Some(k.clone())
                        }
                    })
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        for key in stale_keys {
            let mut rec = None;
            if let Ok(mut m) = self.maker_ladder_open_orders.lock() {
                rec = m.remove(&key);
            }
            if let Some(r) = rec {
                let _ = self._cancel(&r.order_id);
            }
        }
    }

    /// Implements compute rsi for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_compute_rsi(closes: &[f64], period: usize) -> Option<f64> {
        if closes.len() <= period || period < 2 {
            return None;
        }
        let mut gain = 0.0;
        let mut loss = 0.0;
        for i in (closes.len() - period)..closes.len() {
            if i == 0 {
                continue;
            }
            let d = closes[i] - closes[i - 1];
            if d > 0.0 {
                gain += d;
            } else {
                loss += -d;
            }
        }
        let avg_gain = gain / period as f64;
        let avg_loss = loss / period as f64;
        if avg_loss <= 1e-12 {
            Some(100.0)
        } else {
            let rs = avg_gain / avg_loss;
            Some(100.0 - (100.0 / (1.0 + rs)))
        }
    }

    /// Implements stretch bias side for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_stretch_bias_side(&self, default_side: &str) -> String {
        default_side.trim().to_ascii_uppercase()
    }

    /// Implements submit pair orders for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_submit_pair_orders(
        &self,
        size_int: i64,
        y_px: f64,
        n_px: f64,
        order_type: &str,
        post_only: Option<bool>,
        origin: &str,
    ) -> (Option<String>, Option<String>) {
        if size_int <= 0 {
            return (None, None);
        }
        let qty = size_int as f64;
        let tick_size = Self::_tick_size_from_f64(self.cfg.tick.max(0.0001));
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return (None, None),
        };
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let resolved = self._resolve_order_type(order_type);
        let track_taker_fallback = pair_submit_tracks_taker_fallback(&resolved);
        let use_limit_precision = matches!(resolved.as_str(), "GTC" | "GTD");
        let y_qty = if use_limit_precision {
            Self::_maker_limit_exchange_quantized_size(ClobSide::Buy, y_px, qty, tick_size)
        } else {
            qty
        };
        let n_qty = if use_limit_precision {
            Self::_maker_limit_exchange_quantized_size(ClobSide::Buy, n_px, qty, tick_size)
        } else {
            qty
        };
        if y_qty <= 0.0 || n_qty <= 0.0 {
            return (None, None);
        }
        let (yes_pair_preview, no_pair_preview) = if resolved == "GTC" {
            (
                self._maker_pair_gross_preview_gtc_leg(
                    yes,
                    y_px,
                    y_qty,
                    &format!("{origin}_YES"),
                    decide_ts,
                ),
                self._maker_pair_gross_preview_gtc_leg(
                    no,
                    n_px,
                    n_qty,
                    &format!("{origin}_NO"),
                    decide_ts,
                ),
            )
        } else {
            (
                MakerPairGrossPreview {
                    requested_gross_usd: y_px * y_qty,
                    replace_order_ids: Vec::new(),
                },
                MakerPairGrossPreview {
                    requested_gross_usd: n_px * n_qty,
                    replace_order_ids: Vec::new(),
                },
            )
        };
        let pair_gross_snapshot = if origin.starts_with("BOT_") {
            let mut pair_preview = yes_pair_preview.clone();
            pair_preview.requested_gross_usd += no_pair_preview.requested_gross_usd;
            pair_preview
                .replace_order_ids
                .extend(no_pair_preview.replace_order_ids.clone());
            if pair_preview.requested_gross_usd <= 1e-9 {
                None
            } else {
                match self._gross_cap_snapshot(
                    pair_preview.requested_gross_usd,
                    &pair_preview.replace_order_ids,
                ) {
                    Ok(snapshot) => {
                        if let Some(reason) = snapshot.block_reason() {
                            self._gross_cap_reject_submit(
                                reason,
                                self.pair_identity().pair_id.as_str(),
                                "BUY",
                                origin,
                                snapshot,
                            );
                            return (None, None);
                        }
                        Some(snapshot)
                    }
                    Err(err) => {
                        self._gross_cap_shared_state_error("maker_pair_submit_gate", err.as_str());
                        return (None, None);
                    }
                }
            }
        } else {
            None
        };
        let (mut y_oid, mut n_oid) = if resolved == "GTC" {
            if self._maker_single_inflight_enabled() {
                let y_key = MakerOrderKey::buy(yes);
                let n_key = MakerOrderKey::buy(no);
                let yes_gross_cap_preapproved =
                    pair_gross_snapshot.is_some() && yes_pair_preview.requested_gross_usd > 1e-9;
                let y_oid = self._maker_order_upsert_gtc_internal(
                    &y_key,
                    y_px,
                    y_qty,
                    &format!("{origin}_YES"),
                    yes_gross_cap_preapproved,
                );
                let yes_preview_consumed = yes_pair_preview.requested_gross_usd <= 1e-9
                    || y_oid
                        .as_ref()
                        .map(|oid| !self._refresh_cadence_noop_marker_active(oid))
                        .unwrap_or(false);
                let no_gross_cap_preapproved = pair_gross_snapshot.is_some()
                    && no_pair_preview.requested_gross_usd > 1e-9
                    && yes_preview_consumed;
                let n_oid = self._maker_order_upsert_gtc_internal(
                    &n_key,
                    n_px,
                    n_qty,
                    &format!("{origin}_NO"),
                    no_gross_cap_preapproved,
                );
                (y_oid, n_oid)
            } else {
                let yes_gross_cap_preapproved =
                    pair_gross_snapshot.is_some() && yes_pair_preview.requested_gross_usd > 1e-9;
                let y_oid = self._place_limit_bid_gtc_with_origin_internal(
                    yes,
                    y_px,
                    y_qty,
                    post_only,
                    &format!("{origin}_YES"),
                    yes_gross_cap_preapproved,
                    true,
                );
                let yes_preview_consumed = yes_pair_preview.requested_gross_usd <= 1e-9
                    || y_oid
                        .as_ref()
                        .map(|oid| !self._refresh_cadence_noop_marker_active(oid))
                        .unwrap_or(false);
                let no_gross_cap_preapproved = pair_gross_snapshot.is_some()
                    && no_pair_preview.requested_gross_usd > 1e-9
                    && yes_preview_consumed;
                let n_oid = self._place_limit_bid_gtc_with_origin_internal(
                    no,
                    n_px,
                    n_qty,
                    post_only,
                    &format!("{origin}_NO"),
                    no_gross_cap_preapproved,
                    true,
                );
                (y_oid, n_oid)
            }
        } else {
            let signed_y = json!({
                "asset_id": yes,
                "side": "BUY",
                "price": y_px,
                "size": y_qty,
                "origin": format!("{origin}_YES"),
            });
            let signed_n = json!({
                "asset_id": no,
                "side": "BUY",
                "price": n_px,
                "size": n_qty,
                "origin": format!("{origin}_NO"),
            });
            let resps = self._post_orders_compat(&[signed_y, signed_n], &resolved, post_only);
            (
                resps.first().and_then(|o| o.clone()),
                resps.get(1).and_then(|o| o.clone()),
            )
        };
        if let Some(oid) = y_oid.clone() {
            let mut keep_yes_oid = true;
            if track_taker_fallback {
                if !self._remember_taker_order(
                    oid.as_str(),
                    yes,
                    y_qty,
                    y_px,
                    "BUY",
                    LiquidityIntent::TakerException,
                    None,
                    TakerCapPolicy::EnforceCap,
                ) {
                    keep_yes_oid = false;
                }
            } else {
                self._forget_taker_order(oid.as_str());
            }
            if keep_yes_oid && !self._refresh_cadence_noop_marker_active(oid.as_str()) {
                self._track_order_execution_context(
                    oid.as_str(),
                    &json!({
                        "order_id": oid,
                        "asset_id": yes,
                        "side": "BUY",
                        "px_limit": y_px,
                        "size": y_qty,
                        "decision_ts": decide_ts,
                        "decision_ns": decide_ns,
                        "post_start_ts": decide_ts,
                        "post_end_ts": now_ts_f64(),
                        "origin": format!("{origin}_YES"),
                        "liquidity_intent": if track_taker_fallback {
                            LiquidityIntent::TakerException.as_str()
                        } else {
                            LiquidityIntent::Maker.as_str()
                        },
                        "taker_exception_reason": Option::<&str>::None,
                        "taker_cap_policy": if track_taker_fallback {
                            Some(TakerCapPolicy::EnforceCap.as_str())
                        } else {
                            None
                        },
                    }),
                );
                if let Some(snapshot) = pair_gross_snapshot {
                    self._gross_cap_record_order_context(oid.as_str(), snapshot);
                }
            }
            if !keep_yes_oid {
                y_oid = None;
            }
        }
        if let Some(oid) = n_oid.clone() {
            let mut keep_no_oid = true;
            if track_taker_fallback {
                if !self._remember_taker_order(
                    oid.as_str(),
                    no,
                    n_qty,
                    n_px,
                    "BUY",
                    LiquidityIntent::TakerException,
                    None,
                    TakerCapPolicy::EnforceCap,
                ) {
                    keep_no_oid = false;
                }
            } else {
                self._forget_taker_order(oid.as_str());
            }
            if keep_no_oid && !self._refresh_cadence_noop_marker_active(oid.as_str()) {
                self._track_order_execution_context(
                    oid.as_str(),
                    &json!({
                        "order_id": oid,
                        "asset_id": no,
                        "side": "BUY",
                        "px_limit": n_px,
                        "size": n_qty,
                        "decision_ts": decide_ts,
                        "decision_ns": decide_ns,
                        "post_start_ts": decide_ts,
                        "post_end_ts": now_ts_f64(),
                        "origin": format!("{origin}_NO"),
                        "liquidity_intent": if track_taker_fallback {
                            LiquidityIntent::TakerException.as_str()
                        } else {
                            LiquidityIntent::Maker.as_str()
                        },
                        "taker_exception_reason": Option::<&str>::None,
                        "taker_cap_policy": if track_taker_fallback {
                            Some(TakerCapPolicy::EnforceCap.as_str())
                        } else {
                            None
                        },
                    }),
                );
                if let Some(snapshot) = pair_gross_snapshot {
                    self._gross_cap_record_order_context(oid.as_str(), snapshot);
                }
            }
            if !keep_no_oid {
                n_oid = None;
            }
        }
        (y_oid, n_oid)
    }
}
