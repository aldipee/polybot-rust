use super::*;
impl MakerHedgeCapBot {
    /// Returns or derives accumulate allowed for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _accumulate_allowed(&self) -> (bool, String) {
        let now = now_ts_f64();
        if now < (self.start_ts as f64 + self.warmup_seconds as f64) {
            return (false, "warmup".to_string());
        }
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return (false, "missing_assets".to_string()),
        };
        let y = self._best_bid_ask(yes);
        let n = self._best_bid_ask(no);
        if y.is_none() || n.is_none() {
            return (false, "missing_quotes".to_string());
        }
        let (yb, ya) = y.unwrap_or((0.0, 0.0));
        let (nb, na) = n.unwrap_or((0.0, 0.0));
        if yb <= 0.0 || ya <= 0.0 || nb <= 0.0 || na <= 0.0 {
            return (false, "zero_bid_ask".to_string());
        }
        let spr_y_ticks = (ya - yb) / self.cfg.tick.max(0.0001);
        let spr_n_ticks = (na - nb) / self.cfg.tick.max(0.0001);
        if spr_y_ticks > self.max_spread_ticks as f64 || spr_n_ticks > self.max_spread_ticks as f64
        {
            return (
                false,
                format!("wide_spread(y={spr_y_ticks:.1} n={spr_n_ticks:.1})"),
            );
        }
        let mid_y = 0.5 * (yb + ya);
        let mid_n = 0.5 * (nb + na);
        let parity = mid_y + mid_n;
        if (parity - 1.0).abs() > self.parity_tolerance {
            return (false, format!("parity_off({parity:.3})"));
        }
        (true, "ok".to_string())
    }
    /// Implements quote only allowed for the maker-side BOT workflow.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub(in crate::bot) fn _maker_quote_only_allowed(&self, yes: &str, no: &str) -> (bool, String) {
        let y = self._best_bid_ask(yes);
        let n = self._best_bid_ask(no);
        if y.is_none() || n.is_none() {
            return (false, "missing_quotes".to_string());
        }
        let (yb, ya) = y.unwrap_or((0.0, 0.0));
        let (nb, na) = n.unwrap_or((0.0, 0.0));
        if yb <= 0.0 || ya <= 0.0 || nb <= 0.0 || na <= 0.0 {
            return (false, "zero_bid_ask".to_string());
        }
        let tick = self.cfg.tick.max(0.0001);
        let spr_y_ticks = (ya - yb) / tick;
        let spr_n_ticks = (na - nb) / tick;
        if spr_y_ticks > self.max_spread_ticks as f64 || spr_n_ticks > self.max_spread_ticks as f64
        {
            return (
                false,
                format!("spread_too_wide(y={spr_y_ticks:.1} n={spr_n_ticks:.1})"),
            );
        }
        let mid_y = 0.5 * (yb + ya);
        let mid_n = 0.5 * (nb + na);
        let parity = mid_y + mid_n;
        if (parity - 1.0).abs() > self.parity_tolerance {
            return (false, format!("parity_off({parity:.3})"));
        }
        (true, "ok".to_string())
    }
    /// Returns or derives paired quotes active for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _paired_quotes_active(&self) -> bool {
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y, n),
            _ => return false,
        };
        self.state
            .lock()
            .map(|s| s.open_orders.contains_key(yes) && s.open_orders.contains_key(no))
            .unwrap_or(false)
    }
    /// Returns or derives quotes invalidated for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _quotes_invalidated(&self) -> (bool, String) {
        if !env_bool("QUOTE_INVALIDATION_ENABLED", true) {
            return (false, "disabled".to_string());
        }
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return (false, "missing_assets".to_string()),
        };
        let yq = self._best_bid_ask(yes);
        let nq = self._best_bid_ask(no);
        if yq.is_none() || nq.is_none() {
            return (false, "missing_quotes".to_string());
        }
        let (_, y_ask) = yq.unwrap_or((0.0, 0.0));
        let (_, n_ask) = nq.unwrap_or((0.0, 0.0));
        if y_ask <= 0.0 || n_ask <= 0.0 {
            return (false, "zero_ask".to_string());
        }
        let buf = env_float("QUOTE_INVALIDATION_BUFFER_TICKS", 0.0) * self.cfg.tick.max(0.0001);
        let mut reasons: Vec<String> = Vec::new();
        if let Ok(s) = self.state.lock() {
            if let Some(y_o) = s.open_orders.get(yes) {
                let y_p = y_o.price.unwrap_or(0.0);
                if y_p > 0.0 && n_ask > (1.0 - y_p - buf) {
                    reasons.push(format!(
                        "YES bid {y_p:.2} + NO ask {n_ask:.2} > {:.2}",
                        1.0 - buf
                    ));
                }
            }
            if let Some(n_o) = s.open_orders.get(no) {
                let n_p = n_o.price.unwrap_or(0.0);
                if n_p > 0.0 && y_ask > (1.0 - n_p - buf) {
                    reasons.push(format!(
                        "NO bid {n_p:.2} + YES ask {y_ask:.2} > {:.2}",
                        1.0 - buf
                    ));
                }
            }
            if let (Some(y_o), Some(n_o)) = (s.open_orders.get(yes), s.open_orders.get(no)) {
                let y_p = y_o.price.unwrap_or(0.0);
                let n_p = n_o.price.unwrap_or(0.0);
                let min_edge = env_int("MIN_ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks) as i64;
                let edge_ticks = self.cfg.entry_edge_ticks.max(min_edge);
                let entry_edge = edge_ticks as f64 * self.cfg.tick.max(0.0001);
                if (y_p + n_p) > (1.0 - entry_edge) {
                    reasons.push(format!(
                        "edge_lost(sum={:.2} > {:.2})",
                        y_p + n_p,
                        1.0 - entry_edge
                    ));
                }
            }
        }
        if reasons.is_empty() {
            (false, "ok".to_string())
        } else {
            (true, reasons.join("; "))
        }
    }
}

