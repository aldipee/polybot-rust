use super::*;
impl MakerHedgeCapBot {
    /// Cancels internal helper for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _cancel(&self, order_id: &str) -> bool {
        if order_id.trim().is_empty() {
            return false;
        }
        if self.cfg.dry_run {
            self.logger.info(&format!("[DRY] cancel {order_id}"));
            return true;
        }
        if let (Some(rt), Some(client)) = (&self.clob_rt, &self.clob_client) {
            match rt.block_on(client.cancel_order(order_id)) {
                Ok(_) => {
                    if let Ok(mut ex) = self.exchange_orders_cache.lock() {
                        ex.retain(|o| self._extract_order_id(o).as_deref() != Some(order_id));
                    }
                    self._maker_order_on_cancel_ack_by_order_id(order_id);
                    return true;
                }
                Err(e) => {
                    self.logger.error(&format!("Cancel failed: {e}"));
                    return false;
                }
            }
        }
        if let Ok(mut ex) = self.exchange_orders_cache.lock() {
            let before = ex.len();
            ex.retain(|o| self._extract_order_id(o).as_deref() != Some(order_id));
            return ex.len() != before;
        }
        false
    }
    /// Cancels open order local for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _cancel_open_order_local(&self, asset_id: &str, reason: &str) {
        let oid = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.open_orders.get(asset_id).and_then(|o| o.order_id.clone()));
        if let Some(order_id) = oid {
            if !reason.trim().is_empty() {
                self.logger.info(&format!(
                    "Cancel {} ({reason})",
                    asset_id
                        .chars()
                        .rev()
                        .take(6)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                ));
            }
            let _ = self._cancel(&order_id);
        }
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.remove(asset_id);
            let _ = save_state(&self.state_file, &mut s);
        }
    }
    /// Cancels all open orders local for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn cancel_all_open_orders_local(&self, reason: &str) {
        let oo = self
            .state
            .lock()
            .map(|s| s.open_orders.clone())
            .unwrap_or_default();
        if oo.is_empty() {
            return;
        }
        if !reason.trim().is_empty() {
            self.logger
                .info(&format!("Cancel local open orders: {reason}"));
        }
        for row in oo.values() {
            if let Some(oid) = &row.order_id {
                let _ = self._cancel(oid);
            }
        }
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.clear();
            let _ = save_state(&self.state_file, &mut s);
        }
    }
    /// Cancels all open orders local except for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn cancel_all_open_orders_local_except(&self, keep_asset_id: &str, reason: &str) {
        let oo = self
            .state
            .lock()
            .map(|s| s.open_orders.clone())
            .unwrap_or_default();
        if oo.is_empty() {
            return;
        }
        let only_keep_exists = oo.len() == 1 && oo.contains_key(keep_asset_id);
        if only_keep_exists {
            return;
        }
        let mut to_cancel: Vec<String> = Vec::new();
        for (aid, row) in &oo {
            if aid == keep_asset_id {
                continue;
            }
            if let Some(oid) = &row.order_id {
                to_cancel.push(oid.clone());
            }
        }
        if to_cancel.is_empty() {
            return;
        }
        if !reason.trim().is_empty() {
            let tail: String = keep_asset_id
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger.info(&format!(
                "Cancel local open orders (except {tail}): {reason}"
            ));
        }
        for oid in to_cancel {
            let _ = self._cancel(&oid);
        }
        if let Ok(mut s) = self.state.lock() {
            let kept = s.open_orders.get(keep_asset_id).cloned();
            s.open_orders.clear();
            if let Some(v) = kept {
                s.open_orders.insert(keep_asset_id.to_string(), v);
            }
            let _ = save_state(&self.state_file, &mut s);
        }
    }
    /// Extracts order id from the provided payload or state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _extract_order_id(&self, o: &Value) -> Option<String> {
        o.get("id")
            .or_else(|| o.get("order_id"))
            .or_else(|| o.get("orderID"))
            .or_else(|| o.get("orderId"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
    /// Extracts order token id from the provided payload or state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _extract_order_token_id(&self, o: &Value) -> Option<String> {
        o.get("asset_id")
            .or_else(|| o.get("token_id"))
            .or_else(|| o.get("assetId"))
            .or_else(|| o.get("tokenId"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
    /// Extracts order side from the provided payload or state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _extract_order_side(&self, o: &Value) -> String {
        o.get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("BUY")
            .to_ascii_uppercase()
    }
    /// Extracts order price from the provided payload or state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _extract_order_price(&self, o: &Value) -> f64 {
        o.get("price")
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0)
    }
    /// Extracts order remaining size from the provided payload or state.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _extract_order_remaining_size(&self, o: &Value) -> f64 {
        o.get("size")
            .or_else(|| o.get("remaining_size"))
            .or_else(|| o.get("remainingSize"))
            .or_else(|| o.get("original_size"))
            .or_else(|| o.get("originalSize"))
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
            .unwrap_or(0.0)
    }
    /// Lists open orders exchange from the current exchange or runtime context.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _list_open_orders_exchange(&self) -> Vec<Value> {
        let fallback = || {
            self.exchange_orders_cache
                .lock()
                .map(|v| v.clone())
                .unwrap_or_default()
        };
        // Prefer raw endpoint parsing first: CLOB /data/orders may return either an array
        // or an object envelope ({data:[...]}), while typed client decoding expects an array.
        if let Some(out) = self._list_open_orders_exchange_raw() {
            if let Ok(mut cache) = self.exchange_orders_cache.lock() {
                *cache = out.clone();
            }
            return out;
        }
        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return fallback(),
        };
        let params = self
            .condition_id
            .clone()
            .and_then(|v| (!v.trim().is_empty()).then_some(v))
            .map(|market| OpenOrderParams {
                market: Some(market),
                ..OpenOrderParams::default()
            });
        match rt.block_on(client.get_open_orders(params)) {
            Ok(orders) => {
                let mut out = Vec::with_capacity(orders.len());
                for o in orders {
                    let order_id = o.id;
                    let asset_id = o.asset_id;
                    let original_size = o.original_size.parse::<f64>().unwrap_or(0.0);
                    let size_matched = o.size_matched.parse::<f64>().unwrap_or(0.0);
                    let remaining_size = (original_size - size_matched).max(0.0);
                    let price = o.price.parse::<f64>().unwrap_or(0.0);
                    out.push(json!({
                        "id": order_id.clone(),
                        "order_id": order_id,
                        "asset_id": asset_id.clone(),
                        "token_id": asset_id,
                        "side": o.side.to_ascii_uppercase(),
                        "price": price,
                        "size": remaining_size,
                        "remaining_size": remaining_size,
                        "original_size": original_size,
                        "size_matched": size_matched,
                        "status": o.status,
                        "market": o.market,
                        "order_type": o.order_type,
                        "created_at": o.created_at,
                    }));
                }
                if let Ok(mut cache) = self.exchange_orders_cache.lock() {
                    *cache = out.clone();
                }
                out
            }
            Err(e) => {
                self.logger
                    .error(&format!("get_orders failed during reconcile: {e}"));
                fallback()
            }
        }
    }
    /// Cancels exchange orders for assets for the active BOT flow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _cancel_exchange_orders_for_assets(&self, asset_ids: &[String], reason: &str) {
        if self.cfg.dry_run {
            return;
        }
        let aset: HashSet<String> = asset_ids
            .iter()
            .map(|a| a.to_string())
            .filter(|a| !a.trim().is_empty())
            .collect();
        if aset.is_empty() {
            return;
        }
        let orders = self._list_open_orders_exchange();
        for o in orders {
            let aid = self._extract_order_token_id(&o);
            if aid.is_none() {
                continue;
            }
            let aid = aid.unwrap_or_default();
            if !aset.contains(&aid) {
                continue;
            }
            let oid = self._extract_order_id(&o);
            if let Some(oid) = oid {
                if !reason.trim().is_empty() {
                    self.logger.info(&format!(
                        "Cancel exchange order {}.. for {} ({reason})",
                        oid.chars().take(10).collect::<String>(),
                        aid.chars()
                            .rev()
                            .take(6)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>()
                    ));
                }
                let _ = self._cancel(&oid);
            }
        }
    }
    /// Returns or derives reconcile exchange orders for asset for the active BOT execution
    /// path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _reconcile_exchange_orders_for_asset(
        &self,
        asset_id: &str,
        intended_price: Option<f64>,
        force: bool,
    ) {
        if self._maker_single_inflight_enabled() && !self.cfg.dry_run {
            self._maker_order_reconcile_asset(asset_id, intended_price);
            if !env_bool("RECONCILE_EXCHANGE_ORDERS", true) {
                return;
            }
        }
        if !env_bool("RECONCILE_EXCHANGE_ORDERS", true) || self.cfg.dry_run {
            return;
        }
        let now = now_ts_f64();
        let key = format!("__reconcile_last_{asset_id}");
        let last = self._runtime_ts_get(&key);
        let interval = env_float("RECONCILE_INTERVAL_SECONDS", 1.0).max(0.0);
        if !force && (now - last) < interval {
            return;
        }
        self._runtime_ts_set(&key, now);
        let orders = self._list_open_orders_exchange();
        let mut mine: Vec<Value> = Vec::new();
        for o in orders {
            let aid = self._extract_order_token_id(&o);
            if aid.as_deref() != Some(asset_id) {
                continue;
            }
            if self._extract_order_side(&o) != "BUY" {
                continue;
            }
            if self._extract_order_id(&o).is_none() {
                continue;
            }
            mine.push(o);
        }
        if mine.is_empty() {
            if let Ok(mut s) = self.state.lock() {
                if s.open_orders.remove(asset_id).is_some() {
                    let _ = save_state(&self.state_file, &mut s);
                }
            }
            return;
        }
        if mine.len() == 1 {
            let o = &mine[0];
            if let Some(oid) = self._extract_order_id(o) {
                let p = self._extract_order_price(o);
                let sz = self._extract_order_remaining_size(o);
                if let Ok(mut s) = self.state.lock() {
                    let local = s.open_orders.get(asset_id).and_then(|x| x.order_id.clone());
                    if local.as_deref() != Some(oid.as_str()) {
                        s.open_orders.insert(
                            asset_id.to_string(),
                            OpenOrderState {
                                order_id: Some(oid),
                                price: Some(p),
                                size: Some(sz),
                                ts: Some(now),
                            },
                        );
                        let _ = save_state(&self.state_file, &mut s);
                    }
                }
            }
            return;
        }
        let local_keep_id = self
            .state
            .lock()
            .ok()
            .and_then(|s| s.open_orders.get(asset_id).and_then(|o| o.order_id.clone()));
        let mut keep_idx: usize = 0;
        if let Some(keep_id) = local_keep_id {
            if let Some((i, _)) = mine.iter().enumerate().find(|(_, o)| {
                self._extract_order_id(o)
                    .map(|id| id == keep_id)
                    .unwrap_or(false)
            }) {
                keep_idx = i;
            }
        } else if let Some(ip) = intended_price.filter(|p| *p > 0.0) {
            let mut best = f64::INFINITY;
            for (i, o) in mine.iter().enumerate() {
                let d = (self._extract_order_price(o) - ip).abs();
                if d < best {
                    best = d;
                    keep_idx = i;
                }
            }
        } else {
            let mut best = -1.0;
            for (i, o) in mine.iter().enumerate() {
                let p = self._extract_order_price(o);
                if p > best {
                    best = p;
                    keep_idx = i;
                }
            }
        }
        let keep = mine.get(keep_idx).cloned();
        let keep_id = keep.as_ref().and_then(|o| self._extract_order_id(o));
        for o in &mine {
            let oid = self._extract_order_id(o);
            if oid.is_none() {
                continue;
            }
            let oid = oid.unwrap_or_default();
            if keep_id.as_deref() == Some(oid.as_str()) {
                continue;
            }
            self.logger.info(&format!(
                "Reconcile: cancel extra order {}.. for {}",
                oid.chars().take(10).collect::<String>(),
                asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            ));
            let _ = self._cancel(&oid);
        }
        if let (Some(keep), Some(keep_id)) = (keep, keep_id) {
            let p = self._extract_order_price(&keep);
            let sz = self._extract_order_remaining_size(&keep);
            if let Ok(mut s) = self.state.lock() {
                s.open_orders.insert(
                    asset_id.to_string(),
                    OpenOrderState {
                        order_id: Some(keep_id),
                        price: Some(p),
                        size: Some(sz),
                        ts: Some(now),
                    },
                );
                let _ = save_state(&self.state_file, &mut s);
            }
        }
    }
}

