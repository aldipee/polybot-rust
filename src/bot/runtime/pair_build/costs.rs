use super::super::*;

/// Implements pair build projected inventory VWAP sum for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_projected_inventory_vwap_sum(
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    y_bid: f64,
    n_bid: f64,
    clip: f64,
) -> f64 {
    let clip = clip.max(0.0);
    inventory_vwap_sum(
        q_yes.max(0.0) + clip,
        q_no.max(0.0) + clip,
        cost_yes.max(0.0) + (clip * y_bid.max(0.0)),
        cost_no.max(0.0) + (clip * n_bid.max(0.0)),
    )
}

/// Implements pair build projected paired cost band for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_projected_paired_cost_band(
    projected_paired_cost: f64,
) -> BotRuntimePairedCostBand {
    if !projected_paired_cost.is_finite() || projected_paired_cost > 1.02 + 1e-9 {
        BotRuntimePairedCostBand::Freeze
    } else if projected_paired_cost > 1.00 + 1e-9 {
        BotRuntimePairedCostBand::RepairOnly
    } else if projected_paired_cost >= 0.98 - 1e-9 {
        BotRuntimePairedCostBand::ReducedGrowth
    } else if projected_paired_cost >= 0.94 - 1e-9 {
        BotRuntimePairedCostBand::NormalGrowth
    } else {
        BotRuntimePairedCostBand::StrongGrowth
    }
}

/// Implements pair build projected paired cost snapshot for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_projected_paired_cost_snapshot(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    y_bid: f64,
    n_bid: f64,
) -> Option<(f64, BotRuntimePairedCostBand)> {
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth || decision.clip <= 0 {
        return None;
    }
    let projected_paired_cost = bot_runtime_pair_build_projected_inventory_vwap_sum(
        q_yes,
        q_no,
        cost_yes,
        cost_no,
        y_bid,
        n_bid,
        decision.clip.max(0) as f64,
    );
    Some((
        projected_paired_cost,
        bot_runtime_pair_build_projected_paired_cost_band(projected_paired_cost),
    ))
}

/// Implements pair build projected repair inventory VWAP sum for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_projected_repair_inventory_vwap_sum(
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    side: OutcomeSide,
    price: f64,
    clip: f64,
) -> f64 {
    let clip = clip.max(0.0);
    match side {
        OutcomeSide::Yes => inventory_vwap_sum(
            q_yes.max(0.0) + clip,
            q_no.max(0.0),
            cost_yes.max(0.0) + clip * price.max(0.0),
            cost_no.max(0.0),
        ),
        OutcomeSide::No => inventory_vwap_sum(
            q_yes.max(0.0),
            q_no.max(0.0) + clip,
            cost_yes.max(0.0),
            cost_no.max(0.0) + clip * price.max(0.0),
        ),
    }
}

/// Implements post only order meets min maker notional for the maker-side BOT workflow.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn maker_post_only_order_meets_min_maker_notional(
    side: ClobSide,
    price: f64,
    size: f64,
    post_only: bool,
    min_maker_notional: f64,
) -> bool {
    if !post_only || !matches!(side, ClobSide::Buy | ClobSide::Sell) {
        return true;
    }
    if price <= 0.0 || size <= 0.0 {
        return false;
    }
    price * size + 1e-9 >= min_maker_notional.max(0.0)
}
