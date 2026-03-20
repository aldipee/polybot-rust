use super::*;
/// Implements fill segment index for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_fill_segment_index(t_into_s: f64) -> usize {
    if t_into_s < 30.0 {
        0
    } else if t_into_s < 60.0 {
        1
    } else if t_into_s < 180.0 {
        2
    } else if t_into_s < 240.0 {
        3
    } else {
        4
    }
}
/// Implements fill segment label for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_fill_segment_label(index: usize) -> &'static str {
    match index {
        0 => "0-30s",
        1 => "30-60s",
        2 => "60-180s",
        3 => "180-240s",
        _ => "240-300s",
    }
}
/// Implements market participated for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_market_participated(
    total_fill_events: u32,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
) -> bool {
    total_fill_events > 0 || q_yes > 1e-9 || q_no > 1e-9 || total_cost > 1e-9
}
/// Implements fill distribution summary u32 for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_fill_distribution_summary_u32(values: &[u32; 5]) -> String {
    (0..values.len())
        .map(|idx| format!("{}:{}", bot_runtime_fill_segment_label(idx), values[idx]))
        .collect::<Vec<_>>()
        .join(",")
}
/// Implements fill distribution summary f64 for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_fill_distribution_summary_f64(values: &[f64; 5]) -> String {
    (0..values.len())
        .map(|idx| format!("{}:{:.2}", bot_runtime_fill_segment_label(idx), values[idx]))
        .collect::<Vec<_>>()
        .join(",")
}

pub(in crate::bot) fn bot_runtime_late_metric_label(prefix: &str, threshold_s: f64) -> String {
    let clean_threshold = if threshold_s.is_finite() {
        threshold_s.max(0.0)
    } else {
        0.0
    };
    let suffix = if clean_threshold.fract().abs() <= 1e-9 {
        format!("{:.0}", clean_threshold)
    } else {
        format!("{:.1}", clean_threshold).replace('.', "_")
    };
    format!("{prefix}_after_{suffix}")
}

pub(in crate::bot) fn bot_runtime_taker_share(maker_qty: f64, taker_qty: f64) -> f64 {
    let maker = maker_qty.max(0.0);
    let taker = taker_qty.max(0.0);
    let total = maker + taker;
    if total > 1e-9 {
        (taker / total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub(in crate::bot) fn bot_runtime_projected_taker_share(
    maker_qty: f64,
    taker_qty: f64,
    pending_taker_qty: f64,
    requested_taker_qty: f64,
) -> f64 {
    bot_runtime_taker_share(
        maker_qty,
        taker_qty.max(0.0) + pending_taker_qty.max(0.0) + requested_taker_qty.max(0.0),
    )
}

pub(in crate::bot) fn bot_runtime_taker_share_target() -> f64 {
    0.05
}

pub(in crate::bot) fn bot_runtime_taker_share_cap() -> f64 {
    0.10
}

pub(in crate::bot) fn bot_runtime_favorite_underdog_sides(
    y_bid: f64,
    n_bid: f64,
    tick_size: f64,
) -> (Option<OutcomeSide>, Option<OutcomeSide>) {
    let tick = tick_size.max(0.0001);
    if !y_bid.is_finite() || !n_bid.is_finite() || y_bid <= 0.0 || n_bid <= 0.0 {
        return (None, None);
    }
    if (y_bid - n_bid).abs() <= tick + 1e-9 {
        (None, None)
    } else if y_bid > n_bid {
        (Some(OutcomeSide::Yes), Some(OutcomeSide::No))
    } else {
        (Some(OutcomeSide::No), Some(OutcomeSide::Yes))
    }
}

pub(in crate::bot) fn bot_runtime_residual_side(q_yes: f64, q_no: f64) -> Option<OutcomeSide> {
    if q_yes > q_no + 1e-9 {
        Some(OutcomeSide::Yes)
    } else if q_no > q_yes + 1e-9 {
        Some(OutcomeSide::No)
    } else {
        None
    }
}

pub(in crate::bot) fn bot_runtime_residual_magnitude(q_yes: f64, q_no: f64) -> f64 {
    (q_yes.max(0.0) - q_no.max(0.0)).abs()
}

pub(in crate::bot) fn bot_runtime_residual_kind(
    favorite_side: Option<OutcomeSide>,
    underdog_side: Option<OutcomeSide>,
    residual_side: Option<OutcomeSide>,
) -> BotRuntimeResidualKind {
    if residual_side.is_none() {
        BotRuntimeResidualKind::None
    } else if residual_side == favorite_side {
        BotRuntimeResidualKind::Favorite
    } else if residual_side == underdog_side {
        BotRuntimeResidualKind::Underdog
    } else {
        BotRuntimeResidualKind::None
    }
}

pub(in crate::bot) fn bot_runtime_projected_residual_side_and_magnitude(
    mode: BotRuntimePairBuildMode,
    side: Option<OutcomeSide>,
    clip: f64,
    q_yes: f64,
    q_no: f64,
) -> (Option<OutcomeSide>, f64) {
    let mut projected_yes = q_yes.max(0.0);
    let mut projected_no = q_no.max(0.0);
    match mode {
        BotRuntimePairBuildMode::PairedGrowth => {
            projected_yes += clip.max(0.0);
            projected_no += clip.max(0.0);
        }
        BotRuntimePairBuildMode::LighterSideFirst => match side.unwrap_or(OutcomeSide::Yes) {
            OutcomeSide::Yes => projected_yes += clip.max(0.0),
            OutcomeSide::No => projected_no += clip.max(0.0),
        },
    }
    (
        bot_runtime_residual_side(projected_yes, projected_no),
        bot_runtime_residual_magnitude(projected_yes, projected_no),
    )
}

pub(in crate::bot) fn bot_runtime_would_increase_underdog_residual(
    mode: BotRuntimePairBuildMode,
    side: Option<OutcomeSide>,
    clip: f64,
    q_yes: f64,
    q_no: f64,
    y_bid: f64,
    n_bid: f64,
    tick_size: f64,
) -> bool {
    let (_, underdog_side) = bot_runtime_favorite_underdog_sides(y_bid, n_bid, tick_size);
    bot_runtime_would_increase_underdog_residual_for_side(
        mode,
        side,
        clip,
        q_yes,
        q_no,
        underdog_side,
    )
}

pub(in crate::bot) fn bot_runtime_would_increase_underdog_residual_for_side(
    mode: BotRuntimePairBuildMode,
    side: Option<OutcomeSide>,
    clip: f64,
    q_yes: f64,
    q_no: f64,
    underdog_side: Option<OutcomeSide>,
) -> bool {
    let Some(underdog_side) = underdog_side else {
        return false;
    };
    let current_residual_side = bot_runtime_residual_side(q_yes, q_no);
    let current_residual_magnitude = bot_runtime_residual_magnitude(q_yes, q_no);
    let (projected_residual_side, projected_residual_magnitude) =
        bot_runtime_projected_residual_side_and_magnitude(mode, side, clip, q_yes, q_no);
    if projected_residual_side != Some(underdog_side) || projected_residual_magnitude <= 1e-9 {
        return false;
    }
    current_residual_side == Some(underdog_side)
        && projected_residual_magnitude > current_residual_magnitude + 1e-9
        || current_residual_side != Some(underdog_side)
}
/// Implements paired cost band index for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_paired_cost_band_index(band: BotRuntimePairedCostBand) -> usize {
    match band {
        BotRuntimePairedCostBand::Preferred => 0,
        BotRuntimePairedCostBand::Acceptable => 1,
        BotRuntimePairedCostBand::Caution => 2,
        BotRuntimePairedCostBand::StopAdd => 3,
        BotRuntimePairedCostBand::Danger => 4,
    }
}
/// Implements paired cost band label for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_paired_cost_band_label(index: usize) -> &'static str {
    match index {
        0 => BotRuntimePairedCostBand::Preferred.as_str(),
        1 => BotRuntimePairedCostBand::Acceptable.as_str(),
        2 => BotRuntimePairedCostBand::Caution.as_str(),
        3 => BotRuntimePairedCostBand::StopAdd.as_str(),
        _ => BotRuntimePairedCostBand::Danger.as_str(),
    }
}
/// Implements paired cost band summary u32 for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_paired_cost_band_summary_u32(values: &[u32; 5]) -> String {
    (0..values.len())
        .map(|idx| {
            format!(
                "{}:{}",
                bot_runtime_paired_cost_band_label(idx),
                values[idx]
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
/// Implements paired cost band summary fraction for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_paired_cost_band_summary_fraction(values: &[u32; 5]) -> String {
    let total = values.iter().copied().sum::<u32>().max(1) as f64;
    (0..values.len())
        .map(|idx| {
            format!(
                "{}:{:.3}",
                bot_runtime_paired_cost_band_label(idx),
                values[idx] as f64 / total
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
/// Implements paired cost band summary f64 for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_paired_cost_band_summary_f64(values: &[f64; 5]) -> String {
    (0..values.len())
        .map(|idx| {
            format!(
                "{}:{:.2}",
                bot_runtime_paired_cost_band_label(idx),
                values[idx]
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
/// Implements note fill event for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_note_fill_event(
    state: &mut BotRuntimeState,
    t_into_s: f64,
    filled: f64,
    is_maker: bool,
    cfg: &BotRuntimeConfigSnapshot,
) {
    if filled <= 1e-9 {
        return;
    }
    let segment_idx = bot_runtime_fill_segment_index(t_into_s);
    state.total_fill_events = state.total_fill_events.saturating_add(1);
    state.total_fill_shares += filled.max(0.0);
    if is_maker {
        state.maker_fill_events = state.maker_fill_events.saturating_add(1);
        state.maker_fill_shares += filled.max(0.0);
    } else {
        state.taker_fill_events = state.taker_fill_events.saturating_add(1);
        state.taker_fill_shares += filled.max(0.0);
    }
    state.fill_events_by_segment[segment_idx] =
        state.fill_events_by_segment[segment_idx].saturating_add(1);
    state.fill_shares_by_segment[segment_idx] += filled.max(0.0);
    if t_into_s >= cfg.late_reduce_start_seconds {
        state.late_fill_events_after_180 = state.late_fill_events_after_180.saturating_add(1);
    }
    if t_into_s >= cfg.late_balance_only_start_seconds {
        state.late_fill_events_after_225 = state.late_fill_events_after_225.saturating_add(1);
    }
}
/// Implements metrics snapshot for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_metrics_snapshot(
    state: &BotRuntimeState,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    total_cost: f64,
) -> BotRuntimeMetricsSnapshot {
    let total_fill_shares = state.total_fill_shares.max(0.0);
    let maker_fill_share = if total_fill_shares > 1e-9 {
        (state.maker_fill_shares.max(0.0) / total_fill_shares).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let taker_fill_shares = state.taker_fill_shares.max(0.0);
    let pair_taker_share = bot_runtime_taker_share(state.maker_fill_shares, taker_fill_shares);
    let daily_maker_fill_shares = state.daily_maker_fill_shares.max(0.0);
    let daily_taker_fill_shares = state.daily_taker_fill_shares.max(0.0);
    let daily_taker_share =
        bot_runtime_taker_share(daily_maker_fill_shares, daily_taker_fill_shares);
    let below_snapshot_optional_fill_rate = if state.below_snapshot_optional_submit_shares > 1e-9 {
        (state.below_snapshot_optional_fill_shares.max(0.0)
            / state.below_snapshot_optional_submit_shares.max(0.0))
        .clamp(0.0, 1.0)
    } else {
        0.0
    };
    let bad_regime_expensive_ratio = if state.bad_regime_early_observations > 0 {
        (state.bad_regime_expensive_observations as f64
            / state.bad_regime_early_observations as f64)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    BotRuntimeMetricsSnapshot {
        market_participated: bot_runtime_market_participated(
            state.total_fill_events,
            q_yes,
            q_no,
            total_cost,
        ),
        fills_per_market: state.total_fill_events,
        total_fill_shares,
        maker_fill_share,
        taker_fill_events: state.taker_fill_events,
        taker_fill_shares,
        pair_taker_share,
        daily_maker_fill_shares,
        daily_taker_fill_shares,
        daily_taker_share,
        fill_events_by_segment: state.fill_events_by_segment,
        fill_shares_by_segment: state.fill_shares_by_segment,
        paired_size: q_yes.max(0.0).min(q_no.max(0.0)),
        unmatched_size: (q_yes.max(0.0) - q_no.max(0.0)).abs(),
        unmatched_fraction: unmatched_fraction(q_yes, q_no),
        match_ratio: match_ratio(q_yes, q_no),
        imbalance_state: state.imbalance_state,
        safety_gate: state.safety_gate,
        pair_coverage: pair_coverage(q_yes, q_no),
        share_skew_ratio: share_skew_ratio(q_yes, q_no),
        inventory_vwap_sum: inventory_vwap_sum(q_yes, q_no, cost_yes, cost_no),
        late_fill_events_after_180: state.late_fill_events_after_180,
        late_fill_events_after_225: state.late_fill_events_after_225,
        late_new_orders_after_225: state.late_new_orders_after_225,
        late_new_orders_after_240: state.late_new_orders_after_240,
        prearm_ready_before_open: state.prearm_ready_before_open,
        open_both_seed_by_deadline_met: state.open_both_seed_by_deadline_met,
        open_both_late_seed_used: state.open_both_late_seed_unlock_used,
        open_both_first_submit_delta_ms: state.open_both_first_submit_delta_ms,
        open_both_submit_delta_met: state.open_both_submit_delta_met,
        second_side_by_15s: state.second_side_by_15s,
        second_side_by_30s: state.second_side_by_30s,
        first_fill_to_second_fill_ms: state.first_fill_to_second_fill_ms,
        await_second_fill_rescue_used: state.await_second_fill_rescue_used,
        await_second_fill_hard_paused: state.await_second_fill_hard_paused,
        skipped_optional_add_count: state.skipped_optional_add_count,
        repair_reserve_blocked_count: state.repair_reserve_blocked_count,
        floor_tail_blocked_count: state.floor_tail_blocked_count,
        startup_completion_blocked_count: state.startup_completion_blocked_count,
        paired_cost_band_observations: state.paired_cost_band_observations,
        paired_size_delta_by_state: state.paired_size_delta_by_state,
        tail_at_expiry: bot_runtime_tail_size(q_yes, q_no),
        worst_case_settlement_floor: bot_runtime_worst_case_settlement_floor(
            q_yes, q_no, total_cost,
        ),
        bad_regime_expensive_ratio,
        bad_regime_shutdown: state.bad_regime_shutdown,
        below_snapshot_optional_submit_count: state.below_snapshot_optional_submit_count,
        below_snapshot_optional_submit_shares: state.below_snapshot_optional_submit_shares,
        below_snapshot_optional_fill_count: state.below_snapshot_optional_fill_count,
        below_snapshot_optional_fill_shares: state.below_snapshot_optional_fill_shares,
        below_snapshot_optional_fill_rate,
        yes_refresh_cycles_started: state.yes_refresh_cycles_started,
        no_refresh_cycles_started: state.no_refresh_cycles_started,
        yes_refresh_cap_block_count: state.yes_refresh_cap_block_count,
        no_refresh_cap_block_count: state.no_refresh_cap_block_count,
        audit_decision_event_count: state.audit_decision_event_count,
        audit_runtime_event_count: state.audit_runtime_event_count,
    }
}
/// Implements canary success for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_canary_success(metrics: &BotRuntimeMetricsSnapshot) -> bool {
    let core_ok =
        metrics.inventory_vwap_sum.is_finite() && metrics.inventory_vwap_sum <= 0.995 + 1e-9;
    let maker_ok = metrics.maker_fill_share + 1e-9 >= 0.80;
    let tail_ok = if metrics.paired_size > 1e-9 {
        (metrics.tail_at_expiry / metrics.paired_size.max(1e-9)) <= 0.10 + 1e-9
    } else {
        metrics.tail_at_expiry <= 1e-9
    };
    metrics.worst_case_settlement_floor > 0.0 && core_ok && maker_ok && tail_ok
}
/// Implements canary failure summary for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_canary_failure_summary(
    metrics: &BotRuntimeMetricsSnapshot,
) -> String {
    let mut failures = Vec::new();
    if metrics.worst_case_settlement_floor <= 0.0 {
        failures.push(format!("floor={:+.2}", metrics.worst_case_settlement_floor));
    }
    if !metrics.inventory_vwap_sum.is_finite() || metrics.inventory_vwap_sum > 0.995 + 1e-9 {
        failures.push(format!("core_cost={:.3}", metrics.inventory_vwap_sum));
    }
    if metrics.paired_size > 1e-9 {
        let tail_ratio = metrics.tail_at_expiry / metrics.paired_size.max(1e-9);
        if tail_ratio > 0.10 + 1e-9 {
            failures.push(format!("tail_ratio={:.3}", tail_ratio));
        }
    } else if metrics.tail_at_expiry > 1e-9 {
        failures.push(format!("tail={:.2}", metrics.tail_at_expiry));
    }
    if metrics.maker_fill_share + 1e-9 < 0.80 {
        failures.push(format!("maker_share={:.3}", metrics.maker_fill_share));
    }
    if failures.is_empty() {
        "ok".to_string()
    } else {
        failures.join(",")
    }
}
/// Implements worst case settlement floor for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_worst_case_settlement_floor(
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
) -> f64 {
    q_yes.max(0.0).min(q_no.max(0.0)) - total_cost.max(0.0)
}
/// Implements projected tail and floor after decision for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_projected_tail_and_floor_after_decision(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
    y_bid: f64,
    n_bid: f64,
) -> Option<(f64, f64)> {
    let clip = decision.clip.max(0) as f64;
    if clip <= 0.0 {
        return None;
    }
    let mut projected_yes = q_yes.max(0.0);
    let mut projected_no = q_no.max(0.0);
    let mut projected_total_cost = total_cost.max(0.0);
    match decision.mode {
        BotRuntimePairBuildMode::PairedGrowth => {
            projected_yes += clip;
            projected_no += clip;
            projected_total_cost += clip * (y_bid.max(0.0) + n_bid.max(0.0));
        }
        BotRuntimePairBuildMode::LighterSideFirst => match decision.side? {
            OutcomeSide::Yes => {
                projected_yes += clip;
                projected_total_cost += clip * y_bid.max(0.0);
            }
            OutcomeSide::No => {
                projected_no += clip;
                projected_total_cost += clip * n_bid.max(0.0);
            }
        },
    }
    Some((
        bot_runtime_tail_size(projected_yes, projected_no),
        bot_runtime_worst_case_settlement_floor(projected_yes, projected_no, projected_total_cost),
    ))
}
/// Implements taper late action policy for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_taper_late_action_policy(
    taper_mode: BotRuntimeTaperMode,
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
    y_bid: f64,
    n_bid: f64,
) -> Option<BotRuntimeLateActionPolicy> {
    let current_tail_size = bot_runtime_tail_size(q_yes, q_no);
    let current_floor = bot_runtime_worst_case_settlement_floor(q_yes, q_no, total_cost);
    let (projected_tail_size, projected_floor) =
        bot_runtime_projected_tail_and_floor_after_decision(
            decision, q_yes, q_no, total_cost, y_bid, n_bid,
        )?;
    let improves_tail = projected_tail_size + 1e-9 < current_tail_size;
    let improves_floor = projected_floor > current_floor + 1e-9;
    let hold_reason = match taper_mode {
        BotRuntimeTaperMode::ReduceClips
            if decision.mode == BotRuntimePairBuildMode::PairedGrowth
                && current_tail_size > 1e-9 =>
        {
            Some(format!(
                "late_reduce_clips_repair_first_suppress:{:.2}:{:.2}:{:+.2}:{:+.2}",
                current_tail_size, projected_tail_size, current_floor, projected_floor
            ))
        }
        BotRuntimeTaperMode::BalanceOnly
            if decision.mode == BotRuntimePairBuildMode::PairedGrowth =>
        {
            Some(format!(
                "late_balance_only_suppress:{:.2}:{:+.2}:{:+.2}",
                current_tail_size, current_floor, projected_floor
            ))
        }
        _ if !improves_tail && !improves_floor => Some(format!(
            "late_floor_tail_priority:{:.2}:{:.2}:{:+.2}:{:+.2}",
            current_tail_size, projected_tail_size, current_floor, projected_floor
        )),
        _ => None,
    };
    Some(BotRuntimeLateActionPolicy {
        current_tail_size,
        projected_tail_size,
        current_floor,
        projected_floor,
        improves_tail,
        improves_floor,
        hold_reason,
    })
}
