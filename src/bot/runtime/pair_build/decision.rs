use super::super::*;
use super::costs::{
    bot_runtime_pair_build_balanced_add_effective_marginal_cost,
    bot_runtime_pair_build_projected_paired_cost_band,
    bot_runtime_pair_build_rebalance_effective_marginal_cost,
};

/// Implements pair build clip bucket for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_clip_bucket(
    clip: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> &'static str {
    if clip + 1e-9 >= cfg.clip_ladder[2] {
        "large"
    } else if clip + 1e-9 >= cfg.clip_ladder[1] {
        "medium"
    } else {
        "small"
    }
}

pub(in crate::bot) fn bot_runtime_clip_rung_value(
    rung: BotRuntimeClipRung,
    cfg: &BotRuntimeConfigSnapshot,
    exact_gap_clip: Option<f64>,
) -> f64 {
    match rung {
        BotRuntimeClipRung::Seed => cfg.clip_ladder[0],
        BotRuntimeClipRung::Normal => cfg.clip_ladder[1],
        BotRuntimeClipRung::Large1 => cfg.clip_ladder[2],
        BotRuntimeClipRung::Large2 => cfg.clip_ladder[3],
        BotRuntimeClipRung::ExactGapRepair => exact_gap_clip.unwrap_or(0.0).max(0.0),
    }
}

fn bot_runtime_growth_requested_rung(
    current_base: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimeClipRung {
    if current_base + 1e-9 >= cfg.clip_ladder[2] {
        BotRuntimeClipRung::Large2
    } else if current_base + 1e-9 >= cfg.clip_ladder[1] {
        BotRuntimeClipRung::Large1
    } else {
        BotRuntimeClipRung::Normal
    }
}

fn bot_runtime_growth_cpp_clip_cap(
    requested_clip: f64,
    cfg: &BotRuntimeConfigSnapshot,
    cpp_hint: BotRuntimePairBuildCppHint,
    under_min_target: bool,
) -> f64 {
    match cpp_hint {
        BotRuntimePairBuildCppHint::Normal => requested_clip,
        BotRuntimePairBuildCppHint::Medium => requested_clip.min(cfg.clip_ladder[2]),
        BotRuntimePairBuildCppHint::Small => {
            if under_min_target {
                requested_clip.min(cfg.clip_ladder[2])
            } else {
                requested_clip.min(cfg.clip_ladder[1])
            }
        }
    }
}

fn bot_runtime_repair_cpp_clip_cap(
    requested_clip: f64,
    cfg: &BotRuntimeConfigSnapshot,
    cpp_hint: BotRuntimePairBuildCppHint,
) -> f64 {
    match cpp_hint {
        BotRuntimePairBuildCppHint::Normal => requested_clip,
        BotRuntimePairBuildCppHint::Medium => requested_clip.min(cfg.clip_ladder[1]),
        BotRuntimePairBuildCppHint::Small => requested_clip.min(cfg.clip_ladder[0]),
    }
}

fn bot_runtime_growth_clip_choice(
    max_clip: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<(f64, BotRuntimeClipRung)> {
    for (clip, rung) in [
        (cfg.clip_ladder[3], BotRuntimeClipRung::Large2),
        (cfg.clip_ladder[2], BotRuntimeClipRung::Large1),
        (cfg.clip_ladder[1], BotRuntimeClipRung::Normal),
        (cfg.clip_ladder[0], BotRuntimeClipRung::Seed),
    ] {
        if clip <= max_clip + 1e-9 {
            return Some((clip, rung));
        }
    }
    None
}

pub(in crate::bot) fn bot_runtime_repair_clip_choice(
    max_clip: f64,
    qty_gap: f64,
    exact_gap_clip: Option<f64>,
    min_valid_clip: Option<f64>,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<(f64, BotRuntimeClipRung)> {
    let gap = qty_gap.max(0.0);
    let min_valid_clip = min_valid_clip.unwrap_or(0.0).max(0.0);
    for (clip, rung) in [
        (cfg.clip_ladder[3], BotRuntimeClipRung::Large2),
        (cfg.clip_ladder[2], BotRuntimeClipRung::Large1),
        (cfg.clip_ladder[1], BotRuntimeClipRung::Normal),
        (cfg.clip_ladder[0], BotRuntimeClipRung::Seed),
    ] {
        if clip <= max_clip + 1e-9 && clip <= gap + 1e-9 && clip + 1e-9 >= min_valid_clip {
            return Some((clip, rung));
        }
    }
    exact_gap_clip
        .filter(|clip| {
            *clip > 0.0
                && *clip <= max_clip + 1e-9
                && *clip + 1e-9 < cfg.clip_ladder[0]
                && *clip + 1e-9 >= min_valid_clip
        })
        .map(|clip| (clip, BotRuntimeClipRung::ExactGapRepair))
}

pub(in crate::bot) fn bot_runtime_repair_requested_rung(
    qty_gap: f64,
    exact_gap_clip: Option<f64>,
    min_valid_clip: Option<f64>,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<(BotRuntimeClipRung, f64)> {
    bot_runtime_repair_clip_choice(f64::INFINITY, qty_gap, exact_gap_clip, min_valid_clip, cfg)
        .map(|(clip, rung)| (rung, clip))
}

pub(in crate::bot) fn bot_runtime_pair_build_green_conditions(
    mode: BotRuntimePairBuildMode,
    side: Option<OutcomeSide>,
    clip: f64,
    q_yes: f64,
    q_no: f64,
    remaining_budget: f64,
    effective_marginal_pair_cost: f64,
    budget_reference_cost: f64,
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> (bool, bool, bool, bool, bool, bool) {
    let both_sides_filled = q_yes > 1e-9 && q_no > 1e-9;
    let projected_unmatched_fraction =
        bot_runtime_projected_unmatched_fraction(mode, side, clip.max(0.0), q_yes, q_no);
    let price_ok = effective_marginal_pair_cost + 1e-9 < 0.97;
    let imbalance_ok = projected_unmatched_fraction + 1e-9 < cfg.imbalance_target_fraction;
    let time_ok = t_into_s + 1e-9 < 180.0;
    let budget_ok = remaining_budget + 1e-9 >= clip.max(0.0) * budget_reference_cost.max(0.0);
    (
        both_sides_filled,
        price_ok,
        imbalance_ok,
        time_ok,
        budget_ok,
        both_sides_filled && price_ok && imbalance_ok && time_ok && budget_ok,
    )
}

pub(in crate::bot) fn bot_runtime_pair_build_selected_rung(
    mode: BotRuntimePairBuildMode,
    clip: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimeClipRung {
    if mode == BotRuntimePairBuildMode::LighterSideFirst && clip + 1e-9 < cfg.clip_ladder[0] {
        BotRuntimeClipRung::ExactGapRepair
    } else if clip + 1e-9 >= cfg.clip_ladder[3] {
        BotRuntimeClipRung::Large2
    } else if clip + 1e-9 >= cfg.clip_ladder[2] {
        BotRuntimeClipRung::Large1
    } else if clip + 1e-9 >= cfg.clip_ladder[1] {
        BotRuntimeClipRung::Normal
    } else {
        BotRuntimeClipRung::Seed
    }
}

fn bot_runtime_pair_build_refresh_residual_direction(
    decision: &mut BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
) {
    decision.residual_side = bot_runtime_residual_side(q_yes, q_no);
    decision.projected_residual_side = Some(
        bot_runtime_projected_residual_side_and_magnitude(
            decision.mode,
            decision.side,
            decision.clip.max(0) as f64,
            q_yes,
            q_no,
        )
        .0,
    )
    .flatten();
    decision.residual_kind = bot_runtime_residual_kind(
        decision.favorite_side,
        decision.underdog_side,
        decision.residual_side,
    );
    decision.increases_underdog_residual = bot_runtime_would_increase_underdog_residual_for_side(
        decision.mode,
        decision.side,
        decision.clip.max(0) as f64,
        q_yes,
        q_no,
        decision.underdog_side,
    );
}

pub(in crate::bot) fn bot_runtime_pair_build_decision_with_selected_clip(
    mut decision: BotRuntimePairBuildDecision,
    clip: i64,
    q_yes: f64,
    q_no: f64,
    remaining_budget: f64,
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimePairBuildDecision {
    let clip_f = clip.max(0) as f64;
    decision.clip = clip.max(0);
    decision.selected_rung = bot_runtime_pair_build_selected_rung(decision.mode, clip_f, cfg);
    decision.clip_bucket = bot_runtime_pair_build_clip_bucket(clip_f, cfg);
    decision.projected_unmatched_fraction =
        bot_runtime_projected_unmatched_fraction(decision.mode, decision.side, clip_f, q_yes, q_no);
    decision.reduces_imbalance = bot_runtime_order_reduces_imbalance(
        decision.current_unmatched_fraction,
        decision.projected_unmatched_fraction,
    );
    let (
        green_both_sides_filled,
        green_price_ok,
        green_imbalance_ok,
        green_time_ok,
        green_budget_ok,
        green_conditions_met,
    ) = bot_runtime_pair_build_green_conditions(
        decision.mode,
        decision.side,
        clip_f,
        q_yes,
        q_no,
        remaining_budget,
        decision.effective_marginal_pair_cost,
        match decision.marginal_cost_mode {
            BotRuntimeMarginalCostMode::BalancedAdd => decision.effective_marginal_pair_cost,
            BotRuntimeMarginalCostMode::RebalanceAdd => decision
                .lagging_side_quote
                .unwrap_or(decision.effective_marginal_pair_cost),
        },
        t_into_s,
        cfg,
    );
    decision.green_both_sides_filled = green_both_sides_filled;
    decision.green_price_ok = green_price_ok;
    decision.green_imbalance_ok = green_imbalance_ok;
    decision.green_time_ok = green_time_ok;
    decision.green_budget_ok = green_budget_ok;
    decision.green_conditions_met = green_conditions_met;
    decision.one_side_exception_kind = match decision.mode {
        BotRuntimePairBuildMode::PairedGrowth => BotRuntimeOneSideExceptionKind::None,
        BotRuntimePairBuildMode::LighterSideFirst => match decision.one_side_exception_kind {
            BotRuntimeOneSideExceptionKind::SecondSideCompletion => {
                BotRuntimeOneSideExceptionKind::SecondSideCompletion
            }
            _ => BotRuntimeOneSideExceptionKind::LaggingSideRepair,
        },
    };
    bot_runtime_pair_build_refresh_residual_direction(&mut decision, q_yes, q_no);
    decision
}

pub(in crate::bot) fn bot_runtime_pair_build_has_large_clip_intent(
    decision: &BotRuntimePairBuildDecision,
) -> bool {
    decision.requested_large_clip || decision.selected_rung.is_large()
}

/// Implements pair build decision for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_decision(
    t_into_s: f64,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    y_bid: f64,
    y_ask: f64,
    n_bid: f64,
    n_ask: f64,
    total_usable_budget: f64,
    total_cost: f64,
    min_shares: f64,
    min_maker_notional: f64,
    tick_size: f64,
    cfg: &BotRuntimeConfigSnapshot,
    under_min_target: bool,
) -> Result<BotRuntimePairBuildDecision, String> {
    let min_lot = min_shares.max(1.0);
    let remaining_budget = (total_usable_budget.max(0.0) - total_cost.max(0.0)).max(0.0);
    if remaining_budget <= 0.0 {
        return Err("budget_exhausted".to_string());
    }
    if y_bid <= 0.0 || y_ask <= 0.0 {
        return Err("missing_yes_quotes".to_string());
    }
    if n_bid <= 0.0 || n_ask <= 0.0 {
        return Err("missing_no_quotes".to_string());
    }

    let current_base = q_yes.max(0.0).min(q_no.max(0.0));
    let pair_sum = bot_runtime_pair_build_balanced_add_effective_marginal_cost(y_bid, n_bid);
    let current_unmatched_fraction = unmatched_fraction(q_yes, q_no);
    let current_match_ratio = match_ratio(q_yes, q_no);
    let current_imbalance_state = bot_runtime_current_imbalance_state(q_yes, q_no, cfg);
    let (favorite_side, underdog_side) =
        bot_runtime_favorite_underdog_sides(y_bid, n_bid, tick_size);
    let pair_coverage = pair_coverage(q_yes, q_no);
    let skew_ratio = share_skew_ratio(q_yes, q_no);
    let qty_gap = (q_yes.max(0.0) - q_no.max(0.0)).abs();
    let inventory_vwap_sum = inventory_vwap_sum(q_yes, q_no, cost_yes, cost_no);
    let market_snapshot_vwap_sum = market_snapshot_vwap_sum(y_bid, y_ask, n_bid, n_ask);
    if matches!(
        current_imbalance_state,
        BotRuntimeImbalanceState::HardDisable
    ) {
        return Err(format!(
            "hard_imbalance_disable:{current_unmatched_fraction:.3}"
        ));
    }

    let cpp_hint = if !inventory_vwap_sum.is_finite() || !market_snapshot_vwap_sum.is_finite() {
        BotRuntimePairBuildCppHint::Small
    } else {
        let medium_threshold = (market_snapshot_vwap_sum + 0.04).max(1.01);
        let small_threshold = (market_snapshot_vwap_sum + 0.08).max(1.05);
        if inventory_vwap_sum > small_threshold {
            BotRuntimePairBuildCppHint::Small
        } else if inventory_vwap_sum > medium_threshold {
            BotRuntimePairBuildCppHint::Medium
        } else {
            BotRuntimePairBuildCppHint::Normal
        }
    };

    let lighter_side = if q_yes + 1e-9 < q_no {
        Some(OutcomeSide::Yes)
    } else if q_no + 1e-9 < q_yes {
        Some(OutcomeSide::No)
    } else {
        None
    };

    let repair_only_imbalance =
        !matches!(current_imbalance_state, BotRuntimeImbalanceState::Normal);
    let lighter_side_first = lighter_side.is_some()
        && bot_runtime_pair_build_materially_skewed(
            pair_coverage,
            skew_ratio,
            qty_gap,
            min_lot,
            cfg,
        )
        || (repair_only_imbalance && lighter_side.is_some());
    if lighter_side_first {
        let side = lighter_side.unwrap_or(OutcomeSide::Yes);
        let side_bid = match side {
            OutcomeSide::Yes => y_bid,
            OutcomeSide::No => n_bid,
        };
        let sizing = bot_runtime_pair_build_repair_clip_sizing(
            qty_gap,
            side_bid,
            min_lot,
            min_maker_notional,
        );
        let exact_gap_clip = sizing
            .filter(|sizing| sizing.min_valid_clip <= sizing.exact_gap_clip)
            .map(|sizing| sizing.exact_gap_clip as f64);
        let min_valid_clip = sizing.map(|sizing| sizing.min_valid_clip as f64);
        let (effective_marginal_pair_cost, residual_unit_cost) =
            bot_runtime_pair_build_rebalance_effective_marginal_cost(
                q_yes, q_no, cost_yes, cost_no, side, side_bid,
            );
        let price_zone =
            bot_runtime_pair_build_projected_paired_cost_band(effective_marginal_pair_cost);
        if side_bid > 0.0 && side_bid.is_finite() {
            if let Some((requested_rung, requested_clip)) =
                bot_runtime_repair_requested_rung(qty_gap, exact_gap_clip, min_valid_clip, cfg)
            {
                let requested_large_clip = requested_rung.is_large();
                let (
                    requested_green_both_sides_filled,
                    requested_green_price_ok,
                    requested_green_imbalance_ok,
                    requested_green_time_ok,
                    _requested_green_budget_ok,
                    _requested_green_conditions_met,
                ) = bot_runtime_pair_build_green_conditions(
                    BotRuntimePairBuildMode::LighterSideFirst,
                    Some(side),
                    requested_clip,
                    q_yes,
                    q_no,
                    remaining_budget,
                    effective_marginal_pair_cost,
                    side_bid,
                    t_into_s,
                    cfg,
                );
                let structural_green = requested_green_both_sides_filled
                    && requested_green_price_ok
                    && requested_green_imbalance_ok
                    && requested_green_time_ok;
                let large_allowed_clip_cap = if requested_large_clip && !structural_green {
                    cfg.clip_ladder[1]
                } else {
                    requested_clip
                };
                let budget_clip_cap = (remaining_budget / side_bid.max(0.0001)).floor();
                let repair_cpp_cap = bot_runtime_repair_cpp_clip_cap(requested_clip, cfg, cpp_hint);
                let final_clip_cap = large_allowed_clip_cap
                    .min(repair_cpp_cap)
                    .min(budget_clip_cap);
                if let Some((clip, selected_rung)) = bot_runtime_repair_clip_choice(
                    final_clip_cap,
                    qty_gap,
                    exact_gap_clip,
                    min_valid_clip,
                    cfg,
                ) {
                    let projected_unmatched_fraction = bot_runtime_projected_unmatched_fraction(
                        BotRuntimePairBuildMode::LighterSideFirst,
                        Some(side),
                        clip,
                        q_yes,
                        q_no,
                    );
                    if projected_unmatched_fraction + 1e-9 >= cfg.imbalance_disable_fraction {
                        return Err(format!(
                            "projected_hard_imbalance_block:{projected_unmatched_fraction:.3}"
                        ));
                    }
                    let reduces_imbalance = bot_runtime_order_reduces_imbalance(
                        current_unmatched_fraction,
                        projected_unmatched_fraction,
                    );
                    if !reduces_imbalance {
                        return Err(format!(
                            "repair_does_not_reduce_imbalance:{current_unmatched_fraction:.3}:{projected_unmatched_fraction:.3}"
                        ));
                    }
                    let decision = BotRuntimePairBuildDecision {
                        mode: BotRuntimePairBuildMode::LighterSideFirst,
                        side: Some(side),
                        clip: clip as i64,
                        selected_rung,
                        requested_rung,
                        requested_clip,
                        requested_large_clip,
                        clip_bucket: bot_runtime_pair_build_clip_bucket(clip, cfg),
                        cpp_hint,
                        marginal_cost_mode: BotRuntimeMarginalCostMode::RebalanceAdd,
                        effective_marginal_pair_cost,
                        price_zone,
                        residual_unit_cost,
                        lagging_side_quote: Some(side_bid),
                        favorite_side,
                        underdog_side,
                        residual_side: None,
                        projected_residual_side: None,
                        residual_kind: BotRuntimeResidualKind::None,
                        increases_underdog_residual: false,
                        one_side_exception_kind: BotRuntimeOneSideExceptionKind::LaggingSideRepair,
                        pair_sum,
                        current_unmatched_fraction,
                        projected_unmatched_fraction,
                        match_ratio: current_match_ratio,
                        imbalance_state: current_imbalance_state,
                        reduces_imbalance,
                        green_both_sides_filled: false,
                        green_price_ok: false,
                        green_imbalance_ok: false,
                        green_time_ok: false,
                        green_budget_ok: false,
                        green_conditions_met: false,
                        pair_coverage,
                        skew_ratio,
                        current_base,
                        qty_gap,
                        inventory_vwap_sum,
                        market_snapshot_vwap_sum,
                    };
                    return Ok(bot_runtime_pair_build_decision_with_selected_clip(
                        decision,
                        clip as i64,
                        q_yes,
                        q_no,
                        remaining_budget,
                        t_into_s,
                        cfg,
                    ));
                }
            }
        }
        if repair_only_imbalance {
            let reason = match current_imbalance_state {
                BotRuntimeImbalanceState::Throttle => "imbalance_throttle_repair_unavailable",
                BotRuntimeImbalanceState::Warning => "imbalance_warning_repair_unavailable",
                BotRuntimeImbalanceState::HardDisable => "hard_imbalance_disable",
                BotRuntimeImbalanceState::Normal => "imbalance_repair_unavailable",
            };
            return Err(format!(
                "{reason}:{current_unmatched_fraction:.3}:{qty_gap:.2}"
            ));
        }
    }

    if repair_only_imbalance {
        let reason = match current_imbalance_state {
            BotRuntimeImbalanceState::Throttle => "imbalance_throttle",
            BotRuntimeImbalanceState::Warning => "imbalance_warning",
            BotRuntimeImbalanceState::HardDisable => "hard_imbalance_disable",
            BotRuntimeImbalanceState::Normal => "imbalance_repair_only",
        };
        return Err(format!("{reason}:{current_unmatched_fraction:.3}"));
    }
    let price_zone = bot_runtime_pair_build_projected_paired_cost_band(pair_sum);
    let requested_rung = bot_runtime_growth_requested_rung(current_base, cfg);
    let requested_clip = bot_runtime_clip_rung_value(requested_rung, cfg, None);
    let requested_large_clip = requested_rung.is_large();
    let (
        requested_green_both_sides_filled,
        requested_green_price_ok,
        requested_green_imbalance_ok,
        requested_green_time_ok,
        _requested_green_budget_ok,
        _requested_green_conditions_met,
    ) = bot_runtime_pair_build_green_conditions(
        BotRuntimePairBuildMode::PairedGrowth,
        None,
        requested_clip,
        q_yes,
        q_no,
        remaining_budget,
        pair_sum,
        pair_sum,
        t_into_s,
        cfg,
    );
    let structural_green = requested_green_both_sides_filled
        && requested_green_price_ok
        && requested_green_imbalance_ok
        && requested_green_time_ok;
    let large_allowed_clip_cap = if requested_large_clip && !structural_green {
        cfg.clip_ladder[1]
    } else {
        requested_clip
    };
    let pair_budget_clip_cap = (remaining_budget / pair_sum.max(0.0001)).floor();
    let cpp_clip_cap =
        bot_runtime_growth_cpp_clip_cap(requested_clip, cfg, cpp_hint, under_min_target);
    let final_clip_cap = large_allowed_clip_cap
        .min(cpp_clip_cap)
        .min(pair_budget_clip_cap);
    let Some((clip, selected_rung)) = bot_runtime_growth_clip_choice(final_clip_cap, cfg) else {
        return Err("budget_too_small".to_string());
    };
    let projected_unmatched_fraction = bot_runtime_projected_unmatched_fraction(
        BotRuntimePairBuildMode::PairedGrowth,
        None,
        clip,
        q_yes,
        q_no,
    );
    if projected_unmatched_fraction + 1e-9 >= cfg.imbalance_disable_fraction {
        return Err(format!(
            "projected_hard_imbalance_block:{projected_unmatched_fraction:.3}"
        ));
    }
    let reduces_imbalance = bot_runtime_order_reduces_imbalance(
        current_unmatched_fraction,
        projected_unmatched_fraction,
    );

    let decision = BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::PairedGrowth,
        side: None,
        clip: clip as i64,
        selected_rung,
        requested_rung,
        requested_clip,
        requested_large_clip,
        clip_bucket: bot_runtime_pair_build_clip_bucket(clip, cfg),
        cpp_hint,
        marginal_cost_mode: BotRuntimeMarginalCostMode::BalancedAdd,
        effective_marginal_pair_cost: pair_sum,
        price_zone,
        residual_unit_cost: None,
        lagging_side_quote: None,
        favorite_side,
        underdog_side,
        residual_side: None,
        projected_residual_side: None,
        residual_kind: BotRuntimeResidualKind::None,
        increases_underdog_residual: false,
        one_side_exception_kind: BotRuntimeOneSideExceptionKind::None,
        pair_sum,
        current_unmatched_fraction,
        projected_unmatched_fraction,
        match_ratio: current_match_ratio,
        imbalance_state: current_imbalance_state,
        reduces_imbalance,
        green_both_sides_filled: false,
        green_price_ok: false,
        green_imbalance_ok: false,
        green_time_ok: false,
        green_budget_ok: false,
        green_conditions_met: false,
        pair_coverage,
        skew_ratio,
        current_base,
        qty_gap,
        inventory_vwap_sum,
        market_snapshot_vwap_sum,
    };
    Ok(bot_runtime_pair_build_decision_with_selected_clip(
        decision,
        clip as i64,
        q_yes,
        q_no,
        remaining_budget,
        t_into_s,
        cfg,
    ))
}

/// Implements await second fill live order timeout seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_await_second_fill_live_order_timeout_seconds(
    stale_seconds: f64,
) -> f64 {
    (stale_seconds.max(1.0) * 2.0).max(6.0)
}

/// Implements pair build lighter live order timeout seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_lighter_live_order_timeout_seconds(
    stale_seconds: f64,
    decision: &BotRuntimePairBuildDecision,
) -> f64 {
    let base = (stale_seconds.max(1.0) * 2.0).max(6.0);
    if bot_runtime_pair_build_has_large_clip_intent(decision) {
        base.max(7.0)
    } else {
        base
    }
}

/// Implements pair build paired live order timeout seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_paired_live_order_timeout_seconds(
    stale_seconds: f64,
    decision: &BotRuntimePairBuildDecision,
) -> f64 {
    let base = stale_seconds.max(1.0);
    let timeout = match decision.cpp_hint {
        BotRuntimePairBuildCppHint::Normal => (base * 2.0).max(6.0),
        BotRuntimePairBuildCppHint::Medium => (base * 2.5).max(7.0),
        BotRuntimePairBuildCppHint::Small => (base * 3.0).max(8.0),
    };
    if bot_runtime_pair_build_has_large_clip_intent(decision) {
        timeout.max(8.0)
    } else {
        timeout
    }
}

/// Implements pair build asymmetry timeout seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_asymmetry_timeout_seconds(
    stale_seconds: f64,
    decision: &BotRuntimePairBuildDecision,
    broken_submit: bool,
) -> f64 {
    let base = stale_seconds.max(1.0);
    if broken_submit && decision.mode == BotRuntimePairBuildMode::PairedGrowth {
        return if bot_runtime_pair_build_has_large_clip_intent(decision) {
            1.5
        } else {
            1.0
        };
    }
    let timeout = match decision.mode {
        BotRuntimePairBuildMode::LighterSideFirst => (base * 2.0).max(6.0),
        BotRuntimePairBuildMode::PairedGrowth => match decision.cpp_hint {
            BotRuntimePairBuildCppHint::Normal => (base * 1.5).max(5.0),
            BotRuntimePairBuildCppHint::Medium => (base * 2.0).max(5.5),
            BotRuntimePairBuildCppHint::Small => (base * 2.0).max(6.0),
        },
    };
    if bot_runtime_pair_build_has_large_clip_intent(decision) {
        timeout.max(6.0)
    } else {
        timeout
    }
}

/// Implements pair build broken submit recent reject grace seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_broken_submit_recent_reject_grace_seconds() -> f64 {
    2.0
}

/// Implements pair build broken asymmetry for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_broken_asymmetry(
    live_side: OutcomeSide,
    yes_slot: &MakerOrderSlot,
    no_slot: &MakerOrderSlot,
    now: f64,
    reject_cooldown: f64,
    max_reject_cooldown: f64,
) -> bool {
    let missing_slot = match live_side {
        OutcomeSide::Yes => no_slot,
        OutcomeSide::No => yes_slot,
    };
    if !missing_slot.order_id.is_none()
        || missing_slot.last_reject_ts <= 0.0
        || !bot_runtime_origin_is_pair_build(&missing_slot.last_reject_origin)
    {
        return false;
    }
    let reject_age = (now - missing_slot.last_reject_ts).max(0.0);
    let effective_cooldown = maker_order_effective_reject_cooldown_seconds(
        &missing_slot.last_reject_origin,
        missing_slot,
        reject_cooldown,
        max_reject_cooldown,
    );
    let recent_reject_grace = bot_runtime_pair_build_broken_submit_recent_reject_grace_seconds();
    reject_age + 1e-9 >= effective_cooldown
        && reject_age <= effective_cooldown + recent_reject_grace + 1e-9
}

/// Implements pair build buy order is economically invalid for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_buy_order_is_economically_invalid(
    live_price: f64,
    intended_price: f64,
    tick_size: f64,
) -> bool {
    if !live_price.is_finite()
        || !intended_price.is_finite()
        || live_price <= 0.0
        || intended_price <= 0.0
    {
        return true;
    }
    let tick = tick_size.max(0.0001);
    ((live_price - intended_price).abs() / tick) >= (1.0 - 1e-6)
}

/// Implements lighter repair opposite order remaining for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_lighter_repair_opposite_order_remaining(
    slot: &MakerOrderSlot,
) -> f64 {
    match slot.state {
        MakerOrderLifecycle::SubmitPending => slot.remaining.max(slot.size).max(0.0),
        MakerOrderLifecycle::Working | MakerOrderLifecycle::CancelPending => {
            slot.remaining.max(0.0)
        }
        MakerOrderLifecycle::Idle => 0.0,
    }
}

/// Implements live lighter repair compatibility for the BOT runtime.
/// This is a pure pair-build helper used for policy and handler gating.

pub(in crate::bot) fn bot_runtime_live_lighter_repair_is_compatible(
    slot: &MakerOrderSlot,
    family_prefix: &str,
    target_price: f64,
    current_gap: f64,
    tick_size: f64,
) -> bool {
    if !maker_slot_family_live(slot, family_prefix) {
        return false;
    }
    if slot.state == MakerOrderLifecycle::CancelPending {
        return false;
    }
    let remaining = bot_runtime_lighter_repair_opposite_order_remaining(slot);
    let price_compatible = !bot_runtime_pair_build_buy_order_is_economically_invalid(
        slot.price,
        target_price,
        tick_size,
    );
    let size_compatible = remaining > 1e-9 && remaining <= current_gap.max(0.0) + 1e-9;
    price_compatible && size_compatible
}

/// Implements lighter repair opposite order policy for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_lighter_repair_opposite_order_policy(
    decision: &BotRuntimePairBuildDecision,
    inactive_slot: &MakerOrderSlot,
    inactive_target_price: f64,
    tick_size: f64,
) -> Option<BotRuntimeLighterOppositeOrderPolicy> {
    if decision.mode != BotRuntimePairBuildMode::LighterSideFirst
        || inactive_slot.order_id.is_none()
    {
        return None;
    }
    if !matches!(
        inactive_slot.state,
        MakerOrderLifecycle::Working
            | MakerOrderLifecycle::SubmitPending
            | MakerOrderLifecycle::CancelPending
    ) {
        return None;
    }
    let remaining = bot_runtime_lighter_repair_opposite_order_remaining(inactive_slot);
    let compatible_remaining = (decision.clip as f64 - decision.qty_gap.max(0.0)).max(0.0);
    let price_compatible = !bot_runtime_pair_build_buy_order_is_economically_invalid(
        inactive_slot.price,
        inactive_target_price,
        tick_size,
    );
    let size_compatible =
        compatible_remaining > 1e-9 && remaining > 1e-9 && remaining <= compatible_remaining + 1e-9;
    let (preserve, reason) = if inactive_slot.state == MakerOrderLifecycle::CancelPending {
        (false, "cancel_pending")
    } else if !price_compatible {
        (false, "price_incompatible")
    } else if !size_compatible {
        (false, "size_incompatible")
    } else {
        (true, "compatible")
    };
    Some(BotRuntimeLighterOppositeOrderPolicy {
        preserve,
        remaining,
        compatible_remaining,
        live_price: inactive_slot.price,
        target_price: inactive_target_price,
        reason,
    })
}

/// Implements pair build pair orders are economically invalid for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_pair_orders_are_economically_invalid(
    yes_live_price: f64,
    no_live_price: f64,
    yes_intended_price: f64,
    no_intended_price: f64,
    tick_size: f64,
) -> bool {
    if bot_runtime_pair_build_buy_order_is_economically_invalid(
        yes_live_price,
        yes_intended_price,
        tick_size,
    ) || bot_runtime_pair_build_buy_order_is_economically_invalid(
        no_live_price,
        no_intended_price,
        tick_size,
    ) {
        return true;
    }
    let live_pair_sum = yes_live_price + no_live_price;
    let intended_pair_sum = yes_intended_price + no_intended_price;
    if !live_pair_sum.is_finite() || !intended_pair_sum.is_finite() {
        return true;
    }
    let tick = tick_size.max(0.0001);
    ((live_pair_sum - intended_pair_sum).abs() / tick) >= (1.0 - 1e-6)
}

/// Implements pair build repost cooldown seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_repost_cooldown_seconds(
    replace_min_seconds: f64,
    decision: &BotRuntimePairBuildDecision,
) -> f64 {
    let base = replace_min_seconds.max(1.0);
    if bot_runtime_pair_build_has_large_clip_intent(decision) {
        base.max(1.5)
    } else {
        base
    }
}

/// Implements pair build price moved meaningfully for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_price_moved_meaningfully(
    previous_price: f64,
    target_price: f64,
    tick_size: f64,
) -> bool {
    if !previous_price.is_finite()
        || !target_price.is_finite()
        || previous_price <= 0.0
        || target_price <= 0.0
    {
        return true;
    }
    let tick = tick_size.max(0.0001);
    ((previous_price - target_price).abs() / tick) >= (1.0 - 1e-6)
}

/// Implements origin is pair build for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_origin_is_pair_build(origin: &str) -> bool {
    origin.trim().starts_with("BOT_PAIR_BUILD")
}

/// Implements imbalance hold growth order cancellation policy for the BOT runtime.
/// This is a pure pair-build helper used for policy and handler gating.

pub(in crate::bot) fn bot_runtime_imbalance_reason_requires_growth_order_cancel(
    reason: &str,
) -> bool {
    reason.starts_with("imbalance_")
        || reason.starts_with("projected_hard_imbalance_block")
        || reason.starts_with("hard_imbalance_disable")
        || reason.starts_with("repair_does_not_reduce_imbalance")
}

/// Implements price-zone hold growth order cancellation policy for the BOT runtime.
/// This is a pure pair-build helper used for policy and handler gating.

pub(in crate::bot) fn bot_runtime_price_zone_reason_requires_growth_order_cancel(
    reason: &str,
) -> bool {
    reason.starts_with("price_zone_stop_add:") || reason.starts_with("price_zone_danger:")
}

/// Implements rebalance price-zone hold repair cancellation policy for the BOT runtime.
/// This is a pure pair-build helper used for policy and handler gating.

pub(in crate::bot) fn bot_runtime_price_zone_reason_requires_lighter_repair_cancel(
    reason: &str,
) -> bool {
    reason.starts_with("price_zone_stop_add:rebalance_add:")
        || reason.starts_with("price_zone_danger:rebalance_add:")
}

/// Implements residual-direction hold reason for the BOT runtime.
/// This is a pure pair-build helper used for policy and handler gating.

pub(in crate::bot) fn bot_runtime_pair_build_residual_direction_hold_reason(
    decision: &BotRuntimePairBuildDecision,
) -> Option<String> {
    if decision.mode != BotRuntimePairBuildMode::LighterSideFirst {
        return None;
    }
    let side = decision.side.unwrap_or(OutcomeSide::Yes);
    if matches!(
        decision.one_side_exception_kind,
        BotRuntimeOneSideExceptionKind::None
    ) {
        return Some(format!("single_side_speculative_add:{}", side.as_str()));
    }
    if !decision.increases_underdog_residual {
        return None;
    }
    Some(format!(
        "underdog_residual_increase_block:{}:{}:{}",
        side.as_str(),
        decision
            .residual_side
            .map(|value| value.as_str())
            .unwrap_or("NONE"),
        decision
            .underdog_side
            .map(|value| value.as_str())
            .unwrap_or("NONE")
    ))
}

/// Implements residual-direction side cancellation policy for the BOT runtime.
/// This is a pure pair-build helper used for handler-side cleanup.

pub(in crate::bot) fn bot_runtime_residual_reason_cancel_side(reason: &str) -> Option<OutcomeSide> {
    let mut parts = reason.split(':');
    if parts.next() != Some("underdog_residual_increase_block") {
        return None;
    }
    match parts.next().map(|value| value.trim().to_ascii_uppercase()) {
        Some(side) if side == "YES" => Some(OutcomeSide::Yes),
        Some(side) if side == "NO" => Some(OutcomeSide::No),
        _ => None,
    }
}

/// Implements lighter-side repair preservation policy for imbalance holds in the BOT runtime.
/// This is a pure pair-build helper used for policy and handler gating.

pub(in crate::bot) fn bot_runtime_imbalance_reason_preserves_lighter_repair(reason: &str) -> bool {
    reason.contains("_repair_unavailable")
}

/// Implements await second fill bypasses open both reject cooldown for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_await_second_fill_bypasses_open_both_reject_cooldown(
    origin: &str,
    slot: &MakerOrderSlot,
) -> bool {
    origin.starts_with("BOT_AWAIT_SECOND_FILL")
        && slot.last_reject_ts > 0.0
        && slot.last_reject_origin.starts_with("BOT_OPEN_BOTH")
}

/// Implements pair build reject cooldown seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_reject_cooldown_seconds(
    origin: &str,
    slot: &MakerOrderSlot,
) -> Option<f64> {
    if !origin.starts_with("BOT_PAIR_BUILD")
        || slot.last_reject_ts <= 0.0
        || !slot.last_reject_origin.starts_with("BOT_PAIR_BUILD")
    {
        return None;
    }
    if origin.starts_with("BOT_PAIR_BUILD_LIGHTER")
        || slot
            .last_reject_origin
            .starts_with("BOT_PAIR_BUILD_LIGHTER")
    {
        Some(0.5)
    } else {
        Some(1.0)
    }
}

/// Implements order effective reject cooldown seconds for the maker-side BOT workflow.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn maker_order_effective_reject_cooldown_seconds(
    origin: &str,
    slot: &MakerOrderSlot,
    reject_cooldown: f64,
    max_reject_cooldown: f64,
) -> f64 {
    if reject_cooldown <= 0.0 || slot.last_reject_ts <= 0.0 {
        return 0.0;
    }
    if bot_runtime_await_second_fill_bypasses_open_both_reject_cooldown(origin, slot) {
        return 0.0;
    }
    let base_cooldown =
        bot_runtime_pair_build_reject_cooldown_seconds(origin, slot).unwrap_or(reject_cooldown);
    let max_cooldown = max_reject_cooldown.max(reject_cooldown);
    if slot.consecutive_rejects <= 1 {
        base_cooldown
    } else {
        (base_cooldown * 2.0_f64.powi((slot.consecutive_rejects - 1).min(6) as i32))
            .min(max_cooldown)
    }
}

/// Implements pair build CPP pace seconds for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_cpp_pace_seconds(
    decision: &BotRuntimePairBuildDecision,
    under_min_target: bool,
) -> Option<f64> {
    if under_min_target
        || decision.mode != BotRuntimePairBuildMode::PairedGrowth
        || !matches!(decision.imbalance_state, BotRuntimeImbalanceState::Normal)
    {
        return None;
    }
    match decision.cpp_hint {
        BotRuntimePairBuildCppHint::Normal => None,
        BotRuntimePairBuildCppHint::Medium => Some(1.0),
        BotRuntimePairBuildCppHint::Small => Some(2.0),
    }
}

/// Implements pair build materially skewed for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_materially_skewed(
    pair_coverage: f64,
    skew_ratio: f64,
    qty_gap: f64,
    min_lot: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> bool {
    let lot_repairable = qty_gap + 1e-9 >= min_lot.max(1.0);
    let repair_gap_threshold = cfg.clip_ladder[0].max(min_lot);
    lot_repairable
        && (pair_coverage <= 0.97 + 1e-9
            || skew_ratio >= 1.05 - 1e-9
            || qty_gap + 1e-9 >= repair_gap_threshold)
}
