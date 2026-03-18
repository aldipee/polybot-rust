use super::*;
/// Implements taper paired growth submin notional reason for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_taper_paired_growth_submin_notional_reason(
    decision: &BotRuntimePairBuildDecision,
    y_bid: f64,
    n_bid: f64,
    min_maker_notional: f64,
) -> Option<String> {
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth || decision.clip <= 0 {
        return None;
    }
    let clip = decision.clip.max(0) as f64;
    let yes_ok = maker_post_only_order_meets_min_maker_notional(
        ClobSide::Buy,
        y_bid,
        clip,
        true,
        min_maker_notional,
    );
    let no_ok = maker_post_only_order_meets_min_maker_notional(
        ClobSide::Buy,
        n_bid,
        clip,
        true,
        min_maker_notional,
    );
    match (yes_ok, no_ok) {
        (true, true) => None,
        (false, true) => Some(format!(
            "paired_growth_submin_notional:YES:{y_bid:.3}:{clip:.0}"
        )),
        (true, false) => Some(format!(
            "paired_growth_submin_notional:NO:{n_bid:.3}:{clip:.0}"
        )),
        (false, false) => Some(format!(
            "paired_growth_submin_notional:BOTH:{y_bid:.3}:{n_bid:.3}:{clip:.0}"
        )),
    }
}
/// Implements taper mode for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_taper_mode(
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimeTaperMode {
    if t_into_s >= (300.0 - cfg.final_quiet_seconds) {
        BotRuntimeTaperMode::NoOptionalAdds
    } else {
        BotRuntimeTaperMode::RepairFirst
    }
}
/// Implements taper maintenance clip for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_taper_maintenance_clip(min_shares: f64) -> i64 {
    let min_lot = min_shares.max(1.0);
    round_down_to_lot(min_lot, min_lot) as i64
}
/// Implements taper maintenance decision for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_taper_maintenance_decision(
    decision: BotRuntimePairBuildDecision,
    min_shares: f64,
) -> BotRuntimePairBuildDecision {
    BotRuntimePairBuildDecision {
        clip: bot_runtime_taper_maintenance_clip(min_shares),
        clip_bucket: "small",
        ..decision
    }
}
/// Implements tail size for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_tail_size(q_yes: f64, q_no: f64) -> f64 {
    (q_yes.max(0.0) - q_no.max(0.0)).abs()
}
/// Implements tail cap fraction for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_tail_cap_fraction(
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> f64 {
    if t_into_s >= cfg.tail_cap_late_start_seconds {
        cfg.tail_cap_late_fraction.max(0.0)
    } else if t_into_s >= cfg.tail_cap_mid_start_seconds {
        cfg.tail_cap_mid_fraction.max(0.0)
    } else {
        cfg.tail_cap_early_fraction.max(0.0)
    }
}
/// Implements tail cap status for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_tail_cap_status(
    q_yes: f64,
    q_no: f64,
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimeTailCapStatus {
    let paired_size = q_yes.max(0.0).min(q_no.max(0.0));
    let tail_size = bot_runtime_tail_size(q_yes, q_no);
    let cap_fraction = bot_runtime_tail_cap_fraction(t_into_s, cfg);
    BotRuntimeTailCapStatus {
        paired_size,
        tail_size,
        cap_fraction,
        cap_shares: paired_size.max(0.0) * cap_fraction.max(0.0),
    }
}
/// Implements tail cap exceeded for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_tail_cap_exceeded(
    q_yes: f64,
    q_no: f64,
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<BotRuntimeTailCapStatus> {
    let status = bot_runtime_tail_cap_status(q_yes, q_no, t_into_s, cfg);
    if status.tail_size > status.cap_shares + 1e-9 {
        Some(status)
    } else {
        None
    }
}
/// Implements pair build apply tail repair priority for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_pair_build_apply_tail_repair_priority(
    decision: BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    y_bid: f64,
    n_bid: f64,
    remaining_budget: f64,
    min_shares: f64,
    min_maker_notional: f64,
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimePairBuildDecision {
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth
        || bot_runtime_tail_cap_exceeded(q_yes, q_no, t_into_s, cfg).is_none()
    {
        return decision;
    }
    let side = if q_yes + 1e-9 < q_no {
        OutcomeSide::Yes
    } else if q_no + 1e-9 < q_yes {
        OutcomeSide::No
    } else {
        return decision;
    };
    let side_bid = match side {
        OutcomeSide::Yes => y_bid,
        OutcomeSide::No => n_bid,
    };
    if side_bid <= 0.0 || !side_bid.is_finite() {
        return decision;
    }
    let min_lot = min_shares.max(1.0);
    if !bot_runtime_pair_build_exact_gap_repair_is_executable(
        decision.qty_gap,
        side_bid,
        min_lot,
        min_maker_notional,
    ) {
        return decision;
    }
    let budget_clip_cap = (remaining_budget.max(0.0) / side_bid).floor();
    let lighter_clip_after_cost_quality = bot_runtime_pair_build_lighter_clip_after_cost_quality(
        decision.requested_clip,
        decision.qty_gap,
        min_lot,
        cfg,
        decision.cpp_hint,
    );
    let clip = round_down_to_lot(
        lighter_clip_after_cost_quality.min(budget_clip_cap),
        min_lot,
    );
    if clip + 1e-9 < min_lot {
        return decision;
    }
    let (effective_marginal_pair_cost, residual_unit_cost) =
        bot_runtime_pair_build_rebalance_effective_marginal_cost(
            q_yes, q_no, cost_yes, cost_no, side, side_bid,
        );
    let price_zone =
        bot_runtime_pair_build_projected_paired_cost_band(effective_marginal_pair_cost);
    BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::LighterSideFirst,
        side: Some(side),
        clip: clip as i64,
        clip_bucket: bot_runtime_pair_build_clip_bucket(clip, cfg),
        marginal_cost_mode: BotRuntimeMarginalCostMode::RebalanceAdd,
        effective_marginal_pair_cost,
        price_zone,
        residual_unit_cost,
        lagging_side_quote: Some(side_bid),
        ..decision
    }
}
