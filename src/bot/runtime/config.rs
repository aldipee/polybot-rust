use super::*;
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BotRuntimeConfigSnapshot {
    pub(crate) phase_controller: &'static str,
    pub(crate) prearm_lead_seconds: f64,
    pub(crate) open_both_seed_deadline_seconds: f64,
    pub(crate) open_both_submit_delta_max_seconds: f64,
    pub(crate) open_both_allow_single_late_seed: bool,
    pub(crate) seed_budget_min_fraction: f64,
    pub(crate) seed_budget_max_fraction: f64,
    pub(crate) early_budget_min_fraction: f64,
    pub(crate) early_budget_max_fraction: f64,
    pub(crate) main_budget_min_fraction: f64,
    pub(crate) main_budget_max_fraction: f64,
    pub(crate) late_budget_min_fraction: f64,
    pub(crate) late_budget_max_fraction: f64,
    pub(crate) taper_budget_min_fraction: f64,
    pub(crate) taper_budget_max_fraction: f64,
    pub(crate) target_both_sides_by_30s: f64,
    pub(crate) target_both_sides_by_60s: f64,
    pub(crate) late_reduce_start_seconds: f64,
    pub(crate) late_balance_only_start_seconds: f64,
    pub(crate) late_stop_new_orders_start_seconds: f64,
    pub(crate) legacy_late_window_budget_mode: bool,
    pub(crate) imbalance_target_fraction: f64,
    pub(crate) imbalance_warning_fraction: f64,
    pub(crate) imbalance_disable_fraction: f64,
    pub(crate) imbalance_recovery_fraction: f64,
    pub(crate) clip_ladder: [f64; 4],
    pub(crate) repair_reserve_buffer_usd: f64,
    pub(crate) buy_only_normal_flow: bool,
    pub(crate) tail_cap_mid_start_seconds: f64,
    pub(crate) tail_cap_late_start_seconds: f64,
    pub(crate) tail_cap_early_fraction: f64,
    pub(crate) tail_cap_mid_fraction: f64,
    pub(crate) tail_cap_late_fraction: f64,
    pub(crate) bad_regime_window_seconds: f64,
    pub(crate) bad_regime_expensive_fraction: f64,
    pub(crate) mean_reversion_tilt_fraction: f64,
    pub(crate) post_repair_cooldown_cycles: u32,
    pub(crate) repair_refresh_timeout_seconds: f64,
    pub(crate) repair_price_zone_danger: f64,
    pub(crate) repair_price_zone_stop_add: f64,
    pub(crate) green_price_threshold: f64,
    pub(crate) weak_edge_threshold: f64,
    pub(crate) one_directional_threshold: f64,
    pub(crate) one_directional_min_fills: u32,
}
/// Implements config defaults for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(crate) fn bot_runtime_config_defaults() -> BotRuntimeConfigSnapshot {
    BotRuntimeConfigSnapshot {
        phase_controller: "time_plus_inventory",
        prearm_lead_seconds: 20.0,
        open_both_seed_deadline_seconds: 5.0,
        open_both_submit_delta_max_seconds: 1.0,
        open_both_allow_single_late_seed: true,
        seed_budget_min_fraction: 0.10,
        seed_budget_max_fraction: 0.15,
        early_budget_min_fraction: 0.15,
        early_budget_max_fraction: 0.20,
        main_budget_min_fraction: 0.45,
        main_budget_max_fraction: 0.55,
        late_budget_min_fraction: 0.15,
        late_budget_max_fraction: 0.20,
        taper_budget_min_fraction: 0.05,
        taper_budget_max_fraction: 0.10,
        target_both_sides_by_30s: 0.80,
        target_both_sides_by_60s: 0.95,
        late_reduce_start_seconds: 180.0,
        late_balance_only_start_seconds: 225.0,
        late_stop_new_orders_start_seconds: 240.0,
        legacy_late_window_budget_mode: false,
        imbalance_target_fraction: 0.07,
        imbalance_warning_fraction: 0.12,
        imbalance_disable_fraction: 0.20,
        imbalance_recovery_fraction: 0.12,
        clip_ladder: [12.0, 20.0, 40.0, 80.0],
        repair_reserve_buffer_usd: 1.0,
        buy_only_normal_flow: true,
        tail_cap_mid_start_seconds: 210.0,
        tail_cap_late_start_seconds: 240.0,
        tail_cap_early_fraction: 0.10,
        tail_cap_mid_fraction: 0.05,
        tail_cap_late_fraction: 0.02,
        bad_regime_window_seconds: 120.0,
        bad_regime_expensive_fraction: 0.60,
        mean_reversion_tilt_fraction: 0.55,
        post_repair_cooldown_cycles: 2,
        repair_refresh_timeout_seconds: 6.0,
        repair_price_zone_danger: 1.20,
        repair_price_zone_stop_add: 1.10,
        green_price_threshold: 0.99,
        weak_edge_threshold: 0.03,
        one_directional_threshold: 0.70,
        one_directional_min_fills: 4,
    }
}
/// Implements env float for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(crate) fn bot_runtime_env_float<F>(get: &mut F, key: &str, default: f64) -> f64
where
    F: FnMut(&str) -> Option<String>,
{
    get(key)
        .and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<f64>().ok()
            }
        })
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

pub(crate) fn bot_runtime_env_float_optional<F>(get: &mut F, key: &str) -> Option<f64>
where
    F: FnMut(&str) -> Option<String>,
{
    get(key)
        .and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<f64>().ok()
            }
        })
        .filter(|value| value.is_finite())
}
/// Implements env bool for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(crate) fn bot_runtime_env_bool<F>(get: &mut F, key: &str, default: bool) -> bool
where
    F: FnMut(&str) -> Option<String>,
{
    get(key)
        .and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                match trimmed.to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "y" | "on" => Some(true),
                    "0" | "false" | "no" | "n" | "off" => Some(false),
                    _ => None,
                }
            }
        })
        .unwrap_or(default)
}
/// Implements env clip ladder large for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(crate) fn bot_runtime_env_clip_ladder<F>(get: &mut F, key: &str, default: [f64; 4]) -> [f64; 4]
where
    F: FnMut(&str) -> Option<String>,
{
    let Some(raw) = get(key) else {
        return default;
    };
    let mut values: Vec<f64> = raw
        .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
        .filter(|token| !token.trim().is_empty())
        .filter_map(|token| token.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    if values.len() != 4 {
        return default;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    [values[0], values[1], values[2], values[3]]
}
/// Implements config from reader for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(crate) fn bot_runtime_config_from_reader<F>(mut get: F) -> BotRuntimeConfigSnapshot
where
    F: FnMut(&str) -> Option<String>,
{
    let mut cfg = bot_runtime_config_defaults();
    let late_reduce_override =
        bot_runtime_env_float_optional(&mut get, "BOT_LATE_REDUCE_START_SECONDS");
    let late_balance_only_override =
        bot_runtime_env_float_optional(&mut get, "BOT_LATE_BALANCE_ONLY_START_SECONDS");
    let late_stop_new_orders_override =
        bot_runtime_env_float_optional(&mut get, "BOT_LATE_STOP_NEW_ORDERS_START_SECONDS");
    let legacy_taper_start_override =
        bot_runtime_env_float_optional(&mut get, "BOT_TAPER_START_SECONDS");
    let legacy_final_quiet_override =
        bot_runtime_env_float_optional(&mut get, "BOT_FINAL_QUIET_SECONDS");
    cfg.prearm_lead_seconds =
        bot_runtime_env_float(&mut get, "BOT_PREARM_LEAD_SECONDS", cfg.prearm_lead_seconds);
    cfg.open_both_seed_deadline_seconds = bot_runtime_env_float(
        &mut get,
        "BOT_OPEN_BOTH_SEED_DEADLINE_SECONDS",
        cfg.open_both_seed_deadline_seconds,
    );
    cfg.open_both_submit_delta_max_seconds = bot_runtime_env_float(
        &mut get,
        "BOT_OPEN_BOTH_SUBMIT_DELTA_MAX_SECONDS",
        cfg.open_both_submit_delta_max_seconds,
    );
    cfg.open_both_allow_single_late_seed = bot_runtime_env_bool(
        &mut get,
        "BOT_OPEN_BOTH_ALLOW_SINGLE_LATE_SEED",
        cfg.open_both_allow_single_late_seed,
    );
    cfg.clip_ladder = bot_runtime_env_clip_ladder(&mut get, "BOT_CLIP_LADDER", cfg.clip_ladder);
    cfg.repair_reserve_buffer_usd = bot_runtime_env_float(
        &mut get,
        "BOT_REPAIR_RESERVE_BUFFER_USD",
        cfg.repair_reserve_buffer_usd,
    );
    cfg.seed_budget_min_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_SEED_MIN_FRACTION",
        cfg.seed_budget_min_fraction,
    );
    cfg.seed_budget_max_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_SEED_MAX_FRACTION",
        cfg.seed_budget_max_fraction,
    );
    cfg.early_budget_min_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_EARLY_MIN_FRACTION",
        cfg.early_budget_min_fraction,
    );
    cfg.early_budget_max_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_EARLY_MAX_FRACTION",
        cfg.early_budget_max_fraction,
    );
    cfg.main_budget_min_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_MAIN_MIN_FRACTION",
        cfg.main_budget_min_fraction,
    );
    cfg.main_budget_max_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_MAIN_MAX_FRACTION",
        cfg.main_budget_max_fraction,
    );
    cfg.late_budget_min_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_LATE_MIN_FRACTION",
        cfg.late_budget_min_fraction,
    );
    cfg.late_budget_max_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_LATE_MAX_FRACTION",
        cfg.late_budget_max_fraction,
    );
    cfg.taper_budget_min_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_TAPER_MIN_FRACTION",
        cfg.taper_budget_min_fraction,
    );
    cfg.taper_budget_max_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BUDGET_TAPER_MAX_FRACTION",
        cfg.taper_budget_max_fraction,
    );
    cfg.target_both_sides_by_30s = bot_runtime_env_float(
        &mut get,
        "BOT_TARGET_BOTH_SIDES_BY_30S",
        cfg.target_both_sides_by_30s,
    );
    cfg.target_both_sides_by_60s = bot_runtime_env_float(
        &mut get,
        "BOT_TARGET_BOTH_SIDES_BY_60S",
        cfg.target_both_sides_by_60s,
    );
    let has_legacy_late_window_overrides =
        legacy_taper_start_override.is_some() || legacy_final_quiet_override.is_some();
    if has_legacy_late_window_overrides {
        let legacy_taper_start = legacy_taper_start_override.unwrap_or(240.0);
        let legacy_final_quiet = legacy_final_quiet_override.unwrap_or(30.0);
        cfg.late_reduce_start_seconds = legacy_taper_start;
        cfg.late_balance_only_start_seconds = (300.0 - legacy_final_quiet).max(legacy_taper_start);
        cfg.late_stop_new_orders_start_seconds = 300.0;
    }
    if let Some(value) = late_reduce_override {
        cfg.late_reduce_start_seconds = value;
    }
    if let Some(value) = late_balance_only_override {
        cfg.late_balance_only_start_seconds = value;
    }
    if let Some(value) = late_stop_new_orders_override {
        cfg.late_stop_new_orders_start_seconds = value;
    }
    cfg.legacy_late_window_budget_mode = has_legacy_late_window_overrides
        && late_reduce_override.is_none()
        && late_balance_only_override.is_none();
    cfg.imbalance_target_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_IMBALANCE_TARGET_FRACTION",
        cfg.imbalance_target_fraction,
    );
    cfg.imbalance_warning_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_IMBALANCE_WARNING_FRACTION",
        cfg.imbalance_warning_fraction,
    );
    cfg.imbalance_disable_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_IMBALANCE_DISABLE_FRACTION",
        cfg.imbalance_disable_fraction,
    );
    cfg.imbalance_recovery_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_IMBALANCE_RECOVERY_FRACTION",
        cfg.imbalance_recovery_fraction,
    );
    cfg.buy_only_normal_flow = bot_runtime_env_bool(
        &mut get,
        "BOT_BUY_ONLY_NORMAL_FLOW",
        cfg.buy_only_normal_flow,
    );
    cfg.tail_cap_mid_start_seconds = bot_runtime_env_float(
        &mut get,
        "BOT_TAIL_CAP_MID_START_SECONDS",
        cfg.tail_cap_mid_start_seconds,
    );
    cfg.tail_cap_late_start_seconds = bot_runtime_env_float(
        &mut get,
        "BOT_TAIL_CAP_LATE_START_SECONDS",
        cfg.tail_cap_late_start_seconds,
    );
    cfg.tail_cap_early_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_TAIL_CAP_EARLY_FRACTION",
        cfg.tail_cap_early_fraction,
    );
    cfg.tail_cap_mid_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_TAIL_CAP_MID_FRACTION",
        cfg.tail_cap_mid_fraction,
    );
    cfg.tail_cap_late_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_TAIL_CAP_LATE_FRACTION",
        cfg.tail_cap_late_fraction,
    );
    cfg.bad_regime_window_seconds = bot_runtime_env_float(
        &mut get,
        "BOT_BAD_REGIME_WINDOW_SECONDS",
        cfg.bad_regime_window_seconds,
    );
    cfg.bad_regime_expensive_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_BAD_REGIME_EXPENSIVE_FRACTION",
        cfg.bad_regime_expensive_fraction,
    );
    cfg.mean_reversion_tilt_fraction = bot_runtime_env_float(
        &mut get,
        "BOT_MEAN_REVERSION_TILT_FRACTION",
        cfg.mean_reversion_tilt_fraction,
    )
    .clamp(0.50, 0.70);
    cfg.post_repair_cooldown_cycles = bot_runtime_env_float(
        &mut get,
        "BOT_POST_REPAIR_COOLDOWN_CYCLES",
        cfg.post_repair_cooldown_cycles as f64,
    ) as u32;
    cfg.repair_refresh_timeout_seconds = bot_runtime_env_float(
        &mut get,
        "BOT_REPAIR_REFRESH_TIMEOUT_SECONDS",
        cfg.repair_refresh_timeout_seconds,
    );
    cfg.repair_price_zone_danger = bot_runtime_env_float(
        &mut get,
        "BOT_REPAIR_PRICE_ZONE_DANGER",
        cfg.repair_price_zone_danger,
    );
    cfg.repair_price_zone_stop_add = bot_runtime_env_float(
        &mut get,
        "BOT_REPAIR_PRICE_ZONE_STOP_ADD",
        cfg.repair_price_zone_stop_add,
    );
    cfg.green_price_threshold = bot_runtime_env_float(
        &mut get,
        "BOT_GREEN_PRICE_THRESHOLD",
        cfg.green_price_threshold,
    );
    cfg.weak_edge_threshold = bot_runtime_env_float(
        &mut get,
        "BOT_WEAK_EDGE_THRESHOLD",
        cfg.weak_edge_threshold,
    );
    cfg.one_directional_threshold = bot_runtime_env_float(
        &mut get,
        "BOT_ONE_DIRECTIONAL_THRESHOLD",
        cfg.one_directional_threshold,
    );
    cfg.one_directional_min_fills = bot_runtime_env_float(
        &mut get,
        "BOT_ONE_DIRECTIONAL_MIN_FILLS",
        cfg.one_directional_min_fills as f64,
    ) as u32;
    cfg
}
/// Implements config from env for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(crate) fn bot_runtime_config_from_env() -> BotRuntimeConfigSnapshot {
    bot_runtime_config_from_reader(|key| std::env::var(key).ok())
}
/// Implements validate config for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(crate) fn bot_runtime_validate_config(
    cfg: &BotRuntimeConfigSnapshot,
) -> Result<(), &'static str> {
    if !cfg.buy_only_normal_flow {
        return Err("buy_only_normal_flow_false_unsupported");
    }
    if !cfg.prearm_lead_seconds.is_finite() || cfg.prearm_lead_seconds < 0.0 {
        return Err("invalid_prearm_lead_seconds");
    }
    if !cfg.open_both_seed_deadline_seconds.is_finite()
        || cfg.open_both_seed_deadline_seconds <= 0.0
    {
        return Err("invalid_open_both_seed_deadline_seconds");
    }
    if !cfg.open_both_submit_delta_max_seconds.is_finite()
        || cfg.open_both_submit_delta_max_seconds <= 0.0
    {
        return Err("invalid_open_both_submit_delta_max_seconds");
    }
    if cfg.open_both_submit_delta_max_seconds > cfg.open_both_seed_deadline_seconds + 1e-9 {
        return Err("open_both_submit_delta_exceeds_deadline");
    }
    let invalid_late_reduce_start = if cfg.legacy_late_window_budget_mode {
        !cfg.late_reduce_start_seconds.is_finite() || cfg.late_reduce_start_seconds <= 0.0
    } else {
        !cfg.late_reduce_start_seconds.is_finite() || cfg.late_reduce_start_seconds <= 30.0
    };
    if invalid_late_reduce_start {
        return Err("invalid_late_reduce_start_seconds");
    }
    let invalid_late_balance_only = if cfg.legacy_late_window_budget_mode {
        !cfg.late_balance_only_start_seconds.is_finite()
            || cfg.late_balance_only_start_seconds < cfg.late_reduce_start_seconds
    } else {
        !cfg.late_balance_only_start_seconds.is_finite()
            || cfg.late_balance_only_start_seconds <= cfg.late_reduce_start_seconds
    };
    if invalid_late_balance_only {
        return Err("invalid_late_balance_only_start_seconds");
    }
    let invalid_late_stop_new_orders = if cfg.legacy_late_window_budget_mode {
        !cfg.late_stop_new_orders_start_seconds.is_finite()
            || cfg.late_stop_new_orders_start_seconds < cfg.late_balance_only_start_seconds
            || cfg.late_stop_new_orders_start_seconds > 300.0
    } else {
        !cfg.late_stop_new_orders_start_seconds.is_finite()
            || cfg.late_stop_new_orders_start_seconds <= cfg.late_balance_only_start_seconds
            || cfg.late_stop_new_orders_start_seconds > 300.0
    };
    if invalid_late_stop_new_orders {
        return Err("invalid_late_stop_new_orders_start_seconds");
    }
    if !cfg.imbalance_target_fraction.is_finite() || cfg.imbalance_target_fraction <= 0.0 {
        return Err("invalid_imbalance_target_fraction");
    }
    if !cfg.imbalance_warning_fraction.is_finite()
        || cfg.imbalance_warning_fraction <= cfg.imbalance_target_fraction
    {
        return Err("invalid_imbalance_warning_fraction");
    }
    if !cfg.imbalance_disable_fraction.is_finite()
        || cfg.imbalance_disable_fraction <= cfg.imbalance_warning_fraction
        || cfg.imbalance_disable_fraction > 1.0
    {
        return Err("invalid_imbalance_disable_fraction");
    }
    if !cfg.imbalance_recovery_fraction.is_finite()
        || cfg.imbalance_recovery_fraction <= 0.0
        || cfg.imbalance_recovery_fraction >= cfg.imbalance_disable_fraction
    {
        return Err("invalid_imbalance_recovery_fraction");
    }
    if cfg
        .clip_ladder
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err("invalid_clip_ladder");
    }
    if !(cfg.clip_ladder[0] + 1e-9 < cfg.clip_ladder[1]
        && cfg.clip_ladder[1] + 1e-9 < cfg.clip_ladder[2]
        && cfg.clip_ladder[2] + 1e-9 < cfg.clip_ladder[3])
    {
        return Err("invalid_clip_ladder");
    }
    if cfg.clip_ladder[3] > 80.0 + 1e-9 {
        return Err("clip_ladder_exceeds_hard_cap");
    }
    if !cfg.tail_cap_mid_start_seconds.is_finite() || cfg.tail_cap_mid_start_seconds < 0.0 {
        return Err("invalid_tail_cap_mid_start_seconds");
    }
    if !cfg.tail_cap_late_start_seconds.is_finite()
        || cfg.tail_cap_late_start_seconds < cfg.tail_cap_mid_start_seconds
    {
        return Err("invalid_tail_cap_late_start_seconds");
    }
    if !cfg.tail_cap_early_fraction.is_finite() || cfg.tail_cap_early_fraction < 0.0 {
        return Err("invalid_tail_cap_early_fraction");
    }
    if !cfg.tail_cap_mid_fraction.is_finite() || cfg.tail_cap_mid_fraction < 0.0 {
        return Err("invalid_tail_cap_mid_fraction");
    }
    if !cfg.tail_cap_late_fraction.is_finite() || cfg.tail_cap_late_fraction < 0.0 {
        return Err("invalid_tail_cap_late_fraction");
    }
    if !cfg.bad_regime_window_seconds.is_finite() || cfg.bad_regime_window_seconds <= 0.0 {
        return Err("invalid_bad_regime_window_seconds");
    }
    if !cfg.bad_regime_expensive_fraction.is_finite()
        || !(0.0..=1.0).contains(&cfg.bad_regime_expensive_fraction)
    {
        return Err("invalid_bad_regime_expensive_fraction");
    }
    Ok(())
}
/// Implements prearm window active for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_prearm_window_active(
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> bool {
    t_into_s >= -cfg.prearm_lead_seconds.max(0.0) && t_into_s < 0.0
}
