use super::*;
impl MakerHedgeCapBot {
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
        if guard.seen_trade_keys.iter().any(|k| k == trade_key) {
            return false;
        }
        guard.seen_trade_keys.push(trade_key.to_string());
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
        let _ = save_state(&self.state_file, &mut guard);
        drop(guard);
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
        self._bot_runtime_note_observed_fill(asset_id, filled, false, side, None, None);
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

    pub fn _remember_taker_order(
        &self,
        order_id: &str,
        asset_id: &str,
        size: f64,
        px_limit: f64,
        side: &str,
    ) {
        if order_id.trim().is_empty() {
            return;
        }
        let rec = TakerOrderRecord {
            order_id: order_id.to_string(),
            asset_id: asset_id.to_string(),
            size,
            applied: 0.0,
            px_limit,
            side: side.to_ascii_uppercase(),
            ts: now_ts_f64(),
        };
        if let Ok(mut m) = self.taker_orders.lock() {
            m.insert(order_id.to_string(), rec);
        }
    }
    /// Removes taker order from the BOT''s runtime bookkeeping.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _forget_taker_order(&self, order_id: &str) {
        if order_id.trim().is_empty() {
            return;
        }
        if let Ok(mut m) = self.taker_orders.lock() {
            m.remove(order_id);
        }
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
