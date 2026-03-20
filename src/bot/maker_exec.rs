use super::*;

impl MakerHedgeCapBot {
    /// Implements single inflight enabled for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_single_inflight_enabled(&self) -> bool {
        if let Some(enabled) = self
            .runtime_flags
            .get("maker_single_inflight_per_side")
            .and_then(|value| value.as_bool())
        {
            return enabled;
        }
        env_bool("MAKER_SINGLE_INFLIGHT_PER_SIDE", true)
    }

    /// Implements submit pending TTL seconds for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_submit_pending_ttl_seconds(&self) -> f64 {
        env_float("MAKER_SUBMIT_PENDING_TTL_SECONDS", 6.0).max(0.5)
    }

    /// Implements cancel pending TTL seconds for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_cancel_pending_ttl_seconds(&self) -> f64 {
        env_float("MAKER_CANCEL_PENDING_TTL_SECONDS", 3.0).max(0.5)
    }

    /// Implements working missing TTL seconds for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_working_missing_ttl_seconds(&self) -> f64 {
        env_float("MAKER_WORKING_MISSING_TTL_SECONDS", 12.0).max(1.0)
    }

    /// Implements replace min interval seconds for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_replace_min_interval_seconds(&self) -> f64 {
        self.cfg.maker_replace_min_interval_seconds.max(0.0)
    }

    /// Implements submit reject cooldown seconds for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_submit_reject_cooldown_seconds(&self) -> f64 {
        env_float("MAKER_SUBMIT_REJECT_COOLDOWN_SECONDS", 5.0).max(0.0)
    }

    /// Returns or derives pair arb imbalance enter shares for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _pair_arb_imbalance_enter_shares(&self) -> f64 {
        env_float(
            "PAIR_ARB_IMBALANCE_ENTER_SHARES",
            self.cfg.min_shares.max(1.0),
        )
        .max(0.0)
    }

    /// Returns or derives pair arb imbalance release shares for the active BOT execution path.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _pair_arb_imbalance_release_shares(&self) -> f64 {
        env_float("PAIR_ARB_IMBALANCE_RELEASE_SHARES", 1.0)
            .max(0.0)
            .min(self._pair_arb_imbalance_enter_shares())
    }

    /// Implements trade exec candidate for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_trade_exec_candidate(
        &self,
        msg: &Value,
        maker_leg: &Value,
    ) -> Option<MakerExecCandidate> {
        let order_id = maker_leg
            .get("order_id")
            .or_else(|| maker_leg.get("orderId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let asset_id = maker_leg
            .get("asset_id")
            .or_else(|| maker_leg.get("assetId"))
            .or_else(|| maker_leg.get("token_id"))
            .or_else(|| maker_leg.get("tokenId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let side = maker_leg
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        let qty = Self::_value_f64(
            maker_leg
                .get("matched_amount")
                .or_else(|| maker_leg.get("matchedAmount"))
                .or_else(|| maker_leg.get("size"))
                .or_else(|| maker_leg.get("filled")),
        )
        .unwrap_or(0.0);
        let price = Self::_value_f64(maker_leg.get("price")).unwrap_or(0.0);
        if order_id.is_empty()
            || asset_id.is_empty()
            || !matches!(side.as_str(), "BUY" | "SELL")
            || qty <= 0.0
            || price <= 0.0
        {
            return None;
        }
        let tx_hash = msg
            .get("transaction_hash")
            .or_else(|| msg.get("transactionHash"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let trade_id = msg
            .get("id")
            .or_else(|| msg.get("trade_id"))
            .or_else(|| msg.get("tradeId"))
            .or_else(|| msg.get("tradeID"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let taker_order_id = msg
            .get("taker_order_id")
            .or_else(|| msg.get("takerOrderId"))
            .or_else(|| msg.get("taker_orderId"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let match_time = msg
            .get("match_time")
            .or_else(|| msg.get("matchTime"))
            .or_else(|| msg.get("timestamp"))
            .or_else(|| msg.get("ts"))
            .and_then(|v| match v {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty());
        Some(MakerExecCandidate {
            order_id,
            asset_id,
            side,
            qty,
            price,
            tx_hash,
            trade_id,
            taker_order_id,
            match_time,
        })
    }

    /// Implements trade exec aliases for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_trade_exec_aliases(candidate: &MakerExecCandidate) -> Vec<String> {
        let mut aliases: Vec<String> = Vec::new();
        if let Some(tx_hash) = candidate.tx_hash.as_deref() {
            aliases.push(format!(
                "maker_tx:{}:{}:{:.8}:{:.8}",
                candidate.order_id, tx_hash, candidate.qty, candidate.price
            ));
        }
        if let Some(trade_id) = candidate.trade_id.as_deref() {
            aliases.push(format!("maker_trade:{}:{}", candidate.order_id, trade_id));
        }
        if let (Some(taker_oid), Some(match_time)) = (
            candidate.taker_order_id.as_deref(),
            candidate.match_time.as_deref(),
        ) {
            aliases.push(format!(
                "maker_match:{}:{}:{}:{:.8}:{:.8}",
                candidate.order_id, taker_oid, match_time, candidate.qty, candidate.price
            ));
        }
        aliases
    }

    /// Implements exec alias kind for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_exec_alias_kind(exec_id: &str) -> &'static str {
        if exec_id.starts_with("maker_tx:") {
            "tx"
        } else if exec_id.starts_with("maker_trade:") {
            "trade"
        } else if exec_id.starts_with("maker_match:") {
            "match"
        } else {
            "unknown"
        }
    }

    /// Implements exec record matches for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_exec_record_matches(
        record: &MakerExecRecord,
        candidate: &MakerExecCandidate,
    ) -> bool {
        const EPS: f64 = 1e-9;
        record.order_id == candidate.order_id
            && record.asset_id == candidate.asset_id
            && record.side == candidate.side
            && (record.qty - candidate.qty).abs() <= EPS
            && (record.price - candidate.price).abs() <= EPS
    }

    /// Implements exec order sum for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_exec_order_sum(ledger: &MakerExecLedger, order_id: &str) -> f64 {
        if order_id.trim().is_empty() {
            return 0.0;
        }
        ledger
            .records
            .values()
            .filter(|rec| rec.order_id == order_id)
            .map(|rec| rec.qty.max(0.0))
            .sum::<f64>()
    }

    /// Implements exec attach aliases for the maker-side BOT workflow.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn _maker_exec_attach_aliases(
        ledger: &mut MakerExecLedger,
        canonical_id: &str,
        aliases: &[String],
    ) {
        let mut clean_aliases: Vec<String> = aliases
            .iter()
            .filter(|alias| !alias.trim().is_empty())
            .cloned()
            .collect();
        clean_aliases.dedup();
        for alias in &clean_aliases {
            ledger
                .alias_to_canonical
                .insert(alias.clone(), canonical_id.to_string());
        }
        if let Some(record) = ledger.records.get_mut(canonical_id) {
            for alias in clean_aliases {
                if !record.aliases.iter().any(|v| v == &alias) {
                    record.aliases.push(alias);
                }
            }
        }
    }

    /// Implements exec applied quantity for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_exec_applied_qty(&self, order_id: &str) -> f64 {
        if order_id.trim().is_empty() {
            return 0.0;
        }
        self.maker_exec_ledger
            .lock()
            .ok()
            .and_then(|ledger| ledger.per_order_applied.get(order_id).cloned())
            .map(|rec| rec.applied_qty.max(0.0))
            .unwrap_or(0.0)
    }

    /// Implements commit exec fill for the maker-side BOT workflow.
    /// This reads bot-owned state, cached data, or exchange metadata for the active BOT
    /// runtime.

    pub(super) fn _maker_commit_exec_fill(
        &self,
        candidate: MakerExecCandidate,
    ) -> MakerExecApplyResult {
        const EPS: f64 = 1e-9;
        let aliases = Self::_maker_trade_exec_aliases(&candidate);
        if aliases.is_empty() {
            return MakerExecApplyResult::DroppedWeakId {
                reason: "no_strong_alias".to_string(),
            };
        }

        let now = now_ts_f64();
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return MakerExecApplyResult::Conflict {
                    canonical_id: aliases[0].clone(),
                    reason: "state_lock_failed".to_string(),
                }
            }
        };
        let mut ledger = match self.maker_exec_ledger.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return MakerExecApplyResult::Conflict {
                    canonical_id: aliases[0].clone(),
                    reason: "maker_exec_ledger_lock_failed".to_string(),
                }
            }
        };

        let mut canonical_id = aliases
            .iter()
            .find_map(|alias| ledger.alias_to_canonical.get(alias).cloned());
        if canonical_id.is_none() {
            canonical_id = aliases
                .iter()
                .find(|alias| state.has_seen_trade_key(alias))
                .cloned();
        }
        let canonical_id = canonical_id.unwrap_or_else(|| aliases[0].clone());

        if let Some(existing) = ledger.records.get(&canonical_id).cloned() {
            if !Self::_maker_exec_record_matches(&existing, &candidate) {
                return MakerExecApplyResult::Conflict {
                    canonical_id,
                    reason: format!(
                        "alias_resolved_to_existing_record_mismatch order_id={} qty={:.8} price={:.8} asset={} side={}",
                        existing.order_id, existing.qty, existing.price, existing.asset_id, existing.side
                    ),
                };
            }
            Self::_maker_exec_attach_aliases(&mut ledger, &existing.canonical_id, &aliases);
            return MakerExecApplyResult::Duplicate {
                canonical_id: existing.canonical_id,
            };
        }

        if state.has_seen_trade_key(&canonical_id)
            || aliases.iter().any(|alias| state.has_seen_trade_key(alias))
        {
            return MakerExecApplyResult::Duplicate { canonical_id };
        }

        let order_sum_before = Self::_maker_exec_order_sum(&ledger, &candidate.order_id);
        let applied_before = ledger
            .per_order_applied
            .get(&candidate.order_id)
            .map(|rec| rec.applied_qty.max(0.0))
            .unwrap_or(0.0);
        if (order_sum_before - applied_before).abs() > EPS {
            self.logger.warning(&format!(
                "[FILL][MAKER_INVARIANT] oid={}.. applied={applied_before:.8} expected={order_sum_before:.8} stage=pre_apply",
                candidate.order_id.chars().take(10).collect::<String>()
            ));
            return MakerExecApplyResult::Conflict {
                canonical_id,
                reason: format!(
                    "pre_apply_invariant_mismatch applied={applied_before:.8} expected={order_sum_before:.8}"
                ),
            };
        }

        let Some(meta) = self._apply_fill_locked_nodedupe(
            &mut state,
            &candidate.asset_id,
            candidate.price,
            candidate.qty,
            &candidate.side,
        ) else {
            return MakerExecApplyResult::Conflict {
                canonical_id,
                reason: "apply_fill_locked_nodedupe_failed".to_string(),
            };
        };

        state.record_seen_trade_key(&canonical_id, now);
        state.record_pair_liquidity_fill(candidate.qty, true);
        let _ = self._bot_runtime_save_state_or_dependency_pause(&mut state, "maker_exec_fill");
        drop(state);
        let fill_ts = candidate.match_time.as_deref().and_then(|value| {
            self._fill_event_ts_from_value(Some(&Value::String(value.to_string())))
        });
        self._record_daily_liquidity_fill_global(candidate.qty, true, fill_ts);

        let record = MakerExecRecord {
            canonical_id: canonical_id.clone(),
            order_id: candidate.order_id.clone(),
            qty: candidate.qty,
            price: candidate.price,
            asset_id: candidate.asset_id.clone(),
            side: candidate.side.clone(),
            aliases: Vec::new(),
            applied_ts: now,
        };
        ledger.records.insert(canonical_id.clone(), record);
        Self::_maker_exec_attach_aliases(&mut ledger, &canonical_id, &aliases);
        let entry = ledger
            .per_order_applied
            .entry(candidate.order_id.clone())
            .or_default();
        entry.applied_qty += candidate.qty.max(0.0);
        entry.last_update_ts = now;

        let order_sum_after = Self::_maker_exec_order_sum(&ledger, &candidate.order_id);
        let applied_after = ledger
            .per_order_applied
            .get(&candidate.order_id)
            .map(|rec| rec.applied_qty.max(0.0))
            .unwrap_or(0.0);
        if (order_sum_after - applied_after).abs() > EPS {
            self.logger.warning(&format!(
                "[FILL][MAKER_INVARIANT] oid={}.. applied={applied_after:.8} expected={order_sum_after:.8} stage=post_apply",
                candidate.order_id.chars().take(10).collect::<String>()
            ));
            drop(ledger);
            self._apply_fill_finalize(meta);
            return MakerExecApplyResult::Conflict {
                canonical_id,
                reason: format!(
                    "post_apply_invariant_mismatch applied={applied_after:.8} expected={order_sum_after:.8}"
                ),
            };
        }
        drop(ledger);

        self._apply_fill_finalize(meta);
        let fill_origin = self._maker_order_origin_by_order_id(&candidate.order_id);
        self._bot_runtime_note_observed_fill(
            &candidate.asset_id,
            candidate.qty,
            true,
            &candidate.side,
            Some(&candidate.order_id),
            fill_origin.as_deref(),
        );
        self._audit_record_fill_event(
            Some(&candidate.order_id),
            &candidate.asset_id,
            &candidate.side,
            candidate.price,
            candidate.qty,
            true,
            fill_ts.or(Some(now)),
            fill_origin.as_deref(),
        );
        MakerExecApplyResult::Applied { canonical_id }
    }
}
