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
/// Implements paired cost band index for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_paired_cost_band_index(band: BotRuntimePairedCostBand) -> usize {
    match band {
        BotRuntimePairedCostBand::StrongGrowth => 0,
        BotRuntimePairedCostBand::NormalGrowth => 1,
        BotRuntimePairedCostBand::ReducedGrowth => 2,
        BotRuntimePairedCostBand::RepairOnly => 3,
        BotRuntimePairedCostBand::Freeze => 4,
    }
}
/// Implements paired cost band label for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_paired_cost_band_label(index: usize) -> &'static str {
    match index {
        0 => BotRuntimePairedCostBand::StrongGrowth.as_str(),
        1 => BotRuntimePairedCostBand::NormalGrowth.as_str(),
        2 => BotRuntimePairedCostBand::ReducedGrowth.as_str(),
        3 => BotRuntimePairedCostBand::RepairOnly.as_str(),
        _ => BotRuntimePairedCostBand::Freeze.as_str(),
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
    }
    state.fill_events_by_segment[segment_idx] =
        state.fill_events_by_segment[segment_idx].saturating_add(1);
    state.fill_shares_by_segment[segment_idx] += filled.max(0.0);
    if t_into_s >= cfg.taper_start_seconds {
        state.taper_fill_events_after_240 = state.taper_fill_events_after_240.saturating_add(1);
    }
    if t_into_s >= (300.0 - cfg.final_quiet_seconds.max(0.0)) {
        state.taper_fill_events_after_270 = state.taper_fill_events_after_270.saturating_add(1);
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
        fill_events_by_segment: state.fill_events_by_segment,
        fill_shares_by_segment: state.fill_shares_by_segment,
        paired_size: q_yes.max(0.0).min(q_no.max(0.0)),
        unmatched_size: (q_yes.max(0.0) - q_no.max(0.0)).abs(),
        pair_coverage: pair_coverage(q_yes, q_no),
        share_skew_ratio: share_skew_ratio(q_yes, q_no),
        inventory_vwap_sum: inventory_vwap_sum(q_yes, q_no, cost_yes, cost_no),
        taper_fill_events_after_240: state.taper_fill_events_after_240,
        taper_fill_events_after_270: state.taper_fill_events_after_270,
        taper_new_orders_after_240: state.taper_new_orders_after_240,
        taper_new_orders_after_270: state.taper_new_orders_after_270,
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
        BotRuntimeTaperMode::RepairFirst
            if decision.mode == BotRuntimePairBuildMode::PairedGrowth
                && current_tail_size > 1e-9 =>
        {
            Some(format!(
                "late_repair_first_suppress:{:.2}:{:.2}:{:+.2}:{:+.2}",
                current_tail_size, projected_tail_size, current_floor, projected_floor
            ))
        }
        BotRuntimeTaperMode::NoOptionalAdds
            if decision.mode == BotRuntimePairBuildMode::PairedGrowth =>
        {
            Some(format!(
                "late_no_optional_adds_suppress:{:.2}:{:+.2}:{:+.2}",
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
