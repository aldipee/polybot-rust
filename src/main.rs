#![recursion_limit = "256"]

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
use rtds::{get_resolution_snapshot_for_market, RtdsService};
use reqwest::blocking::Client;
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

fn print_pnl_metrics(repo: &BotRepository, bot_id: &str, logger: &Arc<dyn LogLike>) -> String {
    fn pct(part: i64, total: i64) -> f64 {
        if total <= 0 {
            0.0
        } else {
            (part as f64 * 100.0) / total as f64
        }
    }
    fn line(label: &str, s: &BotTradeStats) -> String {
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

    let today = date_jakarta();
    let week_start = week_start_date_jakarta();
    let month_start = month_start_date_jakarta();

    let s_day = repo
        .trade_stats_for_bot_period(bot_id, &today, &today)
        .unwrap_or_default();
    let s_week = repo
        .trade_stats_for_bot_period(bot_id, &week_start, &today)
        .unwrap_or_default();
    let s_month = repo
        .trade_stats_for_bot_period(bot_id, &month_start, &today)
        .unwrap_or_default();
    let s_all = repo.trade_stats_for_bot_all_time(bot_id).unwrap_or_default();
    let a_day = repo
        .trade_stats_all_bots_period(&today, &today)
        .unwrap_or_default();
    let a_week = repo
        .trade_stats_all_bots_period(&week_start, &today)
        .unwrap_or_default();
    let a_month = repo
        .trade_stats_all_bots_period(&month_start, &today)
        .unwrap_or_default();
    let a_all = repo.trade_stats_all_bots_all_time().unwrap_or_default();

    let msg = format!(
        "PNL Summary (Asia/Jakarta, DRAW excluded)\nBot {bot_id}\n{}\n{}\n{}\n{}\nALL bots\n{}\n{}\n{}\n{}",
        line("Daily", &s_day),
        line("Weekly", &s_week),
        line("Monthly", &s_month),
        line("All", &s_all),
        line("Daily", &a_day),
        line("Weekly", &a_week),
        line("Monthly", &a_month),
        line("All", &a_all)
    );
    logger.info(&msg);
    msg
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
        .or_else(|| snapshot.price_to_beat.map(|ptb| snapshot.resolution_price - ptb))
        .filter(|v| v.is_finite());
    let Some(diff_price) = diff_price else {
        return fallback_lp;
    };

    let yes_payout = if diff_price > 0.0 { q_yes.max(0.0) } else { 0.0 };
    let no_payout = if diff_price > 0.0 { 0.0 } else { q_no.max(0.0) };
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
            let today = date_jakarta();
            let (today_pnl, today_trades) = repo
                .pnl_and_trade_count_for_bot(&bot_id, &today, &today)
                .unwrap_or((0.0, 0));
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
            repo.update_trade_result(
                &trade_id,
                &end_trade_iso,
                metrics.lp,
                metrics.total_cost,
                metrics.cpp,
                metrics.q_yes,
                metrics.q_no,
                "FINALIZED",
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
        bot_logger.info(&format!("Waiting 2s before next market... {current_slug}"));
        thread::sleep(Duration::from_secs(2));
    }

    signal_stop_event.store(true, Ordering::SeqCst);
    if let Some(hub) = signal_hub {
        hub.close();
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("fatal: {err:#}");
        std::process::exit(1);
    }
}
