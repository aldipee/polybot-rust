use super::*;

impl MakerHedgeCapBot {
    /// Sets exit reason on shared BOT state.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _set_exit_reason(&self, reason: &str) {
        if let Ok(mut r) = self.exit_reason.lock() {
            *r = reason.to_string();
        }
    }

    /// Returns exit reason from the current BOT context.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _get_exit_reason(&self) -> String {
        self.exit_reason
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "UNKNOWN".to_string())
    }

    /// Returns or derives default entry reason for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _default_entry_reason(&self) -> String {
        "BOT_ENTRY".to_string()
    }

    /// Sets pending entry reason on shared BOT state.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _set_pending_entry_reason(&self, reason: &str) {
        if let Ok(mut pending) = self.pending_entry_reason.lock() {
            *pending = Some(reason.to_string());
        }
    }

    /// Takes pending entry reason out of shared BOT state.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _take_pending_entry_reason(&self) -> Option<String> {
        self.pending_entry_reason
            .lock()
            .ok()
            .and_then(|mut pending| pending.take())
    }

    pub(super) fn _bot_runtime_set_safety_gate(
        &self,
        gate: BotRuntimeSafetyGate,
        reason: &str,
        now: f64,
    ) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.safety_gate = gate;
            st.safety_gate_reason = reason.trim().to_string();
            match gate {
                BotRuntimeSafetyGate::Healthy => {
                    if now.is_finite() && now > 0.0 {
                        st.last_validation_ts = now;
                    }
                }
                BotRuntimeSafetyGate::ReconnectReconPending => {
                    if now.is_finite() && now > 0.0 {
                        st.last_reconnect_reconcile_ts = 0.0;
                    }
                }
                BotRuntimeSafetyGate::DependencyPaused => {
                    if now.is_finite() && now > 0.0 && st.dependency_pause_started_ts <= 0.0 {
                        st.dependency_pause_started_ts = now;
                    }
                    if reason.starts_with("dependency_pause:market_data_stale") {
                        st.market_data_hard_pause_latched = true;
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn _bot_runtime_mark_startup_reconciliation_pending(&self, now: f64) {
        self._bot_runtime_set_safety_gate(
            BotRuntimeSafetyGate::StartupReconPending,
            "startup_reconciliation_pending",
            now,
        );
    }

    pub(super) fn _bot_runtime_mark_reconnect_reconciliation_pending(
        &self,
        channel: &str,
        now: f64,
    ) {
        self._bot_runtime_set_safety_gate(
            BotRuntimeSafetyGate::ReconnectReconPending,
            &format!(
                "reconnect_reconciliation_pending:{}",
                channel.trim().to_ascii_lowercase()
            ),
            now,
        );
    }

    pub(super) fn _bot_runtime_enter_dependency_pause(&self, kind: &str, detail: &str, now: f64) {
        let mut reason = format!("dependency_pause:{}", kind.trim().to_ascii_lowercase());
        if !detail.trim().is_empty() {
            reason.push(':');
            reason.push_str(detail.trim());
        }
        self._bot_runtime_set_safety_gate(BotRuntimeSafetyGate::DependencyPaused, &reason, now);
    }

    pub(super) fn _bot_runtime_mark_validation_failed(&self, reason: &str, now: f64) {
        self._bot_runtime_set_safety_gate(BotRuntimeSafetyGate::ValidationFailed, reason, now);
    }

    pub(super) fn _bot_runtime_mark_reconciliation_clean(&self, scope: &str, now: f64) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.safety_gate = BotRuntimeSafetyGate::Healthy;
            st.safety_gate_reason = format!("reconciliation_clean:{}", scope.trim());
            if now.is_finite() && now > 0.0 {
                st.last_clean_reconcile_ts = now;
                st.last_validation_ts = now;
                if scope.eq_ignore_ascii_case("reconnect") {
                    st.last_reconnect_reconcile_ts = now;
                }
            }
            st.dependency_pause_started_ts = 0.0;
            st.market_data_hard_pause_latched = false;
        }
    }

    pub(super) fn _bot_runtime_market_data_stale_status(&self) -> BotRuntimeMarketDataStaleStatus {
        let add_block_s = self.cfg.market_data_stale_add_block_seconds.max(1) as f64;
        let hard_pause_s = self.cfg.market_data_stale_hard_pause_seconds.max(1) as f64;
        let yes = self.yes_asset.clone();
        let no = self.no_asset.clone();
        let now = now_ts_f64();
        let quotes = match self.best_quotes.lock() {
            Ok(m) => m,
            Err(_) => {
                return BotRuntimeMarketDataStaleStatus {
                    stage: BotRuntimeMarketDataStaleStage::HardPaused,
                    age_seconds: hard_pause_s,
                };
            }
        };
        let mut max_age_s: f64 = 0.0;
        for aid in [yes, no].into_iter().flatten() {
            let (_, _, ts) = match quotes.get(&aid).copied() {
                Some(v) => v,
                None => {
                    return BotRuntimeMarketDataStaleStatus {
                        stage: BotRuntimeMarketDataStaleStage::HardPaused,
                        age_seconds: hard_pause_s,
                    };
                }
            };
            if ts <= 0.0 {
                return BotRuntimeMarketDataStaleStatus {
                    stage: BotRuntimeMarketDataStaleStage::HardPaused,
                    age_seconds: hard_pause_s,
                };
            }
            max_age_s = max_age_s.max((now - ts).max(0.0));
        }
        let stage = if max_age_s >= hard_pause_s {
            BotRuntimeMarketDataStaleStage::HardPaused
        } else if max_age_s >= add_block_s {
            BotRuntimeMarketDataStaleStage::AddBlocked
        } else {
            BotRuntimeMarketDataStaleStage::Fresh
        };
        BotRuntimeMarketDataStaleStatus {
            stage,
            age_seconds: max_age_s,
        }
    }

    fn _bot_runtime_persistence_healthy(&self) -> Result<(), String> {
        let (gate, reason) = self
            .bot_runtime_state
            .lock()
            .map(|st| (st.safety_gate, st.safety_gate_reason.clone()))
            .unwrap_or((BotRuntimeSafetyGate::Healthy, String::new()));
        if !matches!(gate, BotRuntimeSafetyGate::DependencyPaused)
            || !reason.starts_with("dependency_pause:database")
        {
            return Ok(());
        }
        let mut snapshot = self.state.lock().map(|state| state.clone()).map_err(|_| {
            if reason.trim().is_empty() {
                "dependency_pause:database".to_string()
            } else {
                reason.clone()
            }
        })?;
        save_state(&self.state_file, &mut snapshot).map_err(|_| {
            if reason.trim().is_empty() {
                "dependency_pause:database".to_string()
            } else {
                reason.clone()
            }
        })?;
        if reason.starts_with("dependency_pause:database:daily_liquidity") {
            let mut snapshot = self
                .daily_liquidity_state
                .lock()
                .map(|state| state.clone())
                .map_err(|_| {
                    if reason.trim().is_empty() {
                        "dependency_pause:database:daily_liquidity".to_string()
                    } else {
                        reason.clone()
                    }
                })?;
            save_daily_liquidity_state(&self.daily_liquidity_state_file, &mut snapshot).map_err(
                |_| {
                    if reason.trim().is_empty() {
                        "dependency_pause:database:daily_liquidity".to_string()
                    } else {
                        reason
                    }
                },
            )?;
        }
        Ok(())
    }

    pub(super) fn _bot_runtime_dependency_healthy(&self) -> Result<(), String> {
        if !self.market_connected.load(Ordering::SeqCst) {
            return Err("dependency_pause:market_ws".to_string());
        }
        if env_bool("REQUIRE_USER_WS_CONNECTED", true)
            && !self.user_connected.load(Ordering::SeqCst)
        {
            return Err("dependency_pause:user_ws".to_string());
        }
        self._bot_runtime_persistence_healthy()?;
        let stale_pause_active = self
            .bot_runtime_state
            .lock()
            .map(|st| st.market_data_hard_pause_latched)
            .unwrap_or(false);
        if stale_pause_active && !self._market_data_fresh() {
            return Err("dependency_pause:market_data_stale".to_string());
        }
        Ok(())
    }

    pub(super) fn _bot_runtime_save_state_or_dependency_pause(
        &self,
        state: &mut BotState,
        context: &str,
    ) -> bool {
        match save_state(&self.state_file, state) {
            Ok(()) => true,
            Err(err) => {
                let now = now_ts_f64();
                self.logger.warning(&format!(
                    "[BOT][SAFE_PAUSE] state_persist_failed context={} err={:#}",
                    context, err
                ));
                self._bot_runtime_enter_dependency_pause("database", context, now);
                self._audit_record_reconciliation_event(
                    "dependency_pause:database",
                    json!({
                        "context": context,
                        "reconcile_scope": context,
                        "reconcile_clean": false,
                        "dependency_pause_kind": "database",
                        "error": format!("{:#}", err),
                    }),
                );
                false
            }
        }
    }

    fn _bot_runtime_position_truth_snapshot(
        &self,
    ) -> Result<Option<(f64, f64, &'static str)>, String> {
        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) if !y.trim().is_empty() && !n.trim().is_empty() => {
                (y.as_str(), n.as_str())
            }
            _ => return Ok(None),
        };
        let use_data_api = env_bool("RECONCILE_USE_DATA_API", false);
        let use_legacy_balance = env_bool("MISMATCH_RECONCILE_FROM_BALANCE", false);
        if !use_data_api && !use_legacy_balance {
            return Ok(None);
        }
        if use_data_api {
            let yes_pos = self
                ._get_position_size_data_api(yes)
                .ok_or_else(|| "dependency_pause:reconciliation".to_string())?;
            let no_pos = self
                ._get_position_size_data_api(no)
                .ok_or_else(|| "dependency_pause:reconciliation".to_string())?;
            return Ok(Some((yes_pos.max(0.0), no_pos.max(0.0), "data_api")));
        }
        let (yes_bal, _) = self
            ._get_balance_allowance_conditional_cached(yes, 0.0)
            .ok_or_else(|| "dependency_pause:reconciliation".to_string())?;
        let (no_bal, _) = self
            ._get_balance_allowance_conditional_cached(no, 0.0)
            .ok_or_else(|| "dependency_pause:reconciliation".to_string())?;
        Ok(Some((yes_bal.max(0.0), no_bal.max(0.0), "legacy_balance")))
    }

    pub(super) fn _bot_runtime_run_reconciliation_gate(
        &self,
        scope: &str,
        now: f64,
    ) -> Result<(), String> {
        let pair = self.pair_identity();
        if let Some(yes) = pair.yes_asset_id.as_deref() {
            self._reconcile_exchange_orders_for_asset(yes, None, true);
        }
        if let Some(no) = pair.no_asset_id.as_deref() {
            self._reconcile_exchange_orders_for_asset(no, None, true);
        }
        let local = self
            .state
            .lock()
            .map(|state| (state.q_yes.max(0.0), state.q_no.max(0.0)))
            .map_err(|_| "dependency_pause:database".to_string())?;
        let external = self._bot_runtime_position_truth_snapshot()?;
        let threshold = self.cfg.min_shares.max(1e-6);
        if let Some((ext_yes, ext_no, source)) = external {
            let yes_delta = (local.0 - ext_yes).abs();
            let no_delta = (local.1 - ext_no).abs();
            if yes_delta > threshold || no_delta > threshold {
                let reason = "reconciliation_mismatch";
                self._audit_record_reconciliation_event(
                    reason,
                    json!({
                        "pair_id": pair.pair_id,
                        "reconcile_scope": scope,
                        "reconcile_clean": false,
                        "dependency_pause_kind": "reconciliation",
                        "source": source,
                        "local_q_yes": local.0,
                        "local_q_no": local.1,
                        "external_q_yes": ext_yes,
                        "external_q_no": ext_no,
                        "threshold": threshold,
                    }),
                );
                self._bot_runtime_mark_validation_failed(reason, now);
                return Err(reason.to_string());
            }
            self._audit_record_reconciliation_event(
                &format!("{}_clean", scope.trim()),
                json!({
                    "pair_id": pair.pair_id,
                    "reconcile_scope": scope,
                    "reconcile_clean": true,
                    "source": source,
                    "local_q_yes": local.0,
                    "local_q_no": local.1,
                    "external_q_yes": ext_yes,
                    "external_q_no": ext_no,
                }),
            );
        } else {
            self._audit_record_reconciliation_event(
                &format!("{}_clean", scope.trim()),
                json!({
                    "pair_id": pair.pair_id,
                    "reconcile_scope": scope,
                    "reconcile_clean": true,
                    "source": "local_only",
                    "local_q_yes": local.0,
                    "local_q_no": local.1,
                }),
            );
        }
        self._bot_runtime_mark_reconciliation_clean(scope, now);
        Ok(())
    }

    pub(super) fn _bot_runtime_cancel_new_risk_orders(&self, reason: &str) {
        let _ = self._bot_runtime_cancel_order_family("BOT_OPEN_BOTH", None, reason);
        let _ = self._bot_runtime_cancel_await_second_fill_orders(None, reason);
        let _ = self._bot_runtime_cancel_pair_build_orders(None, reason);
        let _ = self._bot_runtime_cancel_taper_orders(None, reason);
    }

    /// Computes env first for the BOT runtime.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _env_first(keys: &[&str]) -> String {
        for key in keys {
            if let Ok(v) = std::env::var(key) {
                let t = v.trim();
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
        String::new()
    }

    /// Returns or derives user WS auth for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _user_ws_auth(&self) -> Option<Value> {
        if let Some(creds) = &self.clob_api_creds {
            return Some(json!({
                "apiKey": creds.key,
                "secret": creds.secret,
                "passphrase": creds.passphrase,
            }));
        }

        let api_key = Self::_env_first(&[
            "POLYMARKET_API_KEY",
            "API_KEY",
            "CLOB_API_KEY",
            "POLY_API_KEY",
        ]);
        let api_secret = Self::_env_first(&[
            "POLYMARKET_API_SECRET",
            "API_SECRET",
            "CLOB_API_SECRET",
            "POLY_API_SECRET",
        ]);
        let passphrase = Self::_env_first(&[
            "POLYMARKET_API_PASSPHRASE",
            "API_PASSPHRASE",
            "CLOB_API_PASSPHRASE",
            "POLY_API_PASSPHRASE",
        ]);
        if api_key.is_empty() || api_secret.is_empty() || passphrase.is_empty() {
            return None;
        }
        Some(json!({
            "apiKey": api_key,
            "secret": api_secret,
            "passphrase": passphrase,
        }))
    }

    /// Sets WS stream timeouts on shared BOT state.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _set_ws_stream_timeouts(
        &self,
        ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        timeout: Duration,
    ) {
        match ws.get_mut() {
            MaybeTlsStream::Plain(sock) => {
                let _ = sock.set_read_timeout(Some(timeout));
                let _ = sock.set_write_timeout(Some(timeout));
            }
            MaybeTlsStream::Rustls(sock) => {
                let tcp = &mut sock.sock;
                let _ = tcp.set_read_timeout(Some(timeout));
                let _ = tcp.set_write_timeout(Some(timeout));
            }
            _ => {}
        }
    }

    /// Returns or derives run for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn run(&self) -> Result<String> {
        if self.yes_asset.is_none() || self.no_asset.is_none() {
            return Err(anyhow!("NO_MARKET"));
        }
        let reason = thread::scope(|scope| {
            scope.spawn(|| self._ws_runner("market"));
            scope.spawn(|| self._ws_runner("user"));
            let out = self._run_bot_runtime_loop();
            self.stop();
            out
        });

        Ok(reason)
    }

    /// Initializes CLOB client for the BOT runtime.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _init_clob_client(&self) -> Option<Value> {
        if self.clob_client.is_none() {
            return None;
        }
        Some(json!({
            "host": self.cfg.clob_host,
            "gamma_host": std::env::var("CLOB_GAMMA_API_URL")
                .or_else(|_| std::env::var("GAMMA_HOST"))
                .unwrap_or_else(|_| "https://gamma-api.polymarket.com".to_string()),
            "chain_id": self.cfg.chain_id,
            "signature_type": self.cfg.signature_type,
            "funder": self.cfg.funder,
            "has_api_creds": self.clob_api_creds.is_some(),
        }))
    }

    /// Returns or derives mk WS for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _mk_ws(&self, channel: &str) -> Value {
        let base = self.cfg.ws_base.trim_end_matches('/');
        let url = format!("{base}/ws/{channel}");
        let subscribe = if channel.eq_ignore_ascii_case("market") {
            match (&self.yes_asset, &self.no_asset) {
                (Some(yes), Some(no)) => Some(json!({
                    "assets_ids": [yes, no],
                    "type": "market",
                    "custom_feature_enabled": true
                })),
                _ => None,
            }
        } else if channel.eq_ignore_ascii_case("user") {
            match (&self.condition_id, self._user_ws_auth()) {
                (Some(condition_id), Some(auth)) => Some(json!({
                    "markets": [condition_id],
                    "type": "user",
                    "auth": auth
                })),
                _ => None,
            }
        } else {
            None
        };
        json!({
            "channel": channel,
            "url": url,
            "subscribe": subscribe,
            "market_slug": self.market_slug,
        })
    }

    /// Returns or derives on open for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _on_open(&self, channel: &str) {
        let now = now_ts_f64();
        if channel.eq_ignore_ascii_case("market") {
            let reopened = self
                .bot_runtime_state
                .lock()
                .map(|state| state.market_ws_ever_opened)
                .unwrap_or(false);
            self.market_connected.store(true, Ordering::SeqCst);
            if let Ok(mut state) = self.bot_runtime_state.lock() {
                state.market_ws_ever_opened = true;
            }
            if reopened {
                self._bot_runtime_mark_reconnect_reconciliation_pending("market_ws", now);
            }
        } else if channel.eq_ignore_ascii_case("user") {
            let reopened = self
                .bot_runtime_state
                .lock()
                .map(|state| state.user_ws_ever_opened)
                .unwrap_or(false);
            self.user_connected.store(true, Ordering::SeqCst);
            if let Ok(mut state) = self.bot_runtime_state.lock() {
                state.user_ws_ever_opened = true;
            }
            if reopened {
                self._bot_runtime_mark_reconnect_reconciliation_pending("user_ws", now);
            }
        }
        self.logger.info(&format!("[{channel}] open"));
    }

    /// Returns or derives on error for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _on_error(&self, channel: &str, err: &str) {
        let now = now_ts_f64();
        if channel.eq_ignore_ascii_case("market") {
            self.market_connected.store(false, Ordering::SeqCst);
            let was_live = self
                .bot_runtime_state
                .lock()
                .map(|state| state.market_ws_ever_opened)
                .unwrap_or(false);
            if was_live {
                self._bot_runtime_enter_dependency_pause("market_ws", "error", now);
            }
        } else if channel.eq_ignore_ascii_case("user") {
            self.user_connected.store(false, Ordering::SeqCst);
            let was_live = self
                .bot_runtime_state
                .lock()
                .map(|state| state.user_ws_ever_opened)
                .unwrap_or(false);
            if was_live {
                self._bot_runtime_enter_dependency_pause("user_ws", "error", now);
            }
        }
        self.logger.error(&format!("[{channel}] error: {err}"));
    }

    /// Returns or derives on close for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _on_close(&self, channel: &str, code: i64, msg: &str) {
        let now = now_ts_f64();
        if channel.eq_ignore_ascii_case("market") {
            self.market_connected.store(false, Ordering::SeqCst);
            let was_live = self
                .bot_runtime_state
                .lock()
                .map(|state| state.market_ws_ever_opened)
                .unwrap_or(false);
            if was_live {
                self._bot_runtime_enter_dependency_pause("market_ws", "closed", now);
            }
        } else if channel.eq_ignore_ascii_case("user") {
            self.user_connected.store(false, Ordering::SeqCst);
            let was_live = self
                .bot_runtime_state
                .lock()
                .map(|state| state.user_ws_ever_opened)
                .unwrap_or(false);
            if was_live {
                self._bot_runtime_enter_dependency_pause("user_ws", "closed", now);
            }
        }
        self.logger
            .warning(&format!("[{channel}] closed: {code} {msg}"));
    }

    /// Returns or derives ping loop for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _ping_loop(&self, channel: &str) {
        self._dbg(
            &format!("[{channel}] ping"),
            &format!("ping_{channel}"),
            Some(10.0),
        );
    }

    /// Returns or derives WS runner for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _ws_runner(&self, channel: &str) {
        let mut backoff = self.cfg.ws_reconnect_min.max(0.1);
        let ping_interval = env_float("WS_PING_INTERVAL", 10.0).max(1.0);
        let io_timeout = env_float("WS_IO_TIMEOUT_SECONDS", 1.0).max(0.25);

        while !self.stop_flag.load(Ordering::SeqCst) {
            if channel.eq_ignore_ascii_case("market") {
                self.market_connected.store(false, Ordering::SeqCst);
            } else if channel.eq_ignore_ascii_case("user") {
                self.user_connected.store(false, Ordering::SeqCst);
            }

            let ws_meta = self._mk_ws(channel);
            let url = ws_meta
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if url.trim().is_empty() {
                self._on_error(channel, "missing ws url");
                break;
            }

            let (mut ws, _) = match connect(url.as_str()) {
                Ok(v) => v,
                Err(e) => {
                    self._on_error(channel, &format!("connect error: {e}"));
                    let sleep_for = backoff.min(self.cfg.ws_reconnect_max.max(0.1))
                        * (0.7 + rand::thread_rng().gen_range(0.0..0.6));
                    self.logger
                        .info(&format!("[{channel}] reconnecting in {sleep_for:.1}s..."));
                    thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
                    backoff = (backoff * 2.0).min(self.cfg.ws_reconnect_max.max(0.1));
                    continue;
                }
            };

            self._set_ws_stream_timeouts(&mut ws, Duration::from_secs_f64(io_timeout));
            self._on_open(channel);
            backoff = self.cfg.ws_reconnect_min.max(0.1);

            if let Some(sub) = ws_meta.get("subscribe").filter(|v| !v.is_null()) {
                let text = sub.to_string();
                if let Err(e) = ws.send(Message::Text(text.into())) {
                    self._on_error(channel, &format!("subscribe error: {e}"));
                    self._on_close(channel, 1006, "subscribe failed");
                    continue;
                }
            } else if channel.eq_ignore_ascii_case("user") {
                self.logger.warning(
                    "[user] missing ws auth or condition id; user feed will be unavailable",
                );
            }

            let mut last_ping = Instant::now();
            let mut close_code: i64 = 1000;
            let mut close_msg = "reconnect".to_string();
            while !self.stop_flag.load(Ordering::SeqCst) {
                if last_ping.elapsed() >= Duration::from_secs_f64(ping_interval) {
                    self._ping_loop(channel);
                    if let Err(e) = ws.send(Message::Ping(Vec::new().into())) {
                        close_code = 1006;
                        close_msg = format!("ping failed: {e}");
                        self._on_error(channel, &close_msg);
                        break;
                    }
                    last_ping = Instant::now();
                }

                match ws.read() {
                    Ok(msg) => match msg {
                        Message::Text(text) => {
                            if channel.eq_ignore_ascii_case("market") {
                                self.on_market_message(text.as_ref());
                            } else if channel.eq_ignore_ascii_case("user") {
                                self.on_user_message(text.as_ref());
                            }
                        }
                        Message::Binary(bin) => {
                            if let Ok(text) = String::from_utf8(bin.to_vec()) {
                                if channel.eq_ignore_ascii_case("market") {
                                    self.on_market_message(&text);
                                } else if channel.eq_ignore_ascii_case("user") {
                                    self.on_user_message(&text);
                                }
                            }
                        }
                        Message::Ping(payload) => {
                            let _ = ws.send(Message::Pong(payload));
                        }
                        Message::Pong(_) => {}
                        Message::Close(frame) => {
                            close_code = frame
                                .as_ref()
                                .map(|f| u16::from(f.code) as i64)
                                .unwrap_or(1000);
                            close_msg = frame
                                .as_ref()
                                .map(|f| f.reason.to_string())
                                .unwrap_or_else(|| "closed".to_string());
                            break;
                        }
                        _ => {}
                    },
                    Err(tungstenite::Error::Io(e))
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                    }
                    Err(tungstenite::Error::ConnectionClosed)
                    | Err(tungstenite::Error::AlreadyClosed) => {
                        close_code = 1000;
                        close_msg = "connection closed".to_string();
                        break;
                    }
                    Err(e) => {
                        close_code = 1006;
                        close_msg = e.to_string();
                        self._on_error(channel, &close_msg);
                        break;
                    }
                }
            }

            if self.stop_flag.load(Ordering::SeqCst) {
                drop(ws);
                break;
            }
            let _ = ws.close(None);
            self._on_close(channel, close_code, &close_msg);

            let mut rng = rand::thread_rng();
            let sleep_for =
                backoff.min(self.cfg.ws_reconnect_max.max(0.1)) * (0.7 + rng.gen_range(0.0..0.6));
            self.logger
                .info(&format!("[{channel}] reconnecting in {sleep_for:.1}s..."));
            thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
            backoff = (backoff * 2.0).min(self.cfg.ws_reconnect_max.max(0.1));
        }
    }

    /// Handles market event for the active BOT flow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _handle_market_event(&self, msg: &Value) {
        let et = msg
            .get("event_type")
            .or_else(|| msg.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(
            et.as_str(),
            "tick_size_change" | "ticksizechange" | "tick_size" | "ticksize"
        ) {
            if let Some(v) = self._extract_float_any(
                msg,
                &[
                    "tick_size",
                    "tickSize",
                    "new_tick_size",
                    "newTickSize",
                    "value",
                ],
            ) {
                self.logger
                    .info(&format!("tick_size change signal detected: {v:.6}"));
            }
            self.cancel_all_open_orders_local("tick size change");
            if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
                self._cancel_exchange_orders_for_assets(
                    &[y.clone(), n.clone()],
                    "tick size change",
                );
            }
            return;
        }
        if !et.is_empty() && et != "best_bid_ask" {
            return;
        }

        let asset_id = msg
            .get("asset_id")
            .or_else(|| msg.get("token_id"))
            .or_else(|| msg.get("asset"))
            .or_else(|| msg.get("token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if asset_id.is_empty() {
            return;
        }
        let bid = self
            ._extract_float_any(msg, &["best_bid", "bid", "b"])
            .unwrap_or(0.0);
        let ask = self
            ._extract_float_any(msg, &["best_ask", "ask", "a"])
            .unwrap_or(0.0);
        let ts = now_ts_f64();
        if let Ok(mut quotes) = self.best_quotes.lock() {
            quotes.insert(asset_id, (bid, ask, ts));
        }
        if let Ok(mut last) = self.market_last_update_ts.lock() {
            *last = ts;
        }
    }

    /// Returns or derives on market message for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn on_market_message(&self, message: &str) {
        if let Ok(v) = serde_json::from_str::<Value>(message) {
            if let Some(items) = v.as_array() {
                for item in items {
                    if item.is_object() {
                        self._handle_market_event(item);
                    }
                }
            } else if v.is_object() {
                self._handle_market_event(&v);
            }
        }
    }

    /// Returns or derives market data fresh for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _market_data_fresh(&self) -> bool {
        if !self.market_connected.load(Ordering::SeqCst) {
            return false;
        }
        if env_bool("REQUIRE_USER_WS_CONNECTED", true)
            && !self.user_connected.load(Ordering::SeqCst)
        {
            return false;
        }
        self._bot_runtime_market_data_stale_status().is_fresh()
    }

    /// Returns the best bid ask available in cached market data.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _best_bid_ask(&self, asset_id: &str) -> Option<(f64, f64)> {
        self.best_quotes
            .lock()
            .ok()
            .and_then(|m| m.get(asset_id).cloned().map(|(b, a, _)| (b, a)))
    }

    /// Returns the best bid ask with timestamp available in cached market data.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _best_bid_ask_with_ts(&self, asset_id: &str) -> Option<(f64, f64, f64)> {
        self.best_quotes
            .lock()
            .ok()
            .and_then(|m| m.get(asset_id).cloned())
    }

    /// Returns or derives dbg for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _dbg(&self, msg: &str, key: &str, throttle_s: Option<f64>) {
        let throttle = throttle_s.unwrap_or(env_float("DEBUG_THROTTLE_SECONDS", 1.0));
        let now = now_ts_f64();
        if let Ok(mut m) = self.debug_last_ts.lock() {
            let last = m.get(key).copied().unwrap_or(0.0);
            if now - last < throttle {
                return;
            }
            m.insert(key.to_string(), now);
        }
        self.logger.info(msg);
    }

    /// Returns or derives dbg maker for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _dbg_maker(&self, msg: &str, key: &str, throttle_s: Option<f64>) {
        self._dbg(msg, key, throttle_s);
    }

    /// Implements dbg idle for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_dbg_idle(&self, msg: &str, key: &str) {
        self._dbg_maker(msg, key, Some(5.0));
    }

    /// Returns or derives book url for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _book_url(&self) -> String {
        format!("{}/book", self.cfg.clob_host.trim_end_matches('/'))
    }

    /// Extracts float any from the provided payload or state.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _extract_float_any(&self, obj: &Value, keys: &[&str]) -> Option<f64> {
        for k in keys {
            let v = obj.get(*k)?;
            let f = match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.parse::<f64>().ok(),
                _ => None,
            };
            if f.is_some() {
                return f;
            }
        }
        None
    }

    /// Returns or derives fetch book summary http for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _fetch_book_summary_http(&self, token_id: &str) -> Option<Value> {
        let url = self._book_url();
        let timeout_s = env_float("ORDERBOOK_HTTP_TIMEOUT", 3.0).max(0.25);
        let client = Client::builder()
            .timeout(Duration::from_secs_f64(timeout_s))
            .build()
            .ok()?;
        client
            .get(url)
            .query(&[("token_id", token_id)])
            .send()
            .ok()?
            .json::<Value>()
            .ok()
    }

    /// Returns book cached from the current BOT context.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _get_book_cached(
        &self,
        token_id: &str,
        max_age_seconds: Option<f64>,
        force: bool,
    ) -> Option<Value> {
        let max_age = max_age_seconds.unwrap_or(env_float("BOOK_CACHE_TTL_SECONDS", 0.5));
        let now = now_ts_f64();
        if !force {
            if let Ok(cache) = self.book_cache.lock() {
                if let Some((v, ts)) = cache.get(token_id) {
                    if now - *ts <= max_age {
                        return Some(v.clone());
                    }
                }
            }
        }
        let book = self._fetch_book_summary_http(token_id)?;
        if let Ok(mut cache) = self.book_cache.lock() {
            cache.insert(token_id.to_string(), (book.clone(), now));
        }
        Some(book)
    }

    /// Returns or derives iter book levels for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _iter_book_levels(&self, levels: &Value) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let arr = match levels {
            Value::Array(a) => a,
            _ => return out,
        };
        for lvl in arr {
            if let Value::Object(map) = lvl {
                let p = map
                    .get("price")
                    .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()));
                let s = map
                    .get("size")
                    .or_else(|| map.get("qty"))
                    .or_else(|| map.get("quantity"))
                    .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()));
                if let (Some(px), Some(sz)) = (p, s) {
                    out.push((px, sz));
                }
            }
        }
        out
    }

    /// Returns or derives book side levels for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _book_side_levels(&self, book: &Value, side: &str) -> Option<Value> {
        let side_l = side.to_ascii_lowercase();
        if side_l.starts_with('b') {
            return book
                .get("bids")
                .cloned()
                .or_else(|| book.get("bid").cloned());
        }
        if side_l.starts_with('a') {
            return book
                .get("asks")
                .cloned()
                .or_else(|| book.get("ask").cloned());
        }
        None
    }

    /// Returns or derives cum depth for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _cum_depth(
        &self,
        token_id: &str,
        side: &str,
        price_limit: f64,
        _max_levels: Option<usize>,
        max_age_seconds: Option<f64>,
    ) -> f64 {
        let book = match self._get_book_cached(token_id, max_age_seconds, false) {
            Some(b) => b,
            None => return 0.0,
        };
        let side_levels = match self._book_side_levels(&book, side) {
            Some(v) => v,
            None => return 0.0,
        };
        let levels = self._iter_book_levels(&side_levels);
        let mut total = 0.0;
        let ask_side = side.to_ascii_lowercase().starts_with('a');
        for (px, sz) in levels {
            let ok = if ask_side {
                px <= price_limit + 1e-12
            } else {
                px >= price_limit - 1e-12
            };
            if ok {
                total += sz.max(0.0);
            }
        }
        total
    }

    /// Applies tick dependent params to the current BOT state.
    /// This updates bot-owned state, clients, or caches that the active BOT runtime depends on.

    pub fn _apply_tick_dependent_params(&mut self) {
        if self.cfg.tick <= 0.0 {
            self.cfg.tick = 0.01;
        }
        self.max_spread_ticks = self.max_spread_ticks.max(1);
        self.hedge_slippage_ticks = self.hedge_slippage_ticks.max(0);
    }

    /// Implements Sync market params from book for the active BOT execution path.
    /// This updates bot-owned state, clients, or caches that the active BOT runtime depends on.

    pub fn _sync_market_params_from_book(&mut self, force: bool) {
        if !force && !env_bool("AUTO_DETECT_MARKET_PARAMS", true) {
            return;
        }
        self._apply_tick_dependent_params();
    }

    /// Returns or derives depth gate accumulate for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _depth_gate_accumulate(
        &self,
        size: f64,
        y_bid: f64,
        n_bid: f64,
        buf: f64,
    ) -> (bool, String) {
        if size <= 0.0 {
            return (false, "size<=0".to_string());
        }
        if y_bid <= 0.0 || n_bid <= 0.0 {
            return (false, "missing bid".to_string());
        }
        let pair = y_bid + n_bid;
        if pair > 1.0 - buf + 1e-12 {
            return (
                false,
                format!("pair too expensive: y_bid+n_bid={pair:.4} buf={buf:.4}"),
            );
        }
        (true, "ok".to_string())
    }

    /// Returns or derives reconcile state from positions for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _reconcile_state_from_positions(&self, reason: &str) -> bool {
        // Primary source is Data API positions. Legacy balance-based mode can be
        // explicitly enabled, but mixed per-leg fallback is intentionally disabled.
        let pair_id = self.pair_identity().pair_id;
        let use_data_api = env_bool("RECONCILE_USE_DATA_API", false);
        let use_legacy_balance = env_bool("MISMATCH_RECONCILE_FROM_BALANCE", false);
        if !use_data_api && !use_legacy_balance {
            return false;
        }

        let now = now_ts_f64();
        let min_interval = env_float("RECONCILE_MIN_INTERVAL_SECONDS", 5.0).max(0.1);
        if let Ok(last) = self.reconcile_last_ts.lock() {
            if now - *last < min_interval {
                return false;
            }
        }

        let (yes, no) = match (&self.yes_asset, &self.no_asset) {
            (Some(y), Some(n)) => (y.as_str(), n.as_str()),
            _ => return false,
        };
        let (yes_bal, no_bal) = if use_data_api {
            let yes_pos = self._get_position_size_data_api(yes);
            let no_pos = self._get_position_size_data_api(no);
            match (yes_pos, no_pos) {
                (Some(y), Some(n)) => (y, n),
                _ => return false,
            }
        } else {
            // Legacy mode only.
            let by = self._get_balance_allowance_conditional_cached(yes, 0.0);
            let bn = self._get_balance_allowance_conditional_cached(no, 0.0);
            match (by, bn) {
                (Some((yb, _)), Some((nb, _))) => (yb, nb),
                _ => return false,
            }
        };

        // Skip if either source returned invalid data.
        if yes_bal < -0.5 || no_bal < -0.5 {
            return false;
        }

        if let Ok(mut last) = self.reconcile_last_ts.lock() {
            *last = now;
        }

        let mut changed = false;
        let y_ba = self._best_bid_ask(yes);
        let n_ba = self._best_bid_ask(no);
        let tick = self.cfg.tick.max(0.0001);
        let mut y_ask = y_ba.map(|(_, a)| a).unwrap_or(0.0);
        let mut n_ask = n_ba.map(|(_, a)| a).unwrap_or(0.0);
        let mut y_bid = y_ba.map(|(b, _)| b).unwrap_or(0.0);
        let mut n_bid = n_ba.map(|(b, _)| b).unwrap_or(0.0);
        y_ask = clamp(if y_ask > 0.0 { y_ask } else { 0.99 }, tick, 0.99);
        n_ask = clamp(if n_ask > 0.0 { n_ask } else { 0.99 }, tick, 0.99);
        y_bid = clamp(if y_bid > 0.0 { y_bid } else { tick }, tick, 0.99);
        n_bid = clamp(if n_bid > 0.0 { n_bid } else { tick }, tick, 0.99);
        let sell_credit_mult = self.reconcile_sell_credit_mult.max(0.0);

        let mut new_q_yes;
        let mut new_q_no;
        let mut new_c_yes;
        let mut new_c_no;
        if let Ok(s) = self.state.lock() {
            new_q_yes = s.q_yes;
            new_q_no = s.q_no;
            new_c_yes = s.c_yes;
            new_c_no = s.c_no;
        } else {
            return false;
        }

        let confirm_delay = env_float("RECONCILE_CONFIRM_DELAY_SECONDS", 3.0).max(0.5);
        let never_zero = env_bool("RECONCILE_NEVER_ZERO_WITHOUT_CONFIRM", true);
        let delta_threshold = self.cfg.min_shares.max(1e-6);

        // --- YES reconciliation ---
        if yes_bal > new_q_yes + delta_threshold {
            // Data API shows MORE than we track ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â missed fills ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢ trust immediately.
            let dq = yes_bal - new_q_yes;
            new_c_yes += dq * y_ask;
            new_q_yes = yes_bal;
            changed = true;
            // Clear suspect since we're adjusting upward
            if let Ok(mut s) = self.reconcile_suspect_yes.lock() {
                *s = None;
            }
        } else if yes_bal + delta_threshold < new_q_yes {
            // Data API shows LESS than we track ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â possible stale data or real sell.
            // Require dual-confirmation: discrepancy must persist across two checks.
            let dq = new_q_yes - yes_bal;

            // Safety: never zero out a large position from a single API check.
            if never_zero && yes_bal < 1e-6 && new_q_yes >= self.cfg.min_shares {
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_yes.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - yes_bal).abs() < 1e-6 => {
                            // Same zero reading twice ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â check delay
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            // First time seeing this discrepancy ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â record and wait
                            *suspect = Some((now, yes_bal));
                            self.logger.warning(&format!(
                                "[RECONCILE] pair_id={} YES suspect: internal={new_q_yes:.2} api={yes_bal:.2} ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â waiting {confirm_delay:.1}s to confirm ({reason})",
                                pair_id
                            ));
                        }
                    }
                }
                if !confirmed {
                    // Don't apply yet ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â wait for confirmation
                } else {
                    new_c_yes -= dq * y_bid * sell_credit_mult;
                    new_q_yes = yes_bal;
                    changed = true;
                    self.logger.warning(&format!(
                        "[RECONCILE] pair_id={} YES confirmed zero after delay: internalÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢{yes_bal:.2} ({reason})",
                        pair_id
                    ));
                }
            } else {
                // Non-zero downward adjustment ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â apply with standard dual-confirm
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_yes.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - yes_bal).abs() < delta_threshold => {
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            *suspect = Some((now, yes_bal));
                        }
                    }
                }
                if confirmed {
                    new_c_yes -= dq * y_bid * sell_credit_mult;
                    new_q_yes = yes_bal;
                    changed = true;
                }
            }
        } else {
            // Consistent ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â clear suspect
            if let Ok(mut s) = self.reconcile_suspect_yes.lock() {
                *s = None;
            }
        }

        // --- NO reconciliation (same logic) ---
        if no_bal > new_q_no + delta_threshold {
            let dq = no_bal - new_q_no;
            new_c_no += dq * n_ask;
            new_q_no = no_bal;
            changed = true;
            if let Ok(mut s) = self.reconcile_suspect_no.lock() {
                *s = None;
            }
        } else if no_bal + delta_threshold < new_q_no {
            let dq = new_q_no - no_bal;

            if never_zero && no_bal < 1e-6 && new_q_no >= self.cfg.min_shares {
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_no.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - no_bal).abs() < 1e-6 => {
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            *suspect = Some((now, no_bal));
                            self.logger.warning(&format!(
                                "[RECONCILE] pair_id={} NO suspect: internal={new_q_no:.2} api={no_bal:.2} ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â waiting {confirm_delay:.1}s to confirm ({reason})",
                                pair_id
                            ));
                        }
                    }
                }
                if confirmed {
                    new_c_no -= dq * n_bid * sell_credit_mult;
                    new_q_no = no_bal;
                    changed = true;
                    self.logger.warning(&format!(
                        "[RECONCILE] pair_id={} NO confirmed zero after delay: internalÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â ÃƒÂ¢Ã¢â€šÂ¬Ã¢â€žÂ¢{no_bal:.2} ({reason})",
                        pair_id
                    ));
                }
            } else {
                let mut confirmed = false;
                if let Ok(mut suspect) = self.reconcile_suspect_no.lock() {
                    match *suspect {
                        Some((ts, prev_bal)) if (prev_bal - no_bal).abs() < delta_threshold => {
                            if now - ts >= confirm_delay {
                                confirmed = true;
                                *suspect = None;
                            }
                        }
                        _ => {
                            *suspect = Some((now, no_bal));
                        }
                    }
                }
                if confirmed {
                    new_c_no -= dq * n_bid * sell_credit_mult;
                    new_q_no = no_bal;
                    changed = true;
                }
            }
        } else {
            if let Ok(mut s) = self.reconcile_suspect_no.lock() {
                *s = None;
            }
        }

        if !changed {
            return false;
        }
        new_c_yes = new_c_yes.max(0.0);
        new_c_no = new_c_no.max(0.0);
        if let Ok(mut s) = self.state.lock() {
            s.q_yes = new_q_yes;
            s.q_no = new_q_no;
            s.c_yes = new_c_yes;
            s.c_no = new_c_no;
            let _ = self._bot_runtime_save_state_or_dependency_pause(
                &mut s,
                "reconcile_state_from_positions",
            );
        }
        let tag = if reason.trim().is_empty() {
            String::new()
        } else {
            format!(" ({reason})")
        };
        self._audit_record_reconciliation_event(
            if reason.trim().is_empty() {
                "reconcile_state_from_positions"
            } else {
                reason
            },
            json!({
                "pair_id": pair_id,
                "reason_code": reason,
                "use_data_api": use_data_api,
                "use_legacy_balance": use_legacy_balance,
                "q_yes": new_q_yes.max(0.0),
                "q_no": new_q_no.max(0.0),
                "total_cost": (new_c_yes + new_c_no).max(0.0),
                "yes_balance": yes_bal.max(0.0),
                "no_balance": no_bal.max(0.0),
            }),
        );
        self.logger.warning(&format!(
            "Reconciled state from positions pair_id={}{} qYES={new_q_yes:.6} qNO={new_q_no:.6} total_cost={:.4}",
            pair_id,
            tag,
            new_c_yes + new_c_no
        ));
        true
    }

    /// Returns or derives chunked unwind heavy leg for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _chunked_unwind_heavy_leg(&self, _delta: f64, reason: &str) {
        let tick = if self.cfg.tick > 0.0 {
            self.cfg.tick
        } else {
            0.01
        };
        let _ = self._reconcile_state_from_positions(&format!("unwind:{reason}"));
        let (qy, qn) = self
            .state
            .lock()
            .map(|s| (s.q_yes, s.q_no))
            .unwrap_or((0.0, 0.0));
        let d = qy - qn;
        if d.abs() < self.cfg.min_shares {
            return;
        }
        let min_int = ((self.cfg.min_shares - 1e-12).ceil() as i64).max(1);
        let remaining = (d.abs() + 1e-12).floor() as i64;
        if remaining < min_int {
            return;
        }
        let mut chunk = env_float("UNWIND_CHUNK_SHARES", self.cfg.min_shares).floor() as i64;
        if chunk < min_int {
            chunk = min_int;
        }
        let max_passes = env_int("UNWIND_MAX_PASSES", 4).max(1) as usize;
        let wait_s = env_float("UNWIND_WAIT_AFTER_ORDER_SECONDS", 0.6).max(0.05);

        self.cancel_all_open_orders_local(&format!("chunked unwind ({reason})"));
        if let (Some(y), Some(n)) = (&self.yes_asset, &self.no_asset) {
            self._cancel_exchange_orders_for_assets(
                &[y.clone(), n.clone()],
                &format!("chunked unwind ({reason})"),
            );
        }

        for i in 0..max_passes {
            if self.stop_flag.load(Ordering::SeqCst) {
                return;
            }
            let (qy2, qn2) = self
                .state
                .lock()
                .map(|s| (s.q_yes, s.q_no))
                .unwrap_or((0.0, 0.0));
            let d2 = qy2 - qn2;
            if d2.abs() < self.cfg.min_shares {
                return;
            }
            let heavy_asset = if d2 > 0.0 {
                self.yes_asset.clone()
            } else {
                self.no_asset.clone()
            };
            let Some(heavy_asset) = heavy_asset else {
                return;
            };
            let rem = (d2.abs() + 1e-12).floor() as i64;
            if rem < min_int {
                return;
            }
            let ba = self._best_bid_ask(&heavy_asset);
            let Some((bid, _)) = ba else {
                return;
            };
            if bid <= 0.0 {
                return;
            }
            let slip_ticks =
                env_int("MAKER_EXPOSURE_UNWIND_SLIPPAGE_TICKS", 0).max(0) as i64 + i as i64;
            let mut px = bid - slip_ticks as f64 * tick;
            px = clamp(round_down(px, tick), tick, 0.99);

            let mut sell_int = rem.min(chunk);
            if env_bool("UNWIND_DEPTH_GATE_ENABLED", true) {
                let levels = env_int("DEPTH_GATE_LEVELS", 50).max(1) as usize;
                let age = env_float("DEPTH_GATE_MAX_AGE_SECONDS", 1.5).max(0.05);
                let depth = self._cum_depth(&heavy_asset, "bids", px, Some(levels), Some(age));
                let mut depth_int = (depth + 1e-9).floor() as i64;
                depth_int = if depth_int >= min_int {
                    (depth_int / min_int) * min_int
                } else {
                    0
                };
                if depth_int >= min_int {
                    sell_int = sell_int.min(depth_int);
                } else {
                    continue;
                }
            }
            if sell_int < min_int {
                continue;
            }
            let ot_name = std::env::var("MAKER_EXPOSURE_UNWIND_ORDER_TYPE")
                .unwrap_or_else(|_| self.hedge_taker_order_type.clone());
            self.logger.info(&format!(
                "CHUNKED UNWIND ({reason}) heavy={} rem={rem} sell={sell_int} bid={bid:.3} px={px:.3} pass={}/{} type={}",
                heavy_asset
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>(),
                i + 1,
                max_passes,
                ot_name.to_ascii_uppercase()
            ));
            self._runtime_ts_set("__taker_inflight_until", now_ts_f64() + wait_s.max(0.75));
            let _ = self._place_taker_ask_fak(
                &heavy_asset,
                px,
                sell_int as f64,
                Some(&ot_name.to_ascii_uppercase()),
                Some(TakerExceptionReason::RecoveryBypass),
                TakerCapPolicy::RecoveryBypass,
            );
            thread::sleep(Duration::from_secs_f64(wait_s));
        }
    }

    /// Returns or derives fsm set state for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub fn _fsm_set_state(&self, new_state: &str, reason: &str) {
        if let Ok(mut st) = self.fsm_state.lock() {
            let old = st.clone();
            *st = new_state.to_string();
            self.logger
                .info(&format!("FSM {old} -> {new_state} ({reason})"));
        }
    }

    /// Applies config overrides from env to the current BOT state.
    /// This updates bot-owned state, clients, or caches that the active BOT runtime depends on.

    pub fn _apply_cfg_overrides_from_env(&mut self) {
        self.cfg.min_shares = env_float("MIN_SHARES", self.cfg.min_shares);
        self.cfg.clip_shares = env_float("CLIP_SHARES", self.cfg.clip_shares);
        self.cfg.max_total_cost = env_float("MAX_TOTAL_COST", self.cfg.max_total_cost);
        self.cfg.reserve_usd = env_float("RESERVE_USD", self.cfg.reserve_usd);
        self.cfg.dry_run = env_bool("DRY_RUN", self.cfg.dry_run);
        self.cfg.log_every = env_int("LOG_EVERY_SECONDS", self.cfg.log_every) as i64;
        self.cfg.market_data_stale_add_block_seconds = env_int(
            "MARKET_DATA_STALE_ADD_BLOCK_SECONDS",
            self.cfg.market_data_stale_add_block_seconds,
        ) as i64;
        self.cfg.market_data_stale_hard_pause_seconds = env_int(
            "MARKET_DATA_STALE_HARD_PAUSE_SECONDS",
            self.cfg.market_data_stale_hard_pause_seconds,
        ) as i64;
        self.cfg.stop_buffer_seconds =
            env_int("STOP_BUFFER_SECONDS", self.cfg.stop_buffer_seconds) as i64;
        self.cfg.entry_edge_ticks = env_int("ENTRY_EDGE_TICKS", self.cfg.entry_edge_ticks) as i64;
        self.cfg.hedge_buffer_ticks =
            env_int("HEDGE_BUFFER_TICKS", self.cfg.hedge_buffer_ticks) as i64;
        self.cfg.maker_buffer_ticks =
            env_int("MAKER_BUFFER_TICKS", self.cfg.maker_buffer_ticks) as i64;
        self.cfg.improve_bid_ticks =
            env_int("IMPROVE_BID_TICKS", self.cfg.improve_bid_ticks) as i64;
        self.cfg.replace_if_price_moves_ticks = env_int(
            "REPLACE_IF_PRICE_MOVES_TICKS",
            self.cfg.replace_if_price_moves_ticks,
        ) as i64;
        self.cfg.stale_seconds = env_int("STALE_SECONDS", self.cfg.stale_seconds) as i64;
    }

    /// Parses clip set from env into a BOT-friendly representation.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _parse_clip_set_from_env(&self, key: &str, default_values: &[i64]) -> Vec<i64> {
        let raw = std::env::var(key).unwrap_or_default();
        let mut out: Vec<i64> = raw
            .split(',')
            .filter_map(|v| v.trim().parse::<i64>().ok())
            .filter(|v| *v > 0)
            .collect();
        if out.is_empty() {
            out = default_values
                .iter()
                .copied()
                .filter(|v| *v > 0)
                .collect::<Vec<i64>>();
        }
        out.sort();
        out.dedup();
        out
    }

    /// Implements price bucket for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_price_bucket(price: f64) -> String {
        if price <= 0.0 {
            "NA".to_string()
        } else if price <= 0.20 {
            "LE_020".to_string()
        } else if price <= 0.35 {
            "020_035".to_string()
        } else if price <= 0.65 {
            "035_065".to_string()
        } else {
            "GT_065".to_string()
        }
    }

    /// Implements clip bucket for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_clip_bucket(clip: f64) -> String {
        if clip <= 0.0 {
            "NA".to_string()
        } else if clip <= 12.0 {
            "SMALL".to_string()
        } else if clip <= 36.0 {
            "MID".to_string()
        } else {
            "LARGE".to_string()
        }
    }

    /// Implements pick clip size for price for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_pick_clip_size_for_price(&self, price: f64, peak_window: bool) -> f64 {
        let small =
            self._parse_clip_set_from_env("MAKER_CLIP_SET_SMALL", &[2, 3, 5, 7, 8, 9, 10, 11, 12]);
        let mid = self._parse_clip_set_from_env("MAKER_CLIP_SET_MID", &[16, 21, 30, 35, 36]);
        let large =
            self._parse_clip_set_from_env("MAKER_CLIP_SET_LARGE", &[40, 42, 45, 48, 54, 56]);

        let mut rng = rand::thread_rng();
        let mut pick = |pool: &[i64], fallback: i64| -> i64 {
            pool.choose(&mut rng).copied().unwrap_or(fallback.max(1))
        };
        let mut clip = if price <= 0.20 {
            pick(&large, 40)
        } else if price <= 0.35 {
            pick(
                &[mid.clone(), large.clone()].concat(),
                mid.first().copied().unwrap_or(16),
            )
        } else if price <= 0.65 {
            pick(
                &[small.clone(), mid.clone()].concat(),
                mid.first().copied().unwrap_or(16),
            )
        } else {
            pick(
                &[small.clone(), mid.clone()].concat(),
                small.first().copied().unwrap_or(8),
            )
        } as f64;

        if peak_window {
            clip *= env_float("MAKER_SKEW_PEAK_CLIP_MULT", 1.25).clamp(1.0, 3.0);
        }
        clip.max(self.cfg.min_shares.max(1.0))
    }
}
