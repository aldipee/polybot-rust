use super::*;

pub struct MakerHedgeCapBot {
    pub cfg: BotConfig,
    pub logger: Arc<dyn LogLike>,
    pub market_slug: String,
    pub config_version: String,
    pub(super) audit_repo: Option<BotRepository>,
    pub(super) active_trade_id: Option<String>,
    pub(super) audit_runtime_tx: Option<SyncSender<AuditWriteTask>>,
    pub(crate) replay_recorder: Option<Arc<crate::replay::ReplayRecorder>>,
    pub(crate) replay_order_acks: Arc<Mutex<VecDeque<crate::replay::ReplayOrderAck>>>,
    pub(super) pair_identity: PairIdentity,
    pub state_file: PathBuf,
    pub state: Arc<Mutex<BotState>>,
    pub daily_liquidity_state_file: PathBuf,
    pub daily_liquidity_state: Arc<Mutex<DailyLiquidityState>>,
    pub start_trade_iso: String,
    pub first_entry_fill_iso: Arc<Mutex<Option<String>>>,
    pub first_entry_reason: Arc<Mutex<Option<String>>>,
    pub pending_entry_reason: Arc<Mutex<Option<String>>>,
    pub active_entry_reason: Arc<Mutex<Option<String>>>,
    pub stop_loss_category: Arc<Mutex<Option<String>>>,
    pub exit_reason: Arc<Mutex<String>>,
    pub stop_flag: Arc<AtomicBool>,
    pub wallet_address: String,
    pub min_maker_notional: f64,
    pub min_taker_notional: f64,
    pub reconcile_sell_credit_mult: f64,
    pub first_clip_shares: f64,
    pub first_hedge_full: bool,
    pub min_entry_edge_ticks: i64,
    pub start_ts: i64,
    pub expiry_ts: i64,
    pub warmup_seconds: i64,
    pub max_spread_ticks: i64,
    pub parity_tolerance: f64,
    pub unhedged_timeout_seconds: f64,
    pub hedge_slippage_ticks: i64,
    pub hedge_taker_order_type: String,
    pub taker_order_ttl_seconds: i64,
    pub taker_fill_fallback_from_order_events: bool,
    pub taker_strict_inflight: bool,
    pub last_taker_hedge_ts: f64,
    pub taker_hedge_min_interval: f64,
    pub exec_mode: String,
    pub configured_order_mode: String,
    pub live_enabled: bool,
    pub loop_wait_seconds_maker: f64,
    pub loop_wait_seconds_taker: f64,
    pub clob_order_meta_warmup: bool,
    pub condition_id: Option<String>,
    pub market_fees_enabled: Option<bool>,
    pub yes_asset: Option<String>,
    pub no_asset: Option<String>,
    pub runtime_flags: HashMap<String, Value>,
    pub market_last_update_ts: Arc<Mutex<f64>>,
    pub best_quotes: Arc<Mutex<HashMap<String, (f64, f64, f64)>>>,
    pub market_connected: Arc<AtomicBool>,
    pub user_connected: Arc<AtomicBool>,
    pub book_cache: Arc<Mutex<HashMap<String, (Value, f64)>>>,
    pub debug_last_ts: Arc<Mutex<HashMap<String, f64>>>,
    pub fsm_state: Arc<Mutex<String>>,
    pub order_exec_context: Arc<Mutex<HashMap<String, Value>>>,
    pub(super) submit_timing_cache: Arc<Mutex<HashMap<String, Value>>>,
    pub(super) taker_orders: Arc<Mutex<HashMap<String, TakerOrderRecord>>>,
    pub latency_log: Option<Arc<LatencyLogService>>,
    pub(super) clob_rt: Option<Arc<TokioRuntime>>,
    pub(super) clob_client: Option<Arc<RsClobClient>>,
    pub(super) clob_api_creds: Option<ApiKeyCreds>,
    pub(super) balance_allowance_cache: Arc<Mutex<HashMap<String, (f64, f64, f64)>>>,
    pub(super) reconcile_suspect_yes: Arc<Mutex<Option<(f64, f64)>>>,
    pub(super) reconcile_suspect_no: Arc<Mutex<Option<(f64, f64)>>>,
    pub(super) reconcile_last_ts: Arc<Mutex<f64>>,
    pub exchange_orders_cache: Arc<Mutex<Vec<Value>>>,
    pub(super) maker_ladder_open_orders: Arc<Mutex<HashMap<String, LadderOrderState>>>,
    pub(super) maker_order_slots: Arc<Mutex<HashMap<MakerOrderKey, MakerOrderSlot>>>,
    pub(super) maker_order_index: Arc<Mutex<HashMap<String, MakerOrderKey>>>,
    pub(super) maker_exec_ledger: Arc<Mutex<MakerExecLedger>>,
    pub(crate) bot_runtime_state: Arc<Mutex<BotRuntimeState>>,
    pub(super) bot_runtime_cfg: BotRuntimeConfigSnapshot,
}

impl MakerHedgeCapBot {
    fn shared_state_wallet_suffix(wallet_address: &str) -> String {
        let slug = wallet_address
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect::<String>();
        if slug.trim_matches('_').is_empty() {
            "default".to_string()
        } else {
            slug.trim_matches('_').to_string()
        }
    }

    fn configured_mode_suffix(order_mode: &str) -> Option<&'static str> {
        match BotOrderMode::from_config_value(order_mode) {
            Some(BotOrderMode::Shadow) => Some("shadow"),
            Some(BotOrderMode::Paper) => Some("paper"),
            _ => None,
        }
    }

    fn mode_scoped_state_file_name(base: &str, configured_order_mode: &str) -> String {
        let Some(suffix) = Self::configured_mode_suffix(configured_order_mode) else {
            return base.to_string();
        };
        if let Some((stem, ext)) = base.rsplit_once('.') {
            format!("{stem}_{suffix}.{ext}")
        } else {
            format!("{base}_{suffix}")
        }
    }

    fn mode_scoped_shared_file_name(base: &str, configured_order_mode: &str) -> String {
        let Some(suffix) = Self::configured_mode_suffix(configured_order_mode) else {
            return base.to_string();
        };
        if let Some((stem, ext)) = base.rsplit_once('.') {
            format!("{stem}_{suffix}.{ext}")
        } else {
            format!("{base}_{suffix}")
        }
    }

    fn shared_state_dir() -> PathBuf {
        if let Ok(raw) = std::env::var("POLYBOT_SHARED_STATE_DIR") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        if let Ok(raw) = std::env::var("BOT_SHARED_STATE_DIR") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.file_name().and_then(|name| name.to_str()) == Some("state") {
                if let Some(instance_dir) = cwd.parent() {
                    if let Some(instances_dir) = instance_dir.parent() {
                        if instances_dir.file_name().and_then(|name| name.to_str())
                            == Some("instances")
                        {
                            if let Some(root_dir) = instances_dir.parent() {
                                return root_dir.join("shared-state");
                            }
                        }
                    }
                }
            }
            return cwd;
        }
        PathBuf::from(".")
    }

    pub(in crate::bot) fn daily_liquidity_state_file_for_wallet(
        wallet_address: &str,
        configured_order_mode: &str,
    ) -> PathBuf {
        let suffix = Self::shared_state_wallet_suffix(wallet_address);
        Self::shared_state_dir().join(Self::mode_scoped_shared_file_name(
            format!("maker_hedgecap_daily_liquidity_{suffix}.json").as_str(),
            configured_order_mode,
        ))
    }

    pub(crate) fn pending_taker_state_file_for_wallet(
        wallet_address: &str,
        configured_order_mode: &str,
    ) -> PathBuf {
        let suffix = Self::shared_state_wallet_suffix(wallet_address);
        Self::shared_state_dir().join(Self::mode_scoped_shared_file_name(
            format!("maker_hedgecap_pending_takers_{suffix}.json").as_str(),
            configured_order_mode,
        ))
    }

    pub(crate) fn gross_exposure_state_file_for_wallet(
        wallet_address: &str,
        configured_order_mode: &str,
    ) -> PathBuf {
        let suffix = Self::shared_state_wallet_suffix(wallet_address);
        Self::shared_state_dir().join(Self::mode_scoped_shared_file_name(
            format!("maker_hedgecap_gross_exposure_{suffix}.json").as_str(),
            configured_order_mode,
        ))
    }

    pub(in crate::bot) fn shared_state_lock_timeout() -> Duration {
        Duration::from_secs(5)
    }

    pub(in crate::bot) fn _hydrate_runtime_liquidity_counters_from_state(&self) {
        if let (Ok(state), Ok(mut runtime_state)) =
            (self.state.lock(), self.bot_runtime_state.lock())
        {
            runtime_state.total_fill_events = state.pair_total_fill_events;
            runtime_state.total_fill_shares = state.pair_total_fill_shares.max(0.0);
            runtime_state.maker_fill_events = state.pair_maker_fill_events;
            runtime_state.maker_fill_shares = state.pair_maker_fill_shares.max(0.0);
            runtime_state.taker_fill_events = state.pair_taker_fill_events;
            runtime_state.taker_fill_shares = state.pair_taker_fill_shares.max(0.0);
        }
        if let (Ok(daily_state), Ok(mut runtime_state)) = (
            self.daily_liquidity_state.lock(),
            self.bot_runtime_state.lock(),
        ) {
            runtime_state.daily_taker_day_key_utc = daily_state.day_key_utc.clone();
            runtime_state.daily_maker_fill_shares = daily_state.maker_fill_shares.max(0.0);
            runtime_state.daily_taker_fill_shares = daily_state.taker_fill_shares.max(0.0);
        }
    }

    pub(in crate::bot) fn _reload_daily_liquidity_state_from_disk(&self) -> DailyLiquidityState {
        let _lock = crate::helpers::acquire_companion_file_lock(
            &self.daily_liquidity_state_file,
            Self::shared_state_lock_timeout(),
        )
        .ok();
        let snapshot =
            load_daily_liquidity_state(&self.daily_liquidity_state_file).unwrap_or_else(|_| {
                self.daily_liquidity_state
                    .lock()
                    .map(|state| state.clone())
                    .unwrap_or_default()
            });
        if let Ok(mut state) = self.daily_liquidity_state.lock() {
            *state = snapshot.clone();
        }
        snapshot
    }

    pub(in crate::bot) fn _pending_taker_state_file(&self) -> PathBuf {
        let suffix = Self::shared_state_wallet_suffix(self.wallet_address.as_str());
        self._shared_state_dir()
            .join(Self::mode_scoped_shared_file_name(
                format!("maker_hedgecap_pending_takers_{suffix}.json").as_str(),
                self.configured_order_mode.as_str(),
            ))
    }

    pub(in crate::bot) fn _gross_exposure_state_file(&self) -> PathBuf {
        let suffix = Self::shared_state_wallet_suffix(self.wallet_address.as_str());
        self._shared_state_dir()
            .join(Self::mode_scoped_shared_file_name(
                format!("maker_hedgecap_gross_exposure_{suffix}.json").as_str(),
                self.configured_order_mode.as_str(),
            ))
    }

    fn _instance_working_dir(&self) -> PathBuf {
        if let Some(raw) = self
            .runtime_flags
            .get("__instance_working_dir_override")
            .and_then(|value| value.as_str())
        {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    pub(in crate::bot) fn _gross_cap_instance_key(&self) -> String {
        let instance_path = if self.state_file.is_absolute() {
            self.state_file.clone()
        } else {
            self._instance_working_dir().join(&self.state_file)
        };
        instance_path
            .to_string_lossy()
            .trim()
            .replace('\\', "/")
            .to_ascii_lowercase()
    }

    pub(in crate::bot) fn _shared_state_dir(&self) -> PathBuf {
        if let Some(raw) = self
            .runtime_flags
            .get("__shared_state_dir_override")
            .and_then(|value| value.as_str())
        {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        Self::shared_state_dir()
    }

    pub(in crate::bot) fn _configured_bot_order_mode(&self) -> BotOrderMode {
        BotOrderMode::from_config_value(self.configured_order_mode.as_str())
            .unwrap_or(BotOrderMode::Shadow)
    }

    pub(in crate::bot) fn _bot_runtime_user_ws_required(&self) -> bool {
        !matches!(self._configured_bot_order_mode(), BotOrderMode::Paper)
            && env_bool("REQUIRE_USER_WS_CONNECTED", true)
    }

    pub(in crate::bot) fn _bot_runtime_live_block_reason(&self) -> Option<String> {
        if !matches!(self._configured_bot_order_mode(), BotOrderMode::Live) {
            return None;
        }
        if !self.live_enabled {
            return Some("live_mode_disarmed".to_string());
        }
        let (safety_gate, safety_gate_reason) = self
            .bot_runtime_state
            .lock()
            .map(|state| (state.safety_gate, state.safety_gate_reason.clone()))
            .unwrap_or((BotRuntimeSafetyGate::DependencyPaused, String::new()));
        if !matches!(safety_gate, BotRuntimeSafetyGate::Healthy) {
            return Some(if safety_gate_reason.trim().is_empty() {
                safety_gate.as_str().to_string()
            } else {
                safety_gate_reason
            });
        }
        if !self.market_connected.load(Ordering::SeqCst) {
            return Some("market_ws_disconnected".to_string());
        }
        if self._bot_runtime_user_ws_required() && !self.user_connected.load(Ordering::SeqCst) {
            return Some("user_ws_disconnected".to_string());
        }
        if self._bot_runtime_persistence_healthy().is_err() {
            return Some("dependency_pause:database".to_string());
        }
        let stale_status = self._bot_runtime_market_data_stale_status();
        if !stale_status.is_fresh() {
            return Some(format!("market_data_stale:{}", stale_status.stage.as_str()));
        }
        None
    }

    pub(in crate::bot) fn _bot_runtime_effective_order_mode(&self) -> BotOrderMode {
        match self._configured_bot_order_mode() {
            BotOrderMode::Paper => BotOrderMode::Paper,
            BotOrderMode::Shadow => BotOrderMode::Shadow,
            BotOrderMode::Live => {
                if self._bot_runtime_live_block_reason().is_some() {
                    BotOrderMode::Shadow
                } else {
                    BotOrderMode::Live
                }
            }
        }
    }

    pub(in crate::bot) fn _bot_runtime_live_write_allowed(&self) -> bool {
        if self._replay_mode_active() {
            return false;
        }
        matches!(self._bot_runtime_effective_order_mode(), BotOrderMode::Live)
    }

    pub(in crate::bot) fn _bot_runtime_live_cancel_allowed(&self) -> bool {
        if self._replay_mode_active() {
            return false;
        }
        if matches!(self._bot_runtime_effective_order_mode(), BotOrderMode::Live) {
            return true;
        }
        matches!(self._configured_bot_order_mode(), BotOrderMode::Live)
            && self
                .bot_runtime_state
                .lock()
                .map(|state| state.live_order_write_armed_once)
                .unwrap_or(false)
    }

    pub(in crate::bot) fn _bot_runtime_venue_reads_allowed(&self) -> bool {
        !matches!(self._configured_bot_order_mode(), BotOrderMode::Paper)
    }

    pub(crate) fn _replay_mode_active(&self) -> bool {
        self.runtime_flags
            .get("__replay_mode")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            || crate::replay::replay_runtime_active()
    }

    pub(in crate::bot) fn _replay_next_order_ack(
        &self,
        asset_id: &str,
        side: &str,
    ) -> Option<crate::replay::ReplayOrderAck> {
        let mut queue = self.replay_order_acks.lock().ok()?;
        crate::replay::replay_take_order_ack(&mut queue, asset_id, side)
    }

    pub(crate) fn _replay_trade_id(&self) -> String {
        self.active_trade_id
            .clone()
            .unwrap_or_else(|| "replay".to_string())
    }

    pub(crate) fn _active_trade_id_opt(&self) -> Option<String> {
        self.active_trade_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn _replay_seed_active_trade_id(&mut self, trade_id: Option<String>) {
        self.active_trade_id = trade_id.filter(|value| !value.trim().is_empty());
    }

    pub(crate) fn _replay_runtime_state_snapshot_json(&self) -> Value {
        self.bot_runtime_state
            .lock()
            .map(|state| {
                json!({
                    "phase": state.phase.as_str(),
                    "owner": state.owner.as_str(),
                    "safety_gate": state.safety_gate.as_str(),
                    "safety_gate_reason": state.safety_gate_reason,
                    "live_order_write_armed_once": state.live_order_write_armed_once,
                    "audit_decision_event_count": state.audit_decision_event_count,
                    "audit_runtime_event_count": state.audit_runtime_event_count,
                })
            })
            .unwrap_or(Value::Null)
    }

    pub(crate) fn _replay_seed_market_metadata(
        &mut self,
        condition_id: Option<String>,
        yes_asset_id: Option<String>,
        no_asset_id: Option<String>,
        market_start_ts: Option<i64>,
        market_expiry_ts: Option<i64>,
    ) {
        self.condition_id = condition_id.filter(|value| !value.trim().is_empty());
        self.yes_asset = yes_asset_id.filter(|value| !value.trim().is_empty());
        self.no_asset = no_asset_id.filter(|value| !value.trim().is_empty());
        if let Some(start_ts) = market_start_ts.filter(|value| *value > 0) {
            self.start_ts = start_ts;
        }
        if let Some(expiry_ts) = market_expiry_ts.filter(|value| *value > 0) {
            self.expiry_ts = expiry_ts;
        }
        self.pair_identity.update_market_metadata(
            self.condition_id.clone(),
            self.yes_asset.clone(),
            self.no_asset.clone(),
        );
    }

    /// Builds a fully wired bot instance from a resolved, pinned config bundle,
    /// derived market metadata, and optional latency/CLOB clients.
    ///
    /// This constructor is the main assembly point for runtime dependencies and
    /// keeps one immutable config_version active for the full market lifecycle.
    pub fn new(
        resolved_cfg: ResolvedVersionedConfigBundle,
        market_slug: &str,
        bot_logger: Arc<dyn LogLike>,
    ) -> Result<Self> {
        let cfg = resolved_cfg.effective_bot_config;
        let bot_runtime_cfg = resolved_cfg.runtime_config;
        let execution_cfg = resolved_cfg.execution_config;
        let configured_order_mode = execution_cfg.order_mode.trim().to_ascii_lowercase();
        let live_enabled = execution_cfg.live_enabled;
        let config_version = resolved_cfg.snapshot.config_version.clone();
        let state_file = PathBuf::from(Self::mode_scoped_state_file_name(
            format!("maker_hedgecap_state_{market_slug}.json").as_str(),
            configured_order_mode.as_str(),
        ));
        let state = load_state(&state_file)?;
        let start_trade_iso = crate::db::now_iso_jakarta();

        let wallet_address = execution_cfg.wallet_address.trim().to_ascii_lowercase();
        let daily_liquidity_state_file = Self::daily_liquidity_state_file_for_wallet(
            wallet_address.as_str(),
            configured_order_mode.as_str(),
        );
        if let Some(parent) = daily_liquidity_state_file.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let daily_liquidity_state = load_daily_liquidity_state(&daily_liquidity_state_file)?;

        let mut start_ts = now_ts();
        let mut expiry_ts = start_ts + cfg.market_duration_seconds;
        let slug_window_start_ts = market_slug
            .split('-')
            .last()
            .and_then(|s| s.parse::<i64>().ok());
        if let Some(raw_ts) = slug_window_start_ts {
            start_ts = raw_ts;
            expiry_ts = raw_ts + cfg.market_duration_seconds;
        }
        let runtime_flags = HashMap::new();
        let latency_log = if execution_cfg.exec_latency_log_enabled
            && execution_cfg.exec_latency_file_log_enabled
        {
            Some(Arc::new(LatencyLogService::new(
                execution_cfg.exec_latency_jsonl_path.clone(),
                execution_cfg.exec_latency_csv_path.clone(),
                true,
                execution_cfg.exec_latency_jsonl_enabled,
                execution_cfg.exec_latency_csv_enabled,
                None,
            )))
        } else {
            None
        };
        let (clob_rt, clob_client, clob_api_creds) = if configured_order_mode
            .eq_ignore_ascii_case("paper")
            || crate::replay::replay_runtime_active()
        {
            (None, None, None)
        } else {
            Self::_init_native_clob_client(&cfg, &bot_logger, &execution_cfg.clob_gamma_host)?
        };

        let mut out = Self {
            cfg,
            logger: bot_logger,
            market_slug: market_slug.to_string(),
            config_version,
            audit_repo: None,
            active_trade_id: None,
            audit_runtime_tx: None,
            replay_recorder: None,
            replay_order_acks: Arc::new(Mutex::new(VecDeque::new())),
            pair_identity: PairIdentity::from_slug(market_slug),
            state_file,
            state: Arc::new(Mutex::new(state)),
            daily_liquidity_state_file,
            daily_liquidity_state: Arc::new(Mutex::new(daily_liquidity_state)),
            start_trade_iso,
            first_entry_fill_iso: Arc::new(Mutex::new(None)),
            first_entry_reason: Arc::new(Mutex::new(None)),
            pending_entry_reason: Arc::new(Mutex::new(None)),
            active_entry_reason: Arc::new(Mutex::new(None)),
            stop_loss_category: Arc::new(Mutex::new(None)),
            exit_reason: Arc::new(Mutex::new("RUNNING".to_string())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            wallet_address,
            min_maker_notional: execution_cfg.min_maker_notional,
            min_taker_notional: execution_cfg.min_taker_notional,
            reconcile_sell_credit_mult: execution_cfg.reconcile_sell_credit_mult,
            first_clip_shares: execution_cfg.first_clip_shares,
            first_hedge_full: execution_cfg.first_hedge_full,
            min_entry_edge_ticks: execution_cfg.min_entry_edge_ticks,
            start_ts,
            expiry_ts,
            warmup_seconds: execution_cfg.warmup_seconds,
            max_spread_ticks: execution_cfg.max_spread_ticks,
            parity_tolerance: execution_cfg.parity_tolerance,
            unhedged_timeout_seconds: execution_cfg.unhedged_timeout_seconds,
            hedge_slippage_ticks: execution_cfg.hedge_slippage_ticks,
            hedge_taker_order_type: execution_cfg.hedge_taker_order_type.clone(),
            taker_order_ttl_seconds: execution_cfg.taker_order_ttl_seconds,
            taker_fill_fallback_from_order_events: execution_cfg
                .taker_fill_fallback_from_order_events,
            taker_strict_inflight: execution_cfg.taker_strict_inflight,
            last_taker_hedge_ts: 0.0,
            taker_hedge_min_interval: execution_cfg.taker_hedge_min_interval,
            exec_mode: execution_cfg.exec_mode.clone(),
            configured_order_mode: configured_order_mode.clone(),
            live_enabled,
            loop_wait_seconds_maker: execution_cfg.loop_wait_seconds_maker,
            loop_wait_seconds_taker: execution_cfg.loop_wait_seconds_taker,
            clob_order_meta_warmup: execution_cfg.clob_order_meta_warmup,
            condition_id: None,
            market_fees_enabled: None,
            yes_asset: None,
            no_asset: None,
            runtime_flags,
            market_last_update_ts: Arc::new(Mutex::new(0.0)),
            best_quotes: Arc::new(Mutex::new(HashMap::new())),
            market_connected: Arc::new(AtomicBool::new(false)),
            user_connected: Arc::new(AtomicBool::new(
                matches!(
                    BotOrderMode::from_config_value(configured_order_mode.as_str()),
                    Some(BotOrderMode::Paper)
                ),
            )),
            book_cache: Arc::new(Mutex::new(HashMap::new())),
            debug_last_ts: Arc::new(Mutex::new(HashMap::new())),
            fsm_state: Arc::new(Mutex::new("ACCUMULATE".to_string())),
            order_exec_context: Arc::new(Mutex::new(HashMap::new())),
            submit_timing_cache: Arc::new(Mutex::new(HashMap::new())),
            taker_orders: Arc::new(Mutex::new(HashMap::new())),
            latency_log,
            clob_rt,
            clob_client,
            clob_api_creds,
            balance_allowance_cache: Arc::new(Mutex::new(HashMap::new())),
            reconcile_suspect_yes: Arc::new(Mutex::new(None)),
            reconcile_suspect_no: Arc::new(Mutex::new(None)),
            reconcile_last_ts: Arc::new(Mutex::new(0.0)),
            exchange_orders_cache: Arc::new(Mutex::new(Vec::new())),
            maker_ladder_open_orders: Arc::new(Mutex::new(HashMap::new())),
            maker_order_slots: Arc::new(Mutex::new(HashMap::new())),
            maker_order_index: Arc::new(Mutex::new(HashMap::new())),
            maker_exec_ledger: Arc::new(Mutex::new(MakerExecLedger::default())),
            bot_runtime_state: Arc::new(Mutex::new(BotRuntimeState::default())),
            bot_runtime_cfg,
        };

        out._hydrate_runtime_liquidity_counters_from_state();

        let effective_entry_edge_ticks = out.cfg.entry_edge_ticks.max(out.min_entry_edge_ticks);
        let stale_policy_requirement_compliant =
            crate::config::stale_data_policy_requirement_compliant(&out.cfg);
        let effective_order_mode = out._bot_runtime_effective_order_mode();
        out.logger.info(&format!(
            "[CFG_EFFECTIVE] config_version={} dry_run={} configured_order_mode={} effective_order_mode={} live_enabled={} max_total_cost={:.2} reserve_usd={:.2} min_shares={:.2} clip_shares={:.2} entry_edge_ticks={} min_entry_edge_ticks={} effective_entry_edge_ticks={} log_every={} market_data_stale_add_block={}s market_data_stale_hard_pause={}s stale_policy_requirement_compliant={} pair_gross_cap_usd={:.2} portfolio_gross_cap_usd={:.2} pair_gross_buffer_usd={:.2} portfolio_gross_buffer_usd={:.2} gross_cap_include_pending_maker={} gross_cap_include_pending_taker={} gross_cap_shared_state_ttl_seconds={:.1} stop_buffer={}s",
            out.config_version,
            out.cfg.dry_run,
            out.configured_order_mode,
            effective_order_mode.as_str(),
            out.live_enabled,
            out.cfg.max_total_cost,
            out.cfg.reserve_usd,
            out.cfg.min_shares,
            out.cfg.clip_shares,
            out.cfg.entry_edge_ticks,
            out.min_entry_edge_ticks,
            effective_entry_edge_ticks,
            out.cfg.log_every,
            out.cfg.market_data_stale_add_block_seconds,
            out.cfg.market_data_stale_hard_pause_seconds,
            stale_policy_requirement_compliant,
            out.cfg.pair_gross_deployed_cost_cap_usd,
            out.cfg.portfolio_gross_deployed_cost_cap_usd,
            out.cfg.pair_gross_deployed_cost_buffer_usd,
            out.cfg.portfolio_gross_deployed_cost_buffer_usd,
            out.cfg.gross_cap_include_pending_maker,
            out.cfg.gross_cap_include_pending_taker,
            out.cfg.gross_cap_shared_state_ttl_seconds,
            out.cfg.stop_buffer_seconds
        ));
        if !stale_policy_requirement_compliant {
            out.logger.warning(&format!(
                "[CFG_EFFECTIVE] stale_policy_noncompliant add_block={} hard_pause={} expected=2/5",
                out.cfg.market_data_stale_add_block_seconds,
                out.cfg.market_data_stale_hard_pause_seconds
            ));
        }

        if !crate::replay::replay_runtime_active() {
            if let Some(market) = fetch_market_by_slug(&out.market_slug, Some(&out.logger))? {
                out.market_fees_enabled = market
                    .get("feesEnabled")
                    .or_else(|| market.get("fees_enabled"))
                    .and_then(|v| v.as_bool());
                if let Ok((yes, no, condition)) = parse_tokens_and_condition(&market) {
                    out.condition_id = Some(condition.clone());
                    out.yes_asset = Some(yes.clone());
                    out.no_asset = Some(no.clone());
                    out.pair_identity.update_market_metadata(
                        Some(condition.clone()),
                        Some(yes.clone()),
                        Some(no.clone()),
                    );
                    if slug_window_start_ts.is_none() {
                        if let Some(st) = market
                            .get("startDate")
                            .and_then(|v| v.as_str())
                            .and_then(iso_to_epoch)
                        {
                            out.start_ts = st;
                        }
                    }
                    if let Some(et) = market
                        .get("endDate")
                        .and_then(|v| v.as_str())
                        .and_then(iso_to_epoch)
                    {
                        out.expiry_ts = et;
                    }
                    out.logger
                        .info(&format!("Market Found: {}", out.market_slug));
                    out.logger.info(&format!("Condition ID: {condition}"));
                    out.logger.info(&format!("YES asset: {yes}"));
                    out.logger.info(&format!("NO  asset: {no}"));
                    out.logger.info(&format!(
                        "Start ts: {} | Expiry ts: {}",
                        out.start_ts, out.expiry_ts
                    ));
                }
            }
        }
        out._log_bot_runtime_cfg();
        out._warm_clob_order_meta_cache();

        Ok(out)
    }

    pub fn with_trade_audit(mut self, repo: BotRepository, trade_id: &str) -> Self {
        let trimmed = trade_id.trim();
        if !trimmed.is_empty() {
            let (tx, rx) = mpsc::sync_channel::<AuditWriteTask>(1024);
            let worker_repo = repo.clone();
            let worker_logger = self.logger.clone();
            thread::spawn(move || {
                while let Ok(task) = rx.recv() {
                    match task {
                        AuditWriteTask::Runtime(row) => {
                            if let Err(err) = worker_repo.insert_trade_runtime_event(&row) {
                                worker_logger.warning(&format!(
                                    "[AUDIT] runtime_event_insert_failed event_id={} event_kind={} trade_id={} err={:#}",
                                    row.event_id, row.event_kind, row.trade_id, err
                                ));
                                let audit_drop = crate::db::TradeRuntimeEventInsert {
                                    event_id: crate::db::new_uuid(),
                                    trade_id: row.trade_id.clone(),
                                    pair_id: row.pair_id.clone(),
                                    market_slug: row.market_slug.clone(),
                                    condition_id: row.condition_id.clone(),
                                    yes_asset_id: row.yes_asset_id.clone(),
                                    no_asset_id: row.no_asset_id.clone(),
                                    config_version: row.config_version.clone(),
                                    event_kind: "audit_drop".to_string(),
                                    event_ts: crate::db::now_iso_jakarta(),
                                    decision_event_id: None,
                                    order_id: None,
                                    asset_id: None,
                                    side: None,
                                    reason_code: Some("runtime_insert_failed".to_string()),
                                    payload_json: serde_json::to_string(&json!({
                                        "drop_stage": "insert",
                                        "dropped_audit_kind": "runtime",
                                        "dropped_identifier": row.event_id,
                                        "dropped_name": row.event_kind,
                                        "error": format!("{:#}", err),
                                    }))
                                    .unwrap_or_else(|_| "{}".to_string()),
                                };
                                let _ = worker_repo.insert_trade_runtime_event(&audit_drop);
                            }
                        }
                        AuditWriteTask::Decision {
                            row,
                            trade_id,
                            latest_summary,
                        } => {
                            if let Err(err) = worker_repo.insert_trade_decision_event(&row) {
                                worker_logger.warning(&format!(
                                    "[AUDIT] decision_event_insert_failed decision_event_id={} decision_scope={} trade_id={} err={:#}",
                                    row.decision_event_id, row.decision_scope, row.trade_id, err
                                ));
                                let audit_drop = crate::db::TradeRuntimeEventInsert {
                                    event_id: crate::db::new_uuid(),
                                    trade_id: row.trade_id.clone(),
                                    pair_id: row.pair_id.clone(),
                                    market_slug: row.market_slug.clone(),
                                    condition_id: row.condition_id.clone(),
                                    yes_asset_id: row.yes_asset_id.clone(),
                                    no_asset_id: row.no_asset_id.clone(),
                                    config_version: row.config_version.clone(),
                                    event_kind: "audit_drop".to_string(),
                                    event_ts: crate::db::now_iso_jakarta(),
                                    decision_event_id: Some(row.decision_event_id.clone()),
                                    order_id: None,
                                    asset_id: None,
                                    side: None,
                                    reason_code: Some("decision_insert_failed".to_string()),
                                    payload_json: serde_json::to_string(&json!({
                                        "drop_stage": "insert",
                                        "dropped_audit_kind": "decision",
                                        "dropped_identifier": row.decision_event_id,
                                        "dropped_name": row.decision_scope,
                                        "error": format!("{:#}", err),
                                    }))
                                    .unwrap_or_else(|_| "{}".to_string()),
                                };
                                let _ = worker_repo.insert_trade_runtime_event(&audit_drop);
                                continue;
                            }
                            if let Err(err) = worker_repo
                                .upsert_trade_decision(trade_id.as_str(), &latest_summary)
                            {
                                worker_logger.warning(&format!(
                                    "[AUDIT] decision_summary_upsert_failed decision_event_id={} trade_id={} err={:#}",
                                    row.decision_event_id, trade_id, err
                                ));
                            }
                        }
                        AuditWriteTask::Flush(ack_tx) => {
                            let _ = ack_tx.send(());
                        }
                    }
                }
            });
            self.audit_repo = Some(repo);
            self.active_trade_id = Some(trimmed.to_string());
            self.audit_runtime_tx = Some(tx);
        }
        self
    }

    /// Warms the native CLOB metadata cache for the active YES/NO assets.
    ///
    /// This front-loads tick size, neg-risk, and fee lookups so the runtime loop
    /// does not pay the first-request latency while it is already trading.
    pub(super) fn _warm_clob_order_meta_cache(&self) {
        if !self.clob_order_meta_warmup {
            return;
        }
        let (rt, client) = match (&self.clob_rt, &self.clob_client) {
            (Some(rt), Some(client)) => (rt, client),
            _ => return,
        };
        let mut assets: Vec<String> = Vec::new();
        if let Some(a) = &self.yes_asset {
            if !a.trim().is_empty() {
                assets.push(a.clone());
            }
        }
        if let Some(a) = &self.no_asset {
            if !a.trim().is_empty() && !assets.iter().any(|v| v == a) {
                assets.push(a.clone());
            }
        }
        for aid in assets {
            let t0 = now_ns();
            let _ = rt.block_on(client.get_tick_size(&aid));
            let _ = rt.block_on(client.get_neg_risk(&aid));
            let _ = rt.block_on(client.get_fee_rate_bps(&aid));
            let ms = ((now_ns() - t0) as f64 / 1_000_000.0).round() as i64;
            let tail: String = aid
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            self.logger
                .info(&format!("[CLOB] warm order meta asset={tail} took={ms}ms"));
        }
    }

    /// Creates the native Rust CLOB client stack and derives authenticated API
    /// credentials from the configured private key when trading is enabled.
    ///
    /// When no private key is configured, this returns an empty client bundle so
    /// dry or partially wired runs can still construct the bot safely.
    pub(super) fn _init_native_clob_client(
        cfg: &BotConfig,
        logger: &Arc<dyn LogLike>,
        gamma_host: &str,
    ) -> Result<(
        Option<Arc<TokioRuntime>>,
        Option<Arc<RsClobClient>>,
        Option<ApiKeyCreds>,
    )> {
        let key = cfg.private_key.trim();
        if key.is_empty() {
            return Ok((None, None, None));
        }

        let rt = Arc::new(
            TokioRuntimeBuilder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow!("failed to create tokio runtime for CLOB client: {e}"))?,
        );

        let chain = match cfg.chain_id {
            137 => Chain::Polygon,
            80002 => Chain::Amoy,
            other => {
                logger.warning(&format!(
                    "Unsupported CHAIN_ID={other}, defaulting CLOB client to Polygon (137)"
                ));
                Chain::Polygon
            }
        };

        let signature_type = cfg.signature_type.and_then(|v| {
            if (0..=u8::MAX as i64).contains(&v) {
                Some(v as u8)
            } else {
                None
            }
        });
        let funder = cfg
            .funder
            .clone()
            .and_then(|v| (!v.trim().is_empty()).then_some(v));
        let normalized_key = if key.starts_with("0x") || key.starts_with("0X") {
            key.to_string()
        } else {
            format!("0x{key}")
        };
        let wallet = normalized_key
            .parse::<PrivateKeySigner>()
            .map_err(|e| anyhow!("failed to parse POLYMARKET_PRIVATE_KEY: {e}"))?;

        let unauth_client = RsClobClient::new(
            cfg.clob_host.clone(),
            gamma_host.to_string(),
            chain,
            Some(wallet.clone()),
            None,
            signature_type,
            funder.clone(),
            None,
            false,
            None,
            None,
        )
        .map_err(|e| anyhow!("failed to initialize CLOB client: {e}"))?;

        let creds = rt
            .block_on(unauth_client.create_or_derive_api_key(None))
            .map_err(|e| anyhow!("failed to derive CLOB API credentials: {e}"))?;

        let authed_client = RsClobClient::new(
            cfg.clob_host.clone(),
            gamma_host.to_string(),
            chain,
            Some(wallet),
            Some(creds.clone()),
            signature_type,
            funder,
            None,
            false,
            None,
            None,
        )
        .map_err(|e| anyhow!("failed to initialize authenticated CLOB client: {e}"))?;

        Ok((Some(rt), Some(Arc::new(authed_client)), Some(creds)))
    }

    /// Normalizes external order-type strings into the rs-clob enum used by the
    /// native client.
    pub(super) fn _clob_order_type(order_type: &str) -> ClobOrderType {
        match order_type.trim().to_ascii_uppercase().as_str() {
            "FAK" => ClobOrderType::Fak,
            "FOK" => ClobOrderType::Fok,
            "GTD" => ClobOrderType::Gtd,
            _ => ClobOrderType::Gtc,
        }
    }

    /// Maps a textual side label into the rs-clob side enum.
    pub(super) fn _clob_side(side: &str) -> Option<ClobSide> {
        match side.trim().to_ascii_uppercase().as_str() {
            "BUY" => Some(ClobSide::Buy),
            "SELL" => Some(ClobSide::Sell),
            _ => None,
        }
    }

    /// Converts a floating-point tick value into the nearest supported CLOB tick
    /// size enum.
    pub(super) fn _tick_size_from_f64(v: f64) -> TickSize {
        let vv = (v * 10_000.0).round() / 10_000.0;
        if (vv - 0.1).abs() < 1e-9 {
            TickSize::ZeroPointOne
        } else if (vv - 0.01).abs() < 1e-9 {
            TickSize::ZeroPointZeroOne
        } else if (vv - 0.001).abs() < 1e-9 {
            TickSize::ZeroPointZeroZeroOne
        } else {
            TickSize::ZeroPointZeroZeroZeroOne
        }
    }

    /// Returns the decimal precision implied by a concrete CLOB tick-size enum.
    pub(super) fn _tick_size_decimals(tick_size: TickSize) -> u32 {
        match tick_size {
            TickSize::ZeroPointOne => 1,
            TickSize::ZeroPointZeroOne => 2,
            TickSize::ZeroPointZeroZeroOne => 3,
            TickSize::ZeroPointZeroZeroZeroOne => 4,
        }
    }

    /// Computes the greatest common divisor used to reduce price/size fractions
    /// into a valid exchange quantum.
    pub(super) fn _gcd_u64(mut a: u64, mut b: u64) -> u64 {
        while b != 0 {
            let r = a % b;
            a = b;
            b = r;
        }
        a.max(1)
    }

    /// Derives the minimum valid buy-size quantum for a limit order at the given
    /// price and tick size.
    ///
    /// Polymarket limit BUY orders can require finer size quantization than the
    /// generic two-decimal share display used by the strategy.
    pub(super) fn _maker_limit_buy_size_quantum(price: f64, tick_size: TickSize) -> f64 {
        if !price.is_finite() || price <= 0.0 {
            return 0.01;
        }
        let price_dp = Self::_tick_size_decimals(tick_size);
        let denom = 10_u64.pow(price_dp);
        let price_units = (q_down(price.max(0.0), price_dp) * denom as f64).round() as u64;
        if price_units == 0 {
            return 0.01;
        }
        let gcd = Self::_gcd_u64(price_units, denom);
        ((denom / gcd) as f64 / 100.0).max(0.01)
    }

    /// Rounds an intended maker order size down to an exchange-acceptable size
    /// for the requested side and tick size.
    pub(super) fn _maker_limit_exchange_quantized_size(
        side: ClobSide,
        price: f64,
        size: f64,
        tick_size: TickSize,
    ) -> f64 {
        let size = q_down(size.max(0.0), 2).max(0.0);
        if size <= 0.0 {
            return 0.0;
        }
        let quantum = match side {
            ClobSide::Buy => Self::_maker_limit_buy_size_quantum(price, tick_size),
            ClobSide::Sell => 0.01,
        };
        if !quantum.is_finite() || quantum <= 0.010_000_001 {
            return size;
        }
        q_down(round_down(size, quantum).max(0.0), 2).max(0.0)
    }

    /// Extracts a floating-point number from a JSON value that may be stored as
    /// either a JSON number or a numeric string.
    pub(super) fn _value_f64(v: Option<&Value>) -> Option<f64> {
        v.and_then(|x| match x {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse::<f64>().ok(),
            _ => None,
        })
    }

    /// Walks an arbitrary JSON tree and returns the largest numeric value found.
    ///
    /// This is used as a permissive extractor when the upstream payload shape is
    /// inconsistent but still contains one meaningful numeric field somewhere.
    pub(super) fn _max_numeric_in_value(v: Option<&Value>) -> Option<f64> {
        // Recursively visits nested arrays and objects so loosely shaped payloads
        // can still yield a best-effort numeric maximum.
        fn walk(node: &Value, best: &mut Option<f64>) {
            match node {
                Value::Number(n) => {
                    if let Some(x) = n.as_f64() {
                        *best = Some(best.map_or(x, |b| b.max(x)));
                    }
                }
                Value::String(s) => {
                    if let Ok(x) = s.parse::<f64>() {
                        *best = Some(best.map_or(x, |b| b.max(x)));
                    }
                }
                Value::Array(a) => {
                    for it in a {
                        walk(it, best);
                    }
                }
                Value::Object(m) => {
                    for it in m.values() {
                        walk(it, best);
                    }
                }
                _ => {}
            }
        }

        let mut best = None;
        if let Some(root) = v {
            walk(root, &mut best);
        }
        best
    }

    /// Pulls a posted order id out of the different response shapes used by the
    /// exchange and local compatibility wrappers.
    pub(super) fn _extract_posted_order_id(resp: &Value) -> Option<String> {
        resp.get("orderID")
            .or_else(|| resp.get("order_id"))
            .or_else(|| resp.get("id"))
            .or_else(|| resp.get("order").and_then(|v| v.get("id")))
            .or_else(|| resp.get("order").and_then(|v| v.get("order_id")))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    /// Builds authenticated level-2 headers for direct CLOB REST requests using
    /// the currently derived API credentials and private key.
    pub(super) fn _build_l2_headers(
        &self,
        method: &str,
        request_path: &str,
        body: Option<&str>,
    ) -> Option<HashMap<String, String>> {
        let creds = self.clob_api_creds.as_ref()?;
        let rt = self.clob_rt.as_ref()?;
        let raw_key = self.cfg.private_key.trim();
        if raw_key.is_empty() {
            return None;
        }
        let normalized_key = if raw_key.starts_with("0x") || raw_key.starts_with("0X") {
            raw_key.to_string()
        } else {
            format!("0x{raw_key}")
        };
        let wallet = normalized_key.parse::<PrivateKeySigner>().ok()?;
        let headers = rt
            .block_on(create_l2_headers(
                &wallet,
                creds,
                method,
                request_path,
                body,
                None,
            ))
            .ok()?;
        Some(headers.to_headers())
    }

    /// Normalizes multiple open-order payload shapes into one local structure the
    /// bot can reconcile against consistently.
    pub(super) fn _normalize_open_orders_payload(payload: &Value) -> Vec<Value> {
        let items = if let Some(a) = payload.as_array() {
            a.clone()
        } else if let Some(a) = payload.get("data").and_then(|v| v.as_array()) {
            a.clone()
        } else if let Some(a) = payload.get("orders").and_then(|v| v.as_array()) {
            a.clone()
        } else if let Some(a) = payload.get("results").and_then(|v| v.as_array()) {
            a.clone()
        } else {
            Vec::new()
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let oid = item
                .get("id")
                .or_else(|| item.get("order_id"))
                .or_else(|| item.get("orderID"))
                .or_else(|| item.get("orderId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if oid.trim().is_empty() {
                continue;
            }
            let asset_id = item
                .get("asset_id")
                .or_else(|| item.get("token_id"))
                .or_else(|| item.get("assetId"))
                .or_else(|| item.get("tokenId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let price = Self::_value_f64(item.get("price")).unwrap_or(0.0);
            let original_size = Self::_value_f64(
                item.get("original_size")
                    .or_else(|| item.get("originalSize"))
                    .or_else(|| item.get("size")),
            )
            .unwrap_or(0.0);
            let size_matched = Self::_value_f64(
                item.get("size_matched")
                    .or_else(|| item.get("sizeMatched"))
                    .or_else(|| item.get("filled")),
            )
            .unwrap_or(0.0);
            let remaining_size = Self::_value_f64(
                item.get("remaining_size")
                    .or_else(|| item.get("remainingSize"))
                    .or_else(|| item.get("size")),
            )
            .unwrap_or_else(|| (original_size - size_matched).max(0.0));
            out.push(json!({
                "id": oid.clone(),
                "order_id": oid,
                "asset_id": asset_id.clone(),
                "token_id": asset_id,
                "side": item
                    .get("side")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_uppercase(),
                "price": price,
                "size": remaining_size,
                "remaining_size": remaining_size,
                "original_size": original_size,
                "size_matched": size_matched,
                "status": item.get("status").cloned().unwrap_or(Value::Null),
                "market": item.get("market").cloned().unwrap_or(Value::Null),
                "order_type": item.get("order_type").cloned().unwrap_or(Value::Null),
                "created_at": item.get("created_at").cloned().unwrap_or(Value::Null),
            }));
        }
        out
    }

    /// Fetches open orders from the exchange's REST endpoint and returns them in
    /// the normalized local payload format.
    pub(super) fn _list_open_orders_exchange_raw(&self) -> Option<Vec<Value>> {
        let endpoint_path = "/data/orders";
        let headers = self._build_l2_headers("GET", endpoint_path, None)?;
        let mut req = Client::new().get(format!(
            "{}{}",
            self.cfg.clob_host.trim_end_matches('/'),
            endpoint_path
        ));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(market) = self
            .condition_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            req = req.query(&[("market", market)]);
        }
        let payload = req.send().ok()?.json::<Value>().ok()?;
        Some(Self::_normalize_open_orders_payload(&payload))
    }

    /// Reads a timestamp-like runtime scratch value from the shared debug/runtime
    /// map, returning `0.0` when no value has been recorded.
    pub(super) fn _runtime_ts_get(&self, key: &str) -> f64 {
        self.debug_last_ts
            .lock()
            .ok()
            .and_then(|m| m.get(key).copied())
            .unwrap_or(0.0)
    }

    /// Stores a timestamp-like runtime scratch value in the shared debug/runtime
    /// map for later cooldown or inflight checks.
    pub(super) fn _runtime_ts_set(&self, key: &str, value: f64) {
        if let Ok(mut m) = self.debug_last_ts.lock() {
            m.insert(key.to_string(), value);
        }
    }
}
