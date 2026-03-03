use crate::binance_feed::{BinanceKline1m, BinanceTick};
use crate::env_utils::{env_bool, env_float, env_int};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn normalize_asset_symbol(raw: &str) -> String {
    let mut s = raw.trim().to_ascii_lowercase();
    if let Some((head, _)) = s.split_once('/') {
        s = head.to_string();
    } else if let Some((head, _)) = s.split_once('-') {
        s = head.to_string();
    }
    if s.ends_with("usdt") && s.len() > 4 {
        s = s[..s.len() - 4].to_string();
    } else if s.ends_with("usd") && s.len() > 3 {
        s = s[..s.len() - 3].to_string();
    }
    match s.as_str() {
        "bitcoin" => "btc".to_string(),
        "ethereum" => "eth".to_string(),
        "solana" => "sol".to_string(),
        "ripple" => "xrp".to_string(),
        "polygon" => "matic".to_string(),
        _ => s,
    }
}

fn parse_symbol_set(name: &str, default_symbol: &str) -> HashSet<String> {
    let raw = std::env::var(name).unwrap_or_else(|_| default_symbol.to_string());
    let mut out: HashSet<String> = raw
        .split(',')
        .map(normalize_asset_symbol)
        .filter(|s| !s.is_empty())
        .collect();
    if out.is_empty() {
        out.insert(normalize_asset_symbol(default_symbol));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakoutDirection {
    Up,
    Down,
    None,
}

impl BreakoutDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::None => "NONE",
        }
    }

    fn from_side(side: &str) -> Option<Self> {
        let s = side.trim().to_ascii_uppercase();
        if s == "YES" {
            Some(Self::Up)
        } else if s == "NO" {
            Some(Self::Down)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakoutMode {
    Required,
    Assist,
}

impl BreakoutMode {
    fn from_env() -> Self {
        match std::env::var("SNIPER_BREAKOUT_MODE")
            .unwrap_or_else(|_| "required".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "assist" => Self::Assist,
            _ => Self::Required,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle1m {
    pub minute_start_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub close_ts_ms: i64,
}

#[derive(Debug, Clone)]
pub struct MomentumConfig {
    pub enabled: bool,
    pub symbols: HashSet<String>,
    pub required_checks: usize,
    pub ema_fast: usize,
    pub ema_slow: usize,
    pub window_candles: usize,
    pub window_min_bullish: usize,
    pub max_snapshot_age_seconds: f64,
    pub candle_history: usize,
    pub log_every_seconds: f64,
}

impl MomentumConfig {
    pub fn from_env() -> Self {
        let ema_fast = env_int("SNIPER_MOMENTUM_EMA_FAST", 3).clamp(2, 100) as usize;
        let ema_slow =
            env_int("SNIPER_MOMENTUM_EMA_SLOW", 8).clamp(ema_fast as i64 + 1, 300) as usize;
        let window_candles = env_int("SNIPER_MOMENTUM_WINDOW_CANDLES", 4).clamp(1, 50) as usize;
        let window_min_bullish = env_int("SNIPER_MOMENTUM_WINDOW_MIN_BULLISH", 3)
            .clamp(1, window_candles as i64) as usize;
        Self {
            enabled: env_bool("SNIPER_MOMENTUM_CONFIRM_ENABLED", false),
            symbols: parse_symbol_set("SNIPER_MOMENTUM_SYMBOLS", "btc"),
            required_checks: env_int("SNIPER_MOMENTUM_REQUIRED_CHECKS", 2).clamp(1, 3) as usize,
            ema_fast,
            ema_slow,
            window_candles,
            window_min_bullish,
            max_snapshot_age_seconds: env_float(
                "SNIPER_MOMENTUM_MAX_SNAPSHOT_AGE_SECONDS",
                env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 2.0),
            )
            .max(0.05),
            candle_history: env_int("SNIPER_MOMENTUM_CANDLE_HISTORY", 128).clamp(32, 2048) as usize,
            log_every_seconds: env_float("SNIPER_MOMENTUM_LOG_EVERY_SECONDS", 1.0).max(0.0),
        }
    }

    pub fn enabled_for_symbol(&self, symbol_asset: &str) -> bool {
        self.enabled && self.symbols.contains(&normalize_asset_symbol(symbol_asset))
    }
}

#[derive(Debug, Clone)]
pub struct BreakoutConfig {
    pub enabled: bool,
    pub symbols: HashSet<String>,
    pub level_lookback_candles: usize,
    pub buffer_bps: f64,
    pub persistence_ms: i64,
    pub rearm_ms: i64,
    pub max_snapshot_age_seconds: f64,
    pub mode: BreakoutMode,
    pub assist_momentum_required_checks: usize,
    pub log_every_seconds: f64,
}

impl BreakoutConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: env_bool("SNIPER_BREAKOUT_ENABLED", false),
            symbols: parse_symbol_set("SNIPER_BREAKOUT_SYMBOLS", "btc"),
            level_lookback_candles: env_int("SNIPER_BREAKOUT_LEVEL_LOOKBACK_CANDLES", 3)
                .clamp(1, 50) as usize,
            buffer_bps: env_float("SNIPER_BREAKOUT_BUFFER_BPS", 5.0).clamp(0.0, 2000.0),
            persistence_ms: env_int("SNIPER_BREAKOUT_PERSISTENCE_MS", 2800).clamp(100, 120_000),
            rearm_ms: env_int("SNIPER_BREAKOUT_REARM_MS", 15_000).clamp(0, 300_000),
            max_snapshot_age_seconds: env_float(
                "SNIPER_BREAKOUT_MAX_SNAPSHOT_AGE_SECONDS",
                env_float("RTDS_ENTRY_GATE_MAX_AGE_SECONDS", 2.0),
            )
            .max(0.05),
            mode: BreakoutMode::from_env(),
            assist_momentum_required_checks: env_int(
                "SNIPER_BREAKOUT_ASSIST_MOMENTUM_REQUIRED_CHECKS",
                3,
            )
            .clamp(1, 3) as usize,
            log_every_seconds: env_float("SNIPER_BREAKOUT_LOG_EVERY_SECONDS", 1.0).max(0.0),
        }
    }

    pub fn enabled_for_symbol(&self, symbol_asset: &str) -> bool {
        self.enabled && self.symbols.contains(&normalize_asset_symbol(symbol_asset))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MomentumDecision {
    pub applied: bool,
    pub passed: bool,
    pub reason: String,
    pub required_checks: usize,
    pub checks_passed: usize,
    pub trend_ok: bool,
    pub slope_ok: bool,
    pub candles_ok: bool,
    pub ema_fast_last: Option<f64>,
    pub ema_slow_last: Option<f64>,
    pub ema_fast_prev: Option<f64>,
    pub bullish_or_bearish_count: Option<usize>,
    pub tick_age_ms: i64,
}

#[derive(Debug, Clone)]
pub struct BreakoutDecision {
    pub applied: bool,
    pub passed: bool,
    pub reason: String,
    pub triggered: bool,
    pub direction: BreakoutDirection,
    pub hk: Option<f64>,
    pub lk: Option<f64>,
    pub buffer_up: Option<f64>,
    pub buffer_dn: Option<f64>,
    pub persist_ms: i64,
    pub elapsed_ms: i64,
    pub cooldown_remaining_ms: i64,
    pub tick_age_ms: i64,
}

impl Default for BreakoutDecision {
    fn default() -> Self {
        Self {
            applied: false,
            passed: false,
            reason: String::new(),
            triggered: false,
            direction: BreakoutDirection::None,
            hk: None,
            lk: None,
            buffer_up: None,
            buffer_dn: None,
            persist_ms: 0,
            elapsed_ms: 0,
            cooldown_remaining_ms: 0,
            tick_age_ms: i64::MAX,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FilterDecision {
    pub allowed: bool,
    pub reason: String,
    pub momentum: MomentumDecision,
    pub breakout: BreakoutDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SniperFilterPersistedState {
    pub symbol_asset: String,
    pub completed_candles: Vec<Candle1m>,
    pub current_candle: Option<Candle1m>,
    pub last_tick_ts_ms: i64,
    pub last_tick_price: f64,
    pub level_hk: Option<f64>,
    pub level_lk: Option<f64>,
    pub buffer_up: Option<f64>,
    pub buffer_dn: Option<f64>,
    pub levels_computed_at_ms: i64,
    pub up_break_started_at_ms: Option<i64>,
    pub dn_break_started_at_ms: Option<i64>,
    pub last_triggered_at_ms: Option<i64>,
    pub active_trigger: BreakoutDirection,
    #[serde(default)]
    pub momentum_yes: Option<MomentumDecision>,
    #[serde(default)]
    pub momentum_no: Option<MomentumDecision>,
}

pub struct SniperFilterEngine {
    symbol_asset: String,
    momentum_cfg: MomentumConfig,
    breakout_cfg: BreakoutConfig,
    completed_candles: VecDeque<Candle1m>,
    current_candle: Option<Candle1m>,
    last_tick_ts_ms: i64,
    last_tick_price: f64,
    level_hk: Option<f64>,
    level_lk: Option<f64>,
    buffer_up: Option<f64>,
    buffer_dn: Option<f64>,
    levels_computed_at_ms: i64,
    up_break_started_at_ms: Option<i64>,
    dn_break_started_at_ms: Option<i64>,
    last_triggered_at_ms: Option<i64>,
    active_trigger: BreakoutDirection,
}

impl SniperFilterEngine {
    pub fn new(symbol_asset_hint: &str) -> Self {
        let symbol_asset = normalize_asset_symbol(symbol_asset_hint);
        let momentum_cfg = MomentumConfig::from_env();
        let breakout_cfg = BreakoutConfig::from_env();
        let history = momentum_cfg
            .candle_history
            .max(breakout_cfg.level_lookback_candles + 8);
        Self {
            symbol_asset,
            momentum_cfg,
            breakout_cfg,
            completed_candles: VecDeque::with_capacity(history),
            current_candle: None,
            last_tick_ts_ms: 0,
            last_tick_price: 0.0,
            level_hk: None,
            level_lk: None,
            buffer_up: None,
            buffer_dn: None,
            levels_computed_at_ms: 0,
            up_break_started_at_ms: None,
            dn_break_started_at_ms: None,
            last_triggered_at_ms: None,
            active_trigger: BreakoutDirection::None,
        }
    }

    pub fn new_with_configs(
        symbol_asset_hint: &str,
        momentum_cfg: MomentumConfig,
        breakout_cfg: BreakoutConfig,
    ) -> Self {
        let symbol_asset = normalize_asset_symbol(symbol_asset_hint);
        let history = momentum_cfg
            .candle_history
            .max(breakout_cfg.level_lookback_candles + 8);
        Self {
            symbol_asset,
            momentum_cfg,
            breakout_cfg,
            completed_candles: VecDeque::with_capacity(history),
            current_candle: None,
            last_tick_ts_ms: 0,
            last_tick_price: 0.0,
            level_hk: None,
            level_lk: None,
            buffer_up: None,
            buffer_dn: None,
            levels_computed_at_ms: 0,
            up_break_started_at_ms: None,
            dn_break_started_at_ms: None,
            last_triggered_at_ms: None,
            active_trigger: BreakoutDirection::None,
        }
    }

    pub fn uses_binance_feed(&self) -> bool {
        self.momentum_cfg.enabled_for_symbol(&self.symbol_asset)
            || self.breakout_cfg.enabled_for_symbol(&self.symbol_asset)
    }

    pub fn momentum_log_every_seconds(&self) -> f64 {
        self.momentum_cfg.log_every_seconds
    }

    pub fn breakout_log_every_seconds(&self) -> f64 {
        self.breakout_cfg.log_every_seconds
    }

    pub fn export_state(&self) -> SniperFilterPersistedState {
        let now = now_ms();
        let momentum_yes = if self.last_tick_ts_ms > 0 {
            Some(self.evaluate_momentum(
                BreakoutDirection::Up,
                now,
                self.momentum_cfg.required_checks,
            ))
        } else {
            None
        };
        let momentum_no = if self.last_tick_ts_ms > 0 {
            Some(self.evaluate_momentum(
                BreakoutDirection::Down,
                now,
                self.momentum_cfg.required_checks,
            ))
        } else {
            None
        };
        SniperFilterPersistedState {
            symbol_asset: self.symbol_asset.clone(),
            completed_candles: self.completed_candles.iter().cloned().collect(),
            current_candle: self.current_candle.clone(),
            last_tick_ts_ms: self.last_tick_ts_ms,
            last_tick_price: self.last_tick_price,
            level_hk: self.level_hk,
            level_lk: self.level_lk,
            buffer_up: self.buffer_up,
            buffer_dn: self.buffer_dn,
            levels_computed_at_ms: self.levels_computed_at_ms,
            up_break_started_at_ms: self.up_break_started_at_ms,
            dn_break_started_at_ms: self.dn_break_started_at_ms,
            last_triggered_at_ms: self.last_triggered_at_ms,
            active_trigger: self.active_trigger,
            momentum_yes,
            momentum_no,
        }
    }

    pub fn import_state(&mut self, st: SniperFilterPersistedState) -> bool {
        let incoming_symbol = normalize_asset_symbol(&st.symbol_asset);
        if !incoming_symbol.is_empty() && incoming_symbol != self.symbol_asset {
            return false;
        }
        self.completed_candles = st.completed_candles.into_iter().collect();
        let keep = self
            .momentum_cfg
            .candle_history
            .max(self.breakout_cfg.level_lookback_candles + 8);
        while self.completed_candles.len() > keep {
            let _ = self.completed_candles.pop_front();
        }
        self.current_candle = st.current_candle;
        self.last_tick_ts_ms = st.last_tick_ts_ms.max(0);
        self.last_tick_price = st.last_tick_price.max(0.0);
        self.level_hk = st.level_hk;
        self.level_lk = st.level_lk;
        self.buffer_up = st.buffer_up;
        self.buffer_dn = st.buffer_dn;
        self.levels_computed_at_ms = st.levels_computed_at_ms.max(0);
        self.up_break_started_at_ms = st.up_break_started_at_ms;
        self.dn_break_started_at_ms = st.dn_break_started_at_ms;
        self.last_triggered_at_ms = st.last_triggered_at_ms;
        self.active_trigger = st.active_trigger;
        true
    }

    pub fn seed_completed_klines(&mut self, klines: &[BinanceKline1m]) {
        if klines.is_empty() {
            return;
        }
        let mut rows = klines.to_vec();
        rows.sort_by_key(|k| k.open_time_ms);
        for row in rows {
            if row.close_time_ms <= 0 || row.close <= 0.0 {
                continue;
            }
            let minute_start_ms = (row.open_time_ms / 60_000) * 60_000;
            let candle = Candle1m {
                minute_start_ms,
                open: row.open,
                high: row.high.max(row.open).max(row.close),
                low: row.low.min(row.open).min(row.close),
                close: row.close,
                close_ts_ms: row.close_time_ms,
            };
            self.push_completed_candle(candle);
            self.last_tick_ts_ms = self.last_tick_ts_ms.max(row.close_time_ms);
            self.last_tick_price = row.close.max(self.last_tick_price);
        }
        self.recompute_breakout_levels(self.last_tick_ts_ms.max(now_ms()));
    }

    pub fn on_tick(&mut self, tick: &BinanceTick) -> bool {
        if tick.ts_ms <= self.last_tick_ts_ms || tick.price <= 0.0 {
            return false;
        }
        self.last_tick_ts_ms = tick.ts_ms;
        self.last_tick_price = tick.price;
        self.update_candle_from_tick(tick.ts_ms, tick.price);
        self.recompute_breakout_levels(tick.ts_ms);
        self.update_breakout_state(tick.ts_ms, tick.price);
        true
    }

    pub fn evaluate_entry(&self, side: &str, now_ms_value: i64) -> FilterDecision {
        let req_dir = match BreakoutDirection::from_side(side) {
            Some(v) => v,
            None => {
                return FilterDecision {
                    allowed: true,
                    reason: "side_not_directional".to_string(),
                    ..FilterDecision::default()
                };
            }
        };

        let breakout_on = self.breakout_cfg.enabled_for_symbol(&self.symbol_asset);
        let momentum_on = self.momentum_cfg.enabled_for_symbol(&self.symbol_asset);
        if !breakout_on && !momentum_on {
            return FilterDecision {
                allowed: true,
                reason: "filters_disabled".to_string(),
                ..FilterDecision::default()
            };
        }

        let mut out = FilterDecision::default();
        if breakout_on {
            let b = self.evaluate_breakout(req_dir, now_ms_value);
            out.breakout = b.clone();
            if !matches!(b.reason.as_str(), "ok" | "wrong_side" | "no_trigger") {
                out.allowed = false;
                out.reason = format!("breakout_{}", b.reason);
                return out;
            }

            if self.breakout_cfg.mode == BreakoutMode::Required {
                if !b.passed {
                    out.allowed = false;
                    out.reason = "breakout_required_not_triggered".to_string();
                    return out;
                }
                if momentum_on {
                    let m = self.evaluate_momentum(
                        req_dir,
                        now_ms_value,
                        self.momentum_cfg.required_checks,
                    );
                    out.momentum = m.clone();
                    if !m.passed {
                        out.allowed = false;
                        out.reason = format!("momentum_{}", m.reason);
                        return out;
                    }
                }
                out.allowed = true;
                out.reason = "ok".to_string();
                return out;
            }

            if b.passed {
                if momentum_on {
                    let m = self.evaluate_momentum(
                        req_dir,
                        now_ms_value,
                        self.momentum_cfg.required_checks,
                    );
                    out.momentum = m.clone();
                    if !m.passed {
                        out.allowed = false;
                        out.reason = format!("momentum_{}", m.reason);
                        return out;
                    }
                }
                out.allowed = true;
                out.reason = "ok".to_string();
                return out;
            }

            let strict = self.evaluate_momentum(
                req_dir,
                now_ms_value,
                self.breakout_cfg.assist_momentum_required_checks,
            );
            out.momentum = strict.clone();
            if !strict.passed {
                out.allowed = false;
                out.reason = "assist_no_breakout_and_strict_momentum_fail".to_string();
                return out;
            }
            out.allowed = true;
            out.reason = "assist_strict_momentum_fallback".to_string();
            return out;
        }

        let m = self.evaluate_momentum(req_dir, now_ms_value, self.momentum_cfg.required_checks);
        out.momentum = m.clone();
        if !m.passed {
            out.allowed = false;
            out.reason = format!("momentum_{}", m.reason);
            return out;
        }
        out.allowed = true;
        out.reason = "ok".to_string();
        out
    }

    fn update_candle_from_tick(&mut self, ts_ms: i64, price: f64) {
        let minute_start = (ts_ms / 60_000) * 60_000;
        match self.current_candle.take() {
            None => {
                self.current_candle = Some(Candle1m {
                    minute_start_ms: minute_start,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    close_ts_ms: ts_ms,
                });
            }
            Some(mut cur) => {
                if minute_start < cur.minute_start_ms {
                    self.current_candle = Some(cur);
                    return;
                }
                if minute_start == cur.minute_start_ms {
                    cur.high = cur.high.max(price);
                    cur.low = cur.low.min(price);
                    cur.close = price;
                    cur.close_ts_ms = ts_ms;
                    self.current_candle = Some(cur);
                    return;
                }
                self.push_completed_candle(cur);
                self.current_candle = Some(Candle1m {
                    minute_start_ms: minute_start,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    close_ts_ms: ts_ms,
                });
            }
        }
    }

    fn push_completed_candle(&mut self, candle: Candle1m) {
        if candle.close <= 0.0 || candle.open <= 0.0 {
            return;
        }
        if let Some(last) = self.completed_candles.back_mut() {
            if candle.minute_start_ms < last.minute_start_ms {
                return;
            }
            if candle.minute_start_ms == last.minute_start_ms {
                *last = candle;
                return;
            }
        }
        self.completed_candles.push_back(candle);
        let keep = self
            .momentum_cfg
            .candle_history
            .max(self.breakout_cfg.level_lookback_candles + 8);
        while self.completed_candles.len() > keep {
            let _ = self.completed_candles.pop_front();
        }
    }

    fn recompute_breakout_levels(&mut self, now_ms_value: i64) {
        let k = self.breakout_cfg.level_lookback_candles;
        if self.completed_candles.len() < k {
            self.level_hk = None;
            self.level_lk = None;
            self.buffer_up = None;
            self.buffer_dn = None;
            return;
        }
        let tail: Vec<&Candle1m> = self.completed_candles.iter().rev().take(k).collect();
        let mut hk: f64 = 0.0;
        let mut lk: f64 = f64::MAX;
        for c in tail {
            hk = hk.max(c.high.max(c.open).max(c.close));
            lk = lk.min(c.low.min(c.open).min(c.close));
        }
        if hk <= 0.0 || lk <= 0.0 || !hk.is_finite() || !lk.is_finite() {
            self.level_hk = None;
            self.level_lk = None;
            self.buffer_up = None;
            self.buffer_dn = None;
            return;
        }
        let buffer_mul = self.breakout_cfg.buffer_bps / 10_000.0;
        self.level_hk = Some(hk);
        self.level_lk = Some(lk);
        self.buffer_up = Some(hk * (1.0 + buffer_mul));
        self.buffer_dn = Some(lk * (1.0 - buffer_mul));
        self.levels_computed_at_ms = now_ms_value;
    }

    fn update_breakout_state(&mut self, ts_ms: i64, price: f64) {
        if self.breakout_cfg.rearm_ms > 0 {
            if let Some(last) = self.last_triggered_at_ms {
                if ts_ms.saturating_sub(last) >= self.breakout_cfg.rearm_ms {
                    self.active_trigger = BreakoutDirection::None;
                    self.last_triggered_at_ms = None;
                }
            }
        }

        let (Some(buffer_up), Some(buffer_dn)) = (self.buffer_up, self.buffer_dn) else {
            self.up_break_started_at_ms = None;
            self.dn_break_started_at_ms = None;
            return;
        };

        let cooldown_active = self
            .last_triggered_at_ms
            .map(|last| ts_ms.saturating_sub(last) < self.breakout_cfg.rearm_ms)
            .unwrap_or(false);

        let mut up_triggered = false;
        let mut dn_triggered = false;

        if price >= buffer_up {
            if self.up_break_started_at_ms.is_none() {
                self.up_break_started_at_ms = Some(ts_ms);
            } else if let Some(start) = self.up_break_started_at_ms {
                if ts_ms.saturating_sub(start) >= self.breakout_cfg.persistence_ms
                    && !cooldown_active
                {
                    up_triggered = true;
                }
            }
        } else {
            self.up_break_started_at_ms = None;
        }

        if price <= buffer_dn {
            if self.dn_break_started_at_ms.is_none() {
                self.dn_break_started_at_ms = Some(ts_ms);
            } else if let Some(start) = self.dn_break_started_at_ms {
                if ts_ms.saturating_sub(start) >= self.breakout_cfg.persistence_ms
                    && !cooldown_active
                {
                    dn_triggered = true;
                }
            }
        } else {
            self.dn_break_started_at_ms = None;
        }

        let trigger = if up_triggered && dn_triggered {
            let up_start = self.up_break_started_at_ms.unwrap_or(ts_ms);
            let dn_start = self.dn_break_started_at_ms.unwrap_or(ts_ms);
            if up_start < dn_start {
                BreakoutDirection::Up
            } else if dn_start < up_start {
                BreakoutDirection::Down
            } else {
                let up_score = self.momentum_score(BreakoutDirection::Up).unwrap_or(0);
                let dn_score = self.momentum_score(BreakoutDirection::Down).unwrap_or(0);
                if up_score >= dn_score {
                    BreakoutDirection::Up
                } else {
                    BreakoutDirection::Down
                }
            }
        } else if up_triggered {
            BreakoutDirection::Up
        } else if dn_triggered {
            BreakoutDirection::Down
        } else {
            BreakoutDirection::None
        };

        if trigger != BreakoutDirection::None {
            self.active_trigger = trigger;
            self.last_triggered_at_ms = Some(ts_ms);
            self.up_break_started_at_ms = None;
            self.dn_break_started_at_ms = None;
        }
    }

    fn evaluate_breakout(&self, req_dir: BreakoutDirection, now_ms_value: i64) -> BreakoutDecision {
        let mut out = BreakoutDecision {
            applied: true,
            persist_ms: self.breakout_cfg.persistence_ms,
            hk: self.level_hk,
            lk: self.level_lk,
            buffer_up: self.buffer_up,
            buffer_dn: self.buffer_dn,
            ..BreakoutDecision::default()
        };
        out.tick_age_ms = if self.last_tick_ts_ms > 0 {
            now_ms_value.saturating_sub(self.last_tick_ts_ms).max(0)
        } else {
            i64::MAX
        };
        let max_age_ms = (self.breakout_cfg.max_snapshot_age_seconds * 1000.0) as i64;
        if self.last_tick_ts_ms <= 0 || out.tick_age_ms > max_age_ms {
            out.reason = "stale".to_string();
            return out;
        }
        if self.completed_candles.len() < self.breakout_cfg.level_lookback_candles {
            out.reason = "insufficient_candles".to_string();
            return out;
        }
        if self.level_hk.is_none()
            || self.level_lk.is_none()
            || self.buffer_up.is_none()
            || self.buffer_dn.is_none()
        {
            out.reason = "no_levels".to_string();
            return out;
        }

        if let Some(last) = self.last_triggered_at_ms {
            if self.breakout_cfg.rearm_ms > 0 {
                let elapsed = now_ms_value.saturating_sub(last).max(0);
                if elapsed < self.breakout_cfg.rearm_ms {
                    out.cooldown_remaining_ms = self.breakout_cfg.rearm_ms - elapsed;
                }
            }
        }

        let mut active = self.active_trigger;
        if active != BreakoutDirection::None && self.breakout_cfg.rearm_ms > 0 {
            if let Some(last) = self.last_triggered_at_ms {
                if now_ms_value.saturating_sub(last) >= self.breakout_cfg.rearm_ms {
                    active = BreakoutDirection::None;
                }
            }
        }
        out.direction = active;
        out.triggered = active != BreakoutDirection::None;
        let started_at = if req_dir == BreakoutDirection::Up {
            self.up_break_started_at_ms
        } else if req_dir == BreakoutDirection::Down {
            self.dn_break_started_at_ms
        } else {
            None
        };
        out.elapsed_ms = started_at
            .map(|ts| now_ms_value.saturating_sub(ts).max(0))
            .unwrap_or(0);

        if active == BreakoutDirection::None {
            out.reason = "no_trigger".to_string();
            return out;
        }
        if active != req_dir {
            out.reason = "wrong_side".to_string();
            return out;
        }
        out.passed = true;
        out.reason = "ok".to_string();
        out
    }

    fn evaluate_momentum(
        &self,
        req_dir: BreakoutDirection,
        now_ms_value: i64,
        required_checks: usize,
    ) -> MomentumDecision {
        let mut out = MomentumDecision {
            applied: true,
            required_checks: required_checks.clamp(1, 3),
            tick_age_ms: if self.last_tick_ts_ms > 0 {
                now_ms_value.saturating_sub(self.last_tick_ts_ms).max(0)
            } else {
                i64::MAX
            },
            ..MomentumDecision::default()
        };
        let max_age_ms = (self.momentum_cfg.max_snapshot_age_seconds * 1000.0) as i64;
        if self.last_tick_ts_ms <= 0 || out.tick_age_ms > max_age_ms {
            out.reason = "stale".to_string();
            return out;
        }
        let min_candles = self
            .momentum_cfg
            .ema_slow
            .max(self.momentum_cfg.window_candles)
            + 1;
        if self.completed_candles.len() < min_candles {
            out.reason = "insufficient_candles".to_string();
            return out;
        }
        let closes: Vec<f64> = self.completed_candles.iter().map(|c| c.close).collect();
        let ema_fast = ema_series(&closes, self.momentum_cfg.ema_fast);
        let ema_slow = ema_series(&closes, self.momentum_cfg.ema_slow);
        if ema_fast.len() < 2 || ema_slow.is_empty() {
            out.reason = "ema_failed".to_string();
            return out;
        }
        let ema_fast_last = *ema_fast.last().unwrap_or(&0.0);
        let ema_fast_prev = ema_fast[ema_fast.len() - 2];
        let ema_slow_last = *ema_slow.last().unwrap_or(&0.0);
        out.ema_fast_last = Some(ema_fast_last);
        out.ema_fast_prev = Some(ema_fast_prev);
        out.ema_slow_last = Some(ema_slow_last);

        let trend_ok = if req_dir == BreakoutDirection::Up {
            ema_fast_last > ema_slow_last
        } else {
            ema_fast_last < ema_slow_last
        };
        let slope = ema_fast_last - ema_fast_prev;
        let slope_ok = if req_dir == BreakoutDirection::Up {
            slope > 0.0
        } else {
            slope < 0.0
        };
        let tail: Vec<&Candle1m> = self
            .completed_candles
            .iter()
            .rev()
            .take(self.momentum_cfg.window_candles)
            .collect();
        let body_count = tail
            .iter()
            .filter(|c| {
                if req_dir == BreakoutDirection::Up {
                    c.close > c.open
                } else {
                    c.close < c.open
                }
            })
            .count();
        let candles_ok = body_count >= self.momentum_cfg.window_min_bullish;
        out.bullish_or_bearish_count = Some(body_count);
        out.trend_ok = trend_ok;
        out.slope_ok = slope_ok;
        out.candles_ok = candles_ok;

        let checks = [trend_ok, slope_ok, candles_ok];
        out.checks_passed = checks.iter().filter(|v| **v).count();
        out.passed = out.checks_passed >= out.required_checks;
        out.reason = if out.passed { "ok" } else { "checks_failed" }.to_string();
        out
    }

    fn momentum_score(&self, req_dir: BreakoutDirection) -> Option<usize> {
        if self.completed_candles.len() < self.momentum_cfg.ema_slow + 1 {
            return None;
        }
        let m = self.evaluate_momentum(req_dir, now_ms(), self.momentum_cfg.required_checks);
        Some(m.checks_passed)
    }
}

fn ema_series(closes: &[f64], period: usize) -> Vec<f64> {
    if closes.is_empty() || period == 0 {
        return Vec::new();
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut out = Vec::with_capacity(closes.len());
    let mut prev = closes[0];
    out.push(prev);
    for &c in &closes[1..] {
        prev = alpha * c + (1.0 - alpha) * prev;
        out.push(prev);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_momentum_cfg() -> MomentumConfig {
        MomentumConfig {
            enabled: true,
            symbols: vec!["btc".to_string()].into_iter().collect(),
            required_checks: 2,
            ema_fast: 3,
            ema_slow: 8,
            window_candles: 4,
            window_min_bullish: 3,
            max_snapshot_age_seconds: 2.0,
            candle_history: 128,
            log_every_seconds: 0.0,
        }
    }

    fn base_breakout_cfg() -> BreakoutConfig {
        BreakoutConfig {
            enabled: true,
            symbols: vec!["btc".to_string()].into_iter().collect(),
            level_lookback_candles: 3,
            buffer_bps: 5.0,
            persistence_ms: 2800,
            rearm_ms: 15_000,
            max_snapshot_age_seconds: 2.0,
            mode: BreakoutMode::Required,
            assist_momentum_required_checks: 3,
            log_every_seconds: 0.0,
        }
    }

    fn push_linear_ticks(
        engine: &mut SniperFilterEngine,
        start_ts_ms: i64,
        minutes: i64,
        start_price: f64,
        step: f64,
    ) {
        let mut price = start_price;
        for m in 0..minutes {
            let base = start_ts_ms + m * 60_000;
            for s in [0i64, 15_000, 30_000, 45_000, 59_000] {
                let tick = BinanceTick {
                    symbol: "BTCUSDT".to_string(),
                    price,
                    ts_ms: base + s,
                    received_at_ms: base + s,
                };
                let _ = engine.on_tick(&tick);
                price += step;
            }
        }
    }

    #[test]
    fn candle_finalization_and_duplicate_tick_rejection() {
        let mut eng =
            SniperFilterEngine::new_with_configs("btc", base_momentum_cfg(), base_breakout_cfg());
        let t1 = BinanceTick {
            symbol: "BTCUSDT".to_string(),
            price: 100.0,
            ts_ms: 120_000,
            received_at_ms: 120_000,
        };
        let t2 = BinanceTick {
            symbol: "BTCUSDT".to_string(),
            price: 101.0,
            ts_ms: 179_000,
            received_at_ms: 179_000,
        };
        let t3 = BinanceTick {
            symbol: "BTCUSDT".to_string(),
            price: 102.0,
            ts_ms: 180_000,
            received_at_ms: 180_000,
        };
        assert!(eng.on_tick(&t1));
        assert!(eng.on_tick(&t2));
        assert!(!eng.on_tick(&t2));
        assert!(eng.on_tick(&t3));
        assert_eq!(eng.completed_candles.len(), 1);
        assert_eq!(eng.completed_candles[0].minute_start_ms, 120_000);
    }

    #[test]
    fn momentum_pass_up_and_down() {
        let mut eng = SniperFilterEngine::new_with_configs(
            "btc",
            base_momentum_cfg(),
            BreakoutConfig {
                enabled: false,
                ..base_breakout_cfg()
            },
        );
        push_linear_ticks(&mut eng, 0, 12, 100.0, 0.2);
        let now = 12 * 60_000 + 1_000;
        let up = eng.evaluate_entry("YES", now);
        assert!(up.allowed);

        let mut eng2 = SniperFilterEngine::new_with_configs(
            "btc",
            base_momentum_cfg(),
            BreakoutConfig {
                enabled: false,
                ..base_breakout_cfg()
            },
        );
        push_linear_ticks(&mut eng2, 0, 12, 200.0, -0.2);
        let down = eng2.evaluate_entry("NO", now);
        assert!(down.allowed);
    }

    #[test]
    fn breakout_required_wrong_side_blocks() {
        let mut eng = SniperFilterEngine::new_with_configs(
            "btc",
            MomentumConfig {
                enabled: false,
                ..base_momentum_cfg()
            },
            base_breakout_cfg(),
        );
        push_linear_ticks(&mut eng, 0, 8, 100.0, 0.1);
        let ts = 8 * 60_000;
        let up_price = eng.buffer_up.unwrap_or(0.0) + 5.0;
        let _ = eng.on_tick(&BinanceTick {
            symbol: "BTCUSDT".to_string(),
            price: up_price,
            ts_ms: ts + 1_000,
            received_at_ms: ts + 1_000,
        });
        let _ = eng.on_tick(&BinanceTick {
            symbol: "BTCUSDT".to_string(),
            price: up_price,
            ts_ms: ts + 5_000,
            received_at_ms: ts + 5_000,
        });
        let yes = eng.evaluate_entry("YES", ts + 5_100);
        assert!(yes.allowed);
        let no = eng.evaluate_entry("NO", ts + 5_100);
        assert!(!no.allowed);
    }

    #[test]
    fn breakout_latch_expires_after_rearm() {
        let mut cfg = base_breakout_cfg();
        cfg.rearm_ms = 5_000;
        cfg.persistence_ms = 1_000;
        let mut eng = SniperFilterEngine::new_with_configs(
            "btc",
            MomentumConfig {
                enabled: false,
                ..base_momentum_cfg()
            },
            cfg,
        );
        push_linear_ticks(&mut eng, 0, 8, 100.0, 0.1);
        let base = 8 * 60_000;
        let price = eng.buffer_up.unwrap_or(0.0) + 5.0;
        let _ = eng.on_tick(&BinanceTick {
            symbol: "BTCUSDT".to_string(),
            price,
            ts_ms: base + 100,
            received_at_ms: base + 100,
        });
        let _ = eng.on_tick(&BinanceTick {
            symbol: "BTCUSDT".to_string(),
            price,
            ts_ms: base + 1_500,
            received_at_ms: base + 1_500,
        });
        assert!(eng.evaluate_entry("YES", base + 1_600).allowed);
        assert!(!eng.evaluate_entry("YES", base + 7_000).allowed);
    }

    #[test]
    fn assist_mode_uses_strict_momentum_fallback() {
        let mut breakout = base_breakout_cfg();
        breakout.mode = BreakoutMode::Assist;
        breakout.assist_momentum_required_checks = 3;
        let mut momentum = base_momentum_cfg();
        momentum.enabled = false;
        let mut eng = SniperFilterEngine::new_with_configs("btc", momentum, breakout);
        push_linear_ticks(&mut eng, 0, 12, 100.0, 0.3);
        let decision = eng.evaluate_entry("YES", 12 * 60_000 + 1_000);
        assert!(decision.allowed);
        assert_eq!(decision.reason, "assist_strict_momentum_fallback");
    }

    #[test]
    fn stale_tick_fail_closed_when_enabled() {
        let mut eng = SniperFilterEngine::new_with_configs(
            "btc",
            base_momentum_cfg(),
            BreakoutConfig {
                enabled: false,
                ..base_breakout_cfg()
            },
        );
        push_linear_ticks(&mut eng, 0, 10, 100.0, 0.1);
        let decision = eng.evaluate_entry("YES", 10 * 60_000 + 10_000);
        assert!(!decision.allowed);
        assert!(decision.reason.contains("momentum_stale"));
    }

    #[test]
    fn non_btc_symbol_bypasses() {
        let eng =
            SniperFilterEngine::new_with_configs("eth", base_momentum_cfg(), base_breakout_cfg());
        let d = eng.evaluate_entry("YES", 12_345);
        assert!(d.allowed);
        assert_eq!(d.reason, "filters_disabled");
    }

    #[test]
    fn persisted_state_roundtrip() {
        let mut eng =
            SniperFilterEngine::new_with_configs("btc", base_momentum_cfg(), base_breakout_cfg());
        push_linear_ticks(&mut eng, 0, 6, 100.0, 0.1);
        let st = eng.export_state();
        assert!(st.momentum_yes.is_some());
        assert!(st.momentum_no.is_some());
        let mut eng2 =
            SniperFilterEngine::new_with_configs("btc", base_momentum_cfg(), base_breakout_cfg());
        assert!(eng2.import_state(st));
        assert_eq!(eng.completed_candles.len(), eng2.completed_candles.len());
        assert_eq!(eng.last_tick_ts_ms, eng2.last_tick_ts_ms);
    }
}
