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
use config::BotConfig;
use db::{
    date_jakarta, make_engine, make_session_factory, month_start_date_jakarta, now_iso_jakarta,
    week_start_date_jakarta, BotRepository, ConfigurationRow,
};
use env_utils::{env_bool, env_float};
use helpers::{get_next_slug, segment, segment_defaults};
use logging::{setup_item_logger, LogLike};
use r2_storage::upload_logs_before_rollover;
use rtds::RtdsService;
use signal::{JsonlFileService, SignalHub, SignalInbox};
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
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

fn print_pnl_metrics(repo: &BotRepository, bot_id: &str, logger: &Arc<dyn LogLike>) -> String {
    let today = date_jakarta();
    let week_start = week_start_date_jakarta();
    let month_start = month_start_date_jakarta();

    let b_today = repo
        .pnl_and_trade_count_for_bot(bot_id, &today, &today)
        .unwrap_or((0.0, 0));
    let b_week = repo
        .pnl_and_trade_count_for_bot(bot_id, &week_start, &today)
        .unwrap_or((0.0, 0));
    let b_month = repo
        .pnl_and_trade_count_for_bot(bot_id, &month_start, &today)
        .unwrap_or((0.0, 0));

    let a_today = repo
        .pnl_and_trade_count_all_bots(&today, &today)
        .unwrap_or((0.0, 0));
    let a_week = repo
        .pnl_and_trade_count_all_bots(&week_start, &today)
        .unwrap_or((0.0, 0));
    let a_month = repo
        .pnl_and_trade_count_all_bots(&month_start, &today)
        .unwrap_or((0.0, 0));

    let msg = format!(
        "PNL Summary (Asia/Jakarta)\n  Bot {bot_id} Today  : PNL={:+.4} | Trades={}\n  Bot {bot_id} Weekly : PNL={:+.4} | Trades={}\n  Bot {bot_id} Monthly: PNL={:+.4} | Trades={}\n  ALL bots Today      : PNL={:+.4} | Trades={}\n  ALL bots Weekly     : PNL={:+.4} | Trades={}\n  ALL bots Monthly    : PNL={:+.4} | Trades={}",
        b_today.0,
        b_today.1,
        b_week.0,
        b_week.1,
        b_month.0,
        b_month.1,
        a_today.0,
        a_today.1,
        a_week.0,
        a_week.1,
        a_month.0,
        a_month.1
    );
    logger.info(&msg);
    msg
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
                } else {
                    return Err(anyhow!(
                        "Missing MARKET_SLUG and no signal received from SIGNAL_WS_URL"
                    ));
                }
            } else {
                return Err(anyhow!("Missing MARKET_SLUG"));
            }
        } else {
            return Err(anyhow!("Missing MARKET_SLUG"));
        }
    }

    cfg.apply_safe_defaults();
    let mut current_slug = slug;

    loop {
        let bot_logger = setup_item_logger(&current_slug);
        bot_logger.info(&format!("\nSTARTING MARKET: {current_slug}"));
        let repo = session_factory.repository();

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

        let metrics = bot.trade_metrics_snapshot();
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
