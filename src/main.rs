#![recursion_limit = "256"]

mod bot;
mod config;
mod db;
mod env_contract;
mod env_utils;
mod gamma;
mod helpers;
mod latency_log;
mod logging;
mod r2_storage;
mod rtds;

use anyhow::{anyhow, Context, Result};
use bot::MakerHedgeCapBot;
use chrono::{Duration as ChronoDuration, Utc};
use chrono_tz::Asia::Jakarta;
use config::BotConfig;
use db::{
    date_jakarta, make_engine, make_session_factory, month_start_date_jakarta, now_iso_jakarta,
    week_start_date_jakarta, BotRepository, BotTradeStats, ConfigurationRow, TradePairMetadata,
};
use env_utils::{env_bool, env_float, env_int};
use gamma::fetch_market_by_slug;
use helpers::{generate_market_slug_from_env_now, get_next_slug, segment, segment_defaults};
use logging::{setup_item_logger, LogLike};
use r2_storage::upload_logs_before_rollover;
use reqwest::blocking::Client;
use rtds::{get_resolution_snapshot_for_market, ResolutionSnapshot, RtdsService};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

fn require_bot_exec_mode() -> Result<String> {
    let exec_mode = env::var("EXEC_MODE")
        .unwrap_or_else(|_| "BOT".to_string())
        .trim()
        .to_ascii_uppercase();
    if exec_mode == "BOT" {
        Ok(exec_mode)
    } else {
        Err(anyhow!(
            "Unsupported EXEC_MODE={exec_mode}. Only BOT is supported."
        ))
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

fn realized_lp_from_resolution_record(
    snapshot: &ResolutionSnapshot,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
    logger: &Arc<dyn LogLike>,
    realized_log_enabled: bool,
) -> Option<f64> {
    if snapshot.source_ts_ms + 1 < snapshot.resolution_ts_ms {
        return None;
    }
    let diff_price = snapshot
        .diff_vs_price_to_beat
        .or_else(|| {
            snapshot
                .price_to_beat
                .map(|ptb| snapshot.resolution_price - ptb)
        })
        .filter(|v| v.is_finite())?;
    let (yes_payout, no_payout) = payout_from_resolution_diff(diff_price, q_yes, q_no);
    let realized_lp = yes_payout + no_payout - total_cost;
    if realized_log_enabled {
        logger.info(&format!(
            "[TRADE][REALIZED] market={} lp={:+.6} q_yes={:.4} q_no={:.4} total_cost={:.6} diff_vs_price_to_beat={:+.6} source_ts_ms={} resolution_ts_ms={}",
            snapshot.market_slug,
            realized_lp,
            q_yes,
            q_no,
            total_cost,
            diff_price,
            snapshot.source_ts_ms,
            snapshot.resolution_ts_ms
        ));
    }
    Some(realized_lp)
}

fn settled_trade_from_resolution_snapshot(
    market_slug: &str,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
    logger: &Arc<dyn LogLike>,
    realized_log_enabled: bool,
) -> Option<(ResolutionSnapshot, f64)> {
    let snapshot = get_resolution_snapshot_for_market(market_slug)?;
    let realized_lp = realized_lp_from_resolution_record(
        &snapshot,
        q_yes,
        q_no,
        total_cost,
        logger,
        realized_log_enabled,
    )?;
    Some((snapshot, realized_lp))
}

#[derive(Debug, Clone, PartialEq)]
struct AwaitSettlementTradeSnapshot {
    end_trade_iso: String,
    raw_exit_reason: String,
    exit_reason_category: String,
    total_cost: f64,
    cpp: f64,
    q_yes: f64,
    q_no: f64,
    entry_time_iso: Option<String>,
    holding_duration_seconds: Option<f64>,
    entry_reason: Option<String>,
    stop_loss_category: Option<String>,
    entry_price: Option<f64>,
}

fn build_await_settlement_trade_snapshot(
    metrics: &bot::TradeMetrics,
    run_reason: &str,
    start_trade_iso: &str,
    end_trade_iso: &str,
) -> AwaitSettlementTradeSnapshot {
    let raw_exit_reason = if metrics.exit_reason.trim().is_empty()
        || metrics.exit_reason.eq_ignore_ascii_case("RUNNING")
    {
        run_reason.to_string()
    } else {
        metrics.exit_reason.clone()
    };
    let exit_reason_category = analytics_exit_reason(&raw_exit_reason);
    let effective_entry_iso = metrics.entry_time_iso.as_deref().unwrap_or(start_trade_iso);
    let holding_duration_seconds = holding_duration_seconds(effective_entry_iso, end_trade_iso);
    let total_qty = metrics.q_yes + metrics.q_no;
    let entry_price = if total_qty > 1e-9 {
        Some(metrics.total_cost / total_qty)
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
    AwaitSettlementTradeSnapshot {
        end_trade_iso: end_trade_iso.to_string(),
        raw_exit_reason,
        exit_reason_category,
        total_cost: metrics.total_cost,
        cpp: metrics.cpp,
        q_yes: metrics.q_yes,
        q_no: metrics.q_no,
        entry_time_iso: metrics.entry_time_iso.clone(),
        holding_duration_seconds,
        entry_reason: metrics.entry_reason.clone(),
        stop_loss_category,
        entry_price,
    }
}

fn settlement_metadata_json(snapshot: &ResolutionSnapshot, realized_lp: f64) -> Option<String> {
    serde_json::to_string(&json!({
        "settlement_source": "RTDS",
        "market_slug": snapshot.market_slug,
        "symbol": snapshot.symbol,
        "asset_id": snapshot.asset_id,
        "resolution_ts_ms": snapshot.resolution_ts_ms,
        "source_ts_ms": snapshot.source_ts_ms,
        "resolution_price": snapshot.resolution_price,
        "resolution_value": snapshot.resolution_value,
        "capture_mode": snapshot.capture_mode,
        "price_to_beat": snapshot.price_to_beat,
        "diff_vs_price_to_beat": snapshot.diff_vs_price_to_beat,
        "diff_vs_price_to_beat_percentage": snapshot.diff_vs_price_to_beat_percentage,
        "captured_at_ms": snapshot.captured_at_ms,
        "realized_lp": realized_lp,
    }))
    .ok()
}

fn payout_from_resolution_diff(diff_price: f64, q_yes: f64, q_no: f64) -> (f64, f64) {
    if diff_price >= 0.0 {
        (q_yes.max(0.0), 0.0)
    } else {
        (0.0, q_no.max(0.0))
    }
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
    let trade_validation_enabled = env_bool("TRADE_VALIDATION_ENABLED", false);
    let trade_validation_after_market_enabled =
        env_bool("TRADE_VALIDATION_AFTER_MARKET_ENABLED", false);
    let pnl_stats_at_end_enabled = env_bool("PNL_STATS_AT_END_ENABLED", false);
    let trade_realized_log_enabled = env_bool("TRADE_REALIZED_LOG_ENABLED", false);

    let sig = env::var("SIGNATURE_TYPE").unwrap_or_else(|_| "1".to_string());
    let funder = env::var("POLYMARKET_FUNDER").unwrap_or_default();
    if !sig.trim().is_empty() && !funder.trim().is_empty() {
        cfg.signature_type = sig.trim().parse::<i64>().ok();
        cfg.funder = Some(funder.trim().to_string());
    }
    if cfg.funder.clone().unwrap_or_default().trim().is_empty() {
        return Err(anyhow!("Missing POLYMARKET_FUNDER"));
    }

    let exec_mode = require_bot_exec_mode()?;
    if telegram_enabled() {
        let startup_logger = setup_item_logger("startup");
        let startup_msg = build_telegram_startup_message(&bot_id, &exec_mode);
        send_telegram_stats_if_enabled(&startup_msg, &startup_logger);
    }

    let mut slug = env::var("MARKET_SLUG").unwrap_or_default();
    if slug.trim().is_empty() {
        if let Some(auto_slug) =
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
    let mut startup_trade_validation_done = false;
    let mut last_daily_limit_telegram_key = String::new();

    loop {
        let bot_logger = setup_item_logger(&current_slug);
        bot_logger.info(&format!("\nSTARTING MARKET: {current_slug}"));
        let repo = session_factory.repository();
        if trade_validation_enabled && !startup_trade_validation_done {
            if let Err(e) =
                reconcile_unvalidated_trades_with_polymarket(&repo, &bot_id, &cfg, &bot_logger)
            {
                bot_logger.warning(&format!("[TRADE_VALIDATE] poll error: {e:#}"));
            }
            startup_trade_validation_done = true;
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

        let bot = MakerHedgeCapBot::new(run_cfg.clone(), &current_slug, bot_logger.clone())
            .with_context(|| format!("failed to initialize bot for {current_slug}"))?;
        let pair = bot.pair_identity();
        let trade_pair = TradePairMetadata {
            pair_id: pair.pair_id.clone(),
            market_slug: pair.market_slug.clone(),
            condition_id: pair.condition_id.clone(),
            yes_asset_id: pair.yes_asset_id.clone(),
            no_asset_id: pair.no_asset_id.clone(),
        };

        let (trade_id, status) = repo.create_pending_trade(
            &bot_id,
            &trade_pair,
            &configuration_id,
            &bot.start_trade_iso,
        )?;
        bot_logger.info(&format!(
            "Created pending trade record: {trade_id} status={status} pair_id={}",
            trade_pair.pair_id
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

        // Grab shared handles to detect when the bot stops trading,
        // so we can roll to the next market without waiting for bot.run()
        // to fully return (WS close handshake can take 10+ seconds).
        let bot_stop_flag = bot.stop_flag.clone();
        let bot_exit_reason = bot.exit_reason.clone();
        // Spawn bot.run() + all post-market cleanup in a background thread.
        // The main loop will poll stop_flag and proceed to the next market
        // as soon as the bot is done trading.
        let bg_slug = current_slug.clone();
        let bg_bot_id = bot_id.clone();
        let bg_cfg = cfg.clone();
        let bg_logger = bot_logger.clone();
        let bg_session_factory = session_factory.clone();
        let bg_trade_validation_enabled = trade_validation_enabled;
        let bg_trade_validation_after_market_enabled = trade_validation_after_market_enabled;
        let bg_pnl_stats_at_end_enabled = pnl_stats_at_end_enabled;
        let bg_trade_realized_log_enabled = trade_realized_log_enabled;
        thread::spawn(move || {
            let run_result = bot.run();

            let run_reason = match run_result {
                Ok(r) => r,
                Err(e) => {
                    bg_logger.warning(&format!("Bot crashed: {e}. Moving to next slug."));
                    format!("CRASH:{}", e)
                }
            };
            bg_logger.info(&format!("Run finished with reason={run_reason}"));

            let repo = bg_session_factory.repository();
            let mut metrics = bot.trade_metrics_snapshot();
            let has_trade_activity = metrics.fill_count > 0
                || metrics.total_cost > 1e-9
                || metrics.q_yes > 1e-9
                || metrics.q_no > 1e-9;
            if !has_trade_activity {
                if let Some(svc) = rtds_service {
                    svc.close();
                }
                let _ = repo.delete_trade(&trade_id);
                bot.persist_state();
                bg_logger.info(&format!(
                    "Deleted pending trade row {trade_id}. reason=NO_TRADE_ACTIVITY"
                ));
            } else {
                let end_trade_iso = now_iso_jakarta();
                let await_snapshot = build_await_settlement_trade_snapshot(
                    &metrics,
                    &run_reason,
                    bot.start_trade_iso.as_str(),
                    &end_trade_iso,
                );
                let _ = repo.update_trade_await_settlement_snapshot(
                    &trade_id,
                    &await_snapshot.end_trade_iso,
                    await_snapshot.total_cost,
                    await_snapshot.cpp,
                    await_snapshot.q_yes,
                    await_snapshot.q_no,
                    &await_snapshot.raw_exit_reason,
                    await_snapshot.entry_time_iso.as_deref(),
                    await_snapshot.holding_duration_seconds,
                    await_snapshot.entry_reason.as_deref(),
                    Some(await_snapshot.exit_reason_category.as_str()),
                    await_snapshot.stop_loss_category.as_deref(),
                    await_snapshot.entry_price,
                );
                bg_logger.info(&format!(
                    "Trade row {trade_id} handed off to settlement. reason={}",
                    await_snapshot.raw_exit_reason
                ));

                // Close RTDS (waits for resolution price) in background.
                if let Some(svc) = rtds_service {
                    svc.close();
                }

                if let Some((snapshot, realized_lp)) = settled_trade_from_resolution_snapshot(
                    &bg_slug,
                    metrics.q_yes,
                    metrics.q_no,
                    metrics.total_cost,
                    &bg_logger,
                    bg_trade_realized_log_enabled,
                ) {
                    metrics.lp = realized_lp;
                    let total_qty = await_snapshot.q_yes + await_snapshot.q_no;
                    let exit_price = if total_qty > 1e-9 {
                        Some((metrics.total_cost + metrics.lp) / total_qty)
                    } else {
                        None
                    };
                    let _ = repo.update_trade_result(
                        &trade_id,
                        &await_snapshot.end_trade_iso,
                        metrics.lp,
                        metrics.total_cost,
                        metrics.cpp,
                        metrics.q_yes,
                        metrics.q_no,
                        &await_snapshot.raw_exit_reason,
                        await_snapshot.entry_time_iso.as_deref(),
                        await_snapshot.holding_duration_seconds,
                        await_snapshot.entry_reason.as_deref(),
                        Some(await_snapshot.exit_reason_category.as_str()),
                        await_snapshot.stop_loss_category.as_deref(),
                        await_snapshot.entry_price,
                        exit_price,
                    );
                    let settlement_meta = settlement_metadata_json(&snapshot, metrics.lp);
                    let _ = repo.update_trade_settlement_fields(
                        &trade_id,
                        Some("SETTLED"),
                        settlement_meta.as_deref(),
                    );
                    bot.persist_state();
                    bg_logger.info(&format!(
                        "Updated trade row {trade_id}. reason=FINALIZED lp={:.4} cost={:.4} cpp={:.4} qYES={:.2} qNO={:.2} claim_status=SETTLED",
                        metrics.lp,
                        metrics.total_cost,
                        metrics.cpp,
                        metrics.q_yes,
                        metrics.q_no
                    ));
                } else {
                    bot.persist_state();
                    bg_logger.info(&format!(
                        "Trade row {trade_id} remains AWAIT_SETTLEMENT. reason=resolution_snapshot_unavailable pair_id={}",
                        metrics.pair_id
                    ));
                }
            }

            if bg_trade_validation_enabled && bg_trade_validation_after_market_enabled {
                let repo = bg_session_factory.repository();
                if let Err(e) = reconcile_unvalidated_trades_with_polymarket(
                    &repo, &bg_bot_id, &bg_cfg, &bg_logger,
                ) {
                    bg_logger.warning(&format!(
                        "[TRADE_VALIDATE] post-market poll error trade_id={} err={e:#}",
                        trade_id
                    ));
                }
            }

            bg_logger.info(&format!("Ending this market {bg_slug}"));
            upload_logs_before_rollover(&bg_slug, &bg_bot_id, &bg_logger);

            if bg_pnl_stats_at_end_enabled {
                let repo = bg_session_factory.repository();
                let _ = print_pnl_metrics(&repo, &bg_bot_id, &bg_logger);
                if telegram_enabled() {
                    let telegram_summary =
                        build_telegram_pnl_summary(&repo, &bg_bot_id, &bg_logger);
                    send_telegram_stats_if_enabled(&telegram_summary, &bg_logger);
                }
            }
        });

        // Wait for the bot to signal it's done trading (stop_flag set).
        // This returns as soon as the bot's main trading loop ends,
        // WITHOUT waiting for WS close handshake or post-market cleanup.
        while !bot_stop_flag.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(50));
        }

        // Read exit reason from the shared mutex (set before stop_flag).
        let run_reason = bot_exit_reason
            .lock()
            .map(|r| r.clone())
            .unwrap_or_else(|_| "AWAIT_SETTLEMENT".to_string());
        bot_logger.info(&format!("Bot stopped trading, reason={run_reason}"));

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
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_await_settlement_trade_snapshot, payout_from_resolution_diff,
        realized_lp_from_resolution_record, require_bot_exec_mode, settlement_metadata_json,
    };
    use crate::bot::TradeMetrics;
    use crate::logging::{setup_item_logger, LogLike};
    use crate::rtds::ResolutionSnapshot;
    use serde_json::Value;
    use std::env;
    use std::sync::Arc;

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

    #[test]
    fn require_bot_exec_mode_defaults_and_rejects_legacy_modes() {
        let original = env::var("EXEC_MODE").ok();

        env::remove_var("EXEC_MODE");
        assert_eq!(require_bot_exec_mode().expect("default mode"), "BOT");

        env::set_var("EXEC_MODE", "BOT");
        assert_eq!(require_bot_exec_mode().expect("bot mode"), "BOT");

        env::set_var("EXEC_MODE", "SNIPER");
        let err = require_bot_exec_mode()
            .expect_err("legacy mode should be rejected")
            .to_string();
        assert!(err.contains("Only BOT is supported"));

        match original {
            Some(value) => env::set_var("EXEC_MODE", value),
            None => env::remove_var("EXEC_MODE"),
        }
    }

    #[test]
    fn realized_lp_from_resolution_record_requires_fresh_resolution_tick() {
        let logger: Arc<dyn LogLike> = setup_item_logger("resolution_test");
        let stale_snapshot = ResolutionSnapshot {
            market_slug: "market-a".to_string(),
            symbol: "BTC".to_string(),
            asset_id: "asset-a".to_string(),
            resolution_ts_ms: 2_000,
            source_ts_ms: 1_500,
            resolution_price: 0.55,
            resolution_value: None,
            capture_mode: "post".to_string(),
            price_to_beat: Some(0.50),
            diff_vs_price_to_beat: Some(0.05),
            diff_vs_price_to_beat_percentage: Some(0.10),
            captured_at_ms: 2_010,
        };
        assert!(
            realized_lp_from_resolution_record(&stale_snapshot, 5.0, 5.0, 4.0, &logger, false)
                .is_none()
        );

        let fresh_snapshot = ResolutionSnapshot {
            source_ts_ms: 2_000,
            ..stale_snapshot
        };
        let realized =
            realized_lp_from_resolution_record(&fresh_snapshot, 5.0, 5.0, 4.0, &logger, false)
                .expect("fresh snapshot should settle");
        assert!((realized - 1.0).abs() < 1e-9);
    }

    #[test]
    fn settlement_metadata_json_contains_resolution_fields() {
        let snapshot = ResolutionSnapshot {
            market_slug: "market-b".to_string(),
            symbol: "ETH".to_string(),
            asset_id: "asset-b".to_string(),
            resolution_ts_ms: 3_000,
            source_ts_ms: 3_000,
            resolution_price: 0.61,
            resolution_value: Some(123.0),
            capture_mode: "first_after_resolution".to_string(),
            price_to_beat: Some(0.58),
            diff_vs_price_to_beat: Some(0.03),
            diff_vs_price_to_beat_percentage: Some(0.0517),
            captured_at_ms: 3_005,
        };
        let meta = settlement_metadata_json(&snapshot, 2.25).expect("metadata json");
        let parsed: Value = serde_json::from_str(&meta).expect("valid json");
        assert_eq!(
            parsed.get("settlement_source").and_then(Value::as_str),
            Some("RTDS")
        );
        assert_eq!(
            parsed.get("market_slug").and_then(Value::as_str),
            Some("market-b")
        );
        assert_eq!(
            parsed.get("capture_mode").and_then(Value::as_str),
            Some("first_after_resolution")
        );
        assert_eq!(
            parsed.get("realized_lp").and_then(Value::as_f64),
            Some(2.25)
        );
    }

    #[test]
    fn await_settlement_trade_snapshot_preserves_terminal_trade_fields() {
        let metrics = TradeMetrics {
            pair_id: "pair-1".to_string(),
            market_slug: "market-1".to_string(),
            condition_id: Some("cond-1".to_string()),
            yes_asset_id: Some("yes-1".to_string()),
            no_asset_id: Some("no-1".to_string()),
            lp: 0.0,
            total_cost: 8.4,
            q_yes: 10.0,
            q_no: 8.0,
            cpp: 0.4666666667,
            entry_time_iso: Some("2024-01-01T00:00:10Z".to_string()),
            entry_reason: Some("BOT_OPEN_BOTH".to_string()),
            stop_loss_category: Some("NONE".to_string()),
            exit_reason: "RUNNING".to_string(),
            fill_count: 4,
        };
        let snapshot = build_await_settlement_trade_snapshot(
            &metrics,
            "AWAIT_SETTLEMENT",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:05:00Z",
        );
        assert_eq!(snapshot.raw_exit_reason, "AWAIT_SETTLEMENT");
        assert_eq!(snapshot.exit_reason_category, "RESOLUTION");
        assert_eq!(snapshot.total_cost, 8.4);
        assert_eq!(snapshot.q_yes, 10.0);
        assert_eq!(snapshot.q_no, 8.0);
        assert!((snapshot.cpp - 0.4666666667).abs() < 1e-9);
        assert_eq!(
            snapshot.entry_time_iso.as_deref(),
            Some("2024-01-01T00:00:10Z")
        );
        assert_eq!(snapshot.entry_reason.as_deref(), Some("BOT_OPEN_BOTH"));
        assert_eq!(snapshot.stop_loss_category, None);
        assert_eq!(snapshot.entry_price, Some(8.4 / 18.0));
        assert!(snapshot.holding_duration_seconds.unwrap_or_default() > 0.0);
        assert_eq!(snapshot.end_trade_iso, "2024-01-01T00:05:00Z");
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("fatal: {err:#}");
        std::process::exit(1);
    }
}
