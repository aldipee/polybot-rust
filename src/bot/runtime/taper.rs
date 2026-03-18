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
    if t_into_s >= cfg.late_balance_only_start_seconds {
        BotRuntimeTaperMode::BalanceOnly
    } else {
        BotRuntimeTaperMode::ReduceClips
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
    q_yes: f64,
    q_no: f64,
    remaining_budget: f64,
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimePairBuildDecision {
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth {
        return decision;
    }
    let maintenance_clip = bot_runtime_taper_maintenance_clip(min_shares);
    if maintenance_clip <= 0 || decision.clip <= maintenance_clip {
        return decision;
    }
    bot_runtime_pair_build_decision_with_selected_clip(
        decision,
        maintenance_clip,
        q_yes,
        q_no,
        remaining_budget,
        t_into_s,
        cfg,
    )
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
    let sizing = bot_runtime_pair_build_repair_clip_sizing(
        decision.qty_gap,
        side_bid,
        min_lot,
        min_maker_notional,
    );
    let exact_gap_clip = sizing
        .filter(|sizing| sizing.min_valid_clip <= sizing.exact_gap_clip)
        .map(|sizing| sizing.exact_gap_clip as f64);
    let min_valid_clip = sizing.map(|sizing| sizing.min_valid_clip as f64);
    if exact_gap_clip.is_none()
        && bot_runtime_repair_requested_rung(decision.qty_gap, None, min_valid_clip, cfg).is_none()
    {
        return decision;
    }
    let requested =
        bot_runtime_repair_requested_rung(decision.qty_gap, exact_gap_clip, min_valid_clip, cfg);
    let Some((requested_rung, requested_clip)) = requested else {
        return decision;
    };
    let (
        green_both_sides_filled,
        green_price_ok,
        green_imbalance_ok,
        green_time_ok,
        _green_budget_ok,
        _green_conditions_met,
    ) = bot_runtime_pair_build_green_conditions(
        BotRuntimePairBuildMode::LighterSideFirst,
        Some(side),
        requested_clip,
        q_yes,
        q_no,
        remaining_budget.max(0.0),
        bot_runtime_pair_build_rebalance_effective_marginal_cost(
            q_yes, q_no, cost_yes, cost_no, side, side_bid,
        )
        .0,
        side_bid,
        t_into_s,
        cfg,
    );
    let structural_green =
        green_both_sides_filled && green_price_ok && green_imbalance_ok && green_time_ok;
    let large_allowed_clip_cap = if requested_rung.is_large() && !structural_green {
        cfg.clip_ladder[1]
    } else {
        requested_clip
    };
    let (effective_marginal_pair_cost, residual_unit_cost) =
        bot_runtime_pair_build_rebalance_effective_marginal_cost(
            q_yes, q_no, cost_yes, cost_no, side, side_bid,
        );
    let budget_clip_cap = (remaining_budget.max(0.0) / side_bid.max(0.0001)).floor();
    let lighter_clip_after_cost_quality = bot_runtime_pair_build_lighter_clip_after_cost_quality(
        requested_clip,
        decision.qty_gap,
        min_lot,
        cfg,
        decision.cpp_hint,
    );
    let final_clip_cap = lighter_clip_after_cost_quality
        .min(budget_clip_cap)
        .min(large_allowed_clip_cap);
    let Some((clip, selected_rung)) = bot_runtime_repair_clip_choice(
        final_clip_cap,
        decision.qty_gap,
        exact_gap_clip,
        min_valid_clip,
        cfg,
    ) else {
        return decision;
    };
    let price_zone =
        bot_runtime_pair_build_projected_paired_cost_band(effective_marginal_pair_cost);
    let rewritten = BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::LighterSideFirst,
        side: Some(side),
        clip: clip as i64,
        selected_rung,
        requested_rung,
        requested_clip,
        requested_large_clip: requested_rung.is_large(),
        clip_bucket: bot_runtime_pair_build_clip_bucket(clip, cfg),
        marginal_cost_mode: BotRuntimeMarginalCostMode::RebalanceAdd,
        effective_marginal_pair_cost,
        price_zone,
        residual_unit_cost,
        lagging_side_quote: Some(side_bid),
        ..decision
    };
    bot_runtime_pair_build_decision_with_selected_clip(
        rewritten,
        clip as i64,
        q_yes,
        q_no,
        remaining_budget.max(0.0),
        t_into_s,
        cfg,
    )
}
