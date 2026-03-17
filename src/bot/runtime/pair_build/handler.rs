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
            cfg,
            budget_snapshot.under_min_target,
        )?;
        decision = bot_runtime_pair_build_apply_tail_repair_priority(
            decision,
            q_yes,
            q_no,
            context.y_bid,
            context.n_bid,
            budget_snapshot.remaining_to_max_cost,
            self.cfg.min_shares,
            self.min_maker_notional,
            t_into_s,
            cfg,
        );

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
            );
            if let Some(policy) = policy.as_ref() {
                if policy.hold_reason.is_none() && policy.clip > 0 && policy.clip != decision.clip {
                    decision.clip = policy.clip;
                    decision.clip_bucket = bot_runtime_pair_build_clip_bucket(policy.clip as f64, cfg);
                }
            }
            policy
        } else {
            None
        };

        if decision.mode == BotRuntimePairBuildMode::LighterSideFirst {
            let side = decision.side.unwrap_or(OutcomeSide::Yes);
            let side_bid = match side {
                OutcomeSide::Yes => context.y_bid,
                OutcomeSide::No => context.n_bid,
            };
            let min_lot = self.cfg.min_shares.max(1.0);
            let adjusted_clip = bot_runtime_pair_build_lighter_clip_after_projected_cost(
                &decision,
                q_yes,
                q_no,
                cost_yes,
                cost_no,
                side,
                side_bid,
                min_lot,
                cfg,
            );
            if adjusted_clip + 1e-9 < decision.clip as f64 {
                decision.clip = adjusted_clip as i64;
                decision.clip_bucket = bot_runtime_pair_build_clip_bucket(adjusted_clip, cfg);
            }
        }

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
                    decision.clip = policy.clip;
                    decision.clip_bucket = bot_runtime_pair_build_clip_bucket(policy.clip as f64, cfg);
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
                    decision.clip = policy.clip;
                    decision.clip_bucket = bot_runtime_pair_build_clip_bucket(policy.clip as f64, cfg);
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
                .unwrap_or(BotRuntimePairedCostBand::Freeze);
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
                    decision.clip = policy.clip;
                    decision.clip_bucket = bot_runtime_pair_build_clip_bucket(policy.clip as f64, cfg);
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
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
        cost_yes: f64,
        cost_no: f64,
        cfg: &BotRuntimeConfigSnapshot,
    ) {
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
        if self._bot_runtime_pair_build_seed_completion_handoff(
            now,
            t_into_s,
            total_cost,
            q_yes,
            q_no,
            &yes_slot,
            &no_slot,
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
        if env_bool("REQUIRE_USER_WS_CONNECTED", true) && !self.user_connected.load(Ordering::SeqCst) {
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
                self._bot_runtime_log_pair_build_state(
                    "hold",
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
                now,
                t_into_s,
                total_cost,
                q_yes,
                q_no,
                cost_yes,
                cost_no,
                &context,
                &plan,
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
            now,
            t_into_s,
            total_cost,
            q_yes,
            q_no,
            &context,
            &plan,
            cfg,
        );
    }
}
