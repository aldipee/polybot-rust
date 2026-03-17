use super::*;

impl MakerHedgeCapBot {
    /// Returns or derives stop for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// Returns or derives trade metrics snapshot for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn trade_metrics_snapshot(&self) -> TradeMetrics {
        let state = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        TradeMetrics {
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
            let _ = save_state(&self.state_file, &mut state);
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
            let _ = save_state(&self.state_file, &mut s);
        }
    }
}

