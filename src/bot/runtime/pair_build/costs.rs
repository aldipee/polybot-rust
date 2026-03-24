use super::super::*;

/// Implements balanced add effective marginal pair cost for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_balanced_add_effective_marginal_cost(
    y_bid: f64,
    n_bid: f64,
) -> f64 {
    if !y_bid.is_finite() || !n_bid.is_finite() || y_bid <= 0.0 || n_bid <= 0.0 {
        f64::INFINITY
    } else {
        y_bid + n_bid
    }
}

/// Implements rebalance add residual unit cost for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_rebalance_residual_unit_cost(
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    lagging_side: OutcomeSide,
) -> Option<f64> {
    let (heavy_qty, heavy_cost) = match lagging_side {
        OutcomeSide::Yes => (q_no.max(0.0), cost_no.max(0.0)),
        OutcomeSide::No => (q_yes.max(0.0), cost_yes.max(0.0)),
    };
    if heavy_qty <= 1e-9 || !heavy_qty.is_finite() || !heavy_cost.is_finite() {
        None
    } else {
        Some(heavy_cost / heavy_qty)
    }
}

/// Implements rebalance add effective marginal pair cost for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_rebalance_effective_marginal_cost(
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    lagging_side: OutcomeSide,
    lagging_side_bid: f64,
) -> (f64, Option<f64>) {
    if !lagging_side_bid.is_finite() || lagging_side_bid <= 0.0 {
        return (f64::INFINITY, None);
    }
    let Some(residual_unit_cost) = bot_runtime_pair_build_rebalance_residual_unit_cost(
        q_yes,
        q_no,
        cost_yes,
        cost_no,
        lagging_side,
    ) else {
        return (f64::INFINITY, None);
    };
    (
        residual_unit_cost + lagging_side_bid,
        Some(residual_unit_cost),
    )
}

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
    if !projected_paired_cost.is_finite() || projected_paired_cost >= 1.03 - 1e-9 {
        BotRuntimePairedCostBand::Danger
    } else if projected_paired_cost >= 1.00 - 1e-9 {
        BotRuntimePairedCostBand::StopAdd
    } else if projected_paired_cost >= 0.97 - 1e-9 {
        BotRuntimePairedCostBand::Caution
    } else if projected_paired_cost >= 0.94 - 1e-9 {
        BotRuntimePairedCostBand::Acceptable
    } else {
        BotRuntimePairedCostBand::Preferred
    }
}

/// Implements repair-specific projected paired cost band with configurable thresholds.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_repair_cost_band(
    projected_paired_cost: f64,
    danger_threshold: f64,
    stop_add_threshold: f64,
) -> BotRuntimePairedCostBand {
    if !projected_paired_cost.is_finite() || projected_paired_cost >= danger_threshold - 1e-9 {
        BotRuntimePairedCostBand::Danger
    } else if projected_paired_cost >= stop_add_threshold - 1e-9 {
        BotRuntimePairedCostBand::StopAdd
    } else if projected_paired_cost >= 0.97 - 1e-9 {
        BotRuntimePairedCostBand::Caution
    } else if projected_paired_cost >= 0.94 - 1e-9 {
        BotRuntimePairedCostBand::Acceptable
    } else {
        BotRuntimePairedCostBand::Preferred
    }
}

/// Implements price-zone hold reason for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_price_zone_hold_reason(
    band: BotRuntimePairedCostBand,
    mode: BotRuntimeMarginalCostMode,
    cost: f64,
) -> Option<String> {
    match band {
        BotRuntimePairedCostBand::StopAdd => {
            Some(format!("price_zone_stop_add:{}:{cost:.3}", mode.as_str()))
        }
        BotRuntimePairedCostBand::Danger => {
            Some(format!("price_zone_danger:{}:{cost:.3}", mode.as_str()))
        }
        _ => None,
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
    let _ = (q_yes, q_no, cost_yes, cost_no, y_bid, n_bid);
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth || decision.clip <= 0 {
        return None;
    }
    Some((decision.effective_marginal_pair_cost, decision.price_zone))
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
