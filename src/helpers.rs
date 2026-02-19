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
    pub seen_signal_keys: Vec<String>,
    pub open_orders: HashMap<String, OpenOrderState>,
    pub sniper_trade_count: i64,
    pub sniper_last_entry_ts: f64,
    pub sniper_last_exit_ts: f64,
    pub sniper_last_side: String,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            q_yes: 0.0,
            q_no: 0.0,
            c_yes: 0.0,
            c_no: 0.0,
            seen_trade_keys: Vec::new(),
            seen_signal_keys: Vec::new(),
            open_orders: HashMap::new(),
            sniper_trade_count: 0,
            sniper_last_entry_ts: 0.0,
            sniper_last_exit_ts: 0.0,
            sniper_last_side: String::new(),
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
    if state.seen_signal_keys.len() > 5000 {
        let start = state.seen_signal_keys.len().saturating_sub(2000);
        state.seen_signal_keys = state.seen_signal_keys[start..].to_vec();
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
