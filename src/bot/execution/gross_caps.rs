use super::*;
use crate::helpers::{
    acquire_companion_file_lock, load_shared_gross_exposure_state, load_shared_pending_taker_state,
    save_shared_gross_exposure_state, SharedGrossExposureState,
};
use std::collections::{HashMap, HashSet};

impl MakerHedgeCapBot {
    fn _gross_cap_local_pair_position(&self) -> PairPosition {
        self.state
            .lock()
            .map(|state| PairPosition {
                q_yes: state.q_yes.max(0.0),
                q_no: state.q_no.max(0.0),
                c_yes: state.c_yes.max(0.0),
                c_no: state.c_no.max(0.0),
            })
            .unwrap_or_default()
    }

    pub(in crate::bot) fn _gross_cap_shared_state_error(&self, context: &str, err: &str) {
        let now = now_ts_f64();
        self.logger.warning(&format!(
            "[BOT][SAFE_PAUSE] gross_cap_state_failed context={} err={}",
            context, err
        ));
        self._bot_runtime_enter_dependency_pause("database", "gross_cap_state", now);
        self._audit_record_reconciliation_event(
            "dependency_pause:database:gross_cap_state",
            json!({
                "context": context,
                "reconcile_scope": context,
                "reconcile_clean": false,
                "dependency_pause_kind": "database:gross_cap_state",
                "error": err,
            }),
        );
    }

    fn _gross_cap_load_shared_state(
        &self,
        context: &str,
    ) -> Result<(crate::helpers::CompanionFileLock, SharedGrossExposureState), String> {
        let state_file = self._gross_exposure_state_file();
        let lock =
            acquire_companion_file_lock(&state_file, MakerHedgeCapBot::shared_state_lock_timeout())
                .map_err(|err| format!("{:#}", err))?;
        let state = load_shared_gross_exposure_state(
            &state_file,
            self.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .map_err(|err| format!("{:#}", err))?;
        if context.trim().is_empty() {
            let _ = context;
        }
        Ok((lock, state))
    }

    fn _gross_cap_write_shared_state(
        &self,
        context: &str,
        mut state: SharedGrossExposureState,
    ) -> Result<(), String> {
        let state_file = self._gross_exposure_state_file();
        let _lock =
            acquire_companion_file_lock(&state_file, MakerHedgeCapBot::shared_state_lock_timeout())
                .map_err(|err| format!("{:#}", err))?;
        save_shared_gross_exposure_state(&state_file, &mut state)
            .map_err(|err| format!("{:#}", err))
            .map_err(|err| {
                let _ = context;
                err
            })
    }

    fn _gross_cap_with_shared_state_mut<R>(
        &self,
        context: &str,
        f: impl FnOnce(&mut SharedGrossExposureState) -> R,
    ) -> Result<R, String> {
        let state_file = self._gross_exposure_state_file();
        let _lock =
            acquire_companion_file_lock(&state_file, MakerHedgeCapBot::shared_state_lock_timeout())
                .map_err(|err| format!("{:#}", err))?;
        let mut state = load_shared_gross_exposure_state(
            &state_file,
            self.cfg.gross_cap_shared_state_ttl_seconds,
        )
        .map_err(|err| format!("{:#}", err))?;
        let out = f(&mut state);
        save_shared_gross_exposure_state(&state_file, &mut state)
            .map_err(|err| format!("{:#}", err))?;
        let _ = context;
        Ok(out)
    }

    pub(in crate::bot) fn _refresh_shared_gross_trade_snapshot(&self) -> bool {
        let Some(trade_id) = self
            .active_trade_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return true;
        };
        let pair_id = self.pair_identity().pair_id;
        let instance_key = self._gross_cap_instance_key();
        let gross_filled_cost = self._gross_cap_local_pair_position().total_cost();
        match self._gross_cap_with_shared_state_mut("gross_trade_snapshot", |state| {
            state.upsert_trade_snapshot(
                trade_id.as_str(),
                pair_id.as_str(),
                instance_key.as_str(),
                gross_filled_cost,
                now_ts_f64(),
            );
        }) {
            Ok(()) => true,
            Err(err) => {
                self._gross_cap_shared_state_error("gross_trade_snapshot", err.as_str());
                false
            }
        }
    }

    pub(in crate::bot) fn _forget_shared_gross_trade_snapshot(&self) {
        let Some(trade_id) = self
            .active_trade_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        if let Err(err) = self._gross_cap_with_shared_state_mut("gross_trade_forget", |state| {
            state.forget_trade_snapshot(trade_id.as_str());
        }) {
            self._gross_cap_shared_state_error("gross_trade_forget", err.as_str());
        }
    }

    pub(in crate::bot) fn _remember_shared_gross_order_reservation(
        &self,
        order_id: &str,
        asset_id: &str,
        side: &str,
        price: f64,
        size: f64,
        origin: &str,
        kind: &str,
    ) -> bool {
        let Some(trade_id) = self
            .active_trade_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return true;
        };
        if order_id.trim().is_empty()
            || asset_id.trim().is_empty()
            || !side.eq_ignore_ascii_case("BUY")
            || !price.is_finite()
            || price <= 0.0
            || !size.is_finite()
            || size <= 0.0
        {
            return true;
        }
        let pair_id = self.pair_identity().pair_id;
        if let Err(err) = self._gross_cap_with_shared_state_mut("gross_order_remember", |state| {
            state.remember_order(
                order_id,
                trade_id.as_str(),
                pair_id.as_str(),
                asset_id,
                origin,
                side,
                price,
                size,
                0.0,
                kind,
                now_ts_f64(),
            );
        }) {
            self._gross_cap_shared_state_error("gross_order_remember", err.as_str());
            return false;
        }
        true
    }

    pub(in crate::bot) fn _set_shared_gross_order_applied(
        &self,
        order_id: &str,
        applied_size: f64,
    ) {
        if order_id.trim().is_empty() {
            return;
        }
        if let Err(err) =
            self._gross_cap_with_shared_state_mut("gross_order_set_applied", |state| {
                state.set_order_applied(order_id, applied_size, now_ts_f64());
            })
        {
            self._gross_cap_shared_state_error("gross_order_set_applied", err.as_str());
        }
    }

    pub(in crate::bot) fn _add_shared_gross_order_applied(&self, order_id: &str, delta: f64) {
        if order_id.trim().is_empty() || !delta.is_finite() || delta <= 0.0 {
            return;
        }
        if let Err(err) =
            self._gross_cap_with_shared_state_mut("gross_order_add_applied", |state| {
                state.add_order_applied(order_id, delta, now_ts_f64());
            })
        {
            self._gross_cap_shared_state_error("gross_order_add_applied", err.as_str());
        }
    }

    pub(in crate::bot) fn _forget_shared_gross_order_reservation(&self, order_id: &str) {
        if order_id.trim().is_empty() {
            return;
        }
        if let Err(err) = self._gross_cap_with_shared_state_mut("gross_order_forget", |state| {
            state.forget_order(order_id);
        }) {
            self._gross_cap_shared_state_error("gross_order_forget", err.as_str());
        }
    }

    pub(in crate::bot) fn _shared_gross_order_reservation_snapshot(
        &self,
        order_id: &str,
    ) -> Option<crate::helpers::SharedGrossOrderReservation> {
        let order_id = order_id.trim();
        if order_id.is_empty() {
            return None;
        }
        match self._gross_cap_load_shared_state("gross_order_lookup") {
            Ok((_lock, state)) => state.pending_orders.get(order_id).cloned(),
            Err(err) => {
                self._gross_cap_shared_state_error("gross_order_lookup", err.as_str());
                None
            }
        }
    }

    pub(in crate::bot) fn _shared_pending_taker_order_exists(&self, order_id: &str) -> bool {
        let order_id = order_id.trim();
        if order_id.is_empty() {
            return false;
        }
        let state_file = self._pending_taker_state_file();
        let Ok(_lock) =
            acquire_companion_file_lock(&state_file, MakerHedgeCapBot::shared_state_lock_timeout())
        else {
            return false;
        };
        load_shared_pending_taker_state(&state_file, self.taker_order_ttl_seconds as f64)
            .map(|state| state.orders.contains_key(order_id))
            .unwrap_or(false)
    }

    pub(in crate::bot) fn _republish_shared_gross_reservations_from_local_state(&self) -> bool {
        let Some(trade_id) = self
            .active_trade_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return true;
        };
        let pair = self.pair_identity();
        let pair_asset_ids: HashSet<String> =
            [pair.yes_asset_id.as_deref(), pair.no_asset_id.as_deref()]
                .into_iter()
                .flatten()
                .map(|asset_id| asset_id.to_string())
                .collect();
        let open_orders = self
            .state
            .lock()
            .map(|state| state.open_orders.clone())
            .unwrap_or_default();
        let exchange_orders = self
            .exchange_orders_cache
            .lock()
            .map(|orders| orders.clone())
            .unwrap_or_default();
        let taker_orders = self
            .taker_orders
            .lock()
            .map(|orders| orders.clone())
            .unwrap_or_default();
        let shared_pending_taker_state = {
            let state_file = self._pending_taker_state_file();
            if let Ok(_lock) = crate::helpers::acquire_companion_file_lock(
                &state_file,
                MakerHedgeCapBot::shared_state_lock_timeout(),
            ) {
                crate::helpers::load_shared_pending_taker_state(
                    &state_file,
                    self.taker_order_ttl_seconds as f64,
                )
                .unwrap_or_default()
            } else {
                crate::helpers::SharedPendingTakerState::default()
            }
        };
        let now = now_ts_f64();
        let taker_ttl_seconds = self.taker_order_ttl_seconds as f64;
        let (
            startup_reconciliation_pending,
            reconnect_reconciliation_pending,
            reconnect_pending_since_ts,
        ) = self
            .bot_runtime_state
            .lock()
            .map(|state| {
                let reconnect_pending =
                    state.safety_gate == BotRuntimeSafetyGate::ReconnectReconPending;
                (
                    state.safety_gate == BotRuntimeSafetyGate::StartupReconPending,
                    reconnect_pending,
                    if reconnect_pending {
                        state.dependency_pause_started_ts
                    } else {
                        0.0
                    },
                )
            })
            .unwrap_or((false, false, 0.0));
        let reconnect_dependencies_ready = !reconnect_reconciliation_pending
            || (self.market_connected.load(Ordering::SeqCst)
                && (!self._bot_runtime_user_ws_required()
                    || self.user_connected.load(Ordering::SeqCst)));
        match self._gross_cap_with_shared_state_mut("gross_order_republish", |state| {
            let mut live_buy_orders: HashMap<String, (String, f64, f64)> = HashMap::new();
            let mut exchange_live_buy_order_ids: HashSet<String> = HashSet::new();
            if !startup_reconciliation_pending && !reconnect_reconciliation_pending {
                for order in exchange_orders.iter() {
                    let Some(order_id) = self._extract_order_id(order) else {
                        continue;
                    };
                    let Some(asset_id) = self._extract_order_token_id(order) else {
                        continue;
                    };
                    if !pair_asset_ids.contains(asset_id.as_str()) {
                        continue;
                    }
                    if !self._extract_order_side(order).eq_ignore_ascii_case("BUY") {
                        continue;
                    }
                    let price = self._extract_order_price(order);
                    let size = self._extract_order_remaining_size(order);
                    if price <= 0.0 || size <= 0.0 {
                        continue;
                    }
                    exchange_live_buy_order_ids.insert(order_id.clone());
                    live_buy_orders.insert(order_id, (asset_id, price, size));
                }
            }
            for asset_id in pair_asset_ids.iter() {
                let Some(order) = open_orders.get(asset_id).cloned() else {
                    continue;
                };
                let Some(order_id) = order.order_id.clone() else {
                    continue;
                };
                let price = order.price.unwrap_or(0.0);
                let size = order.size.unwrap_or(0.0);
                if price <= 0.0 || size <= 0.0 {
                    continue;
                }
                live_buy_orders
                    .entry(order_id)
                    .or_insert_with(|| (asset_id.clone(), price, size));
            }
            for (order_id, (asset_id, price, size)) in live_buy_orders {
                let context = self._get_order_execution_context(order_id.as_str());
                let context_is_taker = context
                    .as_ref()
                    .and_then(|ctx| {
                        ctx.get("liquidity_intent")
                            .and_then(|value| value.as_str())
                            .map(|value| value.eq_ignore_ascii_case("taker_exception"))
                    })
                    .unwrap_or(false);
                let local_order_kind = open_orders
                    .get(asset_id.as_str())
                    .filter(|order| order.order_id.as_deref() == Some(order_id.as_str()))
                    .and_then(|order| order.kind.clone());
                let local_order_is_taker = local_order_kind.as_deref() == Some("taker");
                let local_order_present = local_order_kind.is_some();
                let existing_reservation = state.pending_orders.get(order_id.as_str());
                let recent_taker_record = taker_orders
                    .get(order_id.as_str())
                    .filter(|record| {
                        record.side.eq_ignore_ascii_case("BUY")
                            && record.ts.is_finite()
                            && record.ts > 0.0
                            && now - record.ts <= taker_ttl_seconds
                            && (record.size - record.applied).max(0.0) > 1e-9
                    })
                    .is_some();
                let has_pending_taker_hint = shared_pending_taker_state
                    .orders
                    .contains_key(order_id.as_str());
                let confirmed_by_exchange = exchange_live_buy_order_ids.contains(order_id.as_str());
                let context_confirmed_pre_reconcile = context
                    .as_ref()
                    .and_then(|ctx| ctx.get("ts").and_then(|value| value.as_f64()))
                    .map(|ts| {
                        ts.is_finite()
                            && ts > 0.0
                            && reconnect_pending_since_ts > 0.0
                            && ts > reconnect_pending_since_ts
                    })
                    .unwrap_or(false);
                if startup_reconciliation_pending
                    && local_order_present
                    && !confirmed_by_exchange
                    && !context_confirmed_pre_reconcile
                    && !recent_taker_record
                    && !has_pending_taker_hint
                {
                    if let Some(reservation) = existing_reservation.cloned() {
                        state.remember_order(
                            reservation.order_id.as_str(),
                            reservation.trade_id.as_str(),
                            reservation.pair_id.as_str(),
                            reservation.asset_id.as_str(),
                            reservation.origin.as_str(),
                            reservation.side.as_str(),
                            reservation.price,
                            reservation.size,
                            reservation.applied_size,
                            reservation.kind.as_str(),
                            now,
                        );
                    }
                    continue;
                }
                if reconnect_reconciliation_pending
                    && local_order_present
                    && !confirmed_by_exchange
                    && !context_confirmed_pre_reconcile
                    && !recent_taker_record
                    && !has_pending_taker_hint
                {
                    if !reconnect_dependencies_ready {
                        if let Some(reservation) = existing_reservation.cloned() {
                            state.remember_order(
                                reservation.order_id.as_str(),
                                reservation.trade_id.as_str(),
                                reservation.pair_id.as_str(),
                                reservation.asset_id.as_str(),
                                reservation.origin.as_str(),
                                reservation.side.as_str(),
                                reservation.price,
                                reservation.size,
                                reservation.applied_size,
                                reservation.kind.as_str(),
                                now,
                            );
                        }
                        continue;
                    }
                    state.forget_order(order_id.as_str());
                    continue;
                }
                if !confirmed_by_exchange
                    && local_order_is_taker
                    && !context_is_taker
                    && !recent_taker_record
                    && !has_pending_taker_hint
                {
                    state.forget_order(order_id.as_str());
                    continue;
                }
                let (applied_size, total_size) = existing_reservation
                    .map(|reservation| {
                        (
                            reservation.applied_size.max(0.0),
                            reservation.size.max(size).max(0.0),
                        )
                    })
                    .unwrap_or((0.0, size.max(0.0)));
                let origin = context
                    .as_ref()
                    .and_then(|ctx| {
                        ctx.get("origin")
                            .and_then(|value| value.as_str())
                            .map(|value| value.to_string())
                    })
                    .or_else(|| existing_reservation.map(|reservation| reservation.origin.clone()))
                    .unwrap_or_else(|| "BOT_RECONCILED".to_string());
                let kind = context
                    .as_ref()
                    .and_then(|ctx| {
                        ctx.get("liquidity_intent")
                            .and_then(|value| value.as_str())
                            .map(|value| {
                                if value.eq_ignore_ascii_case("taker_exception") {
                                    "taker".to_string()
                                } else {
                                    "maker".to_string()
                                }
                            })
                    })
                    .or(local_order_kind)
                    .or_else(|| {
                        shared_pending_taker_state
                            .orders
                            .get(order_id.as_str())
                            .map(|_| "taker".to_string())
                    })
                    .or_else(|| existing_reservation.map(|reservation| reservation.kind.clone()))
                    .unwrap_or_else(|| "maker".to_string());
                state.remember_order(
                    order_id.as_str(),
                    trade_id.as_str(),
                    pair.pair_id.as_str(),
                    asset_id.as_str(),
                    origin.as_str(),
                    "BUY",
                    price,
                    total_size,
                    applied_size,
                    kind.as_str(),
                    now,
                );
            }
            for (order_id, record) in taker_orders.iter() {
                let is_recent = record.ts.is_finite()
                    && record.ts > 0.0
                    && now - record.ts <= taker_ttl_seconds;
                if !record.side.eq_ignore_ascii_case("BUY") || !is_recent {
                    state.forget_order(order_id.as_str());
                    continue;
                }
                let remaining_size = (record.size - record.applied).max(0.0);
                if remaining_size <= 1e-9 || record.px_limit <= 0.0 {
                    state.forget_order(order_id.as_str());
                    continue;
                }
                let origin = self
                    ._get_order_execution_context(order_id.as_str())
                    .and_then(|ctx| {
                        ctx.get("origin")
                            .and_then(|value| value.as_str())
                            .map(|value| value.to_string())
                    })
                    .unwrap_or_else(|| "TAKER_RECONCILED".to_string());
                state.remember_order(
                    order_id.as_str(),
                    trade_id.as_str(),
                    pair.pair_id.as_str(),
                    record.asset_id.as_str(),
                    origin.as_str(),
                    "BUY",
                    record.px_limit,
                    record.size.max(0.0),
                    record.applied.max(0.0),
                    "taker",
                    now,
                );
            }
        }) {
            Ok(()) => true,
            Err(err) => {
                self._gross_cap_shared_state_error("gross_order_republish", err.as_str());
                false
            }
        }
    }

    pub(in crate::bot) fn _gross_cap_snapshot(
        &self,
        requested_gross_usd: f64,
        replace_order_ids: &[String],
    ) -> Result<GrossCapSnapshot, String> {
        let requested_gross_usd = requested_gross_usd.max(0.0);
        let pair = self.pair_identity();
        let local = self._gross_cap_local_pair_position();
        let current_pair_filled_gross_usd = local.total_cost();
        let local_instance_key = self._gross_cap_instance_key();
        let active_trade_id = self
            .active_trade_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let (_, shared) = self._gross_cap_load_shared_state("gross_cap_snapshot")?;
        let replace_ids: HashSet<String> = replace_order_ids
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        let mut current_pair_pending_maker_gross_usd = 0.0;
        let mut current_pair_pending_taker_gross_usd = 0.0;
        let mut current_portfolio_pending_gross_usd = 0.0;
        for reservation in shared.pending_orders.values() {
            if replace_ids.contains(reservation.order_id.as_str()) {
                continue;
            }
            let gross = reservation.remaining_gross();
            if gross <= 1e-9 {
                continue;
            }
            let include_kind = if reservation.kind.eq_ignore_ascii_case("maker") {
                self.cfg.gross_cap_include_pending_maker
            } else if reservation.kind.eq_ignore_ascii_case("taker") {
                self.cfg.gross_cap_include_pending_taker
            } else {
                true
            };
            if !include_kind {
                continue;
            }
            current_portfolio_pending_gross_usd += gross;
            let same_pair = reservation.pair_id.trim() == pair.pair_id.trim()
                || pair.yes_asset_id.as_deref() == Some(reservation.asset_id.as_str())
                || pair.no_asset_id.as_deref() == Some(reservation.asset_id.as_str());
            if same_pair {
                if reservation.kind.eq_ignore_ascii_case("taker") {
                    current_pair_pending_taker_gross_usd += gross;
                } else {
                    current_pair_pending_maker_gross_usd += gross;
                }
            }
        }

        let mut current_portfolio_filled_gross_usd = 0.0;
        let mut local_trade_added = false;
        for snapshot in shared.live_trades.values() {
            let same_trade = active_trade_id
                .as_ref()
                .map(|trade_id| snapshot.trade_id.trim() == trade_id.as_str())
                .unwrap_or(false);
            let same_instance = !local_instance_key.trim().is_empty()
                && snapshot.instance_key.trim() == local_instance_key.trim();
            if same_trade || same_instance {
                if !local_trade_added {
                    current_portfolio_filled_gross_usd += current_pair_filled_gross_usd;
                    local_trade_added = true;
                }
            } else {
                current_portfolio_filled_gross_usd += snapshot.gross_filled_cost.max(0.0);
            }
        }
        if active_trade_id.is_some() && !local_trade_added {
            current_portfolio_filled_gross_usd += current_pair_filled_gross_usd;
        }

        let projected_pair_gross_usd = current_pair_filled_gross_usd
            + current_pair_pending_maker_gross_usd
            + current_pair_pending_taker_gross_usd
            + requested_gross_usd;
        let projected_portfolio_gross_usd = current_portfolio_filled_gross_usd
            + current_portfolio_pending_gross_usd
            + requested_gross_usd;
        let pair_cap_usd = self.cfg.pair_gross_deployed_cost_cap_usd.max(0.0);
        let portfolio_cap_usd = self.cfg.portfolio_gross_deployed_cost_cap_usd.max(0.0);
        let pair_buffer_usd = self.cfg.pair_gross_deployed_cost_buffer_usd.max(0.0);
        let portfolio_buffer_usd = self.cfg.portfolio_gross_deployed_cost_buffer_usd.max(0.0);

        Ok(GrossCapSnapshot {
            pair_cap_usd,
            portfolio_cap_usd,
            pair_buffer_usd,
            portfolio_buffer_usd,
            effective_pair_cap_usd: (pair_cap_usd - pair_buffer_usd).max(0.0),
            effective_portfolio_cap_usd: (portfolio_cap_usd - portfolio_buffer_usd).max(0.0),
            current_pair_filled_gross_usd,
            current_pair_pending_maker_gross_usd,
            current_pair_pending_taker_gross_usd,
            requested_gross_usd,
            projected_pair_gross_usd,
            current_portfolio_filled_gross_usd,
            current_portfolio_pending_gross_usd,
            projected_portfolio_gross_usd,
            include_pending_maker: self.cfg.gross_cap_include_pending_maker,
            include_pending_taker: self.cfg.gross_cap_include_pending_taker,
        })
    }

    pub(in crate::bot) fn _gross_cap_snapshot_json(snapshot: GrossCapSnapshot) -> Value {
        json!({
            "pair_cap_usd": snapshot.pair_cap_usd,
            "portfolio_cap_usd": snapshot.portfolio_cap_usd,
            "pair_buffer_usd": snapshot.pair_buffer_usd,
            "portfolio_buffer_usd": snapshot.portfolio_buffer_usd,
            "effective_pair_cap_usd": snapshot.effective_pair_cap_usd,
            "effective_portfolio_cap_usd": snapshot.effective_portfolio_cap_usd,
            "current_pair_filled_gross_usd": snapshot.current_pair_filled_gross_usd,
            "current_pair_pending_maker_gross_usd": snapshot.current_pair_pending_maker_gross_usd,
            "current_pair_pending_taker_gross_usd": snapshot.current_pair_pending_taker_gross_usd,
            "requested_gross_usd": snapshot.requested_gross_usd,
            "projected_pair_gross_usd": snapshot.projected_pair_gross_usd,
            "current_portfolio_filled_gross_usd": snapshot.current_portfolio_filled_gross_usd,
            "current_portfolio_pending_gross_usd": snapshot.current_portfolio_pending_gross_usd,
            "projected_portfolio_gross_usd": snapshot.projected_portfolio_gross_usd,
            "include_pending_maker": snapshot.include_pending_maker,
            "include_pending_taker": snapshot.include_pending_taker,
        })
    }

    pub(in crate::bot) fn _gross_cap_block_reason_for_request(
        &self,
        requested_gross_usd: f64,
        replace_order_ids: &[String],
    ) -> Result<Option<(String, GrossCapSnapshot)>, String> {
        let snapshot = self._gross_cap_snapshot(requested_gross_usd, replace_order_ids)?;
        Ok(snapshot
            .block_reason()
            .map(|reason| (reason.to_string(), snapshot)))
    }

    pub(in crate::bot) fn _gross_cap_record_order_context(
        &self,
        order_id: &str,
        snapshot: GrossCapSnapshot,
    ) {
        self._merge_order_execution_context_fields(
            order_id,
            &json!({
                "gross_cap": Self::_gross_cap_snapshot_json(snapshot),
            }),
        );
    }

    pub(in crate::bot) fn _gross_cap_reject_submit(
        &self,
        reason: &str,
        asset_id: &str,
        side: &str,
        origin: &str,
        snapshot: GrossCapSnapshot,
    ) {
        let payload = json!({
            "pair_id": self.pair_identity().pair_id,
            "asset_id": asset_id,
            "side": side,
            "origin": origin,
            "gross_cap": Self::_gross_cap_snapshot_json(snapshot),
        });
        self.logger.warning(&format!(
            "[BOT][GROSS_CAP] pair_id={} asset={} side={} origin={} hold_reason={} requested_gross_usd={:.4} projected_pair_gross_usd={:.4} projected_portfolio_gross_usd={:.4}",
            self.pair_identity().pair_id,
            asset_id,
            side,
            origin,
            reason,
            snapshot.requested_gross_usd,
            snapshot.projected_pair_gross_usd,
            snapshot.projected_portfolio_gross_usd,
        ));
        let _ = self._audit_insert_runtime_event(
            "risk_block",
            None,
            None,
            Some(asset_id),
            Some(side),
            Some(reason),
            payload,
        );
    }
}
