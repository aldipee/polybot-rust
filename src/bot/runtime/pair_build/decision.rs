use super::super::*;

/// Implements pair build clip bucket for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_clip_bucket(
    clip: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> &'static str {
    if clip + 1e-9 >= cfg.large_clip_ladder[1].max(cfg.large_clip_ladder[0]) {
        "large"
    } else if clip + 1e-9 >= cfg.large_clip_ladder[0].max(cfg.seed_clip_small) {
        "medium"
    } else {
        "small"
    }
}

/// Implements pair build decision for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_decision(
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
    let pair_sum = y_bid + n_bid;
    let pair_coverage = pair_coverage(q_yes, q_no);
    let skew_ratio = share_skew_ratio(q_yes, q_no);
    let qty_gap = (q_yes.max(0.0) - q_no.max(0.0)).abs();
    let inventory_vwap_sum = inventory_vwap_sum(q_yes, q_no, cost_yes, cost_no);
    let market_snapshot_vwap_sum = market_snapshot_vwap_sum(y_bid, y_ask, n_bid, n_ask);

    let requested_clip = if current_base < (2.0 * cfg.seed_clip_small).max(min_lot) {
        cfg.seed_clip_small.max(min_lot)
    } else if under_min_target {
        cfg.large_clip_ladder[1]
            .max(cfg.large_clip_ladder[0])
            .max(min_lot)
    } else if current_base < cfg.large_clip_ladder[0].max(min_lot) {
        cfg.large_clip_ladder[0].max(min_lot)
    } else {
        cfg.large_clip_ladder[1]
            .max(cfg.large_clip_ladder[0])
            .max(min_lot)
    };

    let medium_clip_cap = cfg.large_clip_ladder[0]
        .max(cfg.seed_clip_small)
        .max(min_lot);
    let small_clip_cap = cfg.repair_clip_small.max(cfg.seed_clip_small).max(min_lot);
    let cpp_hint = if !inventory_vwap_sum.is_finite() || !market_snapshot_vwap_sum.is_finite() {
        BotRuntimePairBuildCppHint::Small
    } else {
        let medium_threshold = (market_snapshot_vwap_sum + 0.02).max(0.99);
        let small_threshold = (market_snapshot_vwap_sum + 0.05).max(1.02);
        if inventory_vwap_sum > small_threshold {
            BotRuntimePairBuildCppHint::Small
        } else if inventory_vwap_sum > medium_threshold {
            BotRuntimePairBuildCppHint::Medium
        } else {
            BotRuntimePairBuildCppHint::Normal
        }
    };

    let clip_after_cpp_hint = match cpp_hint {
        BotRuntimePairBuildCppHint::Normal => requested_clip,
        BotRuntimePairBuildCppHint::Medium => requested_clip.min(medium_clip_cap),
        BotRuntimePairBuildCppHint::Small => {
            if under_min_target {
                requested_clip.min(medium_clip_cap)
            } else {
                requested_clip.min(small_clip_cap)
            }
        }
    };

    let lighter_side = if q_yes + 1e-9 < q_no {
        Some(OutcomeSide::Yes)
    } else if q_no + 1e-9 < q_yes {
        Some(OutcomeSide::No)
    } else {
        None
    };

    let lighter_side_first = lighter_side.is_some()
        && bot_runtime_pair_build_materially_skewed(
            pair_coverage,
            skew_ratio,
            qty_gap,
            min_lot,
            cfg,
        );
    if lighter_side_first {
        let side = lighter_side.unwrap_or(OutcomeSide::Yes);
        let side_bid = match side {
            OutcomeSide::Yes => y_bid,
            OutcomeSide::No => n_bid,
        };
        let exact_gap_repair_executable = bot_runtime_pair_build_exact_gap_repair_is_executable(
            qty_gap,
            side_bid,
            min_lot,
            min_maker_notional,
        );
        if side_bid > 0.0 && side_bid.is_finite() && exact_gap_repair_executable {
            let budget_clip_cap = (remaining_budget / side_bid).floor();
            let lighter_clip_after_cost_quality =
                bot_runtime_pair_build_lighter_clip_after_cost_quality(
                    clip_after_cpp_hint,
                    qty_gap,
                    min_lot,
                    cfg,
                    cpp_hint,
                );
            let clip = round_down_to_lot(
                lighter_clip_after_cost_quality.min(budget_clip_cap),
                min_lot,
            );
            if clip + 1e-9 >= min_lot {
                return Ok(BotRuntimePairBuildDecision {
                    mode: BotRuntimePairBuildMode::LighterSideFirst,
                    side: Some(side),
                    clip: clip as i64,
                    requested_clip,
                    clip_bucket: bot_runtime_pair_build_clip_bucket(clip, cfg),
                    cpp_hint,
                    pair_sum,
                    pair_coverage,
                    skew_ratio,
                    current_base,
                    qty_gap,
                    inventory_vwap_sum,
                    market_snapshot_vwap_sum,
                });
            }
        }
    }

    if pair_sum <= 0.0 || !pair_sum.is_finite() {
        return Err("pair_sum_unusable".to_string());
    }
    if pair_sum >= 1.0 {
        return Err(format!("pair_sum_too_high({pair_sum:.3})"));
    }

    let pair_budget_clip_cap = (remaining_budget / pair_sum).floor();
    let clip = round_down_to_lot(clip_after_cpp_hint.min(pair_budget_clip_cap), min_lot);
    if clip + 1e-9 < min_lot {
        return Err("budget_too_small".to_string());
    }

    Ok(BotRuntimePairBuildDecision {
        mode: BotRuntimePairBuildMode::PairedGrowth,
        side: None,
        clip: clip as i64,
        requested_clip,
        clip_bucket: bot_runtime_pair_build_clip_bucket(clip, cfg),
        cpp_hint,
        pair_sum,
        pair_coverage,
        skew_ratio,
        current_base,
        qty_gap,
        inventory_vwap_sum,
        market_snapshot_vwap_sum,
    })
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
    if decision.clip_bucket == "large" || decision.requested_clip >= 10.0 {
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
    if decision.clip_bucket == "large" || decision.requested_clip >= 10.0 {
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
        return if decision.clip_bucket == "large" || decision.requested_clip >= 10.0 {
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
    if decision.clip_bucket == "large" || decision.requested_clip >= 10.0 {
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
    if decision.clip_bucket == "large" || decision.requested_clip >= 10.0 {
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
        || decision.pair_coverage < 0.90
    {
        return None;
    }
    match decision.cpp_hint {
        BotRuntimePairBuildCppHint::Normal => None,
        BotRuntimePairBuildCppHint::Medium => Some(3.0),
        BotRuntimePairBuildCppHint::Small => Some(6.0),
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
    let repair_gap_threshold = cfg.repair_clip_small.max(min_lot);
    lot_repairable
        && (pair_coverage <= 0.97 + 1e-9
            || skew_ratio >= 1.05 - 1e-9
            || qty_gap + 1e-9 >= repair_gap_threshold)
}
