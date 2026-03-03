use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;

use super::SignalTrade;

#[derive(Debug, Clone)]
pub struct CopyTradingLogic {
    wallet_filters: HashSet<String>,
    market_slug_filters: HashSet<String>,
    event_slug_filters: HashSet<String>,
    min_trade_size: f64,
    buy_only: bool,
    subscribe_payload: Option<String>,
}

impl CopyTradingLogic {
    pub fn new(ws_url: &str) -> Self {
        let wallet_filters = Self::parse_csv_set(Self::env_first(&[
            "SIGNAL_COPY_WALLETS",
            "COPYTRADE_WALLETS",
        ]));
        let market_slug_filters = Self::parse_csv_set(Self::env_first(&[
            "SIGNAL_COPY_MARKET_SLUGS",
            "COPYTRADE_MARKET_SLUGS",
        ]));
        let event_slug_filters = Self::parse_csv_set(Self::env_first(&[
            "SIGNAL_COPY_EVENT_SLUGS",
            "COPYTRADE_EVENT_SLUGS",
        ]));
        let min_trade_size =
            Self::env_f64(&["SIGNAL_COPY_MIN_SIZE", "COPYTRADE_MIN_SIZE"], 0.0).max(0.0);
        let buy_only = Self::env_bool(&["SIGNAL_COPY_BUY_ONLY", "COPYTRADE_BUY_ONLY"], false);
        let subscribe_payload =
            Self::build_subscription_payload(ws_url, &market_slug_filters, &event_slug_filters);

        Self {
            wallet_filters,
            market_slug_filters,
            event_slug_filters,
            min_trade_size,
            buy_only,
            subscribe_payload,
        }
    }

    pub fn subscription_payload(&self) -> Option<String> {
        self.subscribe_payload.clone()
    }

    pub fn extract_signal(&self, msg: &Value, received_ts: f64) -> Option<SignalTrade> {
        self.extract_rtds_trade(msg, received_ts)
            .or_else(|| self.extract_legacy_trade(msg, received_ts))
    }

    fn extract_rtds_trade(&self, msg: &Value, received_ts: f64) -> Option<SignalTrade> {
        let topic = Self::value_string(msg.get("topic"))
            .trim()
            .to_ascii_lowercase();
        let mtype = Self::value_string(msg.get("type"))
            .trim()
            .to_ascii_lowercase();
        if topic != "activity" || mtype != "trades" {
            return None;
        }

        let payload = msg.get("payload")?;
        if !payload.is_object() {
            return None;
        }

        let wallet_raw = payload
            .get("proxyWallet")
            .or_else(|| payload.get("proxy_wallet"))
            .or_else(|| payload.get("wallet"))
            .or_else(|| payload.get("userAddress"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if wallet_raw.is_empty() {
            return None;
        }
        let wallet = wallet_raw.to_ascii_lowercase();
        if !self.wallet_filters.is_empty() && !self.wallet_filters.contains(&wallet) {
            return None;
        }

        let market_slug = payload
            .get("slug")
            .or_else(|| payload.get("market_slug"))
            .or_else(|| payload.get("marketSlug"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if market_slug.is_empty() {
            return None;
        }
        if !self.market_slug_filters.is_empty()
            && !self
                .market_slug_filters
                .contains(&market_slug.to_ascii_lowercase())
        {
            return None;
        }

        let event_slug = payload
            .get("eventSlug")
            .or_else(|| payload.get("event_slug"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !self.event_slug_filters.is_empty()
            && (event_slug.is_empty()
                || !self
                    .event_slug_filters
                    .contains(&event_slug.to_ascii_lowercase()))
        {
            return None;
        }

        let side = Self::value_string(payload.get("side"))
            .trim()
            .to_ascii_uppercase();
        if self.buy_only && side != "BUY" {
            return None;
        }

        let size = Self::as_f64(payload.get("size")).max(0.0);
        if size <= 0.0 {
            return None;
        }
        if self.min_trade_size > 0.0 && size + 1e-12 < self.min_trade_size {
            return None;
        }

        let outcome = Self::value_string(payload.get("outcome"));
        let direction = match (Self::normalize_direction(&outcome), side.as_str()) {
            (Some(dir), "SELL") => Self::flip_direction(&dir),
            (Some(dir), _) => dir,
            (None, _) => Self::normalize_direction(&side)?,
        };

        let entry_price = Self::as_f64(payload.get("price")).max(0.0);
        let confidence = {
            let c = Self::as_f64(payload.get("confidence"));
            if c > 0.0 {
                c
            } else {
                1.0
            }
        };

        let event_timestamp = {
            let ts = Self::value_string(payload.get("timestamp"));
            if ts.is_empty() {
                Self::value_string(msg.get("timestamp"))
            } else {
                ts
            }
        };

        let tx_hash = Self::value_string(
            payload
                .get("transactionHash")
                .or_else(|| payload.get("transaction_hash"))
                .or_else(|| payload.get("txHash")),
        )
        .to_ascii_lowercase();
        let key = if !tx_hash.is_empty() {
            format!(
                "RTDS|{tx_hash}|{wallet}|{}|{}|{}|{:.8}|{:.8}",
                market_slug.to_ascii_lowercase(),
                side,
                direction,
                entry_price,
                size
            )
        } else {
            format!(
                "RTDS|{wallet}|{}|{}|{}|{}|{:.8}|{:.8}",
                market_slug.to_ascii_lowercase(),
                side,
                direction,
                event_timestamp,
                entry_price,
                size
            )
        };

        Some(SignalTrade {
            provider: "COPYTRADE_RTDS".to_string(),
            key,
            market_slug,
            direction,
            confidence,
            entry_price,
            event_timestamp,
            raw: Some(msg.clone()),
            received_ts,
        })
    }

    fn extract_legacy_trade(&self, msg: &Value, received_ts: f64) -> Option<SignalTrade> {
        let mtype = msg
            .get("type")
            .or_else(|| msg.get("event"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mtype != "trade" {
            return None;
        }
        let trade = msg.get("trade")?;
        if !trade.is_object() {
            return None;
        }
        let status = trade
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        if !status.is_empty() && status != "SIGNAL" && status != "TRADE" && status != "OPEN" {
            return None;
        }

        let market_slug = trade
            .get("market_slug")
            .or_else(|| trade.get("market"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let direction = trade
            .get("direction")
            .or_else(|| trade.get("side"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();
        if market_slug.is_empty() || direction.is_empty() {
            return None;
        }

        let event_timestamp = {
            let ts = trade
                .get("event_timestamp")
                .or_else(|| trade.get("timestamp"))
                .or_else(|| trade.get("created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if ts.is_empty() {
                Self::value_string(trade.get("timestamp"))
            } else {
                ts
            }
        };
        let key = trade
            .get("id")
            .or_else(|| trade.get("discord_message_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{market_slug}|{direction}|{event_timestamp}"));

        Some(SignalTrade {
            provider: trade
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("WEBSOCKET")
                .trim()
                .to_ascii_uppercase(),
            key,
            market_slug,
            direction,
            confidence: Self::as_f64(trade.get("confidence")),
            entry_price: Self::as_f64(trade.get("entry_price")),
            event_timestamp,
            raw: Some(msg.clone()),
            received_ts,
        })
    }

    fn normalize_direction(raw: &str) -> Option<String> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "YES" | "UP" | "LONG" | "BUY" | "BULL" => Some("YES".to_string()),
            "NO" | "DOWN" | "SHORT" | "SELL" | "BEAR" => Some("NO".to_string()),
            _ => None,
        }
    }

    fn flip_direction(direction: &str) -> String {
        if matches!(direction.trim().to_ascii_uppercase().as_str(), "YES" | "UP") {
            "NO".to_string()
        } else {
            "YES".to_string()
        }
    }

    fn build_subscription_payload(
        ws_url: &str,
        market_slug_filters: &HashSet<String>,
        event_slug_filters: &HashSet<String>,
    ) -> Option<String> {
        if let Some(raw) = Self::env_first(&["SIGNAL_WS_SUBSCRIBE_JSON"]) {
            return Some(raw);
        }
        if !Self::is_rtds_url(ws_url) {
            return None;
        }

        let topic = Self::env_first(&["SIGNAL_WS_TOPIC"]).unwrap_or_else(|| "activity".to_string());
        let sub_type = Self::env_first(&["SIGNAL_WS_TYPE"]).unwrap_or_else(|| "trades".to_string());
        let mut sub = json!({
            "topic": topic,
            "type": sub_type,
        });

        if let Some(filter_json) =
            Self::build_subscription_filter(market_slug_filters, event_slug_filters)
        {
            if let Some(obj) = sub.as_object_mut() {
                obj.insert("filters".to_string(), Value::String(filter_json));
            }
        }

        Some(
            json!({
                "action": "subscribe",
                "subscriptions": [sub],
            })
            .to_string(),
        )
    }

    fn build_subscription_filter(
        market_slug_filters: &HashSet<String>,
        event_slug_filters: &HashSet<String>,
    ) -> Option<String> {
        let market = Self::env_first(&[
            "SIGNAL_COPY_SUBSCRIBE_MARKET_SLUG",
            "SIGNAL_COPY_MARKET_SLUG",
        ])
        .or_else(|| Self::single_set_value(market_slug_filters));
        if let Some(market_slug) = market {
            return serde_json::to_string(&json!({ "market_slug": market_slug })).ok();
        }

        let event_slug =
            Self::env_first(&["SIGNAL_COPY_SUBSCRIBE_EVENT_SLUG", "SIGNAL_COPY_EVENT_SLUG"])
                .or_else(|| Self::single_set_value(event_slug_filters));
        if let Some(event_slug) = event_slug {
            return serde_json::to_string(&json!({ "event_slug": event_slug })).ok();
        }
        None
    }

    fn is_rtds_url(ws_url: &str) -> bool {
        ws_url
            .trim()
            .to_ascii_lowercase()
            .contains("ws-live-data.polymarket.com")
    }

    fn single_set_value(values: &HashSet<String>) -> Option<String> {
        if values.len() == 1 {
            values.iter().next().cloned()
        } else {
            None
        }
    }

    fn parse_csv_set(raw: Option<String>) -> HashSet<String> {
        raw.unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn env_first(keys: &[&str]) -> Option<String> {
        for key in keys {
            if let Ok(v) = env::var(key) {
                let vv = v.trim();
                if !vv.is_empty() {
                    return Some(vv.to_string());
                }
            }
        }
        None
    }

    fn env_bool(keys: &[&str], default: bool) -> bool {
        let raw = match Self::env_first(keys) {
            Some(v) => v,
            None => return default,
        };
        matches!(
            raw.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "y" | "on"
        )
    }

    fn env_f64(keys: &[&str], default: f64) -> f64 {
        Self::env_first(keys)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(default)
    }

    fn as_f64(v: Option<&Value>) -> f64 {
        match v {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
            Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    fn value_string(v: Option<&Value>) -> String {
        match v {
            Some(Value::String(s)) => s.trim().to_string(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        }
    }
}
