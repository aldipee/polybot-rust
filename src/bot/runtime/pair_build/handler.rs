use super::super::*;
use super::*;

impl MakerHedgeCapBot {
    /// Implements pair build plan for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    fn _bot_runtime_pair_build_plan(
        &self,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        cost_yes: f64,
        cost_no: f64,
        context: &BotRuntimePairBuildMarketContext,
        budget_snapshot: BotRuntimeBudgetSnapshot,
        cfg: &BotRuntimeConfigSnapshot,
    ) -> Result<BotRuntimePairBuildPlan, String> {
        let mut decision = bot_runtime_pair_build_decision(
            t_into_s,
            q_yes,
            q_no,
            cost_yes,
            cost_no,
            context.y_bid,
            context.y_ask,
            context.n_bid,
            context.n_ask,
            total_cost + budget_snapshot.remaining_to_max_cost,
            total_cost,
            self.cfg.min_shares,
            self.min_maker_notional,
            self.cfg.tick.max(0.0001),
            cfg,
            budget_snapshot.under_min_target,
        )?;
        decision = bot_runtime_pair_build_apply_tail_repair_priority(
            decision,
            q_yes,
            q_no,
            cost_yes,
            cost_no,
            context.y_bid,
            context.n_bid,
            budget_snapshot.remaining_to_max_cost,
            self.cfg.min_shares,
            self.min_maker_notional,
            t_into_s,
            cfg,
        );
        if let Some(reason) = bot_runtime_pair_build_price_zone_hold_reason(
            decision.price_zone,
            decision.marginal_cost_mode,
            decision.effective_marginal_pair_cost,
        ) {
            // During HardDisable, allow lighter-side repair regardless of price zone;
            // the bid cap in the repair handler still prevents overpaying.
            let hard_disable_repair = decision.mode == BotRuntimePairBuildMode::LighterSideFirst
                && matches!(
                    decision.imbalance_state,
                    BotRuntimeImbalanceState::HardDisable
                );
            if !hard_disable_repair {
                return Err(reason);
            }
        }
        if let Some(reason) = bot_runtime_pair_build_residual_direction_hold_reason(&decision) {
            return Err(reason);
        }

        let lighter_repair_policy = if decision.mode == BotRuntimePairBuildMode::LighterSideFirst {
            let side = decision.side.unwrap_or(OutcomeSide::Yes);
            let side_bid = match side {
                OutcomeSide::Yes => context.y_bid,
                OutcomeSide::No => context.n_bid,
            };
            let policy = bot_runtime_pair_build_lighter_repair_policy(
                &decision,
                side_bid,
                budget_snapshot.remaining_to_max_cost,
                self.cfg.min_shares,
                self.min_maker_notional,
                cfg,
            );
            if let Some(policy) = policy.as_ref() {
                if policy.hold_reason.is_none() && policy.clip > 0 && policy.clip != decision.clip {
                    decision = bot_runtime_pair_build_decision_with_selected_clip(
                        decision,
                        policy.clip,
                        q_yes,
                        q_no,
                        budget_snapshot.remaining_to_max_cost,
                        t_into_s,
                        cfg,
                    );
                }
            }
            policy
        } else {
            None
        };

        let repair_reserve_policy = if decision.mode == BotRuntimePairBuildMode::PairedGrowth {
            let policy = bot_runtime_pair_build_repair_reserve_policy(
                &decision,
                q_yes,
                q_no,
                context.y_bid,
                context.n_bid,
                budget_snapshot.remaining_to_max_cost,
                self.cfg.min_shares,
                self.min_maker_notional,
                cfg,
            );
            if let Some(policy) = policy.as_ref() {
                if policy.clip > 0 && policy.clip < decision.clip {
                    decision = bot_runtime_pair_build_decision_with_selected_clip(
                        decision,
                        policy.clip,
                        q_yes,
                        q_no,
                        budget_snapshot.remaining_to_max_cost,
                        t_into_s,
                        cfg,
                    );
                }
            }
            policy
        } else {
            None
        };

        let optional_growth_policy = if decision.mode == BotRuntimePairBuildMode::PairedGrowth {
            let policy = bot_runtime_pair_build_optional_growth_policy(
                &decision,
                q_yes,
                q_no,
                cost_yes,
                cost_no,
                context.y_bid,
                context.n_bid,
                self.cfg.min_shares,
                cfg,
            );
            if let Some(policy) = policy {
                if policy.clip > 0 && policy.clip < decision.clip {
                    decision = bot_runtime_pair_build_decision_with_selected_clip(
                        decision,
                        policy.clip,
                        q_yes,
                        q_no,
                        budget_snapshot.remaining_to_max_cost,
                        t_into_s,
                        cfg,
                    );
                }
            }
            policy
        } else {
            None
        };

        let optional_buy_policy = if decision.mode == BotRuntimePairBuildMode::PairedGrowth {
            let projected_band = optional_growth_policy
                .as_ref()
                .map(|policy| policy.band)
                .unwrap_or(BotRuntimePairedCostBand::Danger);
            let policy = bot_runtime_pair_build_optional_buy_policy(
                &decision,
                context.y_bid,
                context.y_ask,
                context.n_bid,
                context.n_ask,
                projected_band,
                self.cfg.min_shares,
                cfg,
            );
            if let Some(policy) = policy.as_ref() {
                if policy.clip > 0 && policy.clip < decision.clip {
                    decision = bot_runtime_pair_build_decision_with_selected_clip(
                        decision,
                        policy.clip,
                        q_yes,
                        q_no,
                        budget_snapshot.remaining_to_max_cost,
                        t_into_s,
                        cfg,
                    );
                }
            }
            policy
        } else {
            None
        };

        let paired_cost_observation = bot_runtime_pair_build_projected_paired_cost_snapshot(
            &decision,
            q_yes,
            q_no,
            cost_yes,
            cost_no,
            context.y_bid,
            context.n_bid,
        );
        if let Some((_, band)) = paired_cost_observation {
            self._bot_runtime_note_paired_cost_band_observation(band, t_into_s, cfg);
        }
        let bad_regime_shutdown = self._bot_runtime_bad_regime_shutdown_status();
        Ok(BotRuntimePairBuildPlan {
            decision,
            budget_snapshot,
            lighter_repair_policy,
            repair_reserve_policy,
            optional_growth_policy,
            optional_buy_policy,
            paired_cost_observation,
            bad_regime_shutdown,
        })
    }

    /// Implements pair build handler for the BOT runtime.
    /// This helper supports pair-build planning, repair, pacing, or hold-state handling in the
    /// BOT runtime.

    pub(in crate::bot) fn _bot_runtime_pair_build_handler(
        &self,
        now: f64,
        t_into_s: f64,
        _total_cost: f64,
        q_yes: f64,
        q_no: f64,
        cost_yes: f64,
        cost_no: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
        let pair_snapshot = self._pair_snapshot_from_inputs(
            bot_runtime_phase_from_t_into_s(t_into_s, cfg),
            t_into_s,
            q_yes,
            q_no,
            cost_yes,
            cost_no,
        );
        let total_cost = pair_snapshot.total_cost;
        let q_yes = pair_snapshot.position.q_yes;
        let q_no = pair_snapshot.position.q_no;
        let cost_yes = pair_snapshot.position.c_yes;
        let cost_no = pair_snapshot.position.c_no;
        let await_second_fill_hard_paused = self
            .bot_runtime_state
            .lock()
            .map(|st| st.await_second_fill_hard_paused)
            .unwrap_or(false);
        let imbalance_state = self
            .bot_runtime_state
            .lock()
            .map(|st| st.imbalance_state)
            .unwrap_or_else(|_| bot_runtime_current_imbalance_state(q_yes, q_no, cfg));
        if await_second_fill_hard_paused {
            let cancelled = self._bot_runtime_cancel_pair_build_orders(
                None,
                "bot_runtime_pair_build_startup_hard_paused",
            ) || self._bot_runtime_cancel_await_second_fill_orders(
                None,
                "bot_runtime_pair_build_startup_hard_paused",
            );
            self._bot_runtime_log_pair_build_state(
                if cancelled { "rest" } else { "hold" },
                "startup_hard_paused",
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if matches!(imbalance_state, BotRuntimeImbalanceState::HardDisable) {
            self._bot_runtime_cancel_order_family(
                "BOT_OPEN_BOTH",
                None,
                "bot_runtime_pair_build_hard_imbalance_disable",
            );
            self._bot_runtime_cancel_pair_build_growth_orders(
                None,
                "bot_runtime_pair_build_hard_imbalance_disable",
            );
            self._bot_runtime_cancel_taper_orders(
                None,
                "bot_runtime_pair_build_hard_imbalance_disable",
            );
            self._bot_runtime_cancel_await_second_fill_orders(
                None,
                "bot_runtime_pair_build_hard_imbalance_disable",
            );
            // Fall through to plan computation for lighter-side repair
        }
        if q_yes <= 1e-9 || q_no <= 1e-9 {
            let cancelled = self._bot_runtime_cancel_pair_build_orders(
                None,
                "bot_runtime_pair_build_await_second_fill",
            ) || self._bot_runtime_cancel_await_second_fill_orders(
                None,
                "bot_runtime_pair_build_await_second_fill",
            );
            self._bot_runtime_log_pair_build_state(
                if cancelled { "rest" } else { "hold" },
                "await_second_fill_block",
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let (yes_asset, no_asset) = match (&self.yes_asset, &self.no_asset) {
            (Some(yes_asset), Some(no_asset)) => (yes_asset.as_str(), no_asset.as_str()),
            _ => {
                self._bot_runtime_log_pair_build_state(
                    "hold",
                    "missing_assets",
                    None,
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };
        let yes_key = MakerOrderKey::buy(yes_asset);
        let no_key = MakerOrderKey::buy(no_asset);
        let yes_slot = self._maker_order_slot_get(&yes_key);
        let no_slot = self._maker_order_slot_get(&no_key);
        if self._bot_runtime_pair_build_await_second_fill_handoff(
            now, t_into_s, total_cost, q_yes, q_no, &yes_slot, &no_slot,
        ) {
            return;
        }
        if !self.market_connected.load(Ordering::SeqCst) {
            self._bot_runtime_log_pair_build_state(
                "hold",
                "market_ws_disconnected",
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        if self._bot_runtime_user_ws_required() && !self.user_connected.load(Ordering::SeqCst) {
            self.logger.info(&format!(
                "[BOT][PAIR_BUILD][DIAG] user_ws_required={} user_connected={} configured_order_mode={} t_into={:.1}s",
                self._bot_runtime_user_ws_required(),
                self.user_connected.load(Ordering::SeqCst),
                self.configured_order_mode,
                t_into_s,
            ));
            self._bot_runtime_log_pair_build_state(
                "hold",
                "user_ws_disconnected",
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let (quotes_ready, quote_reason) = self._bot_runtime_quote_input_status();
        if !quotes_ready {
            self._bot_runtime_log_pair_build_state(
                "hold",
                &format!("quote_inputs_unready:{quote_reason}"),
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }
        let Some((y_bid, y_ask)) = self._best_bid_ask(yes_asset) else {
            self._bot_runtime_log_pair_build_state(
                "hold",
                "missing_yes_quotes",
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };
        let Some((n_bid, n_ask)) = self._best_bid_ask(no_asset) else {
            self._bot_runtime_log_pair_build_state(
                "hold",
                "missing_no_quotes",
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        };

        let total_usable_budget =
            usable_budget_after_reserve(self.cfg.max_total_cost, self.cfg.reserve_usd);
        let budget_snapshot =
            bot_runtime_budget_snapshot(t_into_s, total_usable_budget, total_cost, cfg);
        if budget_snapshot.remaining_to_max_cost <= 1e-9 {
            self._bot_runtime_log_pair_build_state(
                "hold",
                "phase_budget_exhausted",
                None,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
            );
            return;
        }

        let context = BotRuntimePairBuildMarketContext {
            yes_asset: yes_asset.to_string(),
            no_asset: no_asset.to_string(),
            yes_key,
            no_key,
            yes_slot,
            no_slot,
            y_bid,
            y_ask,
            n_bid,
            n_ask,
        };
        let plan = match self._bot_runtime_pair_build_plan(
            t_into_s,
            total_cost,
            q_yes,
            q_no,
            cost_yes,
            cost_no,
            &context,
            budget_snapshot,
            cfg,
        ) {
            Ok(plan) => plan,
            Err(reason) => {
                let residual_cancel_side = bot_runtime_residual_reason_cancel_side(&reason);
                let preserve_lighter =
                    if bot_runtime_imbalance_reason_preserves_lighter_repair(&reason) {
                        let qty_gap = (q_yes.max(0.0) - q_no.max(0.0)).abs();
                        let tick_size = self.cfg.tick.max(0.0001);
                        if q_yes + 1e-9 < q_no {
                            bot_runtime_live_lighter_repair_is_compatible(
                                &context.yes_slot,
                                "BOT_PAIR_BUILD_LIGHTER",
                                context.y_bid,
                                qty_gap,
                                tick_size,
                            )
                        } else if q_no + 1e-9 < q_yes {
                            bot_runtime_live_lighter_repair_is_compatible(
                                &context.no_slot,
                                "BOT_PAIR_BUILD_LIGHTER",
                                context.n_bid,
                                qty_gap,
                                tick_size,
                            )
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                let imbalance_growth_cancel =
                    bot_runtime_imbalance_reason_requires_growth_order_cancel(&reason);
                let price_zone_growth_cancel =
                    bot_runtime_price_zone_reason_requires_growth_order_cancel(&reason);
                let cancel_lighter_repairs =
                    bot_runtime_price_zone_reason_requires_lighter_repair_cancel(&reason);
                let cancelled = if imbalance_growth_cancel || price_zone_growth_cancel {
                    let cancel_reason = if price_zone_growth_cancel {
                        "bot_runtime_pair_build_price_zone_hold"
                    } else {
                        "bot_runtime_pair_build_imbalance_hold"
                    };
                    let cancelled_open_both =
                        self._bot_runtime_cancel_order_family("BOT_OPEN_BOTH", None, cancel_reason);
                    let cancelled_pair_build = if cancel_lighter_repairs {
                        self._bot_runtime_cancel_pair_build_orders(None, cancel_reason)
                    } else if preserve_lighter {
                        self._bot_runtime_cancel_pair_build_growth_orders(None, cancel_reason)
                    } else if price_zone_growth_cancel {
                        self._bot_runtime_cancel_pair_build_growth_orders(None, cancel_reason)
                    } else {
                        self._bot_runtime_cancel_pair_build_orders(None, cancel_reason)
                    };
                    let cancelled_taper = if cancel_lighter_repairs {
                        self._bot_runtime_cancel_taper_orders(None, cancel_reason)
                    } else if preserve_lighter {
                        self._bot_runtime_cancel_taper_growth_orders(None, cancel_reason)
                    } else if price_zone_growth_cancel {
                        self._bot_runtime_cancel_taper_growth_orders(None, cancel_reason)
                    } else {
                        self._bot_runtime_cancel_taper_orders(None, cancel_reason)
                    };
                    let cancelled_await_second_fill =
                        self._bot_runtime_cancel_await_second_fill_orders(None, cancel_reason);
                    cancelled_open_both
                        || cancelled_pair_build
                        || cancelled_taper
                        || cancelled_await_second_fill
                } else if let Some(side) = residual_cancel_side {
                    self._bot_runtime_cancel_bot_orders_on_side(
                        side,
                        "bot_runtime_pair_build_residual_hold",
                    )
                } else {
                    false
                };
                self._bot_runtime_log_pair_build_state(
                    if cancelled { "rest" } else { "hold" },
                    &reason,
                    None,
                    t_into_s,
                    total_cost,
                    q_yes,
                    q_no,
                );
                return;
            }
        };

        if self._bot_runtime_pair_build_foreign_order_handoff(
            t_into_s,
            total_cost,
            q_yes,
            q_no,
            plan.decision,
            &context,
        ) {
            return;
        }
        if plan.decision.mode == BotRuntimePairBuildMode::LighterSideFirst {
            self._bot_runtime_pair_build_handle_lighter_side_repair(
                now, t_into_s, total_cost, q_yes, q_no, cost_yes, cost_no, &context, &plan,
            );
            return;
        }
        if self._bot_runtime_pair_build_handle_live_paired_orders(
            now,
            t_into_s,
            total_cost,
            q_yes,
            q_no,
            plan.decision,
            &context,
        ) {
            return;
        }
        if self._bot_runtime_pair_build_handle_asymmetry(
            now,
            t_into_s,
            total_cost,
            q_yes,
            q_no,
            plan.decision,
            &context,
        ) {
            return;
        }
        self._bot_runtime_pair_build_handle_paired_growth(
            now, t_into_s, total_cost, q_yes, q_no, &context, &plan, cfg,
        );
    }
}
