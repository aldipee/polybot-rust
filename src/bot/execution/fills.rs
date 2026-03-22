use super::*;
impl MakerHedgeCapBot {
    /// Parses an exchange/event fill timestamp into epoch seconds for the active BOT execution
    /// path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _fill_event_ts_from_value(&self, value: Option<&Value>) -> Option<f64> {
        let raw = match value? {
            Value::Number(number) => number.as_f64()?,
            Value::String(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return None;
                }
                if let Ok(parsed) = trimmed.parse::<f64>() {
                    parsed
                } else if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
                    parsed.timestamp_millis() as f64 / 1000.0
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        if !raw.is_finite() || raw <= 0.0 {
            return None;
        }
        let ts = if raw >= 10_000_000_000.0 {
            raw / 1000.0
        } else {
            raw
        };
        if ts.is_finite() && ts > 0.0 {
            Some(ts)
        } else {
            None
        }
    }

    /// Records the wallet-global daily liquidity counters for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _record_daily_liquidity_fill_global(
        &self,
        filled: f64,
        is_maker: bool,
        fill_ts: Option<f64>,
    ) {
        let qty = filled.max(0.0);
        if qty <= 1e-9 {
            return;
        }
        let resolved_fill_ts = fill_ts
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or_else(now_ts_f64);
        let day_key = crate::helpers::utc_day_key_from_ts(resolved_fill_ts);
        let state = if let Ok(_lock) = crate::helpers::acquire_companion_file_lock(
            &self.daily_liquidity_state_file,
            MakerHedgeCapBot::shared_state_lock_timeout(),
        ) {
            let mut state = load_daily_liquidity_state(&self.daily_liquidity_state_file)
                .unwrap_or_else(|_| {
                    self.daily_liquidity_state
                        .lock()
                        .map(|state| state.clone())
                        .unwrap_or_default()
                });
            state.record_fill(qty, is_maker, day_key.as_str());
            if let Err(err) =
                save_daily_liquidity_state(&self.daily_liquidity_state_file, &mut state)
            {
                self.logger.warning(&format!(
                    "[BOT][SAFE_PAUSE] daily_liquidity_persist_failed err={:#}",
                    err
                ));
                self._bot_runtime_enter_dependency_pause(
                    "database",
                    "daily_liquidity",
                    now_ts_f64(),
                );
            }
            state
        } else {
            let mut state = self
                .daily_liquidity_state
                .lock()
                .map(|state| state.clone())
                .unwrap_or_default();
            state.record_fill(qty, is_maker, day_key.as_str());
            state
        };
        let state_day_key = state.day_key_utc.clone();
        let maker_qty = state.maker_fill_shares.max(0.0);
        let taker_qty = state.taker_fill_shares.max(0.0);
        if let Ok(mut shared_state) = self.daily_liquidity_state.lock() {
            *shared_state = state;
        }
        if let Ok(mut runtime_state) = self.bot_runtime_state.lock() {
            runtime_state.daily_taker_day_key_utc = state_day_key;
            runtime_state.daily_maker_fill_shares = maker_qty;
            runtime_state.daily_taker_fill_shares = taker_qty;
        }
    }

    pub(in crate::bot) fn _remember_shared_pending_taker_order(
        &self,
        order_id: &str,
        asset_id: &str,
        size: f64,
        applied: f64,
        side: &str,
        ts: f64,
    ) {
        let state_file = self._pending_taker_state_file();
        let Ok(_lock) = crate::helpers::acquire_companion_file_lock(
            &state_file,
            MakerHedgeCapBot::shared_state_lock_timeout(),
        ) else {
            return;
        };
        let mut state = crate::helpers::load_shared_pending_taker_state(
            &state_file,
            self.taker_order_ttl_seconds as f64,
        )
        .unwrap_or_default();
        let pair_id = self.pair_identity().pair_id;
        state.remember_order(
            order_id,
            pair_id.as_str(),
            asset_id,
            side,
            size,
            applied,
            ts,
        );
        let _ = crate::helpers::save_shared_pending_taker_state(&state_file, &mut state);
    }

    pub(in crate::bot) fn _update_shared_pending_taker_order_applied(
        &self,
        order_id: &str,
        applied: f64,
    ) {
        let state_file = self._pending_taker_state_file();
        let Ok(_lock) = crate::helpers::acquire_companion_file_lock(
            &state_file,
            MakerHedgeCapBot::shared_state_lock_timeout(),
        ) else {
            return;
        };
        let mut state = crate::helpers::load_shared_pending_taker_state(
            &state_file,
            self.taker_order_ttl_seconds as f64,
        )
        .unwrap_or_default();
        state.update_applied(order_id, applied, now_ts_f64());
        let _ = crate::helpers::save_shared_pending_taker_state(&state_file, &mut state);
    }

    pub(in crate::bot) fn _forget_shared_pending_taker_order(&self, order_id: &str) {
        let state_file = self._pending_taker_state_file();
        let Ok(_lock) = crate::helpers::acquire_companion_file_lock(
            &state_file,
            MakerHedgeCapBot::shared_state_lock_timeout(),
        ) else {
            return;
        };
        let mut state = crate::helpers::load_shared_pending_taker_state(
            &state_file,
            self.taker_order_ttl_seconds as f64,
        )
        .unwrap_or_default();
        state.forget_order(order_id);
        let _ = crate::helpers::save_shared_pending_taker_state(&state_file, &mut state);
    }
    /// Returns or derives OCO after maker fill for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _oco_after_maker_fill(&self, filled_qty_total: f64) -> bool {
        if env_bool("OCO_ON_FILL", true) && filled_qty_total > 0.0 {
            self.cancel_all_open_orders_local("oco_after_fill");
            return true;
        }
        false
    }
    /// Applies fill to the current BOT state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _apply_fill(
        &self,
        asset_id: &str,
        price: f64,
        filled: f64,
        trade_key: &str,
        side: &str,
    ) -> bool {
        self._apply_fill_with_fill_ts(asset_id, price, filled, trade_key, side, None, None, None)
    }

    pub(in crate::bot) fn _apply_fill_with_fill_ts(
        &self,
        asset_id: &str,
        price: f64,
        filled: f64,
        trade_key: &str,
        side: &str,
        fill_ts: Option<f64>,
        order_id: Option<&str>,
        origin: Option<&str>,
    ) -> bool {
        let side_u = side.trim().to_ascii_uppercase();
        if !matches!(side_u.as_str(), "BUY" | "SELL") {
            return false;
        }
        if filled <= 0.0 || price <= 0.0 || trade_key.trim().is_empty() {
            return false;
        }
        let mut guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if guard.has_seen_trade_key(trade_key) {
            return false;
        }
        guard.record_seen_trade_key(trade_key, fill_ts.unwrap_or_else(now_ts_f64));
        let yes_asset = self.yes_asset.as_deref().unwrap_or_default();
        let sign = if side_u == "BUY" { 1.0 } else { -1.0 };
        let qty = sign * filled;
        let qty_before = guard.q_yes + guard.q_no;
        if asset_id == yes_asset {
            guard.q_yes = (guard.q_yes + qty).max(0.0);
            guard.c_yes = (guard.c_yes + price * qty).max(0.0);
        } else if self.no_asset.as_deref() == Some(asset_id) {
            guard.q_no = (guard.q_no + qty).max(0.0);
            guard.c_no = (guard.c_no + price * qty).max(0.0);
        } else {
            return false;
        }
        let qty_after = guard.q_yes + guard.q_no;
        let opened_position = side_u == "BUY" && qty_before <= 1e-12 && qty_after > 1e-12;
        let closed_position = qty_after <= 1e-12;
        let mark_first_entry_fill = side_u == "BUY" && qty_after > qty_before + 1e-12;
        guard.record_pair_liquidity_fill(filled, false);
        let _ = self._bot_runtime_save_state_or_dependency_pause(&mut guard, "apply_fill");
        drop(guard);
        if let Some(order_id) = order_id {
            if side_u == "BUY" {
                self._add_shared_gross_order_applied(order_id, filled);
            } else {
                self._forget_shared_gross_order_reservation(order_id);
            }
        }
        let _ = self._refresh_shared_gross_trade_snapshot();
        self._record_daily_liquidity_fill_global(filled, false, fill_ts);
        // Clear seed in-flight cooldown on any fill ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â allows immediate re-seeding
        // of the other side instead of waiting for the hardcoded timeout.
        self._runtime_ts_set("__maker_skew_seed_inflight_until", 0.0);
        let mut opened_reason: Option<String> = None;
        if opened_position {
            let reason = self
                ._take_pending_entry_reason()
                .unwrap_or_else(|| self._default_entry_reason());
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = Some(reason.clone());
            }
            opened_reason = Some(reason);
        } else if closed_position {
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = None;
            }
        }
        if mark_first_entry_fill {
            let fill_ts = crate::db::now_iso_jakarta();
            if let Ok(mut first) = self.first_entry_fill_iso.lock() {
                if first.is_none() {
                    *first = Some(fill_ts);
                }
            }
            if let Ok(mut first_reason) = self.first_entry_reason.lock() {
                if first_reason.is_none() {
                    let reason = opened_reason.unwrap_or_else(|| {
                        self._take_pending_entry_reason()
                            .unwrap_or_else(|| self._default_entry_reason())
                    });
                    *first_reason = Some(reason);
                }
            }
        }
        self._bot_runtime_note_observed_fill(asset_id, filled, false, side, order_id, origin);
        self._audit_record_fill_event(
            order_id, asset_id, side, price, filled, false, fill_ts, origin,
        );
        true
    }
    /// Applies fill locked nodedupe to the current BOT state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _apply_fill_locked_nodedupe(
        &self,
        guard: &mut BotState,
        asset_id: &str,
        price: f64,
        filled: f64,
        side: &str,
    ) -> Option<ApplyFillMutationMeta> {
        let side_u = side.trim().to_ascii_uppercase();
        if !matches!(side_u.as_str(), "BUY" | "SELL") {
            return None;
        }
        if filled <= 0.0 || price <= 0.0 {
            return None;
        }
        let yes_asset = self.yes_asset.as_deref().unwrap_or_default();
        let sign = if side_u == "BUY" { 1.0 } else { -1.0 };
        let qty = sign * filled;
        let qty_before = guard.q_yes + guard.q_no;
        if asset_id == yes_asset {
            guard.q_yes = (guard.q_yes + qty).max(0.0);
            guard.c_yes = (guard.c_yes + price * qty).max(0.0);
        } else if self.no_asset.as_deref() == Some(asset_id) {
            guard.q_no = (guard.q_no + qty).max(0.0);
            guard.c_no = (guard.c_no + price * qty).max(0.0);
        } else {
            return None;
        }
        let qty_after = guard.q_yes + guard.q_no;
        Some(ApplyFillMutationMeta {
            opened_position: side_u == "BUY" && qty_before <= 1e-12 && qty_after > 1e-12,
            closed_position: qty_after <= 1e-12,
            mark_first_entry_fill: side_u == "BUY" && qty_after > qty_before + 1e-12,
        })
    }
    /// Applies fill finalize to the current BOT state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _apply_fill_finalize(&self, meta: ApplyFillMutationMeta) {
        let ApplyFillMutationMeta {
            opened_position,
            closed_position,
            mark_first_entry_fill,
        } = meta;
        // Clear seed in-flight cooldown on any fill; allows immediate re-seeding
        // of the other side instead of waiting for the hardcoded timeout.
        self._runtime_ts_set("__maker_skew_seed_inflight_until", 0.0);
        let mut opened_reason: Option<String> = None;
        if opened_position {
            let reason = self
                ._take_pending_entry_reason()
                .unwrap_or_else(|| self._default_entry_reason());
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = Some(reason.clone());
            }
            opened_reason = Some(reason);
        } else if closed_position {
            if let Ok(mut active_reason) = self.active_entry_reason.lock() {
                *active_reason = None;
            }
        }
        if mark_first_entry_fill {
            let fill_ts = crate::db::now_iso_jakarta();
            if let Ok(mut first) = self.first_entry_fill_iso.lock() {
                if first.is_none() {
                    *first = Some(fill_ts);
                }
            }
            if let Ok(mut first_reason) = self.first_entry_reason.lock() {
                if first_reason.is_none() {
                    let reason = opened_reason.unwrap_or_else(|| {
                        self._take_pending_entry_reason()
                            .unwrap_or_else(|| self._default_entry_reason())
                    });
                    *first_reason = Some(reason);
                }
            }
        }
    }
    /// Stores taker order in the BOT''s runtime bookkeeping.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _remember_taker_order(
        &self,
        order_id: &str,
        asset_id: &str,
        size: f64,
        px_limit: f64,
        side: &str,
        liquidity_intent: LiquidityIntent,
        taker_exception_reason: Option<TakerExceptionReason>,
        taker_cap_policy: TakerCapPolicy,
    ) -> bool {
        if order_id.trim().is_empty() {
            return false;
        }
        let now = now_ts_f64();
        let rec = TakerOrderRecord {
            order_id: order_id.to_string(),
            asset_id: asset_id.to_string(),
            size,
            applied: 0.0,
            px_limit,
            side: side.to_ascii_uppercase(),
            ts: now,
            liquidity_intent,
            taker_exception_reason,
            taker_cap_policy,
        };
        if let Ok(mut m) = self.taker_orders.lock() {
            m.insert(order_id.to_string(), rec);
        }
        if side.eq_ignore_ascii_case("BUY") {
            if let Ok(mut state) = self.state.lock() {
                let submit_ts = state
                    .open_orders
                    .get(asset_id)
                    .filter(|row| row.order_id.as_deref() == Some(order_id))
                    .and_then(|row| row.submit_ts.or(row.ts))
                    .unwrap_or(now);
                state.open_orders.insert(
                    asset_id.to_string(),
                    OpenOrderState {
                        order_id: Some(order_id.to_string()),
                        price: Some(px_limit),
                        size: Some(size),
                        ts: Some(now),
                        submit_ts: Some(submit_ts),
                        kind: Some("taker".to_string()),
                    },
                );
                let _ = self._bot_runtime_save_state_or_dependency_pause(
                    &mut state,
                    "remember_taker_open_order",
                );
            }
        }
        self._remember_shared_pending_taker_order(order_id, asset_id, size, 0.0, side, now);
        if side.eq_ignore_ascii_case("BUY") {
            if !self._remember_shared_gross_order_reservation(
                order_id,
                asset_id,
                side,
                px_limit,
                size,
                liquidity_intent.as_str(),
                "taker",
            ) {
                self.logger.warning(&format!(
                    "[BOT][SAFE_PAUSE] shared gross reservation publish failed for taker BUY order_id={} asset={} -> canceling order",
                    order_id,
                    asset_id
                ));
                if self._cancel(order_id) {
                    self._forget_taker_order(order_id);
                    return false;
                }
                self.logger.warning(&format!(
                    "[BOT][SAFE_PAUSE] failed to cancel taker BUY after shared gross reservation publish failure order_id={} asset={}; keeping local taker tracking while dependency pause is active",
                    order_id,
                    asset_id
                ));
            }
        }
        true
    }
    /// Removes taker order from the BOT''s runtime bookkeeping.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _forget_taker_order(&self, order_id: &str) {
        if order_id.trim().is_empty() {
            return;
        }
        let removed = if let Ok(mut m) = self.taker_orders.lock() {
            m.remove(order_id)
        } else {
            None
        };
        if let Ok(mut state) = self.state.lock() {
            let remove_asset = removed
                .as_ref()
                .map(|record| record.asset_id.clone())
                .or_else(|| {
                    state.open_orders.iter().find_map(|(asset_id, row)| {
                        (row.order_id.as_deref() == Some(order_id)
                            && row.kind.as_deref() == Some("taker"))
                        .then(|| asset_id.clone())
                    })
                });
            if let Some(asset_id) = remove_asset {
                let should_remove = state
                    .open_orders
                    .get(asset_id.as_str())
                    .map(|row| {
                        row.order_id.as_deref() == Some(order_id)
                            && row.kind.as_deref() == Some("taker")
                    })
                    .unwrap_or(false);
                if should_remove {
                    state.open_orders.remove(asset_id.as_str());
                    let _ = self._bot_runtime_save_state_or_dependency_pause(
                        &mut state,
                        "forget_taker_open_order",
                    );
                }
            }
        }
        self._forget_shared_pending_taker_order(order_id);
    }
    /// Returns whether recent taker order is true for the current BOT context.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _is_recent_taker_order(&self, order_id: &str) -> bool {
        let ttl = self.taker_order_ttl_seconds as f64;
        self.taker_orders
            .lock()
            .ok()
            .and_then(|m| m.get(order_id).cloned())
            .map(|r| now_ts_f64() - r.ts <= ttl)
            .unwrap_or(false)
    }
    /// Returns whether the BOT currently has pending taker order.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _has_pending_taker_order(&self, side: &str, asset_id: Option<&str>) -> bool {
        let s = side.to_ascii_uppercase();
        self.taker_orders
            .lock()
            .map(|m| {
                m.values().any(|r| {
                    let remaining = (r.size - r.applied).max(0.0);
                    r.side == s
                        && asset_id
                            .map(|aid| aid == r.asset_id.as_str())
                            .unwrap_or(true)
                        && remaining > 1e-9
                        && now_ts_f64() - r.ts <= self.taker_order_ttl_seconds as f64
                })
            })
            .unwrap_or(false)
    }
    /// Returns or derives pending taker notional USD for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _pending_taker_notional_usd(&self, side: &str, asset_id: Option<&str>) -> f64 {
        let s = side.to_ascii_uppercase();
        self.taker_orders
            .lock()
            .map(|m| {
                m.values()
                    .filter(|r| {
                        let remaining = (r.size - r.applied).max(0.0);
                        r.side == s
                            && asset_id
                                .map(|aid| aid == r.asset_id.as_str())
                                .unwrap_or(true)
                            && remaining > 1e-9
                            && now_ts_f64() - r.ts <= self.taker_order_ttl_seconds as f64
                    })
                    .map(|r| (r.size - r.applied).max(0.0) * r.px_limit)
                    .sum()
            })
            .unwrap_or(0.0)
    }

    /// Returns or derives pending taker quantity for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _pending_taker_qty(&self, side: Option<&str>, asset_id: Option<&str>) -> f64 {
        let state_file = self._pending_taker_state_file();
        let Ok(_lock) = crate::helpers::acquire_companion_file_lock(
            &state_file,
            MakerHedgeCapBot::shared_state_lock_timeout(),
        ) else {
            return 0.0;
        };
        crate::helpers::load_shared_pending_taker_state(
            &state_file,
            self.taker_order_ttl_seconds as f64,
        )
        .map(|state| state.pending_qty(side, asset_id, self.taker_order_ttl_seconds as f64))
        .unwrap_or(0.0)
    }

    /// Returns or derives pending taker quantity for the current pair only.
    /// This reads the shared pending taker registry for the active BOT execution path.

    pub(in crate::bot) fn _pending_taker_qty_for_current_pair(&self, side: Option<&str>) -> f64 {
        let pair = self.pair_identity();
        let state_file = self._pending_taker_state_file();
        let Ok(_lock) = crate::helpers::acquire_companion_file_lock(
            &state_file,
            MakerHedgeCapBot::shared_state_lock_timeout(),
        ) else {
            return 0.0;
        };
        crate::helpers::load_shared_pending_taker_state(
            &state_file,
            self.taker_order_ttl_seconds as f64,
        )
        .map(|state| {
            state.pending_qty_for_pair(
                pair.pair_id.as_str(),
                pair.yes_asset_id.as_deref(),
                pair.no_asset_id.as_deref(),
                side,
                self.taker_order_ttl_seconds as f64,
            )
        })
        .unwrap_or(0.0)
    }
    /// Returns whether the BOT currently has pending taker order recent.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _has_pending_taker_order_recent(
        &self,
        side: &str,
        asset_id: Option<&str>,
        max_age_seconds: f64,
    ) -> bool {
        let s = side.to_ascii_uppercase();
        self.taker_orders
            .lock()
            .map(|m| {
                m.values().any(|r| {
                    let remaining = (r.size - r.applied).max(0.0);
                    r.side == s
                        && asset_id
                            .map(|aid| aid == r.asset_id.as_str())
                            .unwrap_or(true)
                        && remaining > 1e-9
                        && now_ts_f64() - r.ts <= max_age_seconds
                })
            })
            .unwrap_or(false)
    }
    /// Returns position size data API from the current BOT context.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _get_position_size_data_api(&self, asset_id: &str) -> Option<f64> {
        let aid = asset_id.trim();
        if aid.is_empty() {
            return None;
        }
        let base = std::env::var("POLY_DATA_API_BASE_URL")
            .unwrap_or_else(|_| "https://data-api.polymarket.com".to_string());
        let url = format!("{}/positions", base.trim_end_matches('/'));
        let timeout_s = env_float("POSITIONS_API_TIMEOUT_SECONDS", 3.0).clamp(0.2, 15.0);
        let http = match Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
        {
            Ok(c) => c,
            Err(_) => return None,
        };
        let mut users: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let override_user = std::env::var("POSITIONS_API_USER").unwrap_or_default();
        for cand in [
            override_user,
            self.cfg.funder.clone().unwrap_or_default(),
            self.wallet_address.clone(),
        ] {
            let t = cand.trim().to_string();
            if t.is_empty() {
                continue;
            }
            let key = t.to_ascii_lowercase();
            if seen.insert(key) {
                users.push(t);
            }
        }
        if users.is_empty() {
            return None;
        }
        let market_filter = env_bool("POSITIONS_API_FILTER_MARKET", false);
        let market = self
            .condition_id
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let mut best_size = 0.0f64;
        let mut any_ok = false;
        for user in users {
            let mut req = http.get(&url).query(&[
                ("user", user.as_str()),
                ("sizeThreshold", "0"),
                ("limit", "500"),
                ("offset", "0"),
            ]);
            if market_filter {
                if let Some(mkt) = market.as_deref() {
                    req = req.query(&[("market", mkt)]);
                }
            }
            let resp = match req.send() {
                Ok(r) => r,
                Err(_) => continue,
            };
            if !resp.status().is_success() {
                continue;
            }
            let payload: Value = match resp.json() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if env_bool("POSITIONS_API_DEBUG_ALL", false) {
                let aid_tail: String = aid
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let user_tail: String = user
                    .chars()
                    .rev()
                    .take(8)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.info(&format!(
                    "[POSITIONS_API][DBG] asset={aid_tail} user=*{user_tail} resp={payload}"
                ));
            }
            let rows = payload
                .as_array()
                .cloned()
                .or_else(|| payload.get("data").and_then(|v| v.as_array()).cloned())
                .unwrap_or_default();
            let mut sz = 0.0f64;
            for row in &rows {
                let row_asset = row
                    .get("asset")
                    .or_else(|| row.get("asset_id"))
                    .or_else(|| row.get("token_id"))
                    .or_else(|| row.get("tokenId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if row_asset != aid {
                    continue;
                }
                let s = Self::_value_f64(row.get("size")).unwrap_or(0.0);
                if s.is_finite() && s > 0.0 {
                    sz += s;
                }
            }
            any_ok = true;
            if sz > best_size {
                best_size = sz;
            }
        }
        if any_ok {
            Some(best_size.max(0.0))
        } else {
            None
        }
    }
    /// Returns balance allowance conditional cached from the current BOT context.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _get_balance_allowance_conditional_cached(
        &self,
        token_id: &str,
        max_age_seconds: f64,
    ) -> Option<(f64, f64)> {
        let tid = token_id.trim().to_string();
        if tid.is_empty() {
            return None;
        }
        let now = now_ts_f64();
        if let Ok(cache) = self.balance_allowance_cache.lock() {
            if let Some((ts, bal, allow)) = cache.get(&tid) {
                if now - *ts <= max_age_seconds.max(0.0) {
                    return Some((*bal, *allow));
                }
            }
        }
        let units_per_share = std::env::var("POLY_CONDITIONAL_UNITS_PER_SHARE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(1_000_000.0);
        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return None,
        };
        let ba_update_enabled = env_bool("BALANCE_ALLOWANCE_UPDATE_ENABLED", true);
        // Keep CLOB service-side allowance snapshot fresh (Python parity: best-effort call).
        // Some deployments reject this endpoint or return non-JSON success bodies; in those
        // cases back off and continue with get_balance_allowance only.
        // continue with get_balance_allowance only.
        if ba_update_enabled && now >= self._runtime_ts_get("__ba_update_disabled_until") {
            if let Err(e) = rt.block_on(client.update_balance_allowance(BalanceAllowanceParams {
                asset_type: AssetType::Conditional,
                token_id: Some(tid.clone()),
            })) {
                let err_s = e.to_string();
                let err_l = err_s.to_ascii_lowercase();
                let disable_refresh = err_s.contains("405")
                    || err_s.contains("404")
                    || err_l.contains("method not allowed")
                    || err_l.contains("not found")
                    || err_l.contains("failed to parse json response")
                    || err_l.contains("error decoding response body")
                    || err_l.contains("eof while parsing a value");
                if disable_refresh {
                    self._runtime_ts_set("__ba_update_disabled_until", now + 3600.0);
                    if now >= self._runtime_ts_get("__ba_update_disable_logged_until") {
                        self.logger.warning(&format!(
                            "[BAL] update_balance_allowance unavailable ({err_s}); disabling this refresh call for 1h and continuing with get_balance_allowance."
                        ));
                        self._runtime_ts_set("__ba_update_disable_logged_until", now + 3600.0);
                    }
                }
            }
        }
        let resp = match rt.block_on(client.get_balance_allowance(BalanceAllowanceParams {
            asset_type: AssetType::Conditional,
            token_id: Some(tid.clone()),
        })) {
            Ok(v) => v,
            Err(e) => {
                let tail: String = tid
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.warning(&format!(
                    "[BAL] get_balance_allowance failed token={tail} err={e}"
                ));
                return None;
            }
        };
        if env_bool("BALANCE_ALLOWANCE_DEBUG_ALL", false) {
            let tail: String = tid
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger.info(&format!(
                "[BAL][DBG_ALL] get_balance_allowance token={tail} resp={}",
                resp
            ));
        }
        let bal_raw = Self::_value_f64(resp.get("balance"))
            .or_else(|| Self::_max_numeric_in_value(resp.get("balances")))
            .unwrap_or(0.0);
        let allow_from_scalar = Self::_value_f64(resp.get("allowance"));
        let allow_from_map = Self::_max_numeric_in_value(resp.get("allowances"));
        let allow_raw = allow_from_scalar.or(allow_from_map).unwrap_or(0.0);
        let bal = bal_raw / units_per_share;
        let allow = allow_raw / units_per_share;
        self._runtime_ts_set("__ba_last_fetch_ts", now);
        self._runtime_ts_set("__ba_last_raw_balance", bal_raw);
        self._runtime_ts_set("__ba_last_raw_allowance", allow_raw);
        self._runtime_ts_set("__ba_last_units_per_share", units_per_share);
        self._runtime_ts_set("__ba_last_balance_shares", bal);
        self._runtime_ts_set("__ba_last_allowance_shares", allow);
        if bal_raw <= 0.0 && allow_raw <= 0.0 {
            let next_dbg = self._runtime_ts_get("__ba_zero_payload_log_until");
            if now >= next_dbg {
                let keys = resp
                    .as_object()
                    .map(|m| m.keys().cloned().collect::<Vec<String>>().join(","))
                    .unwrap_or_else(|| "<non-object>".to_string());
                let allowances_snip = resp
                    .get("allowances")
                    .map(|v| {
                        let s = v.to_string();
                        if s.len() > 220 {
                            format!("{}...", &s[..220])
                        } else {
                            s
                        }
                    })
                    .unwrap_or_else(|| "<none>".to_string());
                let tail: String = tid
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.logger.info(&format!(
                    "[BAL][DBG] token={tail} keys=[{keys}] raw_balance={bal_raw:.0} raw_allowance={allow_raw:.0} units_per_share={units_per_share:.0} allowances={allowances_snip}"
                ));
                self._runtime_ts_set("__ba_zero_payload_log_until", now + 5.0);
            }
        }
        if let Ok(mut cache) = self.balance_allowance_cache.lock() {
            cache.insert(tid, (now, bal, allow));
        }
        Some((bal, allow))
    }
}
