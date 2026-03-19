use super::*;

impl MakerHedgeCapBot {
    /// Returns or derives stop for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// Returns pair identity for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(crate) fn pair_identity(&self) -> PairIdentity {
        let mut identity = self.pair_identity.clone();
        identity.update_market_metadata(
            self.condition_id.clone(),
            self.yes_asset.clone(),
            self.no_asset.clone(),
        );
        identity
    }

    /// Returns or derives pair metadata as a JSON object for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(in crate::bot) fn _pair_metadata_json(&self) -> Value {
        let identity = self.pair_identity();
        json!({
            "pair_id": identity.pair_id,
            "market_slug": identity.market_slug,
            "condition_id": identity.condition_id,
            "yes_asset_id": identity.yes_asset_id,
            "no_asset_id": identity.no_asset_id,
        })
    }

    /// Merges pair metadata into an execution or runtime record for the active BOT execution
    /// path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(in crate::bot) fn _merge_pair_metadata_into_value(&self, rec: &mut Value) {
        if !rec.is_object() {
            *rec = json!({});
        }
        let identity = self.pair_identity();
        if let Some(obj) = rec.as_object_mut() {
            obj.entry("pair_id".to_string())
                .or_insert_with(|| json!(identity.pair_id));
            obj.entry("market_slug".to_string())
                .or_insert_with(|| json!(identity.market_slug));
            obj.entry("condition_id".to_string())
                .or_insert_with(|| json!(identity.condition_id));
            obj.entry("yes_asset_id".to_string())
                .or_insert_with(|| json!(identity.yes_asset_id));
            obj.entry("no_asset_id".to_string())
                .or_insert_with(|| json!(identity.no_asset_id));
            obj.entry("config_version".to_string())
                .or_insert_with(|| json!(self.config_version));
        }
    }

    /// Returns or derives pair snapshot from explicit inputs for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(in crate::bot) fn _pair_snapshot_from_inputs(
        &self,
        phase: BotRuntimePhase,
        t_into_s: f64,
        q_yes: f64,
        q_no: f64,
        c_yes: f64,
        c_no: f64,
    ) -> PairSnapshot {
        let identity = self.pair_identity();
        let position = PairPosition {
            q_yes,
            q_no,
            c_yes,
            c_no,
        };
        let yes_quote = identity
            .yes_asset_id
            .as_deref()
            .and_then(|asset_id| self._best_bid_ask_with_ts(asset_id))
            .map(|(bid, ask, ts)| PairQuote { bid, ask, ts });
        let no_quote = identity
            .no_asset_id
            .as_deref()
            .and_then(|asset_id| self._best_bid_ask_with_ts(asset_id))
            .map(|(bid, ask, ts)| PairQuote { bid, ask, ts });
        PairSnapshot {
            identity,
            position,
            phase: phase.as_str().to_string(),
            t_into_s,
            total_cost: position.total_cost(),
            paired_size: position.paired_size(),
            unmatched_size: position.unmatched_size(),
            yes_quote,
            no_quote,
        }
    }

    /// Returns or derives pair snapshot from current bot state for the active BOT execution
    /// path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(in crate::bot) fn _pair_snapshot_from_state(
        &self,
        phase: BotRuntimePhase,
        t_into_s: f64,
    ) -> PairSnapshot {
        let (q_yes, q_no, c_yes, c_no) = self
            .state
            .lock()
            .map(|state| (state.q_yes, state.q_no, state.c_yes, state.c_no))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        self._pair_snapshot_from_inputs(phase, t_into_s, q_yes, q_no, c_yes, c_no)
    }

    /// Returns or derives trade metrics snapshot for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn trade_metrics_snapshot(&self) -> TradeMetrics {
        let state = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        let identity = self.pair_identity();
        TradeMetrics {
            pair_id: identity.pair_id,
            market_slug: identity.market_slug,
            condition_id: identity.condition_id,
            yes_asset_id: identity.yes_asset_id,
            no_asset_id: identity.no_asset_id,
            lp: locked_profit(&state),
            total_cost: state.c_yes + state.c_no,
            q_yes: state.q_yes,
            q_no: state.q_no,
            cpp: cost_per_pair(&state),
            entry_time_iso: self
                .first_entry_fill_iso
                .lock()
                .ok()
                .and_then(|v| v.clone()),
            entry_reason: self.first_entry_reason.lock().ok().and_then(|v| v.clone()),
            stop_loss_category: self.stop_loss_category.lock().ok().and_then(|v| v.clone()),
            exit_reason: self._get_exit_reason(),
            fill_count: state.seen_trade_keys.len(),
        }
    }

    /// Returns or derives persist state for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn persist_state(&self) {
        if let Ok(mut state) = self.state.lock() {
            let _ = self._bot_runtime_save_state_or_dependency_pause(
                &mut state,
                "public_tail_apply_position",
            );
        }
    }

    /// Cancels all orders exchange for the active BOT flow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn cancel_all_orders_exchange(&self, reason: &str) {
        if !reason.trim().is_empty() {
            self.logger
                .info(&format!("Cancel-all (exchange): {reason}"));
        }
        self._maker_ladder_cancel_all("cancel_all_exchange");
        let orders = self._list_open_orders_exchange();
        for o in orders {
            if let Some(oid) = self._extract_order_id(&o) {
                let _ = self._cancel(&oid);
            }
        }
        if let Ok(mut s) = self.state.lock() {
            s.open_orders.clear();
            let _ = self._bot_runtime_save_state_or_dependency_pause(
                &mut s,
                "public_tail_clear_open_orders",
            );
        }
    }
}
