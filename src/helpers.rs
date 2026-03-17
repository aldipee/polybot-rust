use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;
use regex::Regex;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct SegmentDefaults {
    pub duration: i64,
    pub step: i64,
    pub stop_buffer: i64,
    pub warmup: i64,
}

pub fn segment_defaults(seg: &str) -> SegmentDefaults {
    match seg {
        "5M" => SegmentDefaults {
            duration: 6 * 60,
            step: 5 * 60,
            stop_buffer: 60,
            warmup: 1,
        },
        "15M" => SegmentDefaults {
            duration: 15 * 60,
            step: 15 * 60,
            stop_buffer: 120,
            warmup: 1,
        },
        "1H" => SegmentDefaults {
            duration: 60 * 60,
            step: 60 * 60,
            stop_buffer: 10 * 60,
            warmup: 1,
        },
        "4H" => SegmentDefaults {
            duration: 4 * 60 * 60,
            step: 4 * 60 * 60,
            stop_buffer: 20 * 60,
            warmup: 1,
        },
        "1D" => SegmentDefaults {
            duration: 24 * 60 * 60,
            step: 24 * 60 * 60,
            stop_buffer: 60 * 60,
            warmup: 1,
        },
        _ => segment_defaults("15M"),
    }
}

pub fn segment(name: &str) -> String {
    let n = name.trim().to_ascii_uppercase();
    match n.as_str() {
        "" => "15M".to_string(),
        "5" | "5MIN" | "5M" => "5M".to_string(),
        "15" | "15MIN" | "15M" => "15M".to_string(),
        "60" | "60MIN" | "1H" | "H" | "1HR" => "1H".to_string(),
        "240" | "240MIN" | "4H" | "4HR" => "4H".to_string(),
        "1D" | "D" | "DAY" | "DAILY" => "1D".to_string(),
        other => {
            let valid = ["5M", "15M", "1H", "4H", "1D"];
            if valid.contains(&other) {
                other.to_string()
            } else {
                "15M".to_string()
            }
        }
    }
}

pub fn canonical_pair_id_from_slug(slug: &str) -> String {
    slug.trim().to_ascii_lowercase()
}

fn normalize_market_slug_style(raw: &str) -> String {
    match raw.trim().to_ascii_uppercase().as_str() {
        "HUMAN" | "HUMAN_ET" | "ET" | "HUMAN-ET" | "HUMANET" => "HUMAN_ET".to_string(),
        _ => "TIMESTAMP".to_string(),
    }
}

fn normalize_market_asset_id(raw: &str) -> String {
    let mut out = raw.trim().to_ascii_lowercase();
    if out.is_empty() {
        return String::new();
    }
    if out.contains(',') {
        out = out.split(',').next().unwrap_or("").trim().to_string();
    }
    if out.contains('/') {
        out = out.split('/').next().unwrap_or("").trim().to_string();
    }
    if out.ends_with("-usd") && out.len() > 4 {
        out = out[..out.len() - 4].to_string();
    } else if out.ends_with("usd") && out.len() > 3 {
        out = out[..out.len() - 3].to_string();
    }
    if out.contains('-') {
        out = out.split('-').next().unwrap_or("").trim().to_string();
    }
    out = match out.as_str() {
        "bitcoin" => "btc".to_string(),
        "ethereum" => "eth".to_string(),
        "solana" => "sol".to_string(),
        "ripple" => "xrp".to_string(),
        "polygon" => "matic".to_string(),
        _ => out,
    };
    out.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn market_asset_slug_name(asset_id: &str) -> String {
    match asset_id {
        "btc" => "bitcoin".to_string(),
        "eth" => "ethereum".to_string(),
        "sol" => "solana".to_string(),
        "xrp" => "ripple".to_string(),
        "matic" => "polygon".to_string(),
        other => other.to_string(),
    }
}

pub fn infer_market_asset_id_from_env() -> Option<String> {
    for key in ["MARKET_SYMBOL", "RTDS_SYMBOL"] {
        if let Ok(raw) = env::var(key) {
            let asset = normalize_market_asset_id(&raw);
            if !asset.is_empty() {
                return Some(asset);
            }
        }
    }
    None
}

pub fn generate_market_slug_from_now(
    asset_hint: &str,
    segment_name: &str,
    step_seconds: i64,
) -> Option<String> {
    let asset = normalize_market_asset_id(asset_hint);
    if asset.is_empty() {
        return None;
    }
    generate_market_slug_from_now_with_style(&asset, segment_name, step_seconds, "TIMESTAMP")
}

fn startup_timestamp_slot_ts(
    now_ts: i64,
    step_seconds: i64,
    rollover_buffer_seconds: i64,
) -> Option<i64> {
    if step_seconds <= 0 {
        return None;
    }
    let slot_start_ts = now_ts - now_ts.rem_euclid(step_seconds);
    let elapsed_in_slot = now_ts.rem_euclid(step_seconds);
    let rollover_buffer_seconds = rollover_buffer_seconds.clamp(0, step_seconds.saturating_sub(1));
    if elapsed_in_slot > 0 {
        let remaining_in_slot = step_seconds - elapsed_in_slot;
        if remaining_in_slot <= rollover_buffer_seconds {
            return Some(slot_start_ts + step_seconds);
        }
    }
    Some(slot_start_ts)
}

pub fn generate_market_slug_from_now_with_style(
    asset_hint: &str,
    segment_name: &str,
    step_seconds: i64,
    slug_style: &str,
) -> Option<String> {
    let asset = normalize_market_asset_id(asset_hint);
    if asset.is_empty() {
        return None;
    }
    let seg = segment(segment_name);
    let style = normalize_market_slug_style(slug_style);

    if style == "HUMAN_ET" {
        let asset_name = market_asset_slug_name(&asset);
        let now_et = Utc::now().with_timezone(&New_York);
        if seg == "1H" {
            let hour_start = now_et.with_minute(0)?.with_second(0)?.with_nanosecond(0)?;
            let prefix = format!("{asset_name}-up-or-down-");
            return Some(format_1h_slug_et(&prefix, hour_start));
        }
        if seg == "1D" {
            let day_start = now_et
                .with_hour(0)?
                .with_minute(0)?
                .with_second(0)?
                .with_nanosecond(0)?;
            let prefix = format!("{asset_name}-up-or-down-on-");
            return Some(format_1d_slug_et(&prefix, day_start));
        }
    }

    let seg_slug = match seg.as_str() {
        "5M" => "5m",
        "15M" => "15m",
        "1H" => "1h",
        "4H" => "4h",
        "1D" => "1d",
        _ => "15m",
    };
    let default_step = segment_defaults(&seg).step;
    let step = if step_seconds > 0 {
        step_seconds
    } else {
        default_step
    };
    if step <= 0 {
        return None;
    }
    let now_ts = Utc::now().timestamp();
    let rollover_buffer_seconds = env::var("STOP_BUFFER_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(segment_defaults(&seg).stop_buffer);
    let slot_ts = startup_timestamp_slot_ts(now_ts, step, rollover_buffer_seconds)?;
    Some(format!("{asset}-updown-{seg_slug}-{slot_ts}"))
}

pub fn generate_market_slug_from_env_now(segment_name: &str, step_seconds: i64) -> Option<String> {
    let asset = infer_market_asset_id_from_env()?;
    let style = env::var("MARKET_SLUG_STYLE").unwrap_or_default();
    if normalize_market_slug_style(&style) == "HUMAN_ET" {
        generate_market_slug_from_now_with_style(&asset, segment_name, step_seconds, &style)
    } else {
        generate_market_slug_from_now(&asset, segment_name, step_seconds)
    }
}

pub fn iso_to_epoch(s: &str) -> Option<i64> {
    if s.trim().is_empty() {
        return None;
    }
    let fixed = s.replace('Z', "+00:00");
    if let Ok(dt) = DateTime::parse_from_rfc3339(&fixed) {
        return Some(dt.timestamp());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc);
        return Some(dt.timestamp());
    }
    None
}

fn infer_year_et() -> i32 {
    Utc::now().with_timezone(&New_York).year()
}

fn month_name_to_num(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "january" => Some(1),
        "february" => Some(2),
        "march" => Some(3),
        "april" => Some(4),
        "may" => Some(5),
        "june" => Some(6),
        "july" => Some(7),
        "august" => Some(8),
        "september" => Some(9),
        "october" => Some(10),
        "november" => Some(11),
        "december" => Some(12),
        _ => None,
    }
}

fn month_num_to_name(month: u32) -> &'static str {
    match month {
        1 => "january",
        2 => "february",
        3 => "march",
        4 => "april",
        5 => "may",
        6 => "june",
        7 => "july",
        8 => "august",
        9 => "september",
        10 => "october",
        11 => "november",
        12 => "december",
        _ => "january",
    }
}

fn re_1h() -> Regex {
    Regex::new(
        r"^(?P<prefix>.+-)(?P<month>january|february|march|april|may|june|july|august|september|october|november|december)-(?P<day>\d{1,2})-(?P<hour>\d{1,2})(?P<ampm>am|pm)-et$",
    )
    .expect("invalid re_1h")
}

fn re_1d() -> Regex {
    Regex::new(
        r"^(?P<prefix>.+-on-)(?P<month>january|february|march|april|may|june|july|august|september|october|november|december)-(?P<day>\d{1,2})$",
    )
    .expect("invalid re_1d")
}

pub fn parse_1h_slug_et(slug: &str) -> Option<DateTime<chrono_tz::Tz>> {
    let re = re_1h();
    let caps = re.captures(slug)?;
    let month = month_name_to_num(caps.name("month")?.as_str())?;
    let day = caps.name("day")?.as_str().parse::<u32>().ok()?;
    let mut hour = caps.name("hour")?.as_str().parse::<u32>().ok()?;
    let ampm = caps.name("ampm")?.as_str().to_ascii_lowercase();
    if hour == 12 {
        hour = 0;
    }
    if ampm == "pm" {
        hour += 12;
    }
    let year = infer_year_et();
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let time = NaiveTime::from_hms_opt(hour, 0, 0)?;
    let dt = New_York
        .from_local_datetime(&NaiveDateTime::new(date, time))
        .single()?;
    Some(dt)
}

pub fn format_1h_slug_et(prefix: &str, dt_et: DateTime<chrono_tz::Tz>) -> String {
    let month_name = month_num_to_name(dt_et.month());
    let day = dt_et.day();
    let hour24 = dt_et.hour();
    let ampm = if hour24 < 12 { "am" } else { "pm" };
    let mut hour12 = hour24 % 12;
    if hour12 == 0 {
        hour12 = 12;
    }
    format!("{prefix}{month_name}-{day}-{hour12}{ampm}-et")
}

pub fn parse_1d_slug_et(slug: &str) -> Option<DateTime<chrono_tz::Tz>> {
    let re = re_1d();
    let caps = re.captures(slug)?;
    let month = month_name_to_num(caps.name("month")?.as_str())?;
    let day = caps.name("day")?.as_str().parse::<u32>().ok()?;
    let year = infer_year_et();
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let time = NaiveTime::from_hms_opt(0, 0, 0)?;
    let dt = New_York
        .from_local_datetime(&NaiveDateTime::new(date, time))
        .single()?;
    Some(dt)
}

pub fn format_1d_slug_et(prefix: &str, dt_et: DateTime<chrono_tz::Tz>) -> String {
    let month_name = month_num_to_name(dt_et.month());
    let day = dt_et.day();
    format!("{prefix}{month_name}-{day}")
}

pub fn increment_human_slug(slug: &str, segment_name: &str) -> Option<String> {
    let seg = segment(segment_name);
    if seg == "1H" {
        let re = re_1h();
        let caps = re.captures(slug)?;
        let prefix = caps.name("prefix")?.as_str();
        let dt = parse_1h_slug_et(slug)?;
        let dt2 = dt + chrono::Duration::hours(1);
        return Some(format_1h_slug_et(prefix, dt2));
    }
    if seg == "1D" {
        let re = re_1d();
        let caps = re.captures(slug)?;
        let prefix = caps.name("prefix")?.as_str();
        let dt = parse_1d_slug_et(slug)?;
        let dt2 = dt + chrono::Duration::days(1);
        return Some(format_1d_slug_et(prefix, dt2));
    }
    None
}

pub fn get_next_slug(current_slug: &str) -> String {
    let seg = segment(&env::var("MARKET_SEGMENT").unwrap_or_else(|_| "15M".to_string()));
    let default_step = segment_defaults(&seg).step;
    let step = env::var("MARKET_STEP_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default_step);

    let mut parts: Vec<String> = current_slug.split('-').map(|s| s.to_string()).collect();
    if let Some(last) = parts.last().cloned() {
        if let Ok(ts) = last.parse::<i64>() {
            if let Some(slot) = parts.last_mut() {
                *slot = (ts + step).to_string();
            }
            return parts.join("-");
        }
    }

    increment_human_slug(current_slug, &seg).unwrap_or_else(|| current_slug.to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenOrderState {
    pub order_id: Option<String>,
    pub price: Option<f64>,
    pub size: Option<f64>,
    pub ts: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotState {
    pub q_yes: f64,
    pub q_no: f64,
    pub c_yes: f64,
    pub c_no: f64,
    pub seen_trade_keys: Vec<String>,
    pub open_orders: HashMap<String, OpenOrderState>,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            q_yes: 0.0,
            q_no: 0.0,
            c_yes: 0.0,
            c_no: 0.0,
            seen_trade_keys: Vec::new(),
            open_orders: HashMap::new(),
        }
    }
}

impl BotState {
    pub fn normalize(&mut self) {
        if self.open_orders.is_empty() {
            self.open_orders = HashMap::new();
        }
    }
}

pub fn load_state(state_file: &Path) -> Result<BotState> {
    if state_file.exists() {
        let raw = fs::read_to_string(state_file)
            .with_context(|| format!("failed reading state file {}", state_file.display()))?;
        let mut s: BotState = serde_json::from_str(&raw)
            .with_context(|| format!("failed parsing state JSON {}", state_file.display()))?;
        s.normalize();
        return Ok(s);
    }
    Ok(BotState::default())
}

pub fn save_state(state_file: &Path, state: &mut BotState) -> Result<()> {
    if state.seen_trade_keys.len() > 5000 {
        let start = state.seen_trade_keys.len().saturating_sub(2000);
        state.seen_trade_keys = state.seen_trade_keys[start..].to_vec();
    }
    let raw = serde_json::to_string_pretty(state)?;
    fs::write(state_file, raw)
        .with_context(|| format!("failed writing state file {}", state_file.display()))?;
    Ok(())
}

pub fn locked_profit(state: &BotState) -> f64 {
    let q_pair = state.q_yes.min(state.q_no);
    q_pair - (state.c_yes + state.c_no)
}

pub fn cost_per_pair(state: &BotState) -> f64 {
    let q_pair = state.q_yes.min(state.q_no);
    if q_pair <= 0.0 {
        return f64::INFINITY;
    }
    (state.c_yes + state.c_no) / q_pair
}

pub fn round_down(x: f64, tick: f64) -> f64 {
    (x / tick + 1e-12).floor() * tick
}

pub fn round_up(x: f64, tick: f64) -> f64 {
    (x / tick - 1e-12).ceil() * tick
}

pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

fn decimal_from_float_string(x: f64) -> Decimal {
    Decimal::from_str_exact(&format!("{x}")).unwrap_or(Decimal::ZERO)
}

pub fn _D(x: f64) -> Decimal {
    decimal_from_float_string(x)
}

pub fn q_down(x: f64, dp: u32) -> f64 {
    decimal_from_float_string(x)
        .round_dp_with_strategy(dp, RoundingStrategy::ToZero)
        .to_string()
        .parse::<f64>()
        .unwrap_or(x)
}

pub fn q_up(x: f64, dp: u32) -> f64 {
    decimal_from_float_string(x)
        .round_dp_with_strategy(dp, RoundingStrategy::AwayFromZero)
        .to_string()
        .parse::<f64>()
        .unwrap_or(x)
}

// Python-name compatibility wrappers (for port traceability).
pub fn _segment(name: &str) -> String {
    segment(name)
}

pub fn _iso_to_epoch(s: &str) -> Option<i64> {
    iso_to_epoch(s)
}

pub fn _infer_year_et() -> i32 {
    infer_year_et()
}

pub fn _parse_1h_slug_et(slug: &str) -> Option<DateTime<chrono_tz::Tz>> {
    parse_1h_slug_et(slug)
}

pub fn _format_1h_slug_et(prefix: &str, dt_et: DateTime<chrono_tz::Tz>) -> String {
    format_1h_slug_et(prefix, dt_et)
}

pub fn _parse_1d_slug_et(slug: &str) -> Option<DateTime<chrono_tz::Tz>> {
    parse_1d_slug_et(slug)
}

pub fn _format_1d_slug_et(prefix: &str, dt_et: DateTime<chrono_tz::Tz>) -> String {
    format_1d_slug_et(prefix, dt_et)
}

pub fn _increment_human_slug(slug: &str, segment_name: &str) -> Option<String> {
    increment_human_slug(slug, segment_name)
}

#[cfg(test)]
mod tests {
    use super::startup_timestamp_slot_ts;

    #[test]
    fn startup_timestamp_slot_keeps_current_slot_when_outside_rollover_buffer() {
        assert_eq!(
            startup_timestamp_slot_ts(1_770_000_200, 300, 60),
            Some(1_770_000_000)
        );
    }

    #[test]
    fn startup_timestamp_slot_uses_next_slot_inside_rollover_buffer() {
        assert_eq!(
            startup_timestamp_slot_ts(1_770_000_299, 300, 60),
            Some(1_770_000_300)
        );
        assert_eq!(
            startup_timestamp_slot_ts(1_770_000_240, 300, 60),
            Some(1_770_000_300)
        );
    }

    #[test]
    fn startup_timestamp_slot_keeps_exact_boundary_on_current_slot() {
        assert_eq!(
            startup_timestamp_slot_ts(1_770_000_300, 300, 60),
            Some(1_770_000_300)
        );
    }

    #[test]
    fn startup_timestamp_slot_clamps_large_rollover_buffer() {
        assert_eq!(
            startup_timestamp_slot_ts(1_770_000_001, 300, 999),
            Some(1_770_000_300)
        );
    }
}
