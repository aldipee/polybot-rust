#![recursion_limit = "256"]

mod binance_feed;
mod bot;
mod config;
mod db;
mod env_contract;
mod env_utils;
mod gamma;
mod helpers;
mod logging;
mod r2_storage;
mod rtds;
mod signal;
mod sniper_filters;

use anyhow::{anyhow, Context, Result};
use bot::MakerHedgeCapBot;
use chrono::{Duration as ChronoDuration, Utc};
use chrono_tz::Asia::Jakarta;
use config::BotConfig;
use db::{
    date_jakarta, make_engine, make_session_factory, month_start_date_jakarta, now_iso_jakarta,
    week_start_date_jakarta, BotRepository, BotTradeStats, ConfigurationRow,
};
use env_utils::{env_bool, env_float, env_int};
use gamma::fetch_market_by_slug;
use helpers::{generate_market_slug_from_env_now, get_next_slug, segment, segment_defaults};
use logging::{setup_item_logger, LogLike};
use r2_storage::upload_logs_before_rollover;
use reqwest::blocking::Client;
use rtds::{get_resolution_snapshot_for_market, RtdsService};
use serde::Serialize;
use serde_json::Value;
use signal::{JsonlFileService, SignalHub, SignalInbox};
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn build_version() -> String {
    let commit_id = option_env!("GIT_COMMIT_ID").unwrap_or("unknown");
    format!("{}+{commit_id}", env!("CARGO_PKG_VERSION"))
}

fn db_url_hint(db_url: &str) -> String {
    if db_url.starts_with("sqlite://") {
        return db_url.to_string();
    }
    if let Some((scheme, _)) = db_url.split_once("://") {
        return format!("{scheme}://<redacted>");
    }
    "<invalid-db-url>".to_string()
}

fn holding_duration_seconds(entry_iso: &str, exit_iso: &str) -> Option<f64> {
    let start = chrono::DateTime::parse_from_rfc3339(entry_iso).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(exit_iso).ok()?;
    let ms = (end - start).num_milliseconds();
    Some((ms.max(0) as f64) / 1000.0)
}

fn analytics_exit_reason(raw_reason: &str) -> String {
    let reason = raw_reason.trim().to_ascii_uppercase();
    if reason.contains("STOP_LOSS") || reason.contains("CAP_LOCKED_LOSS") {
        "STOP_LOSS".to_string()
    } else if reason.contains("TAKE_PROFIT") || reason.contains("TARGET_HIT") {
        "TAKE_PROFIT".to_string()
    } else {
        "RESOLUTION".to_string()
    }
}

#[derive(Debug, Clone, Default)]
struct PnlWindowStats {
    h1: BotTradeStats,
    h3: BotTradeStats,
    h6: BotTradeStats,
    h12: BotTradeStats,
    day: BotTradeStats,
    week: BotTradeStats,
    month: BotTradeStats,
    all: BotTradeStats,
}

#[derive(Debug, Clone)]
struct StatsBounds {
    today: String,
    week_start: String,
    month_start: String,
    cutoff_1h: String,
    cutoff_3h: String,
    cutoff_6h: String,
    cutoff_12h: String,
}

fn stats_bounds_now() -> StatsBounds {
    let now_jkt = Utc::now().with_timezone(&Jakarta);
    StatsBounds {
        today: date_jakarta(),
        week_start: week_start_date_jakarta(),
        month_start: month_start_date_jakarta(),
        cutoff_1h: (now_jkt - ChronoDuration::hours(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        cutoff_3h: (now_jkt - ChronoDuration::hours(3))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        cutoff_6h: (now_jkt - ChronoDuration::hours(6))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        cutoff_12h: (now_jkt - ChronoDuration::hours(12))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

fn pct(part: i64, total: i64) -> f64 {
    if total <= 0 {
        0.0
    } else {
        (part as f64 * 100.0) / total as f64
    }
}

fn pnl_line(label: &str, s: &BotTradeStats) -> String {
    let win_rate = pct(s.win_count, s.total_count);
    format!(
        "  {label:<7}: NET={:+.4} PROFIT={:+.4} LOSS={:+.4} | WON={} ({:.2}%) LOSS={} ({:.2}%) | WIN_RATE={:.2}% ({}/{})",
        s.net_pnl,
        s.total_profit,
        s.total_loss,
        s.win_count,
        pct(s.win_count, s.total_count),
        s.loss_count,
        pct(s.loss_count, s.total_count),
        win_rate,
        s.win_count,
        s.total_count
    )
}

fn bot_window_stats(repo: &BotRepository, bot_id: &str, b: &StatsBounds) -> PnlWindowStats {
    PnlWindowStats {
        h1: repo
            .trade_stats_for_bot_recent_hours(bot_id, &b.cutoff_1h)
            .unwrap_or_default(),
        h3: repo
            .trade_stats_for_bot_recent_hours(bot_id, &b.cutoff_3h)
            .unwrap_or_default(),
        h6: repo
            .trade_stats_for_bot_recent_hours(bot_id, &b.cutoff_6h)
            .unwrap_or_default(),
        h12: repo
            .trade_stats_for_bot_recent_hours(bot_id, &b.cutoff_12h)
            .unwrap_or_default(),
        day: repo
            .trade_stats_for_bot_period(bot_id, &b.today, &b.today)
            .unwrap_or_default(),
        week: repo
            .trade_stats_for_bot_period(bot_id, &b.week_start, &b.today)
            .unwrap_or_default(),
        month: repo
            .trade_stats_for_bot_period(bot_id, &b.month_start, &b.today)
            .unwrap_or_default(),
        all: repo
            .trade_stats_for_bot_all_time(bot_id)
            .unwrap_or_default(),
    }
}

fn all_bots_window_stats(repo: &BotRepository, b: &StatsBounds) -> PnlWindowStats {
    PnlWindowStats {
        h1: repo
            .trade_stats_all_bots_recent_hours(&b.cutoff_1h)
            .unwrap_or_default(),
        h3: repo
            .trade_stats_all_bots_recent_hours(&b.cutoff_3h)
            .unwrap_or_default(),
        h6: repo
            .trade_stats_all_bots_recent_hours(&b.cutoff_6h)
            .unwrap_or_default(),
        h12: repo
            .trade_stats_all_bots_recent_hours(&b.cutoff_12h)
            .unwrap_or_default(),
        day: repo
            .trade_stats_all_bots_period(&b.today, &b.today)
            .unwrap_or_default(),
        week: repo
            .trade_stats_all_bots_period(&b.week_start, &b.today)
            .unwrap_or_default(),
        month: repo
            .trade_stats_all_bots_period(&b.month_start, &b.today)
            .unwrap_or_default(),
        all: repo.trade_stats_all_bots_all_time().unwrap_or_default(),
    }
}

fn pnl_section(label: &str, s: &PnlWindowStats) -> String {
    format!(
        "{label}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        pnl_line("1h", &s.h1),
        pnl_line("3h", &s.h3),
        pnl_line("6h", &s.h6),
        pnl_line("12h", &s.h12),
        pnl_line("Daily", &s.day),
        pnl_line("Weekly", &s.week),
        pnl_line("Monthly", &s.month),
        pnl_line("All", &s.all)
    )
}

fn telegram_pnl_line(label: &str, s: &BotTradeStats) -> String {
    let win_rate = pct(s.win_count, s.total_count);
    format!(
        "  {label}: NET {:+.4} | W {} L {} | WR {:.2}% | P {:+.4} L {:+.4}",
        s.net_pnl, s.win_count, s.loss_count, win_rate, s.total_profit, s.total_loss
    )
}

fn telegram_pnl_section(title: &str, s: &PnlWindowStats) -> String {
    format!(
        "{title}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        telegram_pnl_line("1h", &s.h1),
        telegram_pnl_line("3h", &s.h3),
        telegram_pnl_line("6h", &s.h6),
        telegram_pnl_line("12h", &s.h12),
        telegram_pnl_line("1d", &s.day),
        telegram_pnl_line("1w", &s.week),
        telegram_pnl_line("1m", &s.month),
        telegram_pnl_line("all", &s.all)
    )
}

fn print_pnl_metrics(repo: &BotRepository, bot_id: &str, logger: &Arc<dyn LogLike>) -> String {
    let bounds = stats_bounds_now();
    let bot_stats = bot_window_stats(repo, bot_id, &bounds);
    let all_stats = all_bots_window_stats(repo, &bounds);
    let msg = format!(
        "PNL Summary (Asia/Jakarta, DRAW excluded)\n{}\n{}",
        pnl_section(&format!("Bot {bot_id}"), &bot_stats),
        pnl_section("ALL bots", &all_stats)
    );
    logger.info(&msg);
    msg
}

fn build_telegram_pnl_summary(
    repo: &BotRepository,
    current_bot_id: &str,
    logger: &Arc<dyn LogLike>,
) -> String {
    let bounds = stats_bounds_now();
    let bot_stats = bot_window_stats(repo, current_bot_id, &bounds);
    let all_stats = all_bots_window_stats(repo, &bounds);
    let mut bot_ids = match repo.list_all_bot_ids() {
        Ok(v) => v,
        Err(e) => {
            logger.warning(&format!(
                "[TELEGRAM] failed loading bot id list for per-bot breakdown: {e:#}"
            ));
            Vec::new()
        }
    };
    if !bot_ids
        .iter()
        .any(|id| id.trim().eq_ignore_ascii_case(current_bot_id))
    {
        bot_ids.push(current_bot_id.to_string());
    }
    bot_ids.retain(|id| !id.trim().is_empty());
    bot_ids.sort();
    bot_ids.dedup();

    let mut parts = vec![
        "PNL SUMMARY (Asia/Jakarta, DRAW excluded)".to_string(),
        format!("Generated: {}", now_iso_jakarta()),
        "Windows: 1h 3h 6h 12h 1d 1w 1m all".to_string(),
        String::new(),
        "CURRENT BOT".to_string(),
        telegram_pnl_section(&format!("Bot: {current_bot_id}"), &bot_stats),
        String::new(),
        "ALL BOTS".to_string(),
        telegram_pnl_section("Aggregate", &all_stats),
    ];
    if !bot_ids.is_empty() {
        parts.push(String::new());
        parts.push(format!(
            "PER-BOT BREAKDOWN (bot_type=TRADING, {} bots)",
            bot_ids.len()
        ));
        for id in bot_ids {
            let s = bot_window_stats(repo, &id, &bounds);
            parts.push(telegram_pnl_section(&format!("Bot: {id}"), &s));
            parts.push(String::new());
        }
    }
    parts.push(String::new());
    parts.push("ALL BOTS (RECAP)".to_string());
    parts.push(telegram_pnl_section("Aggregate", &all_stats));
    parts.push(String::new());
    parts.push("Legend: 1h=last 1 hour, 3h=last 3 hours, 6h=last 6 hours, 12h=last 12 hours, 1d=daily, 1w=weekly, 1m=monthly, all=all-time".to_string());
    parts.join("\n")
}

fn build_telegram_startup_message(bot_id: &str, exec_mode: &str) -> String {
    let host = env::var("HOSTNAME")
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string();
    let host = if host.is_empty() {
        "unknown".to_string()
    } else {
        host
    };
    format!(
        "POLYBOT RESTART TEST\nBot: {}\nVersion: {}\nMode: {}\nTime (Asia/Jakarta): {}\nHost: {}",
        bot_id,
        build_version(),
        exec_mode,
        now_iso_jakarta(),
        host
    )
}

#[derive(Debug, Clone, Serialize)]
struct TelegramSendMessage<'a> {
    chat_id: &'a str,
    text: &'a str,
    disable_web_page_preview: bool,
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let input_chars = input.chars().count();
    if input_chars <= max_chars {
        return input.to_string();
    }
    let suffix = "...(truncated)";
    let keep = max_chars.saturating_sub(suffix.chars().count());
    let mut out: String = input.chars().take(keep).collect();
    out.push_str(suffix);
    out
}

fn telegram_enabled() -> bool {
    !env::var("TELEGRAM_BOT_ID")
        .unwrap_or_default()
        .trim()
        .is_empty()
}

fn split_text_chunks_by_lines(input: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![String::new()];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();

    for line in input.lines() {
        let candidate = if cur.is_empty() {
            line.to_string()
        } else {
            format!("{cur}\n{line}")
        };
        if candidate.chars().count() <= max_chars {
            cur = candidate;
            continue;
        }

        if !cur.is_empty() {
            chunks.push(cur);
            cur = String::new();
        }

        let line_len = line.chars().count();
        if line_len <= max_chars {
            cur = line.to_string();
            continue;
        }

        let mut segment = String::new();
        for ch in line.chars() {
            if segment.chars().count() >= max_chars {
                chunks.push(segment);
                segment = String::new();
            }
            segment.push(ch);
        }
        if !segment.is_empty() {
            cur = segment;
        }
    }

    if !cur.is_empty() {
        chunks.push(cur);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

fn resolve_telegram_target() -> Option<(String, String)> {
    let telegram_bot_id = env::var("TELEGRAM_BOT_ID")
        .unwrap_or_default()
        .trim()
        .to_string();
    if telegram_bot_id.is_empty() {
        return None;
    }

    let token_env = env::var("TELEGRAM_BOT_TOKEN")
        .unwrap_or_default()
        .trim()
        .to_string();
    let chat_env = env::var("TELEGRAM_CHAT_ID")
        .unwrap_or_default()
        .trim()
        .to_string();

    let looks_like_token = telegram_bot_id.contains(':');
    let token = if !token_env.is_empty() {
        token_env
    } else if looks_like_token {
        telegram_bot_id.clone()
    } else {
        String::new()
    };
    let chat_id = if !chat_env.is_empty() {
        chat_env
    } else if !looks_like_token {
        telegram_bot_id
    } else {
        String::new()
    };

    Some((token, chat_id))
}

fn resolve_telegram_chat_id_from_updates(client: &Client, token: &str) -> Option<String> {
    let endpoint = format!("https://api.telegram.org/bot{token}/getUpdates");
    let resp = client.get(&endpoint).query(&[("limit", 20)]).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let payload = resp.json::<Value>().ok()?;
    let rows = payload.get("result")?.as_array()?;
    rows.iter().rev().find_map(|u| {
        u.get("message")
            .or_else(|| u.get("channel_post"))
            .or_else(|| u.get("edited_message"))
            .or_else(|| u.get("edited_channel_post"))
            .and_then(|m| m.get("chat"))
            .and_then(|c| c.get("id"))
            .and_then(|id| match id {
                Value::String(s) => {
                    let v = s.trim();
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.to_string())
                    }
                }
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
    })
}

fn send_telegram_stats_if_enabled(summary: &str, logger: &Arc<dyn LogLike>) {
    let Some((token, mut chat_id)) = resolve_telegram_target() else {
        return;
    };
    if token.trim().is_empty() {
        logger.warning(
            "[TELEGRAM] TELEGRAM_BOT_ID is set but bot token is missing. Set TELEGRAM_BOT_TOKEN \
or use TELEGRAM_BOT_ID as full bot token.",
        );
        return;
    }

    let timeout_s = env_float("TELEGRAM_TIMEOUT_SECONDS", 6.0).clamp(1.0, 30.0);
    let client = match Client::builder()
        .timeout(Duration::from_secs_f64(timeout_s))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            logger.warning(&format!(
                "[TELEGRAM] skip send: failed creating HTTP client err={e}"
            ));
            return;
        }
    };

    if chat_id.trim().is_empty() {
        if let Some(found) = resolve_telegram_chat_id_from_updates(&client, &token) {
            chat_id = found;
        }
    }
    if chat_id.trim().is_empty() {
        logger.warning(
            "[TELEGRAM] skip send: chat id missing. Set TELEGRAM_CHAT_ID, or open a chat with \
the bot so getUpdates can discover one.",
        );
        return;
    }

    let endpoint = format!("https://api.telegram.org/bot{token}/sendMessage");
    let body_limit = 3900usize;
    let raw_chunks = split_text_chunks_by_lines(summary, body_limit);
    let total = raw_chunks.len();
    for (idx, raw) in raw_chunks.into_iter().enumerate() {
        let text = if total > 1 {
            let prefix = format!("[{}/{}]\n", idx + 1, total);
            let allow = body_limit.saturating_sub(prefix.chars().count());
            format!("{prefix}{}", truncate_chars(&raw, allow))
        } else {
            raw
        };

        let payload = TelegramSendMessage {
            chat_id: &chat_id,
            text: &text,
            disable_web_page_preview: true,
        };
        let resp = match client.post(&endpoint).json(&payload).send() {
            Ok(r) => r,
            Err(e) => {
                logger.warning(&format!(
                    "[TELEGRAM] sendMessage request failed chunk={}/{} err={e}",
                    idx + 1,
                    total
                ));
                return;
            }
        };

        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if !status.is_success() {
            logger.warning(&format!(
                "[TELEGRAM] sendMessage failed chunk={}/{} status={} body={}",
                idx + 1,
                total,
                status,
                truncate_chars(&body, 220)
            ));
            return;
        }
        let ok = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("ok").and_then(|x| x.as_bool()))
            .unwrap_or(true);
        if !ok {
            logger.warning(&format!(
                "[TELEGRAM] sendMessage returned ok=false chunk={}/{} body={}",
                idx + 1,
                total,
                truncate_chars(&body, 220)
            ));
            return;
        }
    }
}

fn realized_lp_from_resolution_snapshot(
    market_slug: &str,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
    fallback_lp: f64,
    logger: &Arc<dyn LogLike>,
) -> f64 {
    let Some(snapshot) = get_resolution_snapshot_for_market(market_slug) else {
        return fallback_lp;
    };
    if snapshot.source_ts_ms + 1 < snapshot.resolution_ts_ms {
        return fallback_lp;
    }
    let diff_price = snapshot
        .diff_vs_price_to_beat
        .or_else(|| {
            snapshot
                .price_to_beat
                .map(|ptb| snapshot.resolution_price - ptb)
        })
        .filter(|v| v.is_finite());
    let Some(diff_price) = diff_price else {
        return fallback_lp;
    };

    let (yes_payout, no_payout) = payout_from_resolution_diff(diff_price, q_yes, q_no);
    let realized_lp = yes_payout + no_payout - total_cost;
    logger.info(&format!(
        "[TRADE][REALIZED] market={} lp={:+.6} fallback_lp={:+.6} q_yes={:.4} q_no={:.4} total_cost={:.6} diff_vs_price_to_beat={:+.6} source_ts_ms={} resolution_ts_ms={}",
        market_slug,
        realized_lp,
        fallback_lp,
        q_yes,
        q_no,
        total_cost,
        diff_price,
        snapshot.source_ts_ms,
        snapshot.resolution_ts_ms
    ));
    realized_lp
}

fn payout_from_resolution_diff(diff_price: f64, q_yes: f64, q_no: f64) -> (f64, f64) {
    if diff_price >= 0.0 {
        (q_yes.max(0.0), 0.0)
    } else {
        (0.0, q_no.max(0.0))
    }
}

fn now_ts_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn value_as_f64(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn closed_position_slug(row: &Value) -> Option<String> {
    row.get("slug")
        .or_else(|| row.get("marketSlug"))
        .or_else(|| row.get("market_slug"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn closed_position_event_slug(row: &Value) -> Option<String> {
    row.get("eventSlug")
        .or_else(|| row.get("event_slug"))
        .or_else(|| row.get("event").and_then(|v| v.get("slug")))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn closed_position_realized_pnl(row: &Value) -> Option<f64> {
    for key in [
        "realizedPnl",
        "realized_pnl",
        "cashPnl",
        "cash_pnl",
        "pnl",
        "profit",
    ] {
        if let Some(v) = value_as_f64(row.get(key)).filter(|v| v.is_finite()) {
            return Some(v);
        }
    }
    None
}

fn trade_validation_users(cfg: &BotConfig) -> Vec<String> {
    let mut users: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let csv = env::var("TRADE_VALIDATION_USERS").unwrap_or_default();
    for part in csv.split(',') {
        let u = part.trim();
        if u.is_empty() {
            continue;
        }
        let key = u.to_ascii_lowercase();
        if seen.insert(key) {
            users.push(u.to_string());
        }
    }
    let single = env::var("TRADE_VALIDATION_USER").unwrap_or_default();
    for cand in [
        single,
        cfg.funder.clone().unwrap_or_default(),
        env::var("POLYMARKET_WALLET_ADDRESS").unwrap_or_default(),
        env::var("WALLET_ADDRESS").unwrap_or_default(),
    ] {
        let u = cand.trim();
        if u.is_empty() {
            continue;
        }
        let key = u.to_ascii_lowercase();
        if seen.insert(key) {
            users.push(u.to_string());
        }
    }
    users
}

#[derive(Debug, Clone, Serialize)]
struct ClosedPositionsQuery<'a> {
    user: &'a str,
    limit: i64,
    offset: i64,
    #[serde(rename = "sortBy")]
    sort_by: &'a str,
    #[serde(rename = "sortDirection")]
    sort_direction: &'a str,
}

fn fetch_closed_positions_for_user(
    http: &Client,
    base_url: &str,
    user: &str,
    page_limit: i64,
    max_pages: i64,
) -> Result<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    let endpoint = format!("{}/closed-positions", base_url.trim_end_matches('/'));
    let mut offset = 0_i64;
    for _ in 0..max_pages {
        let query = ClosedPositionsQuery {
            user,
            limit: page_limit,
            offset,
            sort_by: "TIMESTAMP",
            sort_direction: "DESC",
        };
        let resp = http
            .get(&endpoint)
            .query(&query)
            .send()
            .with_context(|| format!("closed-positions request failed user={user}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "closed-positions non-success status={} user={}",
                resp.status(),
                user
            ));
        }
        let payload: Value = resp
            .json()
            .with_context(|| format!("closed-positions invalid JSON user={user}"))?;
        let rows = payload
            .as_array()
            .cloned()
            .or_else(|| payload.get("data").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default();
        let row_len = rows.len() as i64;
        out.extend(rows);
        if row_len < page_limit {
            break;
        }
        offset += page_limit;
    }
    Ok(out)
}

fn resolve_event_slug_for_market_slug(
    market_slug: &str,
    cache: &mut HashMap<String, Option<String>>,
    logger: &Arc<dyn LogLike>,
) -> Option<String> {
    if let Some(v) = cache.get(market_slug) {
        return v.clone();
    }
    let resolved = fetch_market_by_slug(market_slug, Some(logger))
        .ok()
        .flatten()
        .and_then(|m| {
            m.get("eventSlug")
                .or_else(|| m.get("event_slug"))
                .or_else(|| m.get("event").and_then(|v| v.get("slug")))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    cache.insert(market_slug.to_string(), resolved.clone());
    resolved
}

fn reconcile_unvalidated_trades_with_polymarket(
    repo: &BotRepository,
    bot_id: &str,
    cfg: &BotConfig,
    logger: &Arc<dyn LogLike>,
) -> Result<()> {
    let lookback_days = env_int("TRADE_VALIDATION_LOOKBACK_DAYS", 7).max(0) as i64;
    let max_trades = env_int("TRADE_VALIDATION_MAX_TRADES_PER_POLL", 100).max(1) as i64;
    let page_limit = env_int("TRADE_VALIDATION_PAGE_LIMIT", 50).clamp(1, 50) as i64;
    let max_pages = env_int("TRADE_VALIDATION_MAX_PAGES", 10).clamp(1, 200) as i64;
    let timeout_s = env_float("TRADE_VALIDATION_API_TIMEOUT_SECONDS", 6.0).clamp(0.2, 30.0);
    let start_date = (Utc::now().with_timezone(&Jakarta).date_naive()
        - ChronoDuration::days(lookback_days))
    .format("%Y-%m-%d")
    .to_string();

    let candidates = repo.list_unvalidated_trades_for_bot(bot_id, &start_date, max_trades)?;
    if candidates.is_empty() {
        return Ok(());
    }

    let users = trade_validation_users(cfg);
    if users.is_empty() {
        logger.warning("[TRADE_VALIDATE] skip: no user address configured");
        return Ok(());
    }

    let base = env::var("POLY_DATA_API_BASE_URL")
        .unwrap_or_else(|_| "https://data-api.polymarket.com".to_string());
    let http = Client::builder()
        .timeout(Duration::from_secs_f64(timeout_s))
        .build()
        .context("failed creating HTTP client for trade validation")?;

    let mut all_rows: Vec<Value> = Vec::new();
    let mut any_success = false;
    for user in users {
        match fetch_closed_positions_for_user(&http, &base, &user, page_limit, max_pages) {
            Ok(rows) => {
                any_success = true;
                all_rows.extend(rows);
            }
            Err(e) => logger.warning(&format!(
                "[TRADE_VALIDATE] closed-positions fetch failed user={} err={e}",
                user
            )),
        }
    }
    if !any_success {
        logger.warning("[TRADE_VALIDATE] no successful closed-positions responses");
        return Ok(());
    }

    let mut by_slug: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_event_slug: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, row) in all_rows.iter().enumerate() {
        if let Some(slug) = closed_position_slug(row).map(|s| s.to_ascii_lowercase()) {
            by_slug.entry(slug).or_default().push(idx);
        }
        if let Some(ev) = closed_position_event_slug(row).map(|s| s.to_ascii_lowercase()) {
            by_event_slug.entry(ev).or_default().push(idx);
        }
    }

    let mut event_slug_cache: HashMap<String, Option<String>> = HashMap::new();
    let checked_at = now_iso_jakarta();
    let mut validated_count = 0_i64;
    let mut touched_count = 0_i64;

    for t in candidates {
        let trade_slug = t.slug.trim().to_ascii_lowercase();
        let mut match_idx: Vec<usize> = by_slug.get(&trade_slug).cloned().unwrap_or_default();
        let mut match_key = format!("slug={}", t.slug);
        let mut resolved_event_slug: Option<String> = None;
        if match_idx.is_empty() {
            if let Some(v) = by_event_slug.get(&trade_slug) {
                match_idx = v.clone();
                match_key = format!("eventSlug_direct={}", t.slug);
            }
        }
        if match_idx.is_empty() {
            let event_slug =
                resolve_event_slug_for_market_slug(&t.slug, &mut event_slug_cache, logger);
            resolved_event_slug = event_slug.clone();
            if let Some(ev) = event_slug.map(|s| s.to_ascii_lowercase()) {
                if let Some(v) = by_event_slug.get(&ev) {
                    match_idx = v.clone();
                    match_key = format!("eventSlug={ev}");
                }
            }
        }

        if match_idx.is_empty() {
            repo.touch_trade_validation_checked(&t.trade_id, &checked_at)?;
            touched_count += 1;
            logger.info(&format!(
                "[TRADE_VALIDATE] checked_only trade_id={} slug={} reason=no_closed_position_match event_slug={}",
                t.trade_id,
                t.slug,
                resolved_event_slug.unwrap_or_else(|| "-".to_string())
            ));
            continue;
        }

        let mut pnl_sum = 0.0_f64;
        let mut pnl_rows = 0_i64;
        for idx in match_idx {
            if let Some(pnl) = closed_position_realized_pnl(&all_rows[idx]) {
                pnl_sum += pnl;
                pnl_rows += 1;
            }
        }
        if pnl_rows <= 0 {
            repo.touch_trade_validation_checked(&t.trade_id, &checked_at)?;
            touched_count += 1;
            logger.info(&format!(
                "[TRADE_VALIDATE] checked_only trade_id={} slug={} reason=match_without_realized_pnl source={}",
                t.trade_id, t.slug, match_key
            ));
            continue;
        }

        let source = format!("POLYMARKET_CLOSED_POSITIONS({match_key},rows={pnl_rows})");
        repo.mark_trade_validated_from_polymarket(&t.trade_id, pnl_sum, &checked_at, &source)?;
        validated_count += 1;
        logger.info(&format!(
            "[TRADE_VALIDATE] validated trade_id={} slug={} pnl={:+.6} source={}",
            t.trade_id, t.slug, pnl_sum, source
        ));
    }

    logger.info(&format!(
        "[TRADE_VALIDATE] poll done bot={} candidates={} validated={} checked_only={} rows={}",
        bot_id,
        validated_count + touched_count,
        validated_count,
        touched_count,
        all_rows.len()
    ));
    Ok(())
}

fn cfg_from_row(cfg_row: &ConfigurationRow) -> BotConfig {
    BotConfig {
        clob_host: cfg_row.clob_host.clone(),
        ws_base: cfg_row.ws_base.clone(),
        chain_id: cfg_row.chain_id,
        private_key: cfg_row.private_key.clone(),
        signature_type: cfg_row.signature_type,
        funder: cfg_row.funder.clone(),
        tick: cfg_row.tick,
        min_shares: cfg_row.min_shares,
        lock_profit_target: cfg_row.lock_profit_target,
        clip_shares: cfg_row.clip_shares,
        improve_bid_ticks: cfg_row.improve_bid_ticks,
        maker_buffer_ticks: cfg_row.maker_buffer_ticks,
        replace_if_price_moves_ticks: cfg_row.replace_if_price_moves_ticks,
        stale_seconds: cfg_row.stale_seconds,
        entry_edge_ticks: cfg_row.entry_edge_ticks,
        hedge_buffer_ticks: cfg_row.hedge_buffer_ticks,
        max_total_cost: cfg_row.max_total_cost,
        reserve_usd: cfg_row.reserve_usd,
        cancel_all_on_start: cfg_row.cancel_all_on_start,
        dry_run: cfg_row.dry_run,
        log_every: cfg_row.log_every,
        market_data_stale_seconds: cfg_row.market_data_stale_seconds,
        ws_reconnect_min: cfg_row.ws_reconnect_min,
        ws_reconnect_max: cfg_row.ws_reconnect_max,
        stop_buffer_seconds: cfg_row.stop_buffer_seconds,
        ..BotConfig::default()
    }
}

fn run() -> Result<()> {
    install_rustls_crypto_provider();
    let _ = dotenvy::dotenv();

    if env::var("POLYBOT_PRINT_ENV_CONTRACT").ok().as_deref() == Some("1") {
        for key in env_contract::ENV_CONTRACT_KEYS {
            println!("{key}");
        }
        return Ok(());
    }
    println!("polybot version: {}", build_version());

    let mut cfg = BotConfig::from_env();

    let seg = segment(&env::var("MARKET_SEGMENT").unwrap_or_else(|_| "15M".to_string()));
    let d = segment_defaults(&seg);
    cfg.market_segment = seg;
    cfg.market_duration_seconds = env::var("MARKET_DURATION_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(d.duration);
    cfg.market_step_seconds = env::var("MARKET_STEP_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(d.step);
    env::set_var(
        "MARKET_DURATION_SECONDS",
        cfg.market_duration_seconds.to_string(),
    );
    env::set_var("MARKET_STEP_SECONDS", cfg.market_step_seconds.to_string());
    cfg.stop_buffer_seconds = env::var("STOP_BUFFER_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(d.stop_buffer);

    if cfg.private_key.trim().is_empty() {
        return Err(anyhow!("Missing POLYMARKET_PRIVATE_KEY"));
    }

    let db_url = env::var("DB_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5432/polybot".to_string());
    let db_url_log = db_url_hint(&db_url);
    let engine = make_engine(&db_url);
    let session_factory = make_session_factory(engine.clone());
    let mut last_init_err: Option<String> = None;
    for attempt in 1..=5 {
        match BotRepository::init_schema(&engine) {
            Ok(_) => {
                last_init_err = None;
                break;
            }
            Err(err) => {
                let msg = format!("{err:#}");
                eprintln!("db init attempt {attempt}/5 failed (DB_URL={db_url_log}): {msg}");
                last_init_err = Some(msg);
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
    if let Some(last) = last_init_err {
        return Err(anyhow!(
            "DB Init Error after 5 retries (DB_URL={db_url_log}): {last}"
        ));
    }

    let bot_id = env::var("BOT_ID").unwrap_or_else(|_| "maker_hedgecap_bot".to_string());
    let bot_description =
        env::var("BOT_DESCRIPTION").unwrap_or_else(|_| "Maker+HedgeCap Polymarket bot".to_string());
    let account_name = env::var("ACCOUNT_NAME").unwrap_or_else(|_| "default".to_string());
    let daily_take_profit_usd = env_float("DAILY_PNL_TAKE_PROFIT_USD", 0.0).max(0.0);
    let daily_stop_loss_usd = env_float("DAILY_PNL_STOP_LOSS_USD", 0.0).abs();
    let next_market_delay_seconds = env_float("NEXT_MARKET_DELAY_SECONDS", 2.0).max(0.0);
    let trade_validation_enabled = env_bool("TRADE_VALIDATION_ENABLED", true);
    let trade_validation_after_market_enabled =
        env_bool("TRADE_VALIDATION_AFTER_MARKET_ENABLED", true);
    let trade_validation_poll_seconds = env_float("TRADE_VALIDATION_POLL_SECONDS", 90.0).max(5.0);

    let sig = env::var("SIGNATURE_TYPE").unwrap_or_else(|_| "1".to_string());
    let funder = env::var("POLYMARKET_FUNDER").unwrap_or_default();
    if !sig.trim().is_empty() && !funder.trim().is_empty() {
        cfg.signature_type = sig.trim().parse::<i64>().ok();
        cfg.funder = Some(funder.trim().to_string());
    }
    if cfg.funder.clone().unwrap_or_default().trim().is_empty() {
        return Err(anyhow!("Missing POLYMARKET_FUNDER"));
    }

    let exec_mode = env::var("EXEC_MODE")
        .unwrap_or_else(|_| "MAKER".to_string())
        .trim()
        .to_ascii_uppercase();
    let signal_mode = matches!(
        exec_mode.as_str(),
        "SIGNAL_SNIPPER" | "SIGNAL_SNIPER" | "SIGNAL_SNIPE" | "SIGNAL"
    );
    if telegram_enabled() {
        let startup_logger = setup_item_logger("startup");
        let startup_msg = build_telegram_startup_message(&bot_id, &exec_mode);
        send_telegram_stats_if_enabled(&startup_msg, &startup_logger);
    }

    let mut signal_hub: Option<Arc<SignalHub>> = None;
    let signal_stop_event = Arc::new(AtomicBool::new(false));
    if signal_mode {
        let provider = env::var("SIGNAL_PROVIDER")
            .unwrap_or_else(|_| "WEBSOCKET".to_string())
            .trim()
            .to_ascii_uppercase();
        if provider == "WEBSOCKET" {
            let inbox = Arc::new(SignalInbox::new(Some(signal_stop_event.clone()), 10000));
            let signal_file_dir =
                env::var("SIGNAL_FILE_DIR").unwrap_or_else(|_| "./signals".to_string());
            let signal_file_dir = if signal_file_dir.trim().is_empty() {
                "./signals".to_string()
            } else {
                signal_file_dir
            };
            std::fs::create_dir_all(&signal_file_dir).ok();
            let signal_file_path = env::var("SIGNAL_FILE_PATH")
                .unwrap_or_else(|_| format!("{signal_file_dir}/signal_ws_global.jsonl"));
            let file_log_raw = env_bool("SIGNAL_FILE_LOG_RAW", false);
            let fs = Arc::new(JsonlFileService::new(signal_file_path.clone(), true));
            let ws_url = env::var("SIGNAL_WS_URL").unwrap_or_default();
            if ws_url.trim().is_empty() {
                return Err(anyhow!(
                    "Missing SIGNAL_WS_URL for SIGNAL_PROVIDER=WEBSOCKET"
                ));
            }
            let hub_logger = setup_item_logger("signal_hub");
            let hub = Arc::new(SignalHub::new(
                ws_url.clone(),
                inbox,
                signal_stop_event.clone(),
                Some(fs),
                Some(hub_logger.clone()),
                env_float("SIGNAL_WS_RECONNECT_MIN", 1.0),
                env_float("SIGNAL_WS_RECONNECT_MAX", 30.0),
                env_float("SIGNAL_WS_PING_INTERVAL", 10.0),
                env_float("SIGNAL_WS_PING_TIMEOUT", 7.0),
                env_float("SIGNAL_WS_TLS_MIN", 1.2),
                env_bool("SIGNAL_WS_INSECURE", false),
                env_bool("SIGNAL_WS_DEBUG", false),
                file_log_raw,
            ));
            hub.start();
            hub_logger.info(&format!(
                "[SIGNAL_HUB] started provider=WEBSOCKET url={} file={}",
                ws_url, signal_file_path
            ));
            signal_hub = Some(hub);
        }
    }

    let mut slug = env::var("MARKET_SLUG").unwrap_or_default();
    if slug.trim().is_empty() {
        if signal_mode && env_bool("SIGNAL_FOLLOW_SLUG", false) {
            if let Some(hub) = &signal_hub {
                let wait_logger = setup_item_logger("signal_wait");
                wait_logger.info(
                    "MARKET_SLUG is empty; waiting for first signal (SIGNAL_FOLLOW_SLUG=true)...",
                );
                let first = hub.inbox.peek(None);
                if let Some(sig) = first {
                    slug = sig.market_slug;
                    wait_logger.info(&format!("Using initial market_slug from signal: {slug}"));
                } else if let Some(auto_slug) =
                    generate_market_slug_from_env_now(&cfg.market_segment, cfg.market_step_seconds)
                {
                    wait_logger.warning(&format!(
                        "No signal yet; auto-generated MARKET_SLUG from current time: {auto_slug}"
                    ));
                    slug = auto_slug;
                } else {
                    return Err(anyhow!(
                        "Missing MARKET_SLUG and no signal received from SIGNAL_WS_URL. \
Set MARKET_SLUG or provide MARKET_SYMBOL (or RTDS_SYMBOL) with MARKET_SEGMENT."
                    ));
                }
            } else if let Some(auto_slug) =
                generate_market_slug_from_env_now(&cfg.market_segment, cfg.market_step_seconds)
            {
                let auto_logger = setup_item_logger("slug_auto");
                auto_logger.warning(&format!(
                    "SIGNAL_FOLLOW_SLUG enabled but signal hub unavailable; auto-generated MARKET_SLUG from current time: {auto_slug}"
                ));
                slug = auto_slug;
            } else {
                return Err(anyhow!(
                    "Missing MARKET_SLUG. Set MARKET_SLUG or provide MARKET_SYMBOL \
(or RTDS_SYMBOL) with MARKET_SEGMENT."
                ));
            }
        } else if let Some(auto_slug) =
            generate_market_slug_from_env_now(&cfg.market_segment, cfg.market_step_seconds)
        {
            let auto_logger = setup_item_logger("slug_auto");
            auto_logger.info(&format!(
                "MARKET_SLUG is empty; auto-generated from current time: {auto_slug} \
(segment={}, step={}s)",
                cfg.market_segment, cfg.market_step_seconds
            ));
            slug = auto_slug;
        } else {
            return Err(anyhow!(
                "Missing MARKET_SLUG. Set MARKET_SLUG or provide MARKET_SYMBOL \
(or RTDS_SYMBOL) with MARKET_SEGMENT."
            ));
        }
    }

    cfg.apply_safe_defaults();
    let mut current_slug = slug;
    let mut last_trade_validation_poll_ts = 0.0_f64;
    let mut last_daily_limit_telegram_key = String::new();

    loop {
        let bot_logger = setup_item_logger(&current_slug);
        bot_logger.info(&format!("\nSTARTING MARKET: {current_slug}"));
        let repo = session_factory.repository();
        let now_poll_ts = now_ts_f64();
        if trade_validation_enabled
            && now_poll_ts - last_trade_validation_poll_ts >= trade_validation_poll_seconds
        {
            if let Err(e) =
                reconcile_unvalidated_trades_with_polymarket(&repo, &bot_id, &cfg, &bot_logger)
            {
                bot_logger.warning(&format!("[TRADE_VALIDATE] poll error: {e:#}"));
            }
            last_trade_validation_poll_ts = now_ts_f64();
        }
        if daily_take_profit_usd > 0.0 || daily_stop_loss_usd > 0.0 {
            // Keep daily guardrail window exactly aligned with summary daily window bounds.
            let bounds = stats_bounds_now();
            let today = bounds.today;
            let today_stats = repo
                .trade_stats_for_bot_period(&bot_id, &today, &today)
                .unwrap_or_default();
            let today_pnl = today_stats.net_pnl;
            let today_trades = today_stats.total_count;
            let hit_take_profit = daily_take_profit_usd > 0.0 && today_pnl >= daily_take_profit_usd;
            let hit_stop_loss = daily_stop_loss_usd > 0.0 && today_pnl <= -daily_stop_loss_usd;
            if hit_take_profit || hit_stop_loss {
                let reason = if hit_take_profit {
                    "DAILY_TAKE_PROFIT_HIT"
                } else {
                    "DAILY_STOP_LOSS_HIT"
                };
                bot_logger.warning(&format!(
                    "[DAILY_LIMIT] skip trading bot={} date={} reason={} pnl={:+.4} trades={} take_profit_usd={:.4} stop_loss_usd={:.4}",
                    bot_id,
                    today,
                    reason,
                    today_pnl,
                    today_trades,
                    daily_take_profit_usd,
                    daily_stop_loss_usd
                ));
                if telegram_enabled() {
                    let notify_key = format!("{today}:{reason}");
                    if last_daily_limit_telegram_key != notify_key {
                        let telegram_summary =
                            build_telegram_pnl_summary(&repo, &bot_id, &bot_logger);
                        let telegram_message = format!(
                            "DAILY LIMIT {}\nbot={} date={} pnl={:+.4} trades={} take_profit_usd={:.4} stop_loss_usd={:.4}\n\n{}",
                            reason,
                            bot_id,
                            today,
                            today_pnl,
                            today_trades,
                            daily_take_profit_usd,
                            daily_stop_loss_usd,
                            telegram_summary
                        );
                        send_telegram_stats_if_enabled(&telegram_message, &bot_logger);
                        last_daily_limit_telegram_key = notify_key;
                    }
                }
                thread::sleep(Duration::from_secs(60));
                if let Some(auto_slug) =
                    generate_market_slug_from_env_now(&cfg.market_segment, cfg.market_step_seconds)
                {
                    current_slug = auto_slug;
                }
                continue;
            }
        }

        let mut bot_row = repo.get_bot(&bot_id)?;
        if bot_row.is_none() {
            let bootstrap_cfg_id = repo.upsert_configuration(&cfg)?;
            repo.upsert_bot(
                &bot_id,
                &bot_description,
                &account_name,
                "ACTIVE",
                &bootstrap_cfg_id,
            )?;
            bot_row = repo.get_bot(&bot_id)?;
        }

        if let Some(row) = &bot_row {
            if row.status != "ACTIVE" {
                bot_logger.warning(&format!("Bot DISABLED in DB. Skipping {current_slug}."));
                thread::sleep(Duration::from_secs(2));
                current_slug = get_next_slug(&current_slug);
                continue;
            }
        }

        let mut configuration_id = bot_row
            .as_ref()
            .and_then(|b| b.configuration_id.clone())
            .unwrap_or_default();
        if configuration_id.trim().is_empty() {
            configuration_id = repo.upsert_configuration(&cfg)?;
        }

        let run_cfg = match repo.get_configuration(&configuration_id)? {
            Some(r) => cfg_from_row(&r),
            None => {
                configuration_id = repo.upsert_configuration(&cfg)?;
                cfg.clone()
            }
        };

        let bot = MakerHedgeCapBot::new(
            run_cfg.clone(),
            &current_slug,
            bot_logger.clone(),
            signal_hub.clone(),
        )
        .with_context(|| format!("failed to initialize bot for {current_slug}"))?;

        let (trade_id, status) = repo.create_pending_trade(
            &bot_id,
            &current_slug,
            &configuration_id,
            &bot.start_trade_iso,
        )?;
        bot_logger.info(&format!(
            "Created pending trade record: {trade_id} status={status}"
        ));
        if status != "INITIALIZED" {
            bot_logger.info(&format!(
                "Trade {trade_id} already exists with status={status}. Skipping {current_slug}."
            ));
            thread::sleep(Duration::from_secs(1));
            current_slug = get_next_slug(&current_slug);
            continue;
        }

        let rtds_service =
            match RtdsService::for_market(&current_slug, &run_cfg, bot_logger.clone()) {
                Ok(Some(svc)) => {
                    svc.start();
                    Some(svc)
                }
                Ok(None) => None,
                Err(e) => {
                    bot_logger.warning(&format!(
                        "[RTDS] failed to initialize market={} err={e}",
                        current_slug
                    ));
                    None
                }
            };

        let run_result = bot.run();
        if let Some(svc) = &rtds_service {
            svc.close();
        }

        let run_reason = match run_result {
            Ok(r) => r,
            Err(e) if e.to_string() == "NO_MARKET" => {
                bot_logger.info(&format!("No market yet for {current_slug}. Skipping."));
                thread::sleep(Duration::from_secs(2));
                current_slug = get_next_slug(&current_slug);
                continue;
            }
            Err(e) => {
                bot_logger.warning(&format!("Bot crashed: {e}. Moving to next slug."));
                format!("CRASH:{}", e)
            }
        };
        bot_logger.info(&format!("Run finished with reason={run_reason}"));

        let mut metrics = bot.trade_metrics_snapshot();
        let has_trade_activity = metrics.fill_count > 0
            || metrics.total_cost > 1e-9
            || metrics.q_yes > 1e-9
            || metrics.q_no > 1e-9;
        if !has_trade_activity {
            repo.delete_trade(&trade_id)?;
            bot.persist_state();
            bot_logger.info(&format!(
                "Deleted pending trade row {trade_id}. reason=NO_TRADE_ACTIVITY"
            ));
        } else {
            metrics.lp = realized_lp_from_resolution_snapshot(
                &current_slug,
                metrics.q_yes,
                metrics.q_no,
                metrics.total_cost,
                metrics.lp,
                &bot_logger,
            );
            let end_trade_iso = now_iso_jakarta();
            let raw_exit_reason = if metrics.exit_reason.trim().is_empty()
                || metrics.exit_reason.eq_ignore_ascii_case("RUNNING")
            {
                run_reason.clone()
            } else {
                metrics.exit_reason.clone()
            };
            let exit_reason_category = analytics_exit_reason(&raw_exit_reason);
            let effective_entry_iso = metrics
                .entry_time_iso
                .as_deref()
                .unwrap_or(bot.start_trade_iso.as_str());
            let holding_secs = holding_duration_seconds(effective_entry_iso, &end_trade_iso);
            let total_qty = metrics.q_yes + metrics.q_no;
            let entry_price = if total_qty > 1e-9 {
                Some(metrics.total_cost / total_qty)
            } else {
                None
            };
            let exit_price = if total_qty > 1e-9 {
                Some((metrics.total_cost + metrics.lp) / total_qty)
            } else {
                None
            };
            let stop_loss_category = if exit_reason_category == "STOP_LOSS" {
                Some(
                    metrics
                        .stop_loss_category
                        .clone()
                        .unwrap_or_else(|| "MARKET".to_string()),
                )
            } else {
                None
            };
            let decision_row = bot.trade_decision_snapshot().unwrap_or_default();
            repo.upsert_trade_decision(&trade_id, &decision_row)?;
            repo.update_trade_result(
                &trade_id,
                &end_trade_iso,
                metrics.lp,
                metrics.total_cost,
                metrics.cpp,
                metrics.q_yes,
                metrics.q_no,
                &raw_exit_reason,
                metrics.entry_time_iso.as_deref(),
                holding_secs,
                metrics.entry_reason.as_deref(),
                Some(exit_reason_category.as_str()),
                stop_loss_category.as_deref(),
                entry_price,
                exit_price,
            )?;
            bot.persist_state();
            bot_logger.info(&format!(
                "Updated trade row {trade_id}. reason=FINALIZED lp={:.4} cost={:.4}",
                metrics.lp, metrics.total_cost
            ));
        }
        if trade_validation_enabled && trade_validation_after_market_enabled {
            if let Err(e) =
                reconcile_unvalidated_trades_with_polymarket(&repo, &bot_id, &cfg, &bot_logger)
            {
                bot_logger.warning(&format!(
                    "[TRADE_VALIDATE] post-market poll error trade_id={} err={e:#}",
                    trade_id
                ));
            }
            last_trade_validation_poll_ts = now_ts_f64();
        }

        thread::sleep(Duration::from_secs(2));
        bot_logger.info(&format!("Ending this market {current_slug}"));
        upload_logs_before_rollover(&current_slug, &bot_id, &bot_logger);
        let next_slug = if run_reason.starts_with("SWITCH:") {
            let ns = run_reason.trim_start_matches("SWITCH:").trim().to_string();
            bot_logger.info(&format!(
                "Switching market due to signal: {} -> {}",
                current_slug, ns
            ));
            ns
        } else {
            get_next_slug(&current_slug)
        };

        let is_ts = current_slug
            .split('-')
            .last()
            .and_then(|s| s.parse::<i64>().ok())
            .is_some();
        if next_slug == current_slug && !is_ts {
            bot_logger.info(&format!(
                "Non-timestamp slug '{}' -> no auto-roll. Stopping.",
                current_slug
            ));
            break;
        }
        current_slug = next_slug;

        let repo = session_factory.repository();
        let _ = print_pnl_metrics(&repo, &bot_id, &bot_logger);
        if telegram_enabled() {
            let telegram_summary = build_telegram_pnl_summary(&repo, &bot_id, &bot_logger);
            send_telegram_stats_if_enabled(&telegram_summary, &bot_logger);
        }
        bot_logger.info(&format!(
            "Waiting {:.2}s before next market... {current_slug}",
            next_market_delay_seconds
        ));
        thread::sleep(Duration::from_secs_f64(next_market_delay_seconds));
    }

    signal_stop_event.store(true, Ordering::SeqCst);
    if let Some(hub) = signal_hub {
        hub.close();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::payout_from_resolution_diff;

    #[test]
    fn tie_resolves_to_yes() {
        let (yes, no) = payout_from_resolution_diff(0.0, 12.0, 12.0);
        assert_eq!(yes, 12.0);
        assert_eq!(no, 0.0);
    }

    #[test]
    fn positive_resolves_to_yes() {
        let (yes, no) = payout_from_resolution_diff(0.5, 8.0, 9.0);
        assert_eq!(yes, 8.0);
        assert_eq!(no, 0.0);
    }

    #[test]
    fn negative_resolves_to_no() {
        let (yes, no) = payout_from_resolution_diff(-0.5, 8.0, 9.0);
        assert_eq!(yes, 0.0);
        assert_eq!(no, 9.0);
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("fatal: {err:#}");
        std::process::exit(1);
    }
}
