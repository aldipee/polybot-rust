use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::bot) enum BotRuntimePhase {
    #[default]
    PreArm,
    OpenBoth,
    PairBuild,
    Taper,
    AwaitSettlement,
}
impl BotRuntimePhase {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::PreArm => "PreArm",
            Self::OpenBoth => "OpenBoth",
            Self::PairBuild => "PairBuild",
            Self::Taper => "Taper",
            Self::AwaitSettlement => "AwaitSettlement",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::bot) enum BotRuntimeSafetyGate {
    #[default]
    Healthy,
    StartupReconPending,
    ReconnectReconPending,
    ValidationFailed,
    DependencyPaused,
}

impl BotRuntimeSafetyGate {
    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::StartupReconPending => "startup_reconciliation_pending",
            Self::ReconnectReconPending => "reconnect_reconciliation_pending",
            Self::ValidationFailed => "validation_failed",
            Self::DependencyPaused => "dependency_paused",
        }
    }

    pub(in crate::bot) fn allows_new_risk(self) -> bool {
        matches!(self, Self::Healthy)
    }
}
/// Implements should stop for rollover for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_should_stop_for_rollover(
    seconds_left: f64,
    stop_buffer_seconds: i64,
) -> bool {
    let rollover_seconds_left = seconds_left - 10.0;
    rollover_seconds_left < stop_buffer_seconds.max(0) as f64
}
/// Implements phase from t into s for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_phase_from_t_into_s(
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimePhase {
    if t_into_s < 0.0 {
        BotRuntimePhase::PreArm
    } else if t_into_s < 30.0 {
        BotRuntimePhase::OpenBoth
    } else if t_into_s < cfg.late_reduce_start_seconds {
        BotRuntimePhase::PairBuild
    } else if t_into_s < cfg.late_stop_new_orders_start_seconds {
        BotRuntimePhase::Taper
    } else {
        BotRuntimePhase::AwaitSettlement
    }
}
/// Implements owner for snapshot for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_owner_for_snapshot(
    phase: BotRuntimePhase,
    q_yes: f64,
    q_no: f64,
    await_second_fill_hard_paused: bool,
) -> (BotRuntimeControlOwner, &'static str) {
    let has_yes = q_yes > 1e-9;
    let has_no = q_no > 1e-9;
    match phase {
        BotRuntimePhase::PreArm => (BotRuntimeControlOwner::PreArm, "prearm_window"),
        BotRuntimePhase::AwaitSettlement => {
            (BotRuntimeControlOwner::AwaitSettlement, "await_settlement")
        }
        BotRuntimePhase::OpenBoth => {
            if await_second_fill_hard_paused {
                (
                    BotRuntimeControlOwner::AwaitSecondFill,
                    "startup_hard_paused",
                )
            } else if has_yes ^ has_no {
                (BotRuntimeControlOwner::AwaitSecondFill, "startup_asymmetry")
            } else if has_yes && has_no {
                (BotRuntimeControlOwner::PairBuild, "both_sides_live")
            } else {
                (BotRuntimeControlOwner::OpenBoth, "seed_both_sides")
            }
        }
        BotRuntimePhase::PairBuild => {
            if await_second_fill_hard_paused {
                (
                    BotRuntimeControlOwner::AwaitSecondFill,
                    "startup_hard_paused",
                )
            } else if has_yes ^ has_no {
                (BotRuntimeControlOwner::AwaitSecondFill, "startup_asymmetry")
            } else if has_yes || has_no {
                (BotRuntimeControlOwner::PairBuild, "paired_replenishment")
            } else {
                (BotRuntimeControlOwner::OpenBoth, "seed_both_sides")
            }
        }
        BotRuntimePhase::Taper => {
            if await_second_fill_hard_paused {
                (
                    BotRuntimeControlOwner::AwaitSecondFill,
                    "startup_hard_paused",
                )
            } else if has_yes ^ has_no {
                (BotRuntimeControlOwner::AwaitSecondFill, "startup_asymmetry")
            } else {
                (BotRuntimeControlOwner::Taper, "late_taper")
            }
        }
    }
}
/// Implements should run open both handler for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_should_run_open_both_handler(
    owner: BotRuntimeControlOwner,
) -> bool {
    matches!(owner, BotRuntimeControlOwner::OpenBoth)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::bot) enum BotRuntimeControlOwner {
    #[default]
    PreArm,
    OpenBoth,
    AwaitSecondFill,
    PairBuild,
    Taper,
    AwaitSettlement,
}
impl BotRuntimeControlOwner {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::PreArm => "PreArm",
            Self::OpenBoth => "OpenBoth",
            Self::AwaitSecondFill => "AwaitSecondFill",
            Self::PairBuild => "PairBuild",
            Self::Taper => "Taper",
            Self::AwaitSettlement => "AwaitSettlement",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::bot) enum BotRuntimeImbalanceState {
    #[default]
    Normal,
    Throttle,
    Warning,
    HardDisable,
}
impl BotRuntimeImbalanceState {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Throttle => "Throttle",
            Self::Warning => "Warning",
            Self::HardDisable => "HardDisable",
        }
    }
}
#[derive(Debug, Clone, Default)]
pub(in crate::bot) struct BotRuntimePairBuildSideRepostState {
    pub(in crate::bot) last_cancel_ts: f64,
    pub(in crate::bot) last_cancel_price: f64,
    pub(in crate::bot) last_submit_ts: f64,
    pub(in crate::bot) last_submit_price: f64,
}
#[derive(Debug, Clone, Default)]
pub(in crate::bot) struct BotRuntimeState {
    pub(in crate::bot) phase: BotRuntimePhase,
    pub(in crate::bot) state_enter_ts: f64,
    pub(in crate::bot) owner: BotRuntimeControlOwner,
    pub(in crate::bot) owner_enter_ts: f64,
    pub(in crate::bot) owner_reason: &'static str,
    pub(in crate::bot) safety_gate: BotRuntimeSafetyGate,
    pub(in crate::bot) safety_gate_reason: String,
    pub(in crate::bot) last_clean_reconcile_ts: f64,
    pub(in crate::bot) last_reconnect_reconcile_ts: f64,
    pub(in crate::bot) last_validation_ts: f64,
    pub(in crate::bot) dependency_pause_started_ts: f64,
    pub(in crate::bot) market_ws_ever_opened: bool,
    pub(in crate::bot) user_ws_ever_opened: bool,
    pub(in crate::bot) armed_once: bool,
    pub(in crate::bot) prearm_ready_once: bool,
    pub(in crate::bot) prearm_ready_ts: f64,
    pub(in crate::bot) prearm_ready_before_open: bool,
    pub(in crate::bot) prearm_hold_reason: String,
    pub(in crate::bot) open_confirmed_ts: f64,
    pub(in crate::bot) open_both_first_tradable_post_open_ts: f64,
    pub(in crate::bot) open_both_seed_anchor_ts: f64,
    pub(in crate::bot) open_both_seed_deadline_missed_ts: f64,
    pub(in crate::bot) open_both_late_seed_unlock_used: bool,
    pub(in crate::bot) open_both_late_seed_exhausted: bool,
    pub(in crate::bot) open_both_first_submit_ts: f64,
    pub(in crate::bot) open_both_first_yes_submit_ts: f64,
    pub(in crate::bot) open_both_first_no_submit_ts: f64,
    pub(in crate::bot) open_both_first_submit_delta_ms: f64,
    pub(in crate::bot) open_both_seed_by_deadline_met: bool,
    pub(in crate::bot) open_both_submit_delta_met: bool,
    pub(in crate::bot) open_both_first_fill_ts: f64,
    pub(in crate::bot) open_both_attempt_count: u32,
    pub(in crate::bot) open_both_last_hold_reason: String,
    pub(in crate::bot) await_second_fill_started_ts: f64,
    pub(in crate::bot) await_second_fill_missing_side: Option<OutcomeSide>,
    pub(in crate::bot) await_second_fill_target_missed_ts: f64,
    pub(in crate::bot) await_second_fill_second_fill_ts: f64,
    pub(in crate::bot) await_second_fill_rescue_used: bool,
    pub(in crate::bot) await_second_fill_rescue_attempted_ts: f64,
    pub(in crate::bot) await_second_fill_hard_paused: bool,
    pub(in crate::bot) second_side_by_15s: bool,
    pub(in crate::bot) second_side_by_30s: bool,
    pub(in crate::bot) first_fill_to_second_fill_ms: f64,
    pub(in crate::bot) await_second_fill_last_hold_reason: String,
    pub(in crate::bot) imbalance_state: BotRuntimeImbalanceState,
    pub(in crate::bot) imbalance_state_enter_ts: f64,
    pub(in crate::bot) imbalance_last_hold_reason: String,
    pub(in crate::bot) pair_build_last_hold_reason: String,
    pub(in crate::bot) pair_build_last_optional_growth_submit_ts: f64,
    pub(in crate::bot) pair_build_yes_repost: BotRuntimePairBuildSideRepostState,
    pub(in crate::bot) pair_build_no_repost: BotRuntimePairBuildSideRepostState,
    pub(in crate::bot) pair_build_last_paired_growth_yes_bid: f64,
    pub(in crate::bot) pair_build_last_paired_growth_no_bid: f64,
    pub(in crate::bot) taper_last_hold_reason: String,
    pub(in crate::bot) await_settlement_started_ts: f64,
    pub(in crate::bot) await_settlement_orders_cleared_ts: f64,
    pub(in crate::bot) await_settlement_cancel_requested: bool,
    pub(in crate::bot) total_fill_events: u32,
    pub(in crate::bot) total_fill_shares: f64,
    pub(in crate::bot) maker_fill_events: u32,
    pub(in crate::bot) maker_fill_shares: f64,
    pub(in crate::bot) taker_fill_events: u32,
    pub(in crate::bot) taker_fill_shares: f64,
    pub(in crate::bot) daily_taker_day_key_utc: String,
    pub(in crate::bot) daily_maker_fill_shares: f64,
    pub(in crate::bot) daily_taker_fill_shares: f64,
    pub(in crate::bot) fill_events_by_segment: [u32; 5],
    pub(in crate::bot) fill_shares_by_segment: [f64; 5],
    pub(in crate::bot) late_fill_events_after_180: u32,
    pub(in crate::bot) late_fill_events_after_225: u32,
    pub(in crate::bot) late_new_orders_after_225: u32,
    pub(in crate::bot) late_new_orders_after_240: u32,
    pub(in crate::bot) skipped_optional_add_count: u32,
    pub(in crate::bot) repair_reserve_blocked_count: u32,
    pub(in crate::bot) floor_tail_blocked_count: u32,
    pub(in crate::bot) startup_completion_blocked_count: u32,
    pub(in crate::bot) paired_cost_band_observations: [u32; 5],
    pub(in crate::bot) paired_size_delta_by_state: [f64; 5],
    pub(in crate::bot) bad_regime_early_observations: u32,
    pub(in crate::bot) bad_regime_expensive_observations: u32,
    pub(in crate::bot) bad_regime_shutdown: bool,
    pub(in crate::bot) below_snapshot_optional_submit_count: u32,
    pub(in crate::bot) below_snapshot_optional_submit_shares: f64,
    pub(in crate::bot) below_snapshot_optional_fill_count: u32,
    pub(in crate::bot) below_snapshot_optional_fill_shares: f64,
    pub(in crate::bot) audit_decision_event_count: u32,
    pub(in crate::bot) audit_runtime_event_count: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::bot) struct BotRuntimePreArmStatus {
    pub(in crate::bot) market_selected: bool,
    pub(in crate::bot) asset_ids_ready: bool,
    pub(in crate::bot) market_ws_ready: bool,
    pub(in crate::bot) user_ws_ready: bool,
    pub(in crate::bot) quotes_ready: bool,
    pub(in crate::bot) quote_input_reason: String,
    pub(in crate::bot) paired_quotes_ready: bool,
    pub(in crate::bot) paired_quote_reason: String,
    pub(in crate::bot) ready: bool,
    pub(in crate::bot) hold_reason: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimePairBuildMode {
    PairedGrowth,
    LighterSideFirst,
}
impl BotRuntimePairBuildMode {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::PairedGrowth => "paired_growth",
            Self::LighterSideFirst => "lighter_side_first",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimeResidualKind {
    None,
    Favorite,
    Underdog,
}
impl BotRuntimeResidualKind {
    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Favorite => "favorite",
            Self::Underdog => "underdog",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimeOneSideExceptionKind {
    None,
    SecondSideCompletion,
    LaggingSideRepair,
}
impl BotRuntimeOneSideExceptionKind {
    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SecondSideCompletion => "second_side_completion",
            Self::LaggingSideRepair => "lagging_side_repair",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimePairBuildCppHint {
    Normal,
    Medium,
    Small,
}
impl BotRuntimePairBuildCppHint {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Medium => "medium",
            Self::Small => "small",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimeMarginalCostMode {
    BalancedAdd,
    RebalanceAdd,
}
impl BotRuntimeMarginalCostMode {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::BalancedAdd => "balanced_add",
            Self::RebalanceAdd => "rebalance_add",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimeClipRung {
    Seed,
    Normal,
    Large1,
    Large2,
    ExactGapRepair,
}
impl BotRuntimeClipRung {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Normal => "normal",
            Self::Large1 => "large_1",
            Self::Large2 => "large_2",
            Self::ExactGapRepair => "exact_gap_repair",
        }
    }

    pub(in crate::bot) fn is_large(self) -> bool {
        matches!(self, Self::Large1 | Self::Large2)
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::bot) struct BotRuntimePairBuildDecision {
    pub(in crate::bot) mode: BotRuntimePairBuildMode,
    pub(in crate::bot) side: Option<OutcomeSide>,
    pub(in crate::bot) clip: i64,
    pub(in crate::bot) selected_rung: BotRuntimeClipRung,
    pub(in crate::bot) requested_rung: BotRuntimeClipRung,
    pub(in crate::bot) requested_clip: f64,
    pub(in crate::bot) requested_large_clip: bool,
    pub(in crate::bot) clip_bucket: &'static str,
    pub(in crate::bot) cpp_hint: BotRuntimePairBuildCppHint,
    pub(in crate::bot) marginal_cost_mode: BotRuntimeMarginalCostMode,
    pub(in crate::bot) effective_marginal_pair_cost: f64,
    pub(in crate::bot) price_zone: BotRuntimePairedCostBand,
    pub(in crate::bot) residual_unit_cost: Option<f64>,
    pub(in crate::bot) lagging_side_quote: Option<f64>,
    pub(in crate::bot) favorite_side: Option<OutcomeSide>,
    pub(in crate::bot) underdog_side: Option<OutcomeSide>,
    pub(in crate::bot) residual_side: Option<OutcomeSide>,
    pub(in crate::bot) projected_residual_side: Option<OutcomeSide>,
    pub(in crate::bot) residual_kind: BotRuntimeResidualKind,
    pub(in crate::bot) increases_underdog_residual: bool,
    pub(in crate::bot) one_side_exception_kind: BotRuntimeOneSideExceptionKind,
    pub(in crate::bot) pair_sum: f64,
    pub(in crate::bot) current_unmatched_fraction: f64,
    pub(in crate::bot) projected_unmatched_fraction: f64,
    pub(in crate::bot) match_ratio: f64,
    pub(in crate::bot) imbalance_state: BotRuntimeImbalanceState,
    pub(in crate::bot) reduces_imbalance: bool,
    pub(in crate::bot) green_both_sides_filled: bool,
    pub(in crate::bot) green_price_ok: bool,
    pub(in crate::bot) green_imbalance_ok: bool,
    pub(in crate::bot) green_time_ok: bool,
    pub(in crate::bot) green_budget_ok: bool,
    pub(in crate::bot) green_conditions_met: bool,
    pub(in crate::bot) pair_coverage: f64,
    pub(in crate::bot) skew_ratio: f64,
    pub(in crate::bot) current_base: f64,
    pub(in crate::bot) qty_gap: f64,
    pub(in crate::bot) inventory_vwap_sum: f64,
    pub(in crate::bot) market_snapshot_vwap_sum: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimePairedCostBand {
    Preferred,
    Acceptable,
    Caution,
    StopAdd,
    Danger,
}
impl BotRuntimePairedCostBand {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::Preferred => "preferred",
            Self::Acceptable => "acceptable",
            Self::Caution => "caution",
            Self::StopAdd => "stop_add",
            Self::Danger => "danger",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::bot) struct BotRuntimePairedGrowthPolicy {
    pub(in crate::bot) clip: i64,
    pub(in crate::bot) projected_paired_cost: f64,
    pub(in crate::bot) band: BotRuntimePairedCostBand,
    pub(in crate::bot) clipped_for_band: bool,
    pub(in crate::bot) allowed_averaging_down: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::bot) struct BotRuntimeOptionalBuyPolicy {
    pub(in crate::bot) clip: i64,
    pub(in crate::bot) min_snapshot_edge: f64,
    pub(in crate::bot) weak_edge_reduced: bool,
    pub(in crate::bot) edge_source: &'static str,
    pub(in crate::bot) yes_snapshot_price: f64,
    pub(in crate::bot) no_snapshot_price: f64,
    pub(in crate::bot) yes_snapshot_source: SnapshotPricingSource,
    pub(in crate::bot) no_snapshot_source: SnapshotPricingSource,
    pub(in crate::bot) hold_reason: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) struct BotRuntimeRepairClipSizing {
    pub(in crate::bot) exact_gap_clip: i64,
    pub(in crate::bot) min_valid_clip: i64,
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::bot) struct BotRuntimeLighterRepairPolicy {
    pub(in crate::bot) clip: i64,
    pub(in crate::bot) exact_gap_clip: i64,
    pub(in crate::bot) min_valid_clip: i64,
    pub(in crate::bot) rounded_up_min_valid: bool,
    pub(in crate::bot) clipped_to_budget: bool,
    pub(in crate::bot) hold_reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::bot) struct BotRuntimeRepairReservePolicy {
    pub(in crate::bot) clip: i64,
    pub(in crate::bot) likely_repair_side: OutcomeSide,
    pub(in crate::bot) likely_repair_clip: i64,
    pub(in crate::bot) required_repair_cost: f64,
    pub(in crate::bot) reserve_buffer_usd: f64,
    pub(in crate::bot) total_reserved_budget: f64,
    pub(in crate::bot) remaining_budget_after_clip: f64,
    pub(in crate::bot) clipped_for_reserve: bool,
    pub(in crate::bot) hold_reason: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::bot) struct BotRuntimeTailCapStatus {
    pub(in crate::bot) paired_size: f64,
    pub(in crate::bot) tail_size: f64,
    pub(in crate::bot) cap_fraction: f64,
    pub(in crate::bot) cap_shares: f64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bot) enum BotRuntimeTaperMode {
    ReduceClips,
    BalanceOnly,
}
impl BotRuntimeTaperMode {
    /// Returns the stable string label for this enum or state value.
    /// This is a pure BOT runtime helper used for configuration, policy, or metrics
    /// calculations.

    pub(in crate::bot) fn as_str(self) -> &'static str {
        match self {
            Self::ReduceClips => "reduce_clips",
            Self::BalanceOnly => "balance_only",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::bot) struct BotRuntimeLateActionPolicy {
    pub(in crate::bot) current_tail_size: f64,
    pub(in crate::bot) projected_tail_size: f64,
    pub(in crate::bot) current_floor: f64,
    pub(in crate::bot) projected_floor: f64,
    pub(in crate::bot) improves_tail: bool,
    pub(in crate::bot) improves_floor: bool,
    pub(in crate::bot) hold_reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::bot) struct BotRuntimeLighterOppositeOrderPolicy {
    pub(in crate::bot) preserve: bool,
    pub(in crate::bot) remaining: f64,
    pub(in crate::bot) compatible_remaining: f64,
    pub(in crate::bot) live_price: f64,
    pub(in crate::bot) target_price: f64,
    pub(in crate::bot) reason: &'static str,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::bot) struct BotRuntimeMetricsSnapshot {
    pub(in crate::bot) market_participated: bool,
    pub(in crate::bot) fills_per_market: u32,
    pub(in crate::bot) total_fill_shares: f64,
    pub(in crate::bot) maker_fill_share: f64,
    pub(in crate::bot) taker_fill_events: u32,
    pub(in crate::bot) taker_fill_shares: f64,
    pub(in crate::bot) pair_taker_share: f64,
    pub(in crate::bot) daily_maker_fill_shares: f64,
    pub(in crate::bot) daily_taker_fill_shares: f64,
    pub(in crate::bot) daily_taker_share: f64,
    pub(in crate::bot) fill_events_by_segment: [u32; 5],
    pub(in crate::bot) fill_shares_by_segment: [f64; 5],
    pub(in crate::bot) paired_size: f64,
    pub(in crate::bot) unmatched_size: f64,
    pub(in crate::bot) unmatched_fraction: f64,
    pub(in crate::bot) match_ratio: f64,
    pub(in crate::bot) imbalance_state: BotRuntimeImbalanceState,
    pub(in crate::bot) safety_gate: BotRuntimeSafetyGate,
    pub(in crate::bot) pair_coverage: f64,
    pub(in crate::bot) share_skew_ratio: f64,
    pub(in crate::bot) inventory_vwap_sum: f64,
    pub(in crate::bot) late_fill_events_after_180: u32,
    pub(in crate::bot) late_fill_events_after_225: u32,
    pub(in crate::bot) late_new_orders_after_225: u32,
    pub(in crate::bot) late_new_orders_after_240: u32,
    pub(in crate::bot) prearm_ready_before_open: bool,
    pub(in crate::bot) open_both_seed_by_deadline_met: bool,
    pub(in crate::bot) open_both_late_seed_used: bool,
    pub(in crate::bot) open_both_first_submit_delta_ms: f64,
    pub(in crate::bot) open_both_submit_delta_met: bool,
    pub(in crate::bot) second_side_by_15s: bool,
    pub(in crate::bot) second_side_by_30s: bool,
    pub(in crate::bot) first_fill_to_second_fill_ms: f64,
    pub(in crate::bot) await_second_fill_rescue_used: bool,
    pub(in crate::bot) await_second_fill_hard_paused: bool,
    pub(in crate::bot) skipped_optional_add_count: u32,
    pub(in crate::bot) repair_reserve_blocked_count: u32,
    pub(in crate::bot) floor_tail_blocked_count: u32,
    pub(in crate::bot) startup_completion_blocked_count: u32,
    pub(in crate::bot) paired_cost_band_observations: [u32; 5],
    pub(in crate::bot) paired_size_delta_by_state: [f64; 5],
    pub(in crate::bot) tail_at_expiry: f64,
    pub(in crate::bot) worst_case_settlement_floor: f64,
    pub(in crate::bot) bad_regime_expensive_ratio: f64,
    pub(in crate::bot) bad_regime_shutdown: bool,
    pub(in crate::bot) below_snapshot_optional_submit_count: u32,
    pub(in crate::bot) below_snapshot_optional_submit_shares: f64,
    pub(in crate::bot) below_snapshot_optional_fill_count: u32,
    pub(in crate::bot) below_snapshot_optional_fill_shares: f64,
    pub(in crate::bot) below_snapshot_optional_fill_rate: f64,
    pub(in crate::bot) audit_decision_event_count: u32,
    pub(in crate::bot) audit_runtime_event_count: u32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::bot) struct BotRuntimeBudgetSnapshot {
    pub(in crate::bot) cumulative_min_fraction: f64,
    pub(in crate::bot) cumulative_max_fraction: f64,
    pub(in crate::bot) cumulative_min_cost: f64,
    pub(in crate::bot) cumulative_max_cost: f64,
    pub(in crate::bot) remaining_to_max_cost: f64,
    pub(in crate::bot) under_min_target: bool,
}
