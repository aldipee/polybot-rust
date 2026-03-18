use super::super::*;
use super::costs::{
    bot_runtime_pair_build_projected_inventory_vwap_sum,
    bot_runtime_pair_build_projected_paired_cost_band,
};
use super::decision::bot_runtime_pair_build_cpp_pace_seconds;
use super::repair::bot_runtime_pair_build_exact_gap_repair_is_executable;
use super::state::{BotRuntimePairBuildMarketContext, BotRuntimePairBuildPlan};

/// Implements pair build optional growth policy for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_optional_growth_policy(
    decision: &BotRuntimePairBuildDecision,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    y_bid: f64,
    n_bid: f64,
    min_shares: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<BotRuntimePairedGrowthPolicy> {
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth {
        return None;
    }
    let current_clip = decision.clip.max(0) as f64;
    if current_clip <= 0.0 {
        return None;
    }
    let current_projected_paired_cost = bot_runtime_pair_build_projected_inventory_vwap_sum(
        q_yes,
        q_no,
        cost_yes,
        cost_no,
        y_bid,
        n_bid,
        current_clip,
    );
    let current_band =
        bot_runtime_pair_build_projected_paired_cost_band(current_projected_paired_cost);
    match current_band {
        BotRuntimePairedCostBand::StrongGrowth | BotRuntimePairedCostBand::NormalGrowth => {
            Some(BotRuntimePairedGrowthPolicy {
                clip: decision.clip.max(0),
                projected_paired_cost: current_projected_paired_cost,
                band: current_band,
                clipped_for_band: false,
                allowed_averaging_down: false,
            })
        }
        BotRuntimePairedCostBand::ReducedGrowth => {
            let maintenance_clip_cap = round_down_to_lot(
                cfg.repair_clip_small
                    .max(cfg.seed_clip_small)
                    .max(min_shares.max(1.0)),
                min_shares.max(1.0),
            );
            let reduced_clip =
                round_down_to_lot(current_clip.min(maintenance_clip_cap), min_shares.max(1.0));
            if reduced_clip + 1e-9 < min_shares.max(1.0) || reduced_clip + 1e-9 >= current_clip {
                return Some(BotRuntimePairedGrowthPolicy {
                    clip: decision.clip.max(0),
                    projected_paired_cost: current_projected_paired_cost,
                    band: current_band,
                    clipped_for_band: false,
                    allowed_averaging_down: false,
                });
            }
            let reduced_projected_paired_cost = bot_runtime_pair_build_projected_inventory_vwap_sum(
                q_yes,
                q_no,
                cost_yes,
                cost_no,
                y_bid,
                n_bid,
                reduced_clip,
            );
            Some(BotRuntimePairedGrowthPolicy {
                clip: reduced_clip as i64,
                projected_paired_cost: reduced_projected_paired_cost,
                band: bot_runtime_pair_build_projected_paired_cost_band(
                    reduced_projected_paired_cost,
                ),
                clipped_for_band: true,
                allowed_averaging_down: false,
            })
        }
        BotRuntimePairedCostBand::RepairOnly | BotRuntimePairedCostBand::Freeze => {
            Some(BotRuntimePairedGrowthPolicy {
                clip: decision.clip.max(0),
                projected_paired_cost: current_projected_paired_cost,
                band: current_band,
                clipped_for_band: false,
                allowed_averaging_down: false,
            })
        }
    }
}

/// Implements pair build optional buy policy for the BOT runtime.
/// This is a pure pair-build helper used for BOT runtime policy, math, and decision boundaries.

pub(in crate::bot) fn bot_runtime_pair_build_optional_buy_policy(
    decision: &BotRuntimePairBuildDecision,
    y_bid: f64,
    y_ask: f64,
    n_bid: f64,
    n_ask: f64,
    projected_band: BotRuntimePairedCostBand,
    min_shares: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> Option<BotRuntimeOptionalBuyPolicy> {
    if decision.mode != BotRuntimePairBuildMode::PairedGrowth {
        return None;
    }
    let current_clip = decision.clip.max(0) as f64;
    if current_clip <= 0.0 {
        return None;
    }
    let (yes_snapshot_price, yes_snapshot_source) =
        market_snapshot_price(y_bid, y_ask, n_bid, n_ask)?;
    let (no_snapshot_price, no_snapshot_source) =
        market_snapshot_price(n_bid, n_ask, y_bid, y_ask)?;
    let yes_snapshot_edge = yes_snapshot_price - y_bid.max(0.0);
    let no_snapshot_edge = no_snapshot_price - n_bid.max(0.0);
    let min_snapshot_edge = yes_snapshot_edge.min(no_snapshot_edge);
    let snapshot_reliable = !matches!(
        (yes_snapshot_source, no_snapshot_source),
        (SnapshotPricingSource::FairPriceFallback, _)
            | (_, SnapshotPricingSource::FairPriceFallback)
    );
    let hold_reason = if y_bid + 1e-9 >= yes_snapshot_price {
        Some(format!(
            "optional_buy_not_below_snapshot:YES:{:.3}:{:.3}:{}",
            y_bid,
            yes_snapshot_price,
            yes_snapshot_source.as_str()
        ))
    } else if n_bid + 1e-9 >= no_snapshot_price {
        Some(format!(
            "optional_buy_not_below_snapshot:NO:{:.3}:{:.3}:{}",
            n_bid,
            no_snapshot_price,
            no_snapshot_source.as_str()
        ))
    } else if !matches!(
        projected_band,
        BotRuntimePairedCostBand::StrongGrowth | BotRuntimePairedCostBand::NormalGrowth
    ) {
        Some(format!(
            "optional_buy_requires_cheap_core:{}:{:.3}",
            projected_band.as_str(),
            decision.pair_sum
        ))
    } else {
        None
    };
    let lot = min_shares.max(1.0);
    let small_clip_cap =
        round_down_to_lot(cfg.repair_clip_small.max(cfg.seed_clip_small).max(lot), lot);
    let reduced_clip = round_down_to_lot(current_clip.min(small_clip_cap), lot);
    let weak_edge_reduced = hold_reason.is_none()
        && (!snapshot_reliable || min_snapshot_edge + 1e-9 < 0.05)
        && reduced_clip + 1e-9 >= lot
        && reduced_clip + 1e-9 < current_clip;
    Some(BotRuntimeOptionalBuyPolicy {
        clip: if weak_edge_reduced {
            reduced_clip as i64
        } else {
            decision.clip.max(0)
        },
        min_snapshot_edge,
        weak_edge_reduced,
        edge_source: if snapshot_reliable {
            "snapshot_gap_strict"
        } else {
            "snapshot_gap_fallback"
        },
        yes_snapshot_price,
        no_snapshot_price,
        yes_snapshot_source,
        no_snapshot_source,
        hold_reason,
    })
}

impl MakerHedgeCapBot {
    /// Implements pair build handle paired growth for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_handle_paired_growth(
        &self,
        now: f64,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        context: &BotRuntimePairBuildMarketContext,
        plan: &BotRuntimePairBuildPlan,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
        let decision = plan.decision;
        if let Some(policy) = plan.optional_buy_policy.as_ref() {
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
        if let Some(policy) = plan.optional_growth_policy.as_ref() {
            if matches!(
                policy.band,
                BotRuntimePairedCostBand::RepairOnly | BotRuntimePairedCostBand::Freeze
            ) {
                self._bot_runtime_log_pair_build_state(
                    "hold",
                    &format!(
                        "projected_paired_cost_{}:{:.3}",
                        policy.band.as_str(),
                        policy.projected_paired_cost
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
        if plan.bad_regime_shutdown.0 {
            self._bot_runtime_log_pair_build_state(
                "hold",
                &format!(
                    "bad_regime_optional_growth_shutdown:{:.3}:{}:{}",
                    plan.bad_regime_shutdown.1,
                    plan.bad_regime_shutdown.2,
                    plan.bad_regime_shutdown.3
                ),
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if let Some(policy) = plan.repair_reserve_policy.as_ref() {
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
        if let Some(tail_cap) = bot_runtime_tail_cap_exceeded(q_yes, q_no, t_into_s, cfg) {
            let lighter_side_and_bid = if q_yes + 1e-9 < q_no {
                Some((OutcomeSide::Yes, context.y_bid))
            } else if q_no + 1e-9 < q_yes {
                Some((OutcomeSide::No, context.n_bid))
            } else {
                None
            };
            let repair_currently_executable = lighter_side_and_bid
                .map(|(_, side_bid)| {
                    bot_runtime_pair_build_exact_gap_repair_is_executable(
                        decision.qty_gap,
                        side_bid,
                        self.cfg.min_shares,
                        self.min_maker_notional,
                    )
                })
                .unwrap_or(true);
            if repair_currently_executable {
                self._bot_runtime_log_pair_build_state(
                    "hold",
                    &format!(
                        "tail_cap_repair_priority:{:.2}:{:.2}:{:.3}",
                        tail_cap.tail_size, tail_cap.cap_shares, tail_cap.cap_fraction
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
        if let Some(pace_seconds) = bot_runtime_pair_build_cpp_pace_seconds(
            &decision,
            plan.budget_snapshot.under_min_target,
        ) {
            let last_submit_ts = self._bot_runtime_last_optional_growth_submit_ts();
            if last_submit_ts > 0.0 && (now - last_submit_ts).max(0.0) < pace_seconds {
                self._bot_runtime_log_pair_build_state(
                    "hold",
                    &format!("paired_growth_cpp_paced_{}", decision.cpp_hint.as_str()),
                    Some(decision),
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        }
        for (side, target_price) in [
            (OutcomeSide::Yes, context.y_bid),
            (OutcomeSide::No, context.n_bid),
        ] {
            if let Some(reason) = self._bot_runtime_pair_build_repost_block_reason(
                side,
                target_price,
                now,
                self.cfg.tick.max(0.0001),
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
        }

        self._set_pending_entry_reason("BOT_PAIR_BUILD");
        let submit_started = now_ts_f64();
        let (y_oid, n_oid) = self._maker_submit_pair_orders(
            decision.clip,
            context.y_bid,
            context.n_bid,
            "GTC",
            Some(true),
            "BOT_PAIR_BUILD",
        );
        if y_oid.is_some() {
            self._bot_runtime_pair_build_note_side_submit(OutcomeSide::Yes, context.y_bid, now);
        }
        if n_oid.is_some() {
            self._bot_runtime_pair_build_note_side_submit(OutcomeSide::No, context.n_bid, now);
        }
        if y_oid.is_some() || n_oid.is_some() {
            if let Ok(mut st) = self.bot_runtime_state.lock() {
                st.pair_build_last_paired_growth_yes_bid = context.y_bid;
                st.pair_build_last_paired_growth_no_bid = context.n_bid;
            }
        }
        let submit_elapsed_ms = ((now_ts_f64() - submit_started).max(0.0)) * 1000.0;
        if let Some(asymmetry) = self._maker_pair_order_asymmetry(
            now_ts_f64(),
            &context.yes_asset,
            &context.no_asset,
            "BOT_PAIR_BUILD",
        ) {
            self._bot_runtime_note_pair_build_submit(
                submit_started,
                &decision,
                plan.paired_cost_observation.map(|(_, band)| band),
            );
            self._bot_runtime_log_pair_build_state(
                "rest",
                &format!(
                    "asymmetric_submit:{}:{:.0}ms",
                    asymmetry.live_side.as_str(),
                    submit_elapsed_ms
                ),
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }

        let yes_live_oid = self
            ._maker_order_slot_get(&context.yes_key)
            .order_id
            .or(y_oid);
        let no_live_oid = self
            ._maker_order_slot_get(&context.no_key)
            .order_id
            .or(n_oid);
        if yes_live_oid.is_none() && no_live_oid.is_none() {
            self._bot_runtime_log_pair_build_state(
                "hold",
                "no_pair_build_orders_live",
                Some(decision),
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }

        self._bot_runtime_note_pair_build_submit(
            submit_started,
            &decision,
            plan.paired_cost_observation.map(|(_, band)| band),
        );
        let yes_new = maker_pair_submit_leg_is_new(yes_live_oid.as_deref(), &context.yes_slot);
        let no_new = maker_pair_submit_leg_is_new(no_live_oid.as_deref(), &context.no_slot);
        if yes_new || no_new {
            let projected_paired_cost = plan
                .paired_cost_observation
                .map(|(projected_paired_cost, _)| projected_paired_cost)
                .unwrap_or(f64::INFINITY);
            let paired_cost_band = plan
                .paired_cost_observation
                .map(|(_, band)| band.as_str())
                .unwrap_or("NA");
            let clipped_for_band = plan
                .optional_growth_policy
                .as_ref()
                .map(|policy| policy.clipped_for_band)
                .unwrap_or(false);
            let optional_buy_guard = plan
                .optional_buy_policy
                .as_ref()
                .map(|policy| {
                    if policy.weak_edge_reduced {
                        "weak_edge_reduced_size"
                    } else {
                        "ok"
                    }
                })
                .unwrap_or("NA");
            let optional_buy_edge_source = plan
                .optional_buy_policy
                .as_ref()
                .map(|policy| policy.edge_source)
                .unwrap_or("NA");
            let min_snapshot_edge = plan
                .optional_buy_policy
                .as_ref()
                .map(|policy| policy.min_snapshot_edge)
                .unwrap_or(f64::INFINITY);
            let below_snapshot_optional = decision.mode == BotRuntimePairBuildMode::PairedGrowth
                && plan
                    .optional_buy_policy
                    .as_ref()
                    .map(|policy| policy.hold_reason.is_none())
                    .unwrap_or(false);
            let repair_reserve_side = plan
                .repair_reserve_policy
                .as_ref()
                .map(|policy| policy.likely_repair_side.as_str())
                .unwrap_or("NA");
            let likely_repair_clip = plan
                .repair_reserve_policy
                .as_ref()
                .map(|policy| policy.likely_repair_clip)
                .unwrap_or(0);
            let total_reserved_budget = plan
                .repair_reserve_policy
                .as_ref()
                .map(|policy| policy.total_reserved_budget)
                .unwrap_or(0.0);
            let clipped_for_repair_reserve = plan
                .repair_reserve_policy
                .as_ref()
                .map(|policy| policy.clipped_for_reserve)
                .unwrap_or(false);
            self._bot_runtime_clear_pair_build_hold();
            if below_snapshot_optional {
                for (is_new, live_oid) in [
                    (yes_new, yes_live_oid.as_deref()),
                    (no_new, no_live_oid.as_deref()),
                ] {
                    if !is_new {
                        continue;
                    }
                    if let Some(order_id) = live_oid {
                        self._bot_runtime_note_below_snapshot_optional_submit(decision.clip as f64);
                        self._merge_order_execution_context_fields(
                            order_id,
                            &json!({
                                "bot_runtime_optional_pair_growth": true,
                                "bot_runtime_below_snapshot_optional": true,
                                "bot_runtime_paired_cost_band": paired_cost_band,
                                "bot_runtime_min_snapshot_edge": min_snapshot_edge,
                                "bot_runtime_optional_buy_guard": optional_buy_guard,
                                "bot_runtime_order_size": decision.clip as f64,
                            }),
                        );
                    }
                }
            }
            self.logger.info(&format!(
                "[BOT][PAIR_BUILD] submit mode={} clip={} clip_bucket={} requested_clip={:.0} cpp_hint={} paired_cost_band={} projected_paired_cost={:.3} clipped_for_band={} optional_buy_guard={} optional_buy_edge_source={} min_snapshot_edge={:.3} below_snapshot_optional={} repair_reserve_side={} likely_repair_clip={} total_reserved_budget={:.2} clipped_for_repair_reserve={} bad_regime_shutdown={} bad_regime_ratio={:.3} t_into={:.1}s elapsed_ms={:.0} qYES={:.2} qNO={:.2} total_cost={:.2} pair_sum={:.3} unmatched_fraction={:.3} projected_unmatched_fraction={:.3} match_ratio={:.3} imbalance_state={} reduces_imbalance={} pair_coverage={:.3} skew={:.3} current_base={:.2} inventory_vwap_sum={:.3} market_snapshot_vwap_sum={:.3}",
                decision.mode.as_str(),
                decision.clip,
                decision.clip_bucket,
                decision.requested_clip,
                decision.cpp_hint.as_str(),
                paired_cost_band,
                projected_paired_cost,
                clipped_for_band,
                optional_buy_guard,
                optional_buy_edge_source,
                min_snapshot_edge,
                below_snapshot_optional,
                repair_reserve_side,
                likely_repair_clip,
                total_reserved_budget,
                clipped_for_repair_reserve,
                plan.bad_regime_shutdown.0,
                plan.bad_regime_shutdown.1,
                t_into_s.max(0.0),
                submit_elapsed_ms,
                q_yes,
                q_no,
                total_cost.max(0.0),
                decision.pair_sum,
                decision.current_unmatched_fraction,
                decision.projected_unmatched_fraction,
                decision.match_ratio,
                decision.imbalance_state.as_str(),
                decision.reduces_imbalance,
                decision.pair_coverage,
                decision.skew_ratio,
                decision.current_base,
                decision.inventory_vwap_sum,
                decision.market_snapshot_vwap_sum
            ));
        }
    }
}
