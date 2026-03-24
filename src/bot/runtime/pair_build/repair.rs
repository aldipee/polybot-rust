use super::super::*;
use super::costs::{
    bot_runtime_pair_build_price_zone_hold_reason,
    bot_runtime_pair_build_projected_paired_cost_band,
    bot_runtime_pair_build_projected_repair_inventory_vwap_sum,
};
use super::decision::{
    bot_runtime_lighter_repair_opposite_order_policy,
    bot_runtime_pair_build_buy_order_is_economically_invalid,
    bot_runtime_pair_build_lighter_live_order_timeout_seconds, bot_runtime_repair_clip_choice,
};
use super::state::{BotRuntimePairBuildMarketContext, BotRuntimePairBuildPlan};

/// Implements pair build repair clip sizing for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_repair_clip_sizing(
    qty_gap: f64,
    side_price: f64,
    min_shares: f64,
    min_maker_notional: f64,
) -> Option<BotRuntimeRepairClipSizing> {
    if qty_gap <= 0.0 || !side_price.is_finite() || side_price <= 0.0 {
        return None;
    }
    let lot = min_shares.max(1.0).ceil();
    let exact_gap_clip = round_up_to_lot(qty_gap.max(0.0), lot) as i64;
    if exact_gap_clip <= 0 {
        return None;
    }
    let min_valid_share_clip = lot as i64;
    let min_valid_notional_clip = if min_maker_notional > 0.0 {
        round_up_to_lot(min_maker_notional / side_price, lot) as i64
    } else {
        min_valid_share_clip
    };
    let min_valid_clip = min_valid_share_clip.max(min_valid_notional_clip);
    Some(BotRuntimeRepairClipSizing {
        exact_gap_clip,
        min_valid_clip,
    })
}

/// Implements pair build executable repair clip for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_executable_repair_clip(
    sizing: BotRuntimeRepairClipSizing,
) -> i64 {
    sizing.exact_gap_clip.max(sizing.min_valid_clip)
}

/// Implements pair build exact gap repair is executable for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_exact_gap_repair_is_executable(
    qty_gap: f64,
    side_price: f64,
    min_shares: f64,
    min_maker_notional: f64,
) -> bool {
    bot_runtime_pair_build_repair_clip_sizing(qty_gap, side_price, min_shares, min_maker_notional)
        .map(|sizing| sizing.min_valid_clip <= sizing.exact_gap_clip)
        .unwrap_or(false)
}

/// Implements pair build lighter repair policy for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_lighter_repair_policy(
    decision: &BotRuntimePairBuildDecision,
    side_price: f64,
    remaining_budget: f64,
    min_shares: f64,
    min_maker_notional: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<BotRuntimeLighterRepairPolicy> {
    if decision.mode != BotRuntimePairBuildMode::LighterSideFirst {
        return None;
    }
    let sizing = bot_runtime_pair_build_repair_clip_sizing(
        decision.qty_gap,
        side_price,
        min_shares,
        min_maker_notional,
    )?;
    if sizing.min_valid_clip > sizing.exact_gap_clip {
        return Some(BotRuntimeLighterRepairPolicy {
            clip: 0,
            exact_gap_clip: sizing.exact_gap_clip,
            min_valid_clip: sizing.min_valid_clip,
            rounded_up_min_valid: true,
            clipped_to_budget: false,
            hold_reason: Some(format!(
                "lighter_side_min_valid_would_overshoot:{}:{}:{:.2}",
                sizing.exact_gap_clip, sizing.min_valid_clip, side_price
            )),
        });
    }
    let max_affordable_clip = (remaining_budget.max(0.0) / side_price).floor().max(0.0) as i64;
    if max_affordable_clip < sizing.min_valid_clip {
        return Some(BotRuntimeLighterRepairPolicy {
            clip: 0,
            exact_gap_clip: sizing.exact_gap_clip,
            min_valid_clip: sizing.min_valid_clip,
            rounded_up_min_valid: false,
            clipped_to_budget: true,
            hold_reason: Some(format!(
                "lighter_side_min_valid_repair_unaffordable:{}:{}:{:.2}",
                max_affordable_clip, sizing.min_valid_clip, side_price
            )),
        });
    }
    let clip = bot_runtime_repair_clip_choice(
        (decision.clip.max(0) as f64).min(max_affordable_clip as f64),
        decision.qty_gap,
        Some(sizing.exact_gap_clip as f64),
        Some(sizing.min_valid_clip as f64),
        cfg,
    )
    .map(|(clip, _)| clip as i64)
    .unwrap_or(0);
    if clip <= 0 {
        return Some(BotRuntimeLighterRepairPolicy {
            clip: 0,
            exact_gap_clip: sizing.exact_gap_clip,
            min_valid_clip: sizing.min_valid_clip,
            rounded_up_min_valid: false,
            clipped_to_budget: true,
            hold_reason: Some(format!(
                "lighter_side_repair_clip_unavailable:{}:{}:{:.2}",
                max_affordable_clip, sizing.exact_gap_clip, side_price
            )),
        });
    }
    Some(BotRuntimeLighterRepairPolicy {
        clip,
        exact_gap_clip: sizing.exact_gap_clip,
        min_valid_clip: sizing.min_valid_clip,
        rounded_up_min_valid: false,
        clipped_to_budget: clip < sizing.exact_gap_clip,
        hold_reason: None,
    })
}

/// Implements pair build repair reserve policy for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_repair_reserve_policy(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    y_bid: f64,
    n_bid: f64,
    remaining_budget: f64,
    min_shares: f64,
    min_maker_notional: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<BotRuntimeRepairReservePolicy> {
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth {
        return None;
    }
    if decision.clip <= 0 || !decision.pair_sum.is_finite() || decision.pair_sum <= 0.0 {
        return None;
    }
    let likely_repair_side = if q_yes + 1e-9 < q_no {
        OutcomeSide::Yes
    } else if q_no + 1e-9 < q_yes {
        OutcomeSide::No
    } else {
        return None;
    };
    let lighter_price = match likely_repair_side {
        OutcomeSide::Yes => y_bid,
        OutcomeSide::No => n_bid,
    };
    let sizing = bot_runtime_pair_build_repair_clip_sizing(
        decision.qty_gap,
        lighter_price,
        min_shares,
        min_maker_notional,
    )?;
    if sizing.min_valid_clip > sizing.exact_gap_clip {
        return None;
    }
    let executable_repair_clip = bot_runtime_repair_clip_choice(
        decision.qty_gap,
        decision.qty_gap,
        Some(sizing.exact_gap_clip as f64),
        Some(sizing.min_valid_clip as f64),
        cfg,
    )
    .map(|(clip, _)| clip as i64)?;
    let reserve_buffer_usd = cfg.repair_reserve_buffer_usd.max(0.0);
    let required_repair_cost = executable_repair_clip as f64 * lighter_price.max(0.0);
    let total_reserved_budget = required_repair_cost + reserve_buffer_usd;
    let min_lot = min_shares.max(1.0);
    let growth_budget_cap = (remaining_budget.max(0.0) - total_reserved_budget).max(0.0);
    let max_growth_clip = round_down_to_lot(growth_budget_cap / decision.pair_sum, min_lot);
    if max_growth_clip + 1e-9 < min_lot {
        return Some(BotRuntimeRepairReservePolicy {
            clip: 0,
            likely_repair_side,
            likely_repair_clip: executable_repair_clip,
            required_repair_cost,
            reserve_buffer_usd,
            total_reserved_budget,
            remaining_budget_after_clip: remaining_budget.max(0.0),
            clipped_for_reserve: true,
            hold_reason: Some(format!(
                "repair_reserve_block:{}:{:.2}:{:.2}:{}:{:.2}",
                likely_repair_side.as_str(),
                remaining_budget.max(0.0),
                total_reserved_budget,
                executable_repair_clip,
                reserve_buffer_usd
            )),
        });
    }
    let clipped_clip =
        round_down_to_lot((decision.clip as f64).min(max_growth_clip), min_lot) as i64;
    let remaining_budget_after_clip =
        (remaining_budget.max(0.0) - (clipped_clip as f64 * decision.pair_sum)).max(0.0);
    Some(BotRuntimeRepairReservePolicy {
        clip: clipped_clip,
        likely_repair_side,
        likely_repair_clip: executable_repair_clip,
        required_repair_cost,
        reserve_buffer_usd,
        total_reserved_budget,
        remaining_budget_after_clip,
        clipped_for_reserve: clipped_clip < decision.clip,
        hold_reason: None,
    })
}

/// Implements pair build lighter clip after cost quality for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_lighter_clip_after_cost_quality(
    requested_clip: f64,
    qty_gap: f64,
    min_lot: f64,
    cfg: &BotRuntimeConfigSnapshot,
    cpp_hint: BotRuntimePairBuildCppHint,
) -> f64 {
    let max_gap_clip = qty_gap.max(min_lot);
    let lighter_clip = requested_clip.min(max_gap_clip);
    match cpp_hint {
        BotRuntimePairBuildCppHint::Normal => lighter_clip,
        BotRuntimePairBuildCppHint::Medium => lighter_clip.min(cfg.clip_ladder[1].max(min_lot)),
        BotRuntimePairBuildCppHint::Small => lighter_clip.min(cfg.clip_ladder[0].max(min_lot)),
    }
}

/// Implements pair build lighter clip after projected cost for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_lighter_clip_after_projected_cost(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    side: OutcomeSide,
    price: f64,
    min_lot: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> f64 {
    if decision.mode != BotRuntimePairBuildMode::LighterSideFirst {
        return decision.clip.max(0) as f64;
    }
    let requested_clip = decision.clip.max(0) as f64;
    if requested_clip <= min_lot + 1e-9 || price <= 0.0 || !price.is_finite() {
        return requested_clip;
    }
    let projected_inventory_vwap_sum = bot_runtime_pair_build_projected_repair_inventory_vwap_sum(
        q_yes,
        q_no,
        cost_yes,
        cost_no,
        side,
        price,
        requested_clip,
    );
    if !projected_inventory_vwap_sum.is_finite() || projected_inventory_vwap_sum <= 1.01 + 1e-9 {
        return requested_clip;
    }
    if !decision.inventory_vwap_sum.is_finite() {
        return requested_clip;
    }
    if decision.inventory_vwap_sum.is_finite()
        && projected_inventory_vwap_sum + 1e-9 < decision.inventory_vwap_sum
    {
        return requested_clip;
    }
    let reduced_clip =
        round_down_to_lot(requested_clip.min(cfg.clip_ladder[0].max(min_lot)), min_lot);
    if reduced_clip + 1e-9 < requested_clip {
        reduced_clip
    } else {
        requested_clip
    }
}

/// Implements pair build lighter price discipline block for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_lighter_price_discipline_block(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    side: OutcomeSide,
    price: f64,
    tick_size: f64,
) -> Option<f64> {
    if decision.mode != BotRuntimePairBuildMode::LighterSideFirst
        || !matches!(
            decision.cpp_hint,
            BotRuntimePairBuildCppHint::Medium | BotRuntimePairBuildCppHint::Small
        )
    {
        return None;
    }
    let (side_qty, side_cost) = match side {
        OutcomeSide::Yes => (q_yes.max(0.0), cost_yes.max(0.0)),
        OutcomeSide::No => (q_no.max(0.0), cost_no.max(0.0)),
    };
    if side_qty <= 1e-9 || decision.clip <= 0 || price <= 0.0 || !price.is_finite() {
        return None;
    }
    let total_cost = cost_yes.max(0.0) + cost_no.max(0.0);
    let current_floor = bot_runtime_worst_case_settlement_floor(q_yes, q_no, total_cost);
    let (_, projected_floor) = bot_runtime_projected_tail_and_floor_after_decision(
        decision,
        q_yes,
        q_no,
        total_cost,
        if side == OutcomeSide::Yes { price } else { 0.0 },
        if side == OutcomeSide::No { price } else { 0.0 },
    )
    .unwrap_or((bot_runtime_tail_size(q_yes, q_no), current_floor));
    if projected_floor > current_floor + 1e-9 {
        return None;
    }
    let current_inventory_vwap_sum = inventory_vwap_sum(q_yes, q_no, cost_yes, cost_no);
    let projected_inventory_vwap_sum = bot_runtime_pair_build_projected_repair_inventory_vwap_sum(
        q_yes,
        q_no,
        cost_yes,
        cost_no,
        side,
        price,
        decision.clip as f64,
    );
    if !current_inventory_vwap_sum.is_finite() || !projected_inventory_vwap_sum.is_finite() {
        return None;
    }
    let current_side_avg = side_cost / side_qty;
    let tick = tick_size.max(0.0001);
    let payup_ticks = ((price - current_side_avg) / tick).max(0.0);
    let (max_worsening, max_payup_ticks) = match decision.cpp_hint {
        BotRuntimePairBuildCppHint::Medium => (0.015, 1.0),
        BotRuntimePairBuildCppHint::Small => (0.005, 0.0),
        BotRuntimePairBuildCppHint::Normal => return None,
    };
    let projected_worsens_cost =
        projected_inventory_vwap_sum > current_inventory_vwap_sum + max_worsening + 1e-9;
    if projected_worsens_cost && payup_ticks > max_payup_ticks + 1e-9 {
        Some(projected_inventory_vwap_sum)
    } else {
        None
    }
}

/// Implements pair build lighter extreme projected cost block for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_lighter_extreme_projected_cost_block(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    side: OutcomeSide,
    price: f64,
    tick_size: f64,
) -> Option<f64> {
    if decision.mode != BotRuntimePairBuildMode::LighterSideFirst {
        return None;
    }
    let (side_qty, side_cost) = match side {
        OutcomeSide::Yes => (q_yes.max(0.0), cost_yes.max(0.0)),
        OutcomeSide::No => (q_no.max(0.0), cost_no.max(0.0)),
    };
    if side_qty <= 1e-9 || decision.clip <= 0 || price <= 0.0 || !price.is_finite() {
        return None;
    }
    let projected_inventory_vwap_sum = bot_runtime_pair_build_projected_repair_inventory_vwap_sum(
        q_yes,
        q_no,
        cost_yes,
        cost_no,
        side,
        price,
        decision.clip as f64,
    );
    if !projected_inventory_vwap_sum.is_finite() {
        return None;
    }
    let total_cost = cost_yes.max(0.0) + cost_no.max(0.0);
    let current_floor = bot_runtime_worst_case_settlement_floor(q_yes, q_no, total_cost);
    let (_, projected_floor) = bot_runtime_projected_tail_and_floor_after_decision(
        decision,
        q_yes,
        q_no,
        total_cost,
        if side == OutcomeSide::Yes { price } else { 0.0 },
        if side == OutcomeSide::No { price } else { 0.0 },
    )
    .unwrap_or((bot_runtime_tail_size(q_yes, q_no), current_floor));
    if projected_floor > current_floor + 1e-9 {
        return None;
    }
    let current_side_avg = side_cost / side_qty;
    let tick = tick_size.max(0.0001);
    let payup_ticks = ((price - current_side_avg) / tick).max(0.0);
    let (projected_cap, max_payup_ticks) = match decision.clip_bucket {
        "large" => (1.02, 2.0),
        "medium" => (1.025, 2.0),
        _ => (1.03, 3.0),
    };
    if projected_inventory_vwap_sum > projected_cap + 1e-9 && payup_ticks > max_payup_ticks + 1e-9 {
        Some(projected_inventory_vwap_sum)
    } else {
        None
    }
}

/// Implements pair build lighter repair completion core block for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_lighter_repair_completion_core_block(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    side: OutcomeSide,
    price: f64,
) -> Option<(f64, BotRuntimePairedCostBand, f64)> {
    if decision.mode != BotRuntimePairBuildMode::LighterSideFirst
        || decision.clip <= 0
        || price <= 0.0
        || !price.is_finite()
    {
        return None;
    }
    let projected_inventory_vwap_sum = bot_runtime_pair_build_projected_repair_inventory_vwap_sum(
        q_yes,
        q_no,
        cost_yes,
        cost_no,
        side,
        price,
        decision.clip as f64,
    );
    if !projected_inventory_vwap_sum.is_finite() {
        return None;
    }
    let projected_band =
        bot_runtime_pair_build_projected_paired_cost_band(projected_inventory_vwap_sum);
    if !matches!(
        projected_band,
        BotRuntimePairedCostBand::StopAdd | BotRuntimePairedCostBand::Danger
    ) {
        return None;
    }
    let current_inventory_vwap_sum = inventory_vwap_sum(q_yes, q_no, cost_yes, cost_no);
    if !current_inventory_vwap_sum.is_finite() {
        return None;
    }
    if projected_inventory_vwap_sum + 1e-9 < current_inventory_vwap_sum {
        return None;
    }
    Some((
        projected_inventory_vwap_sum,
        projected_band,
        current_inventory_vwap_sum,
    ))
}

impl MakerHedgeCapBot {
    /// Implements pair build handle lighter side repair for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_handle_lighter_side_repair(
        &self,
        now: f64,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        _cost_yes: f64,
        _cost_no: f64,
        context: &BotRuntimePairBuildMarketContext,
        plan: &BotRuntimePairBuildPlan,
    ) {
        let decision = plan.decision;
        let lighter_live_timeout_s = bot_runtime_pair_build_lighter_live_order_timeout_seconds(
            self.cfg.stale_seconds as f64,
            &decision,
        );
        let price_tick = self.cfg.tick.max(0.0001);
        let active_side = decision.side.unwrap_or(OutcomeSide::Yes);
        let inactive_side = match active_side {
            OutcomeSide::Yes => OutcomeSide::No,
            OutcomeSide::No => OutcomeSide::Yes,
        };
        let (inactive_bid, inactive_key) = match inactive_side {
            OutcomeSide::Yes => (context.y_bid, &context.yes_key),
            OutcomeSide::No => (context.n_bid, &context.no_key),
        };
        let inactive_slot = match inactive_side {
            OutcomeSide::Yes => &context.yes_slot,
            OutcomeSide::No => &context.no_slot,
        };
        if maker_slot_family_live(inactive_slot, "BOT_PAIR_BUILD") {
            let inactive_age_s = (now - inactive_slot.last_submit_ts).max(0.0);
            let ownership_policy = bot_runtime_lighter_repair_opposite_order_policy(
                &decision,
                inactive_slot,
                inactive_bid,
                price_tick,
            );
            if ownership_policy
                .as_ref()
                .map(|policy| policy.preserve)
                .unwrap_or(false)
            {
                if let Some(policy) = ownership_policy.as_ref() {
                    self._bot_runtime_log_lighter_repair_ownership(
                        "PAIR_BUILD",
                        active_side,
                        inactive_side,
                        inactive_age_s,
                        policy,
                    );
                }
            } else {
                if let Some(policy) = ownership_policy.as_ref() {
                    self._bot_runtime_log_lighter_repair_ownership(
                        "PAIR_BUILD",
                        active_side,
                        inactive_side,
                        inactive_age_s,
                        policy,
                    );
                }
                if inactive_slot.state != MakerOrderLifecycle::CancelPending {
                    let _ = self._maker_order_request_cancel_unthrottled(
                        inactive_key,
                        "bot_runtime_pair_build_lighter_side_owner",
                    );
                    self._bot_runtime_pair_build_note_side_cancel(
                        inactive_side,
                        inactive_slot.price,
                        now,
                    );
                }
                self._bot_runtime_log_pair_build_state(
                    "rest",
                    &format!(
                        "lighter_side_handoff:{}:{:.1}",
                        inactive_side.as_str(),
                        inactive_age_s
                    ),
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        }

        let (active_bid, key, prev_slot) = match active_side {
            OutcomeSide::Yes => (context.y_bid, &context.yes_key, &context.yes_slot),
            OutcomeSide::No => (context.n_bid, &context.no_key, &context.no_slot),
        };
        if maker_slot_family_live(prev_slot, "BOT_PAIR_BUILD") {
            let age_s = (now - prev_slot.last_submit_ts).max(0.0);
            let economically_invalid = bot_runtime_pair_build_buy_order_is_economically_invalid(
                prev_slot.price,
                active_bid,
                price_tick,
            ) || prev_slot.remaining > decision.clip as f64 + 1e-9;
            if age_s >= lighter_live_timeout_s
                && economically_invalid
                && prev_slot.state != MakerOrderLifecycle::CancelPending
            {
                match self._maker_order_request_refresh_cancel(
                    key,
                    "bot_runtime_pair_build_lighter_side_invalid",
                ) {
                    Ok(true) => {
                        self._bot_runtime_pair_build_note_side_cancel(
                            active_side,
                            prev_slot.price,
                            now,
                        );
                    }
                    Err(reason) => {
                        self._bot_runtime_log_pair_build_state(
                            "hold",
                            &reason,
                            Some(decision),
                            t_into_s,
                            total_cost,
                            q_yes,
                            q_no,
                        );
                        return;
                    }
                    Ok(false) => {}
                }
                self._bot_runtime_log_pair_build_state(
                    "rest",
                    &format!("lighter_side_live_order_invalid_cancel:{age_s:.1}"),
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            } else {
                self._bot_runtime_log_pair_build_state(
                    "rest",
                    &format!(
                        "awaiting_lighter_side_live_order:{}:{age_s:.1}",
                        maker_order_lifecycle_label(prev_slot.state)
                    ),
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
            }
            return;
        }

        if let Some(reason) = self._bot_runtime_pair_build_repost_block_reason(
            active_side,
            active_bid,
            now,
            price_tick,
            &decision,
        ) {
            self._bot_runtime_log_pair_build_state(
                "hold",
                &reason,
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if let Some(policy) = plan.lighter_repair_policy.as_ref() {
            if let Some(reason) = policy.hold_reason.as_deref() {
                self._bot_runtime_log_pair_build_state(
                    "hold",
                    reason,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        }
        if let Some(reason) = bot_runtime_pair_build_price_zone_hold_reason(
            decision.price_zone,
            decision.marginal_cost_mode,
            decision.effective_marginal_pair_cost,
        ) {
            // During HardDisable, allow lighter-side repair regardless of price zone;
            // the bid cap below still prevents overpaying.
            // Use the sticky runtime state (not the raw fraction-based state in the decision)
            // so this also covers the hysteresis band where fraction is between recovery
            // and disable thresholds.
            let runtime_hard_disable = self
                .bot_runtime_state
                .lock()
                .map(|st| matches!(st.imbalance_state, BotRuntimeImbalanceState::HardDisable))
                .unwrap_or(false);
            if !runtime_hard_disable {
                self._bot_runtime_log_pair_build_state(
                    "hold",
                    &reason,
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        }
        let (repair_bid_capped, repair_original_bid) = {
            let (pg_y_bid, pg_n_bid) = self
                .bot_runtime_state
                .lock()
                .map(|st| {
                    (
                        st.pair_build_last_paired_growth_yes_bid,
                        st.pair_build_last_paired_growth_no_bid,
                    )
                })
                .unwrap_or((0.0, 0.0));
            let filled_side_price = match active_side {
                OutcomeSide::Yes => pg_n_bid,
                OutcomeSide::No => pg_y_bid,
            };
            if filled_side_price > 0.0 && pg_y_bid > 0.0 && pg_n_bid > 0.0 {
                let target_pair_sum = pg_y_bid + pg_n_bid;
                let pair_economics_cap = target_pair_sum - filled_side_price;
                let tick = self.cfg.tick.max(0.0001);
                let max_adverse_spread = (tick * 3.0).max(0.03);
                let spread_floor = active_bid - max_adverse_spread;
                let effective_cap = pair_economics_cap.max(spread_floor);
                if effective_cap > 0.0 && effective_cap + 1e-9 < active_bid {
                    let ticked = (effective_cap / tick).floor() * tick;
                    if ticked > 0.0 {
                        (ticked, active_bid)
                    } else {
                        (active_bid, active_bid)
                    }
                } else {
                    (active_bid, active_bid)
                }
            } else {
                (active_bid, active_bid)
            }
        };

        self._set_pending_entry_reason("BOT_PAIR_BUILD");
        let oid = self._maker_order_upsert_gtc(
            key,
            repair_bid_capped,
            decision.clip as f64,
            "BOT_PAIR_BUILD_LIGHTER",
        );
        if let Some(order_id) = oid.as_deref() {
            let refresh_noop = self._consume_refresh_cadence_noop_marker(order_id);
            let is_new_submit = !refresh_noop
                && (prev_slot.order_id.as_deref() != Some(order_id)
                    || prev_slot.state != MakerOrderLifecycle::Working);
            if is_new_submit {
                if let Some(decision_event_id) = self._audit_insert_decision_event(
                    "pair_build",
                    Some(&decision),
                    true,
                    "pair_build_lighter_submit",
                    Some("BOT_PAIR_BUILD_LIGHTER"),
                    Some(active_side.as_str()),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                ) {
                    self._audit_attach_decision_context(
                        order_id,
                        decision_event_id.as_str(),
                        "pair_build_lighter_submit",
                    );
                    self._merge_order_execution_context_fields(
                        order_id,
                        &json!({
                            "submit_origin": "BOT_PAIR_BUILD_LIGHTER",
                            "submit_side": active_side.as_str(),
                        }),
                    );
                }
                self._bot_runtime_pair_build_note_side_submit(active_side, repair_bid_capped, now);
                self._bot_runtime_clear_pair_build_hold();
                let exact_gap_clip = plan
                    .lighter_repair_policy
                    .as_ref()
                    .map(|policy| policy.exact_gap_clip)
                    .unwrap_or(decision.qty_gap.max(0.0).ceil() as i64);
                let min_valid_clip = plan
                    .lighter_repair_policy
                    .as_ref()
                    .map(|policy| policy.min_valid_clip)
                    .unwrap_or(self.cfg.min_shares.max(1.0).ceil() as i64);
                let rounded_up_min_valid = plan
                    .lighter_repair_policy
                    .as_ref()
                    .map(|policy| policy.rounded_up_min_valid)
                    .unwrap_or(false);
                let clipped_to_budget = plan
                    .lighter_repair_policy
                    .as_ref()
                    .map(|policy| policy.clipped_to_budget)
                    .unwrap_or(false);
                let bid_cap_applied = (repair_original_bid - repair_bid_capped).abs() > 1e-9;
                self.logger.info(&format!(
                    "[BOT][PAIR_BUILD] submit mode={} side={} clip={} clip_bucket={} selected_rung={} requested_rung={} requested_large_clip={} requested_clip={:.0} cpp_hint={} price_zone={} marginal_cost_mode={} effective_marginal_pair_cost={:.3} residual_unit_cost={} lagging_side_quote={} heavier_side={} favorite_side={} underdog_side={} residual_side={} projected_residual_side={} residual_kind={} one_side_exception_kind={} increases_underdog_residual={} exact_gap_clip={} min_valid_repair_clip={} rounded_up_min_valid={} clipped_to_budget={} bid_cap_applied={} bid={:.3} original_bid={:.3} green_conditions_met={} green_both_sides_filled={} green_price_ok={} green_imbalance_ok={} green_time_ok={} green_budget_ok={} t_into={:.1}s qYES={:.2} qNO={:.2} total_cost={:.2} qty_gap={:.2} unmatched_fraction={:.3} projected_unmatched_fraction={:.3} match_ratio={:.3} imbalance_state={} reduces_imbalance={} pair_coverage={:.3} skew={:.3} inventory_vwap_sum={:.3} market_snapshot_vwap_sum={:.3}",
                    decision.mode.as_str(),
                    active_side.as_str(),
                    decision.clip,
                    decision.clip_bucket,
                    decision.selected_rung.as_str(),
                    decision.requested_rung.as_str(),
                    decision.requested_large_clip,
                    decision.requested_clip,
                    decision.cpp_hint.as_str(),
                    decision.price_zone.as_str(),
                    decision.marginal_cost_mode.as_str(),
                    decision.effective_marginal_pair_cost,
                    decision
                        .residual_unit_cost
                        .map(|value| format!("{value:.3}"))
                        .unwrap_or_else(|| "NA".to_string()),
                    decision
                        .lagging_side_quote
                        .map(|value| format!("{value:.3}"))
                        .unwrap_or_else(|| "NA".to_string()),
                    active_side.opposite().as_str(),
                    decision
                        .favorite_side
                        .map(|value| value.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    decision
                        .underdog_side
                        .map(|value| value.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    decision
                        .residual_side
                        .map(|value| value.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    decision
                        .projected_residual_side
                        .map(|value| value.as_str().to_string())
                        .unwrap_or_else(|| "NA".to_string()),
                    decision.residual_kind.as_str(),
                    decision.one_side_exception_kind.as_str(),
                    decision.increases_underdog_residual,
                    exact_gap_clip,
                    min_valid_clip,
                    rounded_up_min_valid,
                    clipped_to_budget,
                    bid_cap_applied,
                    repair_bid_capped,
                    repair_original_bid,
                    decision.green_conditions_met,
                    decision.green_both_sides_filled,
                    decision.green_price_ok,
                    decision.green_imbalance_ok,
                    decision.green_time_ok,
                    decision.green_budget_ok,
                    t_into_s.max(0.0),
                    q_yes,
                    q_no,
                    total_cost.max(0.0),
                    decision.qty_gap,
                    decision.current_unmatched_fraction,
                    decision.projected_unmatched_fraction,
                    decision.match_ratio,
                    decision.imbalance_state.as_str(),
                    decision.reduces_imbalance,
                    decision.pair_coverage,
                    decision.skew_ratio,
                    decision.inventory_vwap_sum,
                    decision.market_snapshot_vwap_sum
                ));
            }
        } else {
            self._bot_runtime_log_pair_build_state(
                "hold",
                "no_lighter_side_order_live",
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
        }
    }
}
