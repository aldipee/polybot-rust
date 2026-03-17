use super::*;
impl MakerHedgeCapBot {
    /// Returns or derives taker order fallback on order event for the active BOT execution
    /// path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _taker_order_fallback_on_order_event(&self, msg: &Value) {
        let order_id = msg
            .get("order_id")
            .or_else(|| msg.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if order_id.is_empty() {
            return;
        }
        // Only apply fallback for recent taker orders we submitted.
        let rec0 = self
            .taker_orders
            .lock()
            .ok()
            .and_then(|m| m.get(order_id).cloned());
        let Some(rec0) = rec0 else {
            return;
        };
        let mut matched_total = Self::_value_f64(
            msg.get("size_matched")
                .or_else(|| msg.get("matched_size"))
                .or_else(|| msg.get("filled_size"))
                .or_else(|| msg.get("filled")),
        )
        .unwrap_or(0.0);
        if rec0.size > 0.0 {
            matched_total = matched_total.min(rec0.size.max(0.0));
        }
        let inc = (matched_total - rec0.applied).max(0.0);
        if inc > 1e-9 {
            let price = Self::_value_f64(msg.get("price")).unwrap_or(rec0.px_limit);
            let asset = msg
                .get("asset_id")
                .or_else(|| msg.get("token_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(rec0.asset_id.as_str());
            let side = msg
                .get("side")
                .and_then(|v| v.as_str())
                .filter(|s| matches!(s.trim().to_ascii_uppercase().as_str(), "BUY" | "SELL"))
                .unwrap_or(rec0.side.as_str());
            let key = format!("order_evt:{order_id}:{matched_total:.8}");
            let applied = self._apply_fill(asset, price, inc, &key, side);
            if applied {
                self._log_execution_latency_on_fill(order_id, now_ts_f64());
            }
        }
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
        let done_hint = matches!(
            typ.as_str(),
            "CANCELLATION" | "CANCELED" | "CANCELLED" | "REJECTION" | "REJECTED"
        ) || matches!(
            status.as_str(),
            "CANCELED" | "CANCELLED" | "REJECTED" | "FILLED"
        );
        let mut remove_oid = false;
        if let Ok(mut m) = self.taker_orders.lock() {
            if let Some(rec) = m.get_mut(order_id) {
                rec.applied = rec.applied.max(matched_total);
                rec.ts = now_ts_f64();
                if done_hint || (rec.size > 0.0 && rec.applied >= rec.size - 1e-9) {
                    remove_oid = true;
                }
            }
            if remove_oid {
                m.remove(order_id);
            }
        }
    }
    /// Handles user trade event for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _handle_user_trade_event(&self, msg: &Value) {
        let event_type = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !event_type.is_empty() && !event_type.contains("trade") {
            return;
        }
        let status = msg
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let trade_id = msg
            .get("id")
            .or_else(|| msg.get("trade_id"))
            .or_else(|| msg.get("tradeId"))
            .or_else(|| msg.get("tradeID"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let taker_oid = msg
            .get("taker_order_id")
            .or_else(|| msg.get("takerOrderId"))
            .or_else(|| msg.get("taker_orderId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let taker_rec = if taker_oid.trim().is_empty() {
            None
        } else {
            self.taker_orders
                .lock()
                .ok()
                .and_then(|m| m.get(&taker_oid).cloned())
        };
        let taker_ctx = if taker_oid.trim().is_empty() {
            None
        } else {
            self._get_order_execution_context(&taker_oid)
        };
        if !status.is_empty() && !matches!(status.as_str(), "MATCHED" | "MINED" | "CONFIRMED") {
            return;
        }
        // CASE A: Taker trade event that matches a recent locally-submitted taker order.
        if taker_rec.is_some() || taker_ctx.is_some() {
            let msg_asset = msg
                .get("asset_id")
                .or_else(|| msg.get("token_id"))
                .or_else(|| msg.get("assetId"))
                .or_else(|| msg.get("tokenId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let msg_side = msg
                .get("side")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_uppercase();
            let ctx_asset = taker_ctx
                .as_ref()
                .and_then(|c| {
                    c.get("asset_id")
                        .or_else(|| c.get("token_id"))
                        .or_else(|| c.get("assetId"))
                        .or_else(|| c.get("tokenId"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("")
                .to_string();
            let ctx_side = taker_ctx
                .as_ref()
                .and_then(|c| c.get("side").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_ascii_uppercase();
            let ctx_px_limit = taker_ctx
                .as_ref()
                .and_then(|c| Self::_value_f64(c.get("px_limit").or_else(|| c.get("price"))))
                .unwrap_or(0.0);
            let ctx_size = taker_ctx
                .as_ref()
                .and_then(|c| Self::_value_f64(c.get("size")))
                .unwrap_or(0.0);
            let mut asset = taker_rec
                .as_ref()
                .map(|r| r.asset_id.clone())
                .unwrap_or_else(|| ctx_asset.clone());
            if asset.trim().is_empty() {
                asset = msg_asset.clone();
            }
            let mut side = taker_rec
                .as_ref()
                .map(|r| r.side.clone())
                .unwrap_or_else(|| ctx_side.clone());
            if !matches!(side.as_str(), "BUY" | "SELL") {
                side = msg_side.clone();
            }
            if (!msg_asset.trim().is_empty() && msg_asset != asset)
                || (matches!(msg_side.as_str(), "BUY" | "SELL") && msg_side != side)
            {
                let msg_tail: String = msg_asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let rec_tail: String = asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let mpx = Self::_value_f64(msg.get("price")).unwrap_or(0.0);
                let msz = Self::_value_f64(
                    msg.get("size")
                        .or_else(|| msg.get("filled"))
                        .or_else(|| msg.get("matched_amount"))
                        .or_else(|| msg.get("matchedAmount"))
                        .or_else(|| msg.get("amount")),
                )
                .unwrap_or(0.0);
                self.logger.info(&format!(
                    "[FILL][DBG_MAP] taker_oid={}.. msg_asset={} rec_asset={} msg_side={} rec_side={} msg_px={mpx:.4} msg_sz={msz:.6}",
                    taker_oid.chars().take(10).collect::<String>(),
                    msg_tail,
                    rec_tail,
                    msg_side,
                    side
                ));
            }
            let mut price = Self::_value_f64(msg.get("price")).unwrap_or_else(|| {
                taker_rec
                    .as_ref()
                    .map(|r| r.px_limit)
                    .unwrap_or(ctx_px_limit)
            });
            if price <= 0.0 {
                price = taker_rec
                    .as_ref()
                    .map(|r| r.px_limit)
                    .unwrap_or(ctx_px_limit);
            }
            let mut size = Self::_value_f64(
                msg.get("size")
                    .or_else(|| msg.get("filled"))
                    .or_else(|| msg.get("matched_amount"))
                    .or_else(|| msg.get("matchedAmount"))
                    .or_else(|| msg.get("amount")),
            )
            .unwrap_or(0.0);
            if size <= 0.0 {
                size = taker_rec
                    .as_ref()
                    .map(|r| (r.size - r.applied).max(0.0))
                    .unwrap_or(ctx_size.max(0.0));
            }
            if let Some(rec) = &taker_rec {
                let remaining = (rec.size - rec.applied).max(0.0);
                if remaining > 0.0 {
                    size = size.min(remaining);
                }
            }
            if size <= 0.0 || price <= 0.0 || asset.trim().is_empty() {
                return;
            }
            if !matches!(side.as_str(), "BUY" | "SELL") {
                return;
            }
            let key = if !trade_id.is_empty() {
                format!("{trade_id}:taker")
            } else {
                format!("trade_fallback:taker:{taker_oid}:{asset}:{side}:{size:.8}:{price:.8}")
            };
            let applied = self._apply_fill(&asset, price, size, &key, &side);
            if applied {
                self._log_execution_latency_on_fill(&taker_oid, now_ts_f64());
                let mut remove_oid = false;
                if let Ok(mut m) = self.taker_orders.lock() {
                    if let Some(r) = m.get_mut(&taker_oid) {
                        r.applied += size.max(0.0);
                        r.ts = now_ts_f64();
                        if r.size > 0.0 && r.applied >= r.size - 1e-9 {
                            remove_oid = true;
                        }
                    }
                    if remove_oid {
                        m.remove(&taker_oid);
                    }
                }
            }
            return;
        }
        // CASE B: Maker trade event. Apply only if maker leg matches our wallet.
        let wallet = self.wallet_address.to_ascii_lowercase();
        let maker_orders = msg
            .get("maker_orders")
            .or_else(|| msg.get("makerOrders"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if !maker_orders.is_empty() {
            let mut maker_leg: Option<Value> = None;
            if !wallet.trim().is_empty() {
                for mo in &maker_orders {
                    let mo_addr = mo
                        .get("maker_address")
                        .or_else(|| mo.get("makerAddress"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if !mo_addr.is_empty() && mo_addr == wallet {
                        maker_leg = Some(mo.clone());
                        break;
                    }
                }
            }
            if let Some(mo) = maker_leg {
                if let Some(candidate) = self._maker_trade_exec_candidate(msg, &mo) {
                    let maker_oid = candidate.order_id.clone();
                    let trade_id = candidate.trade_id.clone().unwrap_or_default();
                    let tx_hash = candidate.tx_hash.clone().unwrap_or_default();
                    let taker_oid = candidate.taker_order_id.clone().unwrap_or_default();
                    let match_time = candidate.match_time.clone().unwrap_or_default();
                    let qty = candidate.qty;
                    let px = candidate.price;
                    match self._maker_commit_exec_fill(candidate) {
                        MakerExecApplyResult::Applied { canonical_id } => {
                            let alias_kind = Self::_maker_exec_alias_kind(&canonical_id);
                            self.logger.info(&format!(
                                "[FILL][MAKER_APPLY] oid={}.. canonical={} alias_kind={} qty={qty:.6} px={px:.4}",
                                maker_oid.chars().take(10).collect::<String>(),
                                canonical_id,
                                alias_kind
                            ));
                            self._log_execution_latency_on_fill(&maker_oid, now_ts_f64());
                        }
                        MakerExecApplyResult::Duplicate { canonical_id } => {
                            let alias_kind = Self::_maker_exec_alias_kind(&canonical_id);
                            self.logger.info(&format!(
                                "[FILL][MAKER_DEDUPE] drop oid={}.. canonical={} alias_kind={} qty={qty:.6} px={px:.4} trade_id={} tx={} taker_oid={} match_time={}",
                                maker_oid.chars().take(10).collect::<String>(),
                                canonical_id,
                                alias_kind,
                                trade_id,
                                tx_hash,
                                taker_oid,
                                match_time
                            ));
                        }
                        MakerExecApplyResult::Conflict {
                            canonical_id,
                            reason,
                        } => {
                            let alias_kind = Self::_maker_exec_alias_kind(&canonical_id);
                            self.logger.warning(&format!(
                                "[FILL][MAKER_CONFLICT] oid={}.. canonical={} alias_kind={} reason={} qty={qty:.6} px={px:.4} trade_id={} tx={} taker_oid={} match_time={}",
                                maker_oid.chars().take(10).collect::<String>(),
                                canonical_id,
                                alias_kind,
                                reason,
                                trade_id,
                                tx_hash,
                                taker_oid,
                                match_time
                            ));
                        }
                        MakerExecApplyResult::DroppedWeakId { reason } => {
                            self.logger.warning(&format!(
                                "[FILL][MAKER_DROP_WEAK] oid={}.. reason={} qty={qty:.6} px={px:.4} trade_id={} tx={} taker_oid={} match_time={}",
                                maker_oid.chars().take(10).collect::<String>(),
                                reason,
                                trade_id,
                                tx_hash,
                                taker_oid,
                                match_time
                            ));
                        }
                    }
                    return;
                }
            }
        }
        // Ambiguous trade event: ignore instead of corrupting local state.
        if env_bool("USER_TRADE_DEBUG", false) {
            let has_maker = msg
                .get("maker_orders")
                .or_else(|| msg.get("makerOrders"))
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            self.logger.info(&format!(
                "[FILL][DBG_DROP] drop ambiguous trade event id={} taker_oid={} has_maker_orders={}",
                trade_id,
                taker_oid,
                has_maker
            ));
        }
    }
    /// Handles user order event for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _handle_user_order_event(&self, msg: &Value) {
        let event_type = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !event_type.is_empty() && !event_type.contains("order") {
            return;
        }
        if self.taker_fill_fallback_from_order_events {
            self._taker_order_fallback_on_order_event(msg);
        }
        let asset_id = msg
            .get("asset_id")
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("assetId"))
            .or_else(|| msg.get("tokenId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if asset_id.trim().is_empty() {
            return;
        }
        let is_yn = self.yes_asset.as_deref() == Some(asset_id.as_str())
            || self.no_asset.as_deref() == Some(asset_id.as_str());
        if !is_yn {
            return;
        }
        self._maker_order_on_user_event(msg);
        let side = msg
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if side != "BUY" {
            return;
        }
        let oid = self._extract_order_id(msg).unwrap_or_default();
        if oid.trim().is_empty() {
            return;
        }
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
        if cancelish || remaining <= 0.0 {
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
    /// Handles user event for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _handle_user_event(&self, msg: &Value) {
        let t = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .or_else(|| msg.get("event"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if t.contains("trade") {
            self._handle_user_trade_event(msg);
        } else if t.contains("order") {
            self._handle_user_order_event(msg);
        }
    }
    /// Returns or derives on user message for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn on_user_message(&self, message: &str) {
        if let Ok(v) = serde_json::from_str::<Value>(message) {
            if let Some(items) = v.as_array() {
                for item in items {
                    if item.is_object() {
                        self._handle_user_event(item);
                    }
                }
            } else if v.is_object() {
                self._handle_user_event(&v);
            }
        }
    }
}

