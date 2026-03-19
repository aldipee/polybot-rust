use super::*;
use crate::db::{
    new_uuid, now_iso_jakarta, TradeDecisionEventInsert, TradeDecisionUpsert,
    TradeRuntimeEventInsert,
};
use crate::logging::structured_event_record;

#[derive(Debug, Clone)]
pub(crate) enum AuditWriteTask {
    Runtime(TradeRuntimeEventInsert),
    Decision {
        row: TradeDecisionEventInsert,
        trade_id: String,
        latest_summary: TradeDecisionUpsert,
    },
}

impl MakerHedgeCapBot {
    pub(in crate::bot) fn _audit_trade_ctx(&self) -> Option<(BotRepository, String)> {
        let repo = self.audit_repo.clone()?;
        let trade_id = self.active_trade_id.clone()?;
        if trade_id.trim().is_empty() {
            None
        } else {
            Some((repo, trade_id))
        }
    }

    fn _audit_pair_identity_fields(
        &self,
    ) -> (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let identity = self.pair_identity();
        (
            identity.pair_id,
            identity.market_slug,
            identity.condition_id,
            identity.yes_asset_id,
            identity.no_asset_id,
        )
    }

    fn _audit_note_decision_event_inserted(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.audit_decision_event_count = st.audit_decision_event_count.saturating_add(1);
        }
    }

    fn _audit_note_runtime_event_inserted(&self) {
        if let Ok(mut st) = self.bot_runtime_state.lock() {
            st.audit_runtime_event_count = st.audit_runtime_event_count.saturating_add(1);
        }
    }

    fn _audit_logger_level_for_event_kind(event_kind: &str) -> &'static str {
        match event_kind {
            "risk_block" => "WARN",
            "reconciliation" => "WARN",
            "settlement" => "INFO",
            _ => "INFO",
        }
    }

    pub(in crate::bot) fn _audit_emit_structured_event(
        &self,
        event_kind: &str,
        message: &str,
        payload: Value,
    ) {
        let level = Self::_audit_logger_level_for_event_kind(event_kind);
        self.logger.event(
            level,
            &structured_event_record(event_kind, message, payload),
        );
    }

    pub(in crate::bot) fn _audit_insert_runtime_event(
        &self,
        event_kind: &str,
        decision_event_id: Option<&str>,
        order_id: Option<&str>,
        asset_id: Option<&str>,
        side: Option<&str>,
        reason_code: Option<&str>,
        payload: Value,
    ) -> Option<String> {
        let trade_id = self.active_trade_id.clone()?;
        let tx = self.audit_runtime_tx.clone()?;
        let (pair_id, market_slug, condition_id, yes_asset_id, no_asset_id) =
            self._audit_pair_identity_fields();
        let event_id = new_uuid();
        let payload_text = serde_json::to_string(&payload).ok()?;
        let row = TradeRuntimeEventInsert {
            event_id: event_id.clone(),
            trade_id: trade_id.clone(),
            pair_id,
            market_slug,
            condition_id,
            yes_asset_id,
            no_asset_id,
            config_version: self.config_version.clone(),
            event_kind: event_kind.to_string(),
            event_ts: now_iso_jakarta(),
            decision_event_id: decision_event_id.map(|value| value.to_string()),
            order_id: order_id.map(|value| value.to_string()),
            asset_id: asset_id.map(|value| value.to_string()),
            side: side.map(|value| value.to_string()),
            reason_code: reason_code.map(|value| value.to_string()),
            payload_json: payload_text,
        };
        self._audit_emit_structured_event(event_kind, event_kind, payload);
        match tx.try_send(AuditWriteTask::Runtime(row)) {
            Ok(()) => {
                self._audit_note_runtime_event_inserted();
                Some(event_id)
            }
            Err(TrySendError::Full(_)) => {
                self.logger.warning(&format!(
                    "[AUDIT] runtime_event_drop reason=queue_full event_id={} event_kind={} trade_id={}",
                    event_id, event_kind, trade_id
                ));
                None
            }
            Err(TrySendError::Disconnected(_)) => {
                self.logger.warning(&format!(
                    "[AUDIT] runtime_event_drop reason=queue_disconnected event_id={} event_kind={} trade_id={}",
                    event_id, event_kind, trade_id
                ));
                None
            }
        }
    }

    pub(in crate::bot) fn _audit_insert_decision_event(
        &self,
        decision_scope: &str,
        decision: Option<&BotRuntimePairBuildDecision>,
        approved: bool,
        reason_code: &str,
        order_origin: Option<&str>,
        order_side: Option<&str>,
        t_into_s: f64,
        total_cost: f64,
        q_yes: f64,
        q_no: f64,
    ) -> Option<String> {
        let trade_id = self.active_trade_id.clone()?;
        let tx = self.audit_runtime_tx.clone()?;
        let decision_event_id = new_uuid();
        let (pair_id, market_slug, condition_id, yes_asset_id, no_asset_id) =
            self._audit_pair_identity_fields();
        let (phase, owner, pair_taker_share, daily_taker_share) = self
            .bot_runtime_state
            .lock()
            .map(|st| {
                (
                    st.phase.as_str().to_string(),
                    st.owner.as_str().to_string(),
                    bot_runtime_taker_share(st.maker_fill_shares, st.taker_fill_shares),
                    bot_runtime_taker_share(st.daily_maker_fill_shares, st.daily_taker_fill_shares),
                )
            })
            .unwrap_or_else(|_| {
                (
                    BotRuntimePhase::PreArm.as_str().to_string(),
                    BotRuntimeControlOwner::PreArm.as_str().to_string(),
                    0.0,
                    0.0,
                )
            });
        let t_left_seconds = (self.expiry_ts as f64 - now_ts_f64()).max(0.0);
        let unmatched_fraction_now = unmatched_fraction(q_yes, q_no);
        let match_ratio_now = match_ratio(q_yes, q_no);
        let combined_avg_paid = if let Some(value) = decision.map(|value| value.inventory_vwap_sum)
        {
            if value.is_finite() {
                Some(value)
            } else {
                None
            }
        } else {
            let inv = inventory_vwap_sum(q_yes, q_no, 0.0, 0.0);
            if inv.is_finite() {
                Some(inv)
            } else {
                None
            }
        };
        let payload = json!({
            "decision_event_id": decision_event_id.clone(),
            "trade_id": trade_id.clone(),
            "pair_id": pair_id.clone(),
            "market_slug": market_slug.clone(),
            "condition_id": condition_id.clone(),
            "yes_asset_id": yes_asset_id.clone(),
            "no_asset_id": no_asset_id.clone(),
            "config_version": self.config_version.clone(),
            "decision_scope": decision_scope,
            "approved": approved,
            "reason_code": reason_code,
            "phase": phase.clone(),
            "owner": owner.clone(),
            "submit_origin": order_origin,
            "submit_side": order_side,
            "t_into_seconds": t_into_s.max(0.0),
            "t_left_seconds": t_left_seconds,
            "total_cost": total_cost.max(0.0),
            "q_yes": q_yes.max(0.0),
            "q_no": q_no.max(0.0),
            "combined_avg_paid": combined_avg_paid,
            "unmatched_fraction": decision.map(|value| value.current_unmatched_fraction).unwrap_or(unmatched_fraction_now),
            "projected_unmatched_fraction": decision.map(|value| value.projected_unmatched_fraction).unwrap_or(unmatched_fraction_now),
            "match_ratio": decision.map(|value| value.match_ratio).unwrap_or(match_ratio_now),
            "pair_taker_share": pair_taker_share,
            "daily_taker_share": daily_taker_share,
            "mode": decision.map(|value| value.mode.as_str()),
            "price_zone": decision.map(|value| value.price_zone.as_str()),
            "imbalance_state": decision.map(|value| value.imbalance_state.as_str()),
            "effective_marginal_pair_cost": decision.map(|value| value.effective_marginal_pair_cost),
            "marginal_pair_sum": decision.map(|value| value.pair_sum),
            "marginal_cost_mode": decision.map(|value| value.marginal_cost_mode.as_str()),
            "residual_unit_cost": decision.and_then(|value| value.residual_unit_cost),
            "lagging_side_quote": decision.and_then(|value| value.lagging_side_quote),
            "favorite_side": decision.and_then(|value| value.favorite_side.map(|side| side.as_str())),
            "underdog_side": decision.and_then(|value| value.underdog_side.map(|side| side.as_str())),
            "residual_side": decision.and_then(|value| value.residual_side.map(|side| side.as_str())),
            "projected_residual_side": decision.and_then(|value| value.projected_residual_side.map(|side| side.as_str())),
            "residual_kind": decision.map(|value| value.residual_kind.as_str()),
            "one_side_exception_kind": decision.map(|value| value.one_side_exception_kind.as_str()),
            "increases_underdog_residual": decision.map(|value| value.increases_underdog_residual),
            "reduces_imbalance": decision.map(|value| value.reduces_imbalance),
            "clip": decision.map(|value| value.clip),
            "requested_clip": decision.map(|value| value.requested_clip),
            "selected_rung": decision.map(|value| value.selected_rung.as_str()),
            "requested_rung": decision.map(|value| value.requested_rung.as_str()),
            "clip_bucket": decision.map(|value| value.clip_bucket),
            "requested_large_clip": decision.map(|value| value.requested_large_clip),
        });
        let payload_text = serde_json::to_string(&payload).ok()?;
        let row = TradeDecisionEventInsert {
            decision_event_id: decision_event_id.clone(),
            trade_id: trade_id.clone(),
            pair_id: pair_id.clone(),
            market_slug: market_slug.clone(),
            condition_id: condition_id.clone(),
            yes_asset_id: yes_asset_id.clone(),
            no_asset_id: no_asset_id.clone(),
            config_version: self.config_version.clone(),
            decision_scope: decision_scope.to_string(),
            decision_ts: now_iso_jakarta(),
            phase: Some(phase),
            owner: Some(owner),
            approved,
            reason_code: reason_code.to_string(),
            submit_origin: order_origin.map(|value| value.to_string()),
            submit_side: order_side.map(|value| value.to_string()),
            payload_json: payload_text,
        };
        let latest_summary = TradeDecisionUpsert {
            config_version: Some(self.config_version.clone()),
            pair_id: Some(pair_id),
            market_slug: Some(market_slug),
            condition_id,
            yes_asset_id,
            no_asset_id,
            t_left_seconds: Some(t_left_seconds),
            submit_origin: order_origin.map(|value| value.to_string()),
            submit_side: order_side.map(|value| value.to_string()),
            maker_t_into_s: Some(t_into_s.max(0.0)),
            maker_price_bucket: decision.map(|value| value.price_zone.as_str().to_string()),
            maker_clip_bucket: decision.map(|value| value.clip_bucket.to_string()),
            ..TradeDecisionUpsert::default()
        };
        self._audit_emit_structured_event(
            if approved {
                "decision_approved"
            } else {
                "decision_blocked"
            },
            reason_code,
            payload,
        );
        match tx.try_send(AuditWriteTask::Decision {
            row,
            trade_id: trade_id.clone(),
            latest_summary,
        }) {
            Ok(()) => {
                self._audit_note_decision_event_inserted();
                Some(decision_event_id)
            }
            Err(TrySendError::Full(_)) => {
                self.logger.warning(&format!(
                    "[AUDIT] decision_event_drop reason=queue_full decision_event_id={} trade_id={}",
                    decision_event_id, trade_id
                ));
                None
            }
            Err(TrySendError::Disconnected(_)) => {
                self.logger.warning(&format!(
                    "[AUDIT] decision_event_drop reason=queue_disconnected decision_event_id={} trade_id={}",
                    decision_event_id, trade_id
                ));
                None
            }
        }
    }

    pub(in crate::bot) fn _audit_attach_decision_context(
        &self,
        order_id: &str,
        decision_event_id: &str,
        reason_code: &str,
    ) {
        self._merge_order_execution_context_fields(
            order_id,
            &json!({
                "decision_event_id": decision_event_id,
                "reason_code": reason_code,
                "trade_id": self.active_trade_id.clone(),
                "config_version": self.config_version.clone(),
            }),
        );
    }

    pub(in crate::bot) fn _audit_record_order_context_events(&self, order_id: &str) {
        let trimmed = order_id.trim();
        if trimmed.is_empty() {
            return;
        }
        let Some(mut ctx) = self._get_order_execution_context(trimmed) else {
            return;
        };
        let decision_event_id = ctx
            .get("decision_event_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.to_string());
        if decision_event_id.is_none() {
            return;
        }
        let reason_code = ctx
            .get("reason_code")
            .and_then(|value| value.as_str())
            .or_else(|| ctx.get("origin").and_then(|value| value.as_str()))
            .map(|value| value.to_string());
        let asset_id = ctx
            .get("asset_id")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let side = ctx
            .get("side")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let intent_recorded = ctx
            .get("audit_order_intent_recorded")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !intent_recorded {
            let payload = json!({
                "order_id": trimmed,
                "decision_event_id": decision_event_id.clone(),
                "reason_code": reason_code,
                "meta_json": ctx.clone(),
            });
            if self
                ._audit_insert_runtime_event(
                    "order_intent",
                    decision_event_id.as_deref(),
                    Some(trimmed),
                    asset_id.as_deref(),
                    side.as_deref(),
                    reason_code.as_deref(),
                    payload,
                )
                .is_some()
            {
                if let Ok(mut map) = self.order_exec_context.lock() {
                    if let Some(existing) =
                        map.get_mut(trimmed).and_then(|value| value.as_object_mut())
                    {
                        existing.insert("audit_order_intent_recorded".to_string(), json!(true));
                    }
                }
                ctx["audit_order_intent_recorded"] = json!(true);
            }
        }
        let ack_recorded = ctx
            .get("audit_order_ack_recorded")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !ack_recorded {
            let payload = json!({
                "order_id": trimmed,
                "decision_event_id": decision_event_id.clone(),
                "reason_code": reason_code,
                "order_submit_ts": ctx.get("order_submit_ts").cloned().unwrap_or(Value::Null),
                "post_end_ts": ctx.get("post_end_ts").cloned().unwrap_or(Value::Null),
                "meta_json": ctx,
            });
            if self
                ._audit_insert_runtime_event(
                    "order_ack",
                    decision_event_id.as_deref(),
                    Some(trimmed),
                    asset_id.as_deref(),
                    side.as_deref(),
                    reason_code.as_deref(),
                    payload,
                )
                .is_some()
            {
                if let Ok(mut map) = self.order_exec_context.lock() {
                    if let Some(existing) =
                        map.get_mut(trimmed).and_then(|value| value.as_object_mut())
                    {
                        existing.insert("audit_order_ack_recorded".to_string(), json!(true));
                    }
                }
            }
        }
    }

    pub(in crate::bot) fn _audit_record_fill_event(
        &self,
        order_id: Option<&str>,
        asset_id: &str,
        side: &str,
        price: f64,
        filled: f64,
        is_maker: bool,
        fill_ts: Option<f64>,
        origin: Option<&str>,
    ) {
        let ctx = order_id.and_then(|value| self._get_order_execution_context(value));
        let decision_event_id = ctx
            .as_ref()
            .and_then(|value| value.get("decision_event_id"))
            .and_then(|value| value.as_str());
        let reason_code = ctx
            .as_ref()
            .and_then(|value| value.get("reason_code"))
            .and_then(|value| value.as_str())
            .or(origin);
        let payload = json!({
            "order_id": order_id,
            "asset_id": asset_id,
            "side": side,
            "price": price,
            "filled": filled.max(0.0),
            "is_maker": is_maker,
            "fill_ts": fill_ts.unwrap_or_else(now_ts_f64),
            "origin": origin,
            "decision_event_id": decision_event_id,
            "reason_code": reason_code,
            "meta_json": ctx,
        });
        let _ = self._audit_insert_runtime_event(
            "fill",
            decision_event_id,
            order_id,
            Some(asset_id),
            Some(side),
            reason_code,
            payload,
        );
    }

    pub(in crate::bot) fn _audit_record_state_transition(
        &self,
        prev_phase: BotRuntimePhase,
        next_phase: BotRuntimePhase,
        prev_owner: BotRuntimeControlOwner,
        next_owner: BotRuntimeControlOwner,
        owner_reason: &str,
        t_into_s: f64,
        q_yes: f64,
        q_no: f64,
        total_cost: f64,
    ) {
        let payload = json!({
            "prev_phase": prev_phase.as_str(),
            "next_phase": next_phase.as_str(),
            "prev_owner": prev_owner.as_str(),
            "next_owner": next_owner.as_str(),
            "owner_reason": owner_reason,
            "t_into_seconds": t_into_s.max(0.0),
            "q_yes": q_yes.max(0.0),
            "q_no": q_no.max(0.0),
            "total_cost": total_cost.max(0.0),
        });
        let _ = self._audit_insert_runtime_event(
            "state_transition",
            None,
            None,
            None,
            None,
            Some(owner_reason),
            payload,
        );
    }

    pub(in crate::bot) fn _audit_record_reconciliation_event(
        &self,
        reason_code: &str,
        payload: Value,
    ) {
        let _ = self._audit_insert_runtime_event(
            "reconciliation",
            None,
            None,
            None,
            None,
            Some(reason_code),
            payload,
        );
    }

    pub(crate) fn _audit_record_settlement_event(&self, reason_code: &str, payload: Value) {
        let _ = self._audit_insert_runtime_event(
            "settlement",
            None,
            None,
            None,
            None,
            Some(reason_code),
            payload,
        );
    }
}
