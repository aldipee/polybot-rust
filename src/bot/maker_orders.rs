use super::*;

impl MakerHedgeCapBot {
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
    ) {
        if !self._maker_single_inflight_enabled() || order_id.trim().is_empty() {
            return;
        }
        let trimmed_oid = order_id.trim();
        let trimmed_origin = origin.trim();
        if let Ok(mut ledger) = self.maker_exec_ledger.lock() {
            let entry = ledger
                .per_order_origin
                .entry(trimmed_oid.to_string())
                .or_default();
            let existing = entry.trim();
            let can_upgrade_reconcile =
                existing == "RECONCILE" && !trimmed_origin.is_empty() && trimmed_origin != "RECONCILE";
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
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    key.asset_id.clone(),
                    OpenOrderState {
                        order_id: Some(order_id.to_string()),
                        price: Some(price),
                        size: Some(size.max(0.0)),
                        ts: Some(now),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
        }
    }

    /// Implements order on submit reject for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_on_submit_reject(&self, key: &MakerOrderKey, origin: &str, reason: &str) {
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
        if now - slot.last_cancel_ts < self._maker_replace_min_interval_seconds() {
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

    /// Implements order cancel all except asset for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_order_cancel_all_except_asset(&self, keep_asset_id: Option<&str>, reason: &str) {
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
            let _ = self._maker_order_request_cancel(&key, reason);
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
        if !self._maker_single_inflight_enabled() || self.cfg.dry_run {
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
            let price = self._extract_order_price(&order);
            let remaining = self._extract_order_remaining_size(&order).max(0.0);
            let size = remaining.max(0.0);
            let reconcile_origin = self._maker_reconcile_inferred_origin(&key);
            self._maker_order_on_submit_ack(&oid, &key, price, size, &reconcile_origin);
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
                let _ = save_state(&self.state_file, &mut s);
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
                    let _ = save_state(&self.state_file, &mut s);
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
                s.open_orders.insert(
                    asset_id,
                    OpenOrderState {
                        order_id: Some(oid),
                        price: Some(price),
                        size: Some(remaining),
                        ts: Some(now_ts_f64()),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
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
        if key.asset_id.trim().is_empty() || key.side != "BUY" {
            return None;
        }
        if !self._maker_single_inflight_enabled() {
            return self._place_limit_bid_gtc_with_origin(&key.asset_id, price, size, Some(true), origin);
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
            let max_reject_cooldown = env_float("MAKER_SUBMIT_REJECT_MAX_COOLDOWN_SECONDS", 60.0)
                .max(reject_cooldown);
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
                if now - slot.last_cancel_ts < replace_min {
                    return None;
                }
                if self._maker_order_request_cancel(key, "maker_order_replace") {
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
        let oid = self._place_limit_bid_gtc_with_origin(
            &key.asset_id,
            target_price,
            target_size,
            Some(true),
            &target_origin,
        );
        if let Some(oid) = oid {
            self._maker_order_on_submit_ack(&oid, key, target_price, target_size, &target_origin);
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

    pub(super) fn _maker_ladder_cancel_except_role_asset(&self, keep_role: &str, keep_asset_id: &str) {
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
        let (y_oid, n_oid) = if resolved == "GTC" && self._maker_single_inflight_enabled() {
            let y_key = MakerOrderKey::buy(yes);
            let n_key = MakerOrderKey::buy(no);
            let y_oid =
                self._maker_order_upsert_gtc(&y_key, y_px, y_qty, &format!("{origin}_YES"));
            let n_oid =
                self._maker_order_upsert_gtc(&n_key, n_px, n_qty, &format!("{origin}_NO"));
            (y_oid, n_oid)
        } else {
            let signed_y = json!({
                "asset_id": yes,
                "side": "BUY",
                "price": y_px,
                "size": y_qty,
            });
            let signed_n = json!({
                "asset_id": no,
                "side": "BUY",
                "price": n_px,
                "size": n_qty,
            });
            let resps = self._post_orders_compat(&[signed_y, signed_n], &resolved, post_only);
            (
                resps.first().and_then(|o| o.clone()),
                resps.get(1).and_then(|o| o.clone()),
            )
        };
        if let Some(oid) = &y_oid {
            if track_taker_fallback {
                self._remember_taker_order(oid, yes, y_qty, y_px, "BUY");
            } else {
                self._forget_taker_order(oid);
            }
            self._track_order_execution_context(
                oid,
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
                }),
            );
        }
        if let Some(oid) = &n_oid {
            if track_taker_fallback {
                self._remember_taker_order(oid, no, n_qty, n_px, "BUY");
            } else {
                self._forget_taker_order(oid);
            }
            self._track_order_execution_context(
                oid,
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
                }),
            );
        }
        (y_oid, n_oid)
    }
}

