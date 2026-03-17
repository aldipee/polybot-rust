use super::*;
impl MakerHedgeCapBot {
    /// Submits order compat through the compatibility execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _post_order_compat(
        &self,
        signed_order: &Value,
        order_type: &str,
        post_only: Option<bool>,
    ) -> Option<String> {
        if self.cfg.dry_run {
            return None;
        }
        let asset_id = signed_order
            .get("asset_id")
            .or_else(|| signed_order.get("token_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if asset_id.trim().is_empty() {
            return None;
        }
        let side_u = signed_order
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("BUY")
            .to_ascii_uppercase();
        let side = Self::_clob_side(&side_u)?;
        let price = signed_order
            .get("price")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0);
        let mut size = signed_order
            .get("size")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0);
        if price <= 0.0 || size <= 0.0 {
            return None;
        }
        let clob_order_type = Self::_clob_order_type(order_type);
        if matches!(clob_order_type, ClobOrderType::Gtc | ClobOrderType::Gtd) {
            let tick_size_guess = Self::_tick_size_from_f64(self.cfg.tick.max(0.0001));
            size = Self::_maker_limit_exchange_quantized_size(side, price, size, tick_size_guess);
            if size <= 0.0 {
                return None;
            }
        }
        if post_only.unwrap_or(false) {
            if let Some((bid, ask)) = self._best_bid_ask(&asset_id) {
                let tick = self.cfg.tick.max(0.0001);
                if matches!(side, ClobSide::Buy) && price >= (ask - tick * 0.5) {
                    return None;
                }
                if matches!(side, ClobSide::Sell) && price <= (bid + tick * 0.5) {
                    return None;
                }
            }
        }
        let local_fallback = || {
            let oid = crate::db::new_uuid();
            let row = json!({
                "id": oid,
                "order_id": oid,
                "asset_id": asset_id,
                "side": side_u,
                "price": price,
                "size": size,
                "order_type": order_type.to_ascii_uppercase(),
                "post_only": post_only,
                "ts": now_ts_f64(),
            });
            if let Ok(mut ex) = self.exchange_orders_cache.lock() {
                ex.push(row);
            }
            Some(oid)
        };
        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return local_fallback(),
        };
        let prep_start_ns = now_ns();
        let prep_start_ts = now_ts_f64();
        let tick_size = rt
            .block_on(client.get_tick_size(&asset_id))
            .unwrap_or_else(|_| Self::_tick_size_from_f64(self.cfg.tick.max(0.0001)));
        let neg_risk = rt.block_on(client.get_neg_risk(&asset_id)).unwrap_or(false);
        let fee_rate_bps = rt.block_on(client.get_fee_rate_bps(&asset_id)).ok();
        let normalized_size = if matches!(clob_order_type, ClobOrderType::Gtc | ClobOrderType::Gtd)
        {
            Self::_maker_limit_exchange_quantized_size(side, price, size, tick_size)
        } else {
            size
        };
        if normalized_size <= 0.0 {
            return None;
        }
        if (normalized_size - size).abs() > 1e-9 {
            self.logger.info(&format!(
                "[CLOB][PRECISION] quantized limit order asset={} side={} price={:.3} requested_size={:.4} normalized_size={:.4} order_type={} post_only={}",
                asset_id,
                side_u,
                price,
                size,
                normalized_size,
                order_type.to_ascii_uppercase(),
                post_only.unwrap_or(false),
            ));
        }
        if matches!(clob_order_type, ClobOrderType::Gtc | ClobOrderType::Gtd)
            && !maker_post_only_order_meets_min_maker_notional(
                side,
                price,
                normalized_size,
                post_only.unwrap_or(false),
                self.min_maker_notional,
            )
        {
            self.logger.info(&format!(
                "[CLOB][PRECISION] skip post-only sub-min maker notional asset={} side={} price={:.3} normalized_size={:.4} notional={:.3} min_maker_notional={:.2}",
                asset_id,
                side_u,
                price,
                normalized_size,
                price * normalized_size,
                self.min_maker_notional
            ));
            return None;
        }
        size = normalized_size;
        let prep_end_ns = now_ns();
        let prep_end_ts = now_ts_f64();
        let user_order = UserLimitOrder {
            token_id: asset_id.clone(),
            price,
            size,
            side,
            fee_rate_bps,
            nonce: None,
            expiration: None,
            taker: None,
        };
        let create_opts = Some(CreateOrderOptions {
            tick_size,
            neg_risk: Some(neg_risk),
        });
        let sign_start_ns = now_ns();
        let sign_start_ts = now_ts_f64();
        let signed = match rt.block_on(client.create_limit_order(&user_order, create_opts)) {
            Ok(v) => v,
            Err(e) => {
                let err_s = e.to_string();
                let no_match = matches!(clob_order_type, ClobOrderType::Fak)
                    && err_s
                        .to_ascii_lowercase()
                        .contains("no orders found to match");
                if no_match {
                    self._runtime_ts_set("__last_fak_no_match_ts", now_ts_f64());
                }
                if post_only.unwrap_or(false) {
                    self.logger
                        .warning(&format!("post-only order rejected: {err_s}"));
                } else {
                    self.logger.error(&format!("post_order failed: {err_s}"));
                }
                return None;
            }
        };
        let sign_end_ns = now_ns();
        let sign_end_ts = now_ts_f64();
        let post_start_ns = now_ns();
        let post_start_ts = now_ts_f64();
        let posted = rt.block_on(client.post_order(signed, clob_order_type));
        let post_end_ns = now_ns();
        let post_end_ts = now_ts_f64();
        let resp = match posted {
            Ok(v) => v,
            Err(e) => {
                let err_s = e.to_string();
                let no_match = matches!(clob_order_type, ClobOrderType::Fak)
                    && err_s
                        .to_ascii_lowercase()
                        .contains("no orders found to match");
                if no_match {
                    self._runtime_ts_set("__last_fak_no_match_ts", now_ts_f64());
                }
                if post_only.unwrap_or(false) {
                    self.logger
                        .warning(&format!("post-only order rejected: {err_s}"));
                } else {
                    self.logger.error(&format!("post_order failed: {err_s}"));
                }
                return None;
            }
        };
        let oid = Self::_extract_posted_order_id(&resp)?;
        let row = json!({
            "id": oid.clone(),
            "order_id": oid.clone(),
            "asset_id": asset_id,
            "side": side_u,
            "price": price,
            "size": size,
            "order_type": order_type.to_ascii_uppercase(),
            "post_only": post_only,
            "ts": now_ts_f64(),
        });
        if let Ok(mut ex) = self.exchange_orders_cache.lock() {
            ex.push(row);
        }
        if let Ok(mut m) = self.submit_timing_cache.lock() {
            let mut submit_timing = json!({
                "sign_start_ns": sign_start_ns,
                "sign_end_ns": sign_end_ns,
                "sign_start_ts": sign_start_ts,
                "sign_end_ts": sign_end_ts,
                "prep_start_ns": prep_start_ns,
                "prep_end_ns": prep_end_ns,
                "prep_start_ts": prep_start_ts,
                "prep_end_ts": prep_end_ts,
                "post_start_ns": post_start_ns,
                "post_end_ns": post_end_ns,
                "post_start_ts": post_start_ts,
                "post_end_ts": post_end_ts,
                "order_submit_ts": post_end_ts,
                "fee_rate_bps": fee_rate_bps,
                "tick_size": tick_size.as_f64(),
                "neg_risk": neg_risk,
            });
            self._merge_pair_metadata_into_value(&mut submit_timing);
            m.insert(oid.clone(), submit_timing);
        }
        Some(oid)
    }
    /// Submits orders compat through the compatibility execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _post_orders_compat(
        &self,
        signed_orders: &[Value],
        order_type: &str,
        post_only: Option<bool>,
    ) -> Vec<Option<String>> {
        if self.cfg.dry_run {
            return signed_orders.iter().map(|_| None).collect();
        }
        signed_orders
            .iter()
            .map(|o| self._post_order_compat(o, order_type, post_only))
            .collect()
    }
    /// Places postonly bid through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _place_postonly_bid(&self, asset_id: &str, price: f64, size: f64) -> Option<String> {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let price = round_down(price.max(0.0), tick);
        let tick_size = Self::_tick_size_from_f64(tick);
        let mut size =
            Self::_maker_limit_exchange_quantized_size(ClobSide::Buy, price, size, tick_size);
        if size < self.cfg.min_shares || price <= 0.0 {
            return None;
        }
        if price * size < self.min_maker_notional {
            let need_size = Self::_maker_limit_exchange_quantized_size(
                ClobSide::Buy,
                price,
                self.min_maker_notional / price,
                tick_size,
            );
            size = Self::_maker_limit_exchange_quantized_size(
                ClobSide::Buy,
                price,
                size.max(need_size).max(self.cfg.min_shares),
                tick_size,
            );
            if price * size < self.min_maker_notional {
                return None;
            }
        }
        let (_bid, ask) = self._best_bid_ask(asset_id)?;
        let maker_max = round_down(
            ask - self.cfg.maker_buffer_ticks as f64 * self.cfg.tick.max(0.0001),
            self.cfg.tick.max(0.0001),
        );
        if price > maker_max {
            return None;
        }
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] POSTONLY BID asset={} price={price:.2} size={size:.4} notional={:.2}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>(),
                price * size
            ));
            return None;
        }
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": price,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, "GTC", Some(true))?;
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": price,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": "MAKER_POSTONLY_GTC",
            }),
        );
        Some(oid)
    }
    /// Places limit bid GTC through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _place_limit_bid_gtc(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        post_only: Option<bool>,
    ) -> Option<String> {
        let origin = if post_only.unwrap_or(false) {
            "LIMIT_GTC_POSTONLY"
        } else {
            "LIMIT_GTC"
        };
        self._place_limit_bid_gtc_with_origin(asset_id, price, size, post_only, origin)
    }
    /// Places limit bid GTC with origin through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _place_limit_bid_gtc_with_origin(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        post_only: Option<bool>,
        origin: &str,
    ) -> Option<String> {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = clamp(price, tick, 0.99);
        px = round_down(px, tick);
        px = clamp(px, tick, 0.99);
        let tick_size = Self::_tick_size_from_f64(tick);
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let mut sz_int = (size + 1e-12).floor() as i64;
        if sz_int < min_int {
            sz_int = min_int;
        }
        sz_int = (sz_int / min_int) * min_int;
        if sz_int < min_int {
            sz_int = min_int;
        }
        let size =
            Self::_maker_limit_exchange_quantized_size(ClobSide::Buy, px, sz_int as f64, tick_size);
        if size + 1e-9 < min_int as f64 {
            return None;
        }
        if self.cfg.dry_run {
            let oid = format!("DRY_LIMIT_GTC_{}", (now_ts_f64() * 1000.0) as i64);
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    asset_id.to_string(),
                    OpenOrderState {
                        order_id: Some(oid.clone()),
                        price: Some(px),
                        size: Some(size),
                        ts: Some(now_ts_f64()),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
            self.logger.info(&format!(
                "[DRY] limit bid GTC asset={} px={px:.3} size={size:.2} post_only={post_only:?}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return Some(oid);
        }
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": px,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, "GTC", post_only)?;
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.insert(
                asset_id.to_string(),
                OpenOrderState {
                    order_id: Some(oid.clone()),
                    price: Some(px),
                    size: Some(size),
                    ts: Some(now_ts_f64()),
                },
            );
            let _ = save_state(&self.state_file, &mut s);
        }
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": origin,
            }),
        );
        Some(oid)
    }
    /// Places limit bid GTC exact with origin through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _place_limit_bid_gtc_exact_with_origin(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        post_only: Option<bool>,
        origin: &str,
    ) -> Option<String> {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = clamp(price, tick, 0.99);
        px = round_down(px, tick);
        px = clamp(px, tick, 0.99);
        let tick_size = Self::_tick_size_from_f64(tick);
        let size = Self::_maker_limit_exchange_quantized_size(ClobSide::Buy, px, size, tick_size);
        if size < 0.01 {
            return None;
        }
        if self.cfg.dry_run {
            let oid = format!("DRY_LIMIT_GTC_EXACT_{}", (now_ts_f64() * 1000.0) as i64);
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    asset_id.to_string(),
                    OpenOrderState {
                        order_id: Some(oid.clone()),
                        price: Some(px),
                        size: Some(size),
                        ts: Some(now_ts_f64()),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
            self.logger.info(&format!(
                "[DRY] limit bid GTC exact asset={} px={px:.3} size={size:.2} post_only={post_only:?}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return Some(oid);
        }
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": px,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, "GTC", post_only)?;
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.insert(
                asset_id.to_string(),
                OpenOrderState {
                    order_id: Some(oid.clone()),
                    price: Some(px),
                    size: Some(size),
                    ts: Some(now_ts_f64()),
                },
            );
            let _ = save_state(&self.state_file, &mut s);
        }
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": origin,
            }),
        );
        Some(oid)
    }
    /// Resolves order type for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _resolve_order_type(&self, name: &str) -> String {
        let mut n = name.trim().to_ascii_uppercase();
        if matches!(n.as_str(), "LIMIT" | "LIMIT_GTC" | "GTC_LIMIT") {
            n = "GTC".to_string();
        }
        if matches!(n.as_str(), "IOC" | "IOK" | "FILL_AND_KILL" | "FILLANDKILL") {
            n = "FAK".to_string();
        }
        if matches!(n.as_str(), "FILL_OR_KILL" | "FILLORKILL") {
            n = "FOK".to_string();
        }
        match n.as_str() {
            "FAK" | "FOK" | "GTC" => n,
            _ => {
                self.logger
                    .warning(&format!("Unknown OrderType '{n}'. Falling back to GTC."));
                "GTC".to_string()
            }
        }
    }
    /// Places taker bid FAK through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _place_taker_bid_fak(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        order_type_name: Option<&str>,
    ) -> Option<String> {
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = round_up(price, tick);
        px = clamp(px, tick, 0.99);
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let size_int = (size + 1e-12).floor() as i64;
        if size_int < min_int {
            return None;
        }
        let size = size_int as f64;
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER HEDGE BUY asset={} price={px:.2} size={size_int} type={ot}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return None;
        }
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": px,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, &ot, None)?;
        self._remember_taker_order(&oid, asset_id, size, px, "BUY");
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": format!("TAKER_{}_BUY", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent BUY asset={} px={px:.4} sz={size:.0} oid={oid}",
            asset_id
        ));
        Some(oid)
    }
    /// Places taker bid FAK exact through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _place_taker_bid_fak_exact(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        order_type_name: Option<&str>,
    ) -> Option<String> {
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = round_up(price, tick);
        px = clamp(px, tick, 0.99);
        let size = round_down(size.max(0.0), 0.01);
        if size < 0.01 {
            return None;
        }
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER HEDGE BUY EXACT asset={} price={px:.2} size={size:.2} type={ot}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return None;
        }
        let signed = json!({
            "asset_id": asset_id,
            "side": "BUY",
            "price": px,
            "size": size,
        });
        let oid = self._post_order_compat(&signed, &ot, None)?;
        self._remember_taker_order(&oid, asset_id, size, px, "BUY");
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "BUY",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": format!("TAKER_{}_BUY_EXACT", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent BUY asset={} px={px:.4} sz={size:.2} oid={oid}",
            asset_id
        ));
        Some(oid)
    }
    /// Places taker ask FAK through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _place_taker_ask_fak(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        order_type_name: Option<&str>,
    ) -> Option<String> {
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = round_down(price, tick);
        px = clamp(px, tick, 0.99);
        let mut dp_i = env_int("SIZE_DECIMALS", 6);
        dp_i = dp_i.clamp(0, 8);
        let dp = dp_i as u32;
        let allow_fractional = env_bool("TAKER_EXIT_ALLOW_FRACTIONAL_SIZE", false);
        let size = if allow_fractional {
            let min_step = 10f64.powi(-(dp as i32));
            let min_size = env_float("TAKER_EXIT_MIN_ORDER_SIZE", 0.1).max(min_step);
            let q = q_down(size.max(0.0), dp);
            if q + 1e-12 < min_size {
                return None;
            }
            q
        } else {
            let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
            let size_int = (size + 1e-12).floor() as i64;
            if size_int < min_int {
                return None;
            }
            size_int as f64
        };
        let sz_disp = if (size - size.round()).abs() <= 1e-9 {
            format!("{:.0}", size)
        } else {
            format!("{:.4}", size)
        };
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER SELL asset={} price={px:.2} size={sz_disp} type={ot}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return None;
        }
        let signed = json!({
            "asset_id": asset_id,
            "side": "SELL",
            "price": px,
            "size": size,
        });
        let oid = match self._post_order_compat(&signed, &ot, None) {
            Some(v) => v,
            None => {
                self.logger.warning(&format!(
                    "[TAKER {ot}] rejected SELL asset={} px={px:.4} sz={sz_disp} (no oid)",
                    asset_id
                ));
                return None;
            }
        };
        self._remember_taker_order(&oid, asset_id, size, px, "SELL");
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "SELL",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": format!("TAKER_{}_SELL", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent SELL asset={} px={px:.4} sz={sz_disp} oid={oid}",
            asset_id
        ));
        Some(oid)
    }
    /// Places taker ask FAK exact through the bot''s execution layer.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _place_taker_ask_fak_exact(
        &self,
        asset_id: &str,
        price: f64,
        size: f64,
        order_type_name: Option<&str>,
    ) -> Option<String> {
        let decide_ts = now_ts_f64();
        let decide_ns = now_ns();
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let mut px = round_down(price, tick);
        px = clamp(px, tick, 0.99);
        let size = q_down(size.max(0.0), 4);
        if size < 0.0001 {
            return None;
        }
        let ot_name = order_type_name.unwrap_or(&self.hedge_taker_order_type);
        let ot = self._resolve_order_type(ot_name);
        if self.cfg.dry_run {
            self.logger.info(&format!(
                "[DRY] TAKER SELL EXACT asset={} price={px:.4} size={size:.4} type={ot}",
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            return None;
        }
        let signed = json!({
            "asset_id": asset_id,
            "side": "SELL",
            "price": px,
            "size": size,
        });
        let oid = match self._post_order_compat(&signed, &ot, None) {
            Some(v) => v,
            None => {
                self.logger.warning(&format!(
                    "[TAKER {ot}] rejected SELL exact asset={} px={px:.4} sz={size:.4} (no oid)",
                    asset_id
                ));
                return None;
            }
        };
        self._remember_taker_order(&oid, asset_id, size, px, "SELL");
        self._track_order_execution_context(
            &oid,
            &json!({
                "order_id": oid,
                "asset_id": asset_id,
                "side": "SELL",
                "px_limit": px,
                "size": size,
                "decision_ts": decide_ts,
                "decision_ns": decide_ns,
                "post_start_ts": decide_ts,
                "post_end_ts": now_ts_f64(),
                "origin": format!("TAKER_{}_SELL_EXACT", ot),
            }),
        );
        self.logger.info(&format!(
            "[TAKER {ot}] sent SELL asset={} px={px:.4} sz={size:.4} oid={oid}",
            asset_id
        ));
        Some(oid)
    }
}
