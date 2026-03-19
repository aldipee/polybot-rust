use super::*;
impl MakerHedgeCapBot {
    /// Returns or derives lat ms for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _lat_ms(&self, t1: f64, t0: f64) -> Option<i64> {
        if !t1.is_finite() || !t0.is_finite() {
            return None;
        }
        Some(((t1 - t0) * 1000.0).round() as i64)
    }
    /// Returns or derives lat us for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _lat_us(&self, t1: f64, t0: f64) -> Option<i64> {
        if !t1.is_finite() || !t0.is_finite() {
            return None;
        }
        Some(((t1 - t0) * 1_000_000.0).round() as i64)
    }
    /// Returns or derives UTC ISO for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _utc_iso(&self, ts: f64) -> String {
        let sec = ts.floor() as i64;
        let nsec = ((ts - sec as f64).max(0.0) * 1_000_000_000.0) as u32;
        Utc.timestamp_opt(sec, nsec)
            .single()
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| Utc::now().to_rfc3339())
    }
    /// Returns whether file log submit event should happen in the BOT runtime.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _should_file_log_submit_event(&self, _context_ts: f64) -> bool {
        if !env_bool("EXEC_LATENCY_FILE_LOG_ENABLED", true) {
            return false;
        }
        env_bool("EXEC_LATENCY_FILE_LOG_SUBMIT_ALL_EVENTS", false)
            || env_bool("EXEC_LATENCY_FILE_LOG_SUBMIT_EVENTS", true)
    }
    /// Returns or derives latency file append for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _latency_file_append(&self, rec: &Value) {
        if let Some(svc) = &self.latency_log {
            svc.append(rec);
        }
    }
    /// Returns or derives prune order exec context locked for the active BOT execution path.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _prune_order_exec_context_locked(&self, now_ts: f64) {
        let ttl = env_float("EXEC_LATENCY_CONTEXT_TTL_SECONDS", 21600.0).max(1.0);
        let max_records = env_int("EXEC_LATENCY_MAX_CONTEXT_RECORDS", 50000).max(10) as usize;
        if let Ok(mut map) = self.order_exec_context.lock() {
            map.retain(|_, v| {
                let ts = v
                    .get("ts")
                    .and_then(|x| x.as_f64())
                    .or_else(|| v.get("post_start_ts").and_then(|x| x.as_f64()))
                    .unwrap_or(now_ts);
                now_ts - ts <= ttl
            });
            if map.len() > max_records {
                let mut keys: Vec<String> = map.keys().cloned().collect();
                keys.sort();
                let drop_n = map.len() - max_records;
                for k in keys.into_iter().take(drop_n) {
                    map.remove(&k);
                }
            }
        }
    }
    /// Tracks order execution context for later BOT execution analysis.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _track_order_execution_context(&self, order_id: &str, rec: &Value) {
        if order_id.trim().is_empty() {
            return;
        }
        if !env_bool("EXEC_LATENCY_LOG_ENABLED", true) {
            return;
        }
        let now = now_ts_f64();
        let mut rec2 = rec.clone();
        if !rec2.is_object() {
            rec2 = json!({});
        }
        self._merge_pair_metadata_into_value(&mut rec2);
        if let Ok(mut timings) = self.submit_timing_cache.lock() {
            if let Some(t) = timings.remove(order_id) {
                if let (Some(dst), Some(src)) = (rec2.as_object_mut(), t.as_object()) {
                    for k in [
                        "sign_start_ns",
                        "sign_end_ns",
                        "sign_start_ts",
                        "sign_end_ts",
                        "prep_start_ns",
                        "prep_end_ns",
                        "prep_start_ts",
                        "prep_end_ts",
                        "post_start_ns",
                        "post_end_ns",
                        "post_start_ts",
                        "post_end_ts",
                        "order_submit_ts",
                        "fee_rate_bps",
                        "tick_size",
                        "neg_risk",
                        "pair_id",
                        "market_slug",
                        "condition_id",
                        "yes_asset_id",
                        "no_asset_id",
                    ] {
                        if let Some(v) = src.get(k) {
                            dst.insert(k.to_string(), v.clone());
                        }
                    }
                }
            }
        }
        let value_i64 = |v: Option<&Value>| -> Option<i64> {
            v.and_then(|x| match x {
                Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
                Value::String(s) => s.parse::<i64>().ok(),
                _ => None,
            })
        };
        let diff_us_ns = |start_ns: Option<i64>, end_ns: Option<i64>| -> Option<i64> {
            match (start_ns, end_ns) {
                (Some(start), Some(end)) if end >= start => {
                    Some(((end - start) as f64 / 1_000.0).round() as i64)
                }
                _ => None,
            }
        };
        let us_to_ms =
            |us: Option<i64>| -> Option<i64> { us.map(|v| ((v as f64) / 1000.0).round() as i64) };
        let submit_ts = Self::_value_f64(rec2.get("order_submit_ts"))
            .or_else(|| Self::_value_f64(rec2.get("post_end_ts")))
            .unwrap_or(now);
        let send_ts = Self::_value_f64(rec2.get("post_start_ts")).unwrap_or(submit_ts);
        let decide_ts = Self::_value_f64(rec2.get("decision_ts")).unwrap_or(send_ts);
        let decision_ns = value_i64(rec2.get("decision_ns"));
        let prep_start_ns = value_i64(rec2.get("prep_start_ns"));
        let prep_end_ns = value_i64(rec2.get("prep_end_ns"));
        let sign_start_ns = value_i64(rec2.get("sign_start_ns"));
        let sign_end_ns = value_i64(rec2.get("sign_end_ns"));
        let post_start_ns = value_i64(rec2.get("post_start_ns"));
        let post_end_ns = value_i64(rec2.get("post_end_ns"));
        let mut prep_us = diff_us_ns(prep_start_ns, prep_end_ns).or_else(|| {
            rec2.get("prep_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
        });
        if prep_us.is_none() {
            prep_us = rec2
                .get("prep_ms")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .map(|ms| ms * 1000);
        }
        if prep_us.is_none() {
            let prep_start_ts = Self::_value_f64(rec2.get("prep_start_ts")).unwrap_or(0.0);
            let prep_end_ts = Self::_value_f64(rec2.get("prep_end_ts")).unwrap_or(0.0);
            if prep_start_ts > 0.0 && prep_end_ts > 0.0 {
                prep_us = self._lat_us(prep_end_ts, prep_start_ts);
            }
        }
        let prep_ms = us_to_ms(prep_us);
        let sign_us = diff_us_ns(sign_start_ns, sign_end_ns).or_else(|| {
            rec2.get("sign_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("sign_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });
        let sign_ms = us_to_ms(sign_us);
        let sign_total_us: Option<i64> = if let (Some(p), Some(s)) = (prep_us, sign_us) {
            Some(p + s)
        } else {
            rec2.get("sign_total_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("sign_total_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        };
        let sign_total_ms = us_to_ms(sign_total_us);
        let mut decide_to_send_us = diff_us_ns(decision_ns, post_start_ns).or_else(|| {
            rec2.get("decision_to_post_start_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("decision_to_post_start_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });
        let mut send_to_ack_us = diff_us_ns(post_start_ns, post_end_ns).or_else(|| {
            rec2.get("post_start_to_post_end_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("post_start_to_post_end_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });
        let mut decide_to_ack_us = diff_us_ns(decision_ns, post_end_ns).or_else(|| {
            rec2.get("decision_to_post_end_us")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                .or_else(|| {
                    rec2.get("decision_to_post_end_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x.round() as i64)))
                        .map(|ms| ms * 1000)
                })
        });
        if decide_to_send_us.is_none() {
            decide_to_send_us = self._lat_us(send_ts, decide_ts);
        }
        if send_to_ack_us.is_none() {
            send_to_ack_us = self._lat_us(submit_ts, send_ts);
        }
        if decide_to_ack_us.is_none() {
            decide_to_ack_us = self._lat_us(submit_ts, decide_ts);
        }
        let decide_to_send_ms = us_to_ms(decide_to_send_us);
        let send_to_ack_ms = us_to_ms(send_to_ack_us);
        let decide_to_ack_ms = us_to_ms(decide_to_ack_us);
        let asset_id = rec2
            .get("asset_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let side = rec2
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let origin = rec2
            .get("origin")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let liquidity_intent = rec2
            .get("liquidity_intent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let taker_exception_reason = rec2
            .get("taker_exception_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let pair_id = rec2
            .get("pair_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let px_limit = Self::_value_f64(rec2.get("px_limit"));
        let size = Self::_value_f64(rec2.get("size"));
        if let Some(obj) = rec2.as_object_mut() {
            obj.insert("order_id".to_string(), json!(order_id));
            obj.insert("order_submit_ts".to_string(), json!(submit_ts));
            obj.insert("post_end_ts".to_string(), json!(submit_ts));
            obj.insert("post_start_ts".to_string(), json!(send_ts));
            obj.insert("decision_ts".to_string(), json!(decide_ts));
            obj.insert("decision_ns".to_string(), json!(decision_ns));
            obj.insert("prep_start_ns".to_string(), json!(prep_start_ns));
            obj.insert("prep_end_ns".to_string(), json!(prep_end_ns));
            obj.insert("prep_us".to_string(), json!(prep_us));
            obj.insert("prep_ms".to_string(), json!(prep_ms));
            obj.insert("sign_start_ns".to_string(), json!(sign_start_ns));
            obj.insert("sign_end_ns".to_string(), json!(sign_end_ns));
            obj.insert("post_start_ns".to_string(), json!(post_start_ns));
            obj.insert("post_end_ns".to_string(), json!(post_end_ns));
            obj.insert("sign_us".to_string(), json!(sign_us));
            obj.insert("sign_ms".to_string(), json!(sign_ms));
            obj.insert("sign_total_us".to_string(), json!(sign_total_us));
            obj.insert("sign_total_ms".to_string(), json!(sign_total_ms));
            obj.insert(
                "decision_to_post_start_us".to_string(),
                json!(decide_to_send_us),
            );
            obj.insert(
                "decision_to_post_start_ms".to_string(),
                json!(decide_to_send_ms),
            );
            obj.insert(
                "post_start_to_post_end_us".to_string(),
                json!(send_to_ack_us),
            );
            obj.insert(
                "post_start_to_post_end_ms".to_string(),
                json!(send_to_ack_ms),
            );
            obj.insert(
                "decision_to_post_end_us".to_string(),
                json!(decide_to_ack_us),
            );
            obj.insert(
                "decision_to_post_end_ms".to_string(),
                json!(decide_to_ack_ms),
            );
            if !obj.contains_key("ts") {
                obj.insert("ts".to_string(), json!(now));
            }
        }
        if env_bool("EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE", true) {
            let em = self.exec_mode.trim().to_ascii_uppercase();
            let allow_maker = env_bool("EXEC_LATENCY_LOG_SUBMIT_BREAKDOWN_CONSOLE_MAKER", false);
            let allow = !(em == "MAKER"
                && !allow_maker
                && !origin.trim().to_ascii_uppercase().starts_with("TAKER"));
            if allow {
                let aid_tail: String = asset_id
                    .chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let d2s_us = decide_to_send_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let s2a_us = send_to_ack_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let d2a_us = decide_to_ack_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let pm_us = prep_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let sm_us = sign_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                let stm_us = sign_total_us
                    .map(|v| format!("{v}"))
                    .unwrap_or_else(|| "None".to_string());
                self.logger.info(&format!(
                    "[LATENCY][SUBMIT] pair_id={} decide->send={d2s_us}us send->ack={s2a_us}us decide->ack={d2a_us}us prep={pm_us}us sign={sm_us}us sign_total={stm_us}us oid={}.. asset={aid_tail} side={side} origin={origin} liquidity_intent={} taker_exception_reason={}",
                    pair_id,
                    order_id.chars().take(10).collect::<String>(),
                    if liquidity_intent.is_empty() {
                        "NA"
                    } else {
                        liquidity_intent.as_str()
                    },
                    if taker_exception_reason.is_empty() {
                        "NA"
                    } else {
                        taker_exception_reason.as_str()
                    },
                ));
            }
        }
        if self._should_file_log_submit_event(decide_ts) {
            let row = json!({
                "event": "SUBMIT",
                "ts": submit_ts,
                "ts_utc": self._utc_iso(submit_ts),
                "pair_id": rec2.get("pair_id").cloned().unwrap_or(Value::Null),
                "market_slug": rec2
                    .get("market_slug")
                    .cloned()
                    .unwrap_or_else(|| json!(self.market_slug)),
                "condition_id": rec2.get("condition_id").cloned().unwrap_or(Value::Null),
                "yes_asset_id": rec2.get("yes_asset_id").cloned().unwrap_or(Value::Null),
                "no_asset_id": rec2.get("no_asset_id").cloned().unwrap_or(Value::Null),
                "exec_mode": self.exec_mode,
                "order_id": order_id,
                "asset_id": asset_id,
                "side": side,
                "origin": origin,
                "liquidity_intent": rec2
                    .get("liquidity_intent")
                    .cloned()
                    .unwrap_or(Value::Null),
                "taker_exception_reason": rec2
                    .get("taker_exception_reason")
                    .cloned()
                    .unwrap_or(Value::Null),
                "source": "ORDER_SUBMIT",
                "price": px_limit,
                "qty": size,
                "decision_ts": decide_ts,
                "post_start_ts": send_ts,
                "post_end_ts": submit_ts,
                "order_submit_ts": submit_ts,
                "fill_ts": Value::Null,
                "prep_us": prep_us,
                "prep_ms": prep_ms,
                "sign_us": sign_us,
                "sign_ms": sign_ms,
                "sign_total_us": sign_total_us,
                "sign_total_ms": sign_total_ms,
                "decision_to_post_start_us": decide_to_send_us,
                "decision_to_post_start_ms": decide_to_send_ms,
                "post_start_to_post_end_us": send_to_ack_us,
                "post_start_to_post_end_ms": send_to_ack_ms,
                "decision_to_post_end_us": decide_to_ack_us,
                "decision_to_post_end_ms": decide_to_ack_ms,
                "post_start_to_fill_ms": Value::Null,
                "decision_to_fill_ms": Value::Null,
                "submit_to_fill_ms": Value::Null,
                "meta_json": rec2,
            });
            self._latency_file_append(&row);
        }
        if let Ok(mut map) = self.order_exec_context.lock() {
            map.insert(order_id.to_string(), rec2);
        }
        self._prune_order_exec_context_locked(now);
        self._audit_record_order_context_events(order_id);
    }
    /// Merges order execution context fields into the current BOT record.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _merge_order_execution_context_fields(&self, order_id: &str, fields: &Value) {
        let trimmed = order_id.trim();
        if trimmed.is_empty() {
            return;
        }
        let now = now_ts_f64();
        if let Ok(mut map) = self.order_exec_context.lock() {
            let mut merged = map
                .get(trimmed)
                .cloned()
                .unwrap_or_else(|| json!({ "order_id": trimmed }));
            if !merged.is_object() {
                merged = json!({});
            }
            if let (Some(dst), Some(src)) = (merged.as_object_mut(), fields.as_object()) {
                for (key, value) in src {
                    dst.insert(key.clone(), value.clone());
                }
                dst.entry("ts".to_string())
                    .or_insert_with(|| Value::from(now));
            }
            self._merge_pair_metadata_into_value(&mut merged);
            map.insert(trimmed.to_string(), merged);
        }
        self._prune_order_exec_context_locked(now);
        self._audit_record_order_context_events(trimmed);
    }
    /// Returns order execution context from the current BOT context.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _get_order_execution_context(&self, order_id: &str) -> Option<Value> {
        self.order_exec_context
            .lock()
            .ok()
            .and_then(|m| m.get(order_id).cloned())
    }
    /// Logs execution latency on fill for diagnostics and operator visibility.
    /// This reads execution state, exchange payloads, or cached order context for the active
    /// BOT runtime.

    pub fn _log_execution_latency_on_fill(&self, order_id: &str, fill_ts: f64) {
        if !env_bool("EXEC_LATENCY_LOG_ENABLED", true) {
            return;
        }
        if let Some(ctx) = self._get_order_execution_context(order_id) {
            let mut rec = json!({
                "ts_utc": self._utc_iso(fill_ts),
                "event": "FILL",
                "order_id": order_id,
                "fill_ts": fill_ts,
                "meta_json": ctx,
            });
            self._merge_pair_metadata_into_value(&mut rec);
            let submit_ts = rec
                .get("meta_json")
                .and_then(|m| m.get("order_submit_ts"))
                .and_then(|x| x.as_f64())
                .or_else(|| {
                    rec.get("meta_json")
                        .and_then(|m| m.get("post_end_ts"))
                        .and_then(|x| x.as_f64())
                });
            if let Some(submit_ts) = submit_ts {
                if let Some(ms) = self._lat_ms(fill_ts, submit_ts) {
                    rec["submit_to_fill_ms"] = json!(ms);
                }
            }
            let pair_id = rec
                .get("pair_id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if let Some(ms) = rec.get("submit_to_fill_ms").and_then(|x| x.as_i64()) {
                self.logger.info(&format!(
                    "[LATENCY][FILL] pair_id={} submit->fill={ms}ms oid={}..",
                    pair_id,
                    order_id.chars().take(10).collect::<String>()
                ));
            } else {
                self.logger.info(&format!(
                    "[LATENCY][FILL] pair_id={} no_timing_ctx oid={}..",
                    pair_id,
                    order_id.chars().take(10).collect::<String>()
                ));
            }
            self._latency_file_append(&rec);
        }
    }
}
