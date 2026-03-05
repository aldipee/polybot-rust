use crate::config::BotConfig;
use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration, Utc};
use chrono_tz::Asia::Jakarta;
use native_tls::TlsConnector;
use postgres::Client;
use postgres_native_tls::MakeTlsConnector;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Engine {
    pub db_url: String,
}

pub fn make_engine(db_url: &str) -> Engine {
    Engine {
        db_url: db_url.to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct SessionFactory {
    engine: Engine,
}

pub fn make_session_factory(engine: Engine) -> SessionFactory {
    SessionFactory { engine }
}

impl SessionFactory {
    pub fn repository(&self) -> BotRepository {
        BotRepository {
            engine: self.engine.clone(),
        }
    }
}

fn open_conn(engine: &Engine) -> Result<Client> {
    if !engine.db_url.starts_with("postgres://") && !engine.db_url.starts_with("postgresql://") {
        return Err(anyhow!(
            "unsupported DB_URL (expected postgres:// or postgresql://): {}",
            engine.db_url
        ));
    }

    let tls = TlsConnector::builder()
        .build()
        .context("failed creating postgres TLS connector")?;
    let tls = MakeTlsConnector::new(tls);

    Client::connect(&engine.db_url, tls)
        .with_context(|| format!("failed opening postgres db {}", engine.db_url))
}

#[derive(Debug, Clone)]
pub struct BotRow {
    pub bot_id: String,
    pub bot_description: Option<String>,
    pub account_name: Option<String>,
    pub status: String,
    pub configuration_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ConfigurationRow {
    pub configuration_id: String,
    pub config_hash: String,
    pub clob_host: String,
    pub ws_base: String,
    pub chain_id: i64,
    pub private_key: String,
    pub signature_type: Option<i64>,
    pub funder: Option<String>,
    pub tick: f64,
    pub min_shares: f64,
    pub lock_profit_target: f64,
    pub clip_shares: f64,
    pub improve_bid_ticks: i64,
    pub maker_buffer_ticks: i64,
    pub replace_if_price_moves_ticks: i64,
    pub stale_seconds: i64,
    pub entry_edge_ticks: i64,
    pub hedge_buffer_ticks: i64,
    pub max_total_cost: f64,
    pub reserve_usd: f64,
    pub cancel_all_on_start: bool,
    pub dry_run: bool,
    pub log_every: i64,
    pub market_data_stale_seconds: i64,
    pub ws_reconnect_min: f64,
    pub ws_reconnect_max: f64,
    pub stop_buffer_seconds: i64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TradeRow {
    pub trade_id: String,
    pub bot_id: String,
    pub slug: String,
    pub configuration_id: String,
    pub date: String,
    pub start_trade: String,
    pub end_trade: String,
    pub entry_time: Option<String>,
    pub holding_duration_seconds: Option<f64>,
    pub entry_reason: Option<String>,
    pub exit_time: Option<String>,
    pub exit_reason_category: Option<String>,
    pub stop_loss_category: Option<String>,
    pub entry_price: Option<f64>,
    pub exit_price: Option<f64>,
    pub lp: f64,
    pub total_cost: f64,
    pub q_yes: f64,
    pub q_no: f64,
    pub cpp: f64,
    pub status: Option<String>,
    pub claim_status: Option<String>,
    pub meta_data: Option<String>,
    pub exit_reason: String,
    pub validation_status: String,
    pub validation_checked_at: Option<String>,
    pub validation_validated_at: Option<String>,
    pub validation_source: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BotTradeStats {
    pub net_pnl: f64,
    pub total_profit: f64,
    pub total_loss: f64,
    pub win_count: i64,
    pub loss_count: i64,
    pub total_count: i64,
}

#[derive(Debug, Clone)]
pub struct UnvalidatedTradeRow {
    pub trade_id: String,
    pub slug: String,
}

#[derive(Debug, Clone, Default)]
pub struct TradeDecisionUpsert {
    pub t_left_seconds: Option<f64>,
    pub tick_age_ms: Option<i64>,
    pub momentum_checks_passed: Option<i64>,
    pub momentum_checks_required: Option<i64>,
    pub momentum_trend_ok: Option<bool>,
    pub momentum_slope_ok: Option<bool>,
    pub momentum_candles_ok: Option<bool>,
    pub momentum_ema_fast_last: Option<f64>,
    pub momentum_ema_slow_last: Option<f64>,
    pub momentum_ema_fast_prev: Option<f64>,
    pub momentum_body_count: Option<i64>,
    pub breakout_dir: Option<String>,
    pub breakout_triggered: Option<bool>,
    pub breakout_reason: Option<String>,
    pub breakout_hk: Option<f64>,
    pub breakout_lk: Option<f64>,
    pub breakout_buf_up: Option<f64>,
    pub breakout_buf_dn: Option<f64>,
    pub breakout_persist_ms: Option<i64>,
    pub breakout_elapsed_ms: Option<i64>,
    pub breakout_cooldown_ms: Option<i64>,
    pub submit_origin: Option<String>,
    pub submit_side: Option<String>,
    pub submit_order_type: Option<String>,
    pub pm_best_bid: Option<f64>,
    pub pm_best_ask: Option<f64>,
    pub pm_mid: Option<f64>,
    pub pm_spread_abs: Option<f64>,
    pub pm_spread_pct: Option<f64>,
    pub pm_depth_bid_1tick: Option<f64>,
    pub pm_depth_ask_1tick: Option<f64>,
    pub order_type: Option<String>,
    pub limit_price_submitted: Option<f64>,
    pub fill_price_avg: Option<f64>,
    pub qty_requested: Option<f64>,
    pub qty_filled: Option<f64>,
    pub slippage_bps_vs_mid: Option<f64>,
    pub fees_paid: Option<f64>,
    pub decide_to_send_us: Option<i64>,
    pub send_to_ack_us: Option<i64>,
    pub decide_to_ack_us: Option<i64>,
    pub maker_downside: Option<f64>,
    pub maker_upside: Option<f64>,
    pub maker_skew_ratio: Option<f64>,
    pub maker_arb_triggered: Option<bool>,
    pub maker_arb_edge_after_fees: Option<f64>,
    pub maker_t_into_s: Option<f64>,
    pub maker_price_bucket: Option<String>,
    pub maker_clip_bucket: Option<String>,
}

pub fn now_iso_jakarta() -> String {
    Utc::now()
        .with_timezone(&Jakarta)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn date_jakarta() -> String {
    Utc::now()
        .with_timezone(&Jakarta)
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
}

pub fn week_start_date_jakarta() -> String {
    let d = Utc::now().with_timezone(&Jakarta).date_naive();
    let start = d - Duration::days(d.weekday().num_days_from_monday() as i64);
    start.format("%Y-%m-%d").to_string()
}

pub fn month_start_date_jakarta() -> String {
    let d = Utc::now().with_timezone(&Jakarta).date_naive();
    format!("{:04}-{:02}-01", d.year(), d.month())
}

fn canonicalize_json(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = BTreeMap::new();
            for (k, val) in map {
                out.insert(k, canonicalize_json(val));
            }
            let mut m = serde_json::Map::new();
            for (k, val) in out {
                m.insert(k, val);
            }
            Value::Object(m)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(canonicalize_json).collect()),
        other => other,
    }
}

pub fn cfg_hash(cfg: &BotConfig) -> String {
    let v = serde_json::to_value(cfg).unwrap_or(Value::Null);
    let canonical = canonicalize_json(v);
    let payload = serde_json::to_string(&canonical).unwrap_or_else(|_| "{}".to_string());
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Debug, Clone)]
pub struct BotRepository {
    engine: Engine,
}

impl BotRepository {
    pub fn init_schema(engine: &Engine) -> Result<()> {
        let mut conn = open_conn(engine)?;
        conn.batch_execute(
            r#"
CREATE TABLE IF NOT EXISTS bot (
  bot_id TEXT PRIMARY KEY,
  bot_description TEXT NULL,
  account_name TEXT NULL,
  status TEXT NOT NULL CHECK (status IN ('ACTIVE','DISABLED')),
  configuration_id TEXT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS configuration (
  configuration_id TEXT PRIMARY KEY,
  config_hash TEXT NOT NULL UNIQUE,
  clob_host TEXT NOT NULL,
  ws_base TEXT NOT NULL,
  chain_id BIGINT NOT NULL,
  private_key TEXT NOT NULL,
  signature_type BIGINT NULL,
  funder TEXT NULL,
  tick DOUBLE PRECISION NOT NULL,
  min_shares DOUBLE PRECISION NOT NULL,
  lock_profit_target DOUBLE PRECISION NOT NULL,
  clip_shares DOUBLE PRECISION NOT NULL,
  improve_bid_ticks BIGINT NOT NULL,
  maker_buffer_ticks BIGINT NOT NULL,
  replace_if_price_moves_ticks BIGINT NOT NULL,
  stale_seconds BIGINT NOT NULL,
  entry_edge_ticks BIGINT NOT NULL,
  hedge_buffer_ticks BIGINT NOT NULL,
  max_total_cost DOUBLE PRECISION NOT NULL,
  reserve_usd DOUBLE PRECISION NOT NULL,
  cancel_all_on_start BOOLEAN NOT NULL,
  dry_run BOOLEAN NOT NULL,
  log_every BIGINT NOT NULL,
  market_data_stale_seconds BIGINT NOT NULL,
  ws_reconnect_min DOUBLE PRECISION NOT NULL,
  ws_reconnect_max DOUBLE PRECISION NOT NULL,
  stop_buffer_seconds BIGINT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS trade (
  trade_id TEXT PRIMARY KEY,
  exit_reason TEXT NOT NULL,
  bot_id TEXT NOT NULL,
  slug TEXT NOT NULL,
  configuration_id TEXT NOT NULL,
  date TEXT NOT NULL,
  start_trade TEXT NOT NULL,
  end_trade TEXT NOT NULL,
  entry_time TEXT NULL,
  holding_duration_seconds DOUBLE PRECISION NULL,
  entry_reason TEXT NULL,
  exit_time TEXT NULL,
  exit_reason_category TEXT NULL,
  stop_loss_category TEXT NULL,
  entry_price DOUBLE PRECISION NULL,
  exit_price DOUBLE PRECISION NULL,
  lp DOUBLE PRECISION NOT NULL,
  total_cost DOUBLE PRECISION NOT NULL,
  q_yes DOUBLE PRECISION NOT NULL,
  q_no DOUBLE PRECISION NOT NULL,
  cpp DOUBLE PRECISION NOT NULL DEFAULT 0.0,
  status TEXT NULL,
  claim_status TEXT NULL,
  meta_data TEXT NULL,
  validation_status TEXT NOT NULL DEFAULT 'PENDING',
  validation_checked_at TEXT NULL,
  validation_validated_at TEXT NULL,
  validation_source TEXT NULL
);

CREATE TABLE IF NOT EXISTS trade_decisions (
  trade_id TEXT PRIMARY KEY,
  t_left_seconds DOUBLE PRECISION NULL,
  tick_age_ms BIGINT NULL,
  momentum_checks_passed BIGINT NULL,
  momentum_checks_required BIGINT NULL,
  momentum_trend_ok BOOLEAN NULL,
  momentum_slope_ok BOOLEAN NULL,
  momentum_candles_ok BOOLEAN NULL,
  momentum_ema_fast_last DOUBLE PRECISION NULL,
  momentum_ema_slow_last DOUBLE PRECISION NULL,
  momentum_ema_fast_prev DOUBLE PRECISION NULL,
  momentum_body_count BIGINT NULL,
  breakout_dir TEXT NULL,
  breakout_triggered BOOLEAN NULL,
  breakout_reason TEXT NULL,
  breakout_hk DOUBLE PRECISION NULL,
  breakout_lk DOUBLE PRECISION NULL,
  breakout_buf_up DOUBLE PRECISION NULL,
  breakout_buf_dn DOUBLE PRECISION NULL,
  breakout_persist_ms BIGINT NULL,
  breakout_elapsed_ms BIGINT NULL,
  breakout_cooldown_ms BIGINT NULL,
  submit_origin TEXT NULL,
  submit_side TEXT NULL,
  submit_order_type TEXT NULL,
  pm_best_bid DOUBLE PRECISION NULL,
  pm_best_ask DOUBLE PRECISION NULL,
  pm_mid DOUBLE PRECISION NULL,
  pm_spread_abs DOUBLE PRECISION NULL,
  pm_spread_pct DOUBLE PRECISION NULL,
  pm_depth_bid_1tick DOUBLE PRECISION NULL,
  pm_depth_ask_1tick DOUBLE PRECISION NULL,
  order_type TEXT NULL,
  limit_price_submitted DOUBLE PRECISION NULL,
  fill_price_avg DOUBLE PRECISION NULL,
  qty_requested DOUBLE PRECISION NULL,
  qty_filled DOUBLE PRECISION NULL,
  slippage_bps_vs_mid DOUBLE PRECISION NULL,
  fees_paid DOUBLE PRECISION NULL,
  decide_to_send_us BIGINT NULL,
  send_to_ack_us BIGINT NULL,
  decide_to_ack_us BIGINT NULL,
  maker_downside DOUBLE PRECISION NULL,
  maker_upside DOUBLE PRECISION NULL,
  maker_skew_ratio DOUBLE PRECISION NULL,
  maker_arb_triggered BOOLEAN NULL,
  maker_arb_edge_after_fees DOUBLE PRECISION NULL,
  maker_t_into_s DOUBLE PRECISION NULL,
  maker_price_bucket TEXT NULL,
  maker_clip_bucket TEXT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
"#,
        )
        .context("failed creating schema")?;
        // Compatibility migration for pre-existing Postgres schemas that used int4/legacy flag types.
        conn.batch_execute(
            r#"
ALTER TABLE configuration ALTER COLUMN chain_id TYPE BIGINT USING chain_id::BIGINT;
ALTER TABLE configuration ALTER COLUMN signature_type TYPE BIGINT USING signature_type::BIGINT;
ALTER TABLE configuration ALTER COLUMN improve_bid_ticks TYPE BIGINT USING improve_bid_ticks::BIGINT;
ALTER TABLE configuration ALTER COLUMN maker_buffer_ticks TYPE BIGINT USING maker_buffer_ticks::BIGINT;
ALTER TABLE configuration ALTER COLUMN replace_if_price_moves_ticks TYPE BIGINT USING replace_if_price_moves_ticks::BIGINT;
ALTER TABLE configuration ALTER COLUMN stale_seconds TYPE BIGINT USING stale_seconds::BIGINT;
ALTER TABLE configuration ALTER COLUMN entry_edge_ticks TYPE BIGINT USING entry_edge_ticks::BIGINT;
ALTER TABLE configuration ALTER COLUMN hedge_buffer_ticks TYPE BIGINT USING hedge_buffer_ticks::BIGINT;
ALTER TABLE configuration ALTER COLUMN log_every TYPE BIGINT USING log_every::BIGINT;
ALTER TABLE configuration ALTER COLUMN market_data_stale_seconds TYPE BIGINT USING market_data_stale_seconds::BIGINT;
ALTER TABLE configuration ALTER COLUMN stop_buffer_seconds TYPE BIGINT USING stop_buffer_seconds::BIGINT;
ALTER TABLE configuration ALTER COLUMN cancel_all_on_start TYPE BOOLEAN USING
    CASE
        WHEN cancel_all_on_start::TEXT IN ('1', 't', 'true', 'TRUE') THEN TRUE
        ELSE FALSE
    END;
ALTER TABLE configuration ALTER COLUMN dry_run TYPE BOOLEAN USING
    CASE
        WHEN dry_run::TEXT IN ('1', 't', 'true', 'TRUE') THEN TRUE
        ELSE FALSE
    END;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_status TEXT NOT NULL DEFAULT 'PENDING';
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_checked_at TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_validated_at TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_source TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS entry_time TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS holding_duration_seconds DOUBLE PRECISION NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS entry_reason TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS exit_time TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS exit_reason_category TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS stop_loss DOUBLE PRECISION NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS stop_loss_category TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS entry_price DOUBLE PRECISION NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS exit_price DOUBLE PRECISION NULL;
UPDATE trade
SET validation_status = 'PENDING'
WHERE validation_status IS NULL OR trim(validation_status) = '';
UPDATE trade
SET entry_time = start_trade
WHERE COALESCE(trim(entry_time), '') = ''
  AND COALESCE(trim(start_trade), '') <> '';
UPDATE trade
SET exit_time = end_trade
WHERE COALESCE(trim(exit_time), '') = ''
  AND COALESCE(trim(end_trade), '') <> '';
UPDATE trade
SET entry_reason = 'INITIALIZED'
WHERE entry_reason IS NULL OR trim(entry_reason) = '';
UPDATE trade
SET exit_reason_category = CASE
    WHEN upper(COALESCE(exit_reason, '')) LIKE '%STOP_LOSS%' THEN 'STOP_LOSS'
    WHEN upper(COALESCE(exit_reason, '')) LIKE '%TAKE_PROFIT%'
      OR upper(COALESCE(exit_reason, '')) LIKE '%TARGET_HIT%' THEN 'TAKE_PROFIT'
    ELSE 'RESOLUTION'
END
WHERE exit_reason_category IS NULL OR trim(exit_reason_category) = '';
UPDATE trade
SET stop_loss_category = 'MARKET'
WHERE (stop_loss_category IS NULL OR trim(stop_loss_category) = '')
  AND (
    upper(COALESCE(exit_reason, '')) LIKE '%STOP_LOSS%'
    OR upper(COALESCE(exit_reason, '')) LIKE '%CAP_LOCKED_LOSS%'
  );
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS t_left_seconds DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS tick_age_ms BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_checks_passed BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_checks_required BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_trend_ok BOOLEAN NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_slope_ok BOOLEAN NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_candles_ok BOOLEAN NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_ema_fast_last DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_ema_slow_last DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_ema_fast_prev DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS momentum_body_count BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_dir TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_triggered BOOLEAN NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_reason TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_hk DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_lk DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_buf_up DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_buf_dn DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_persist_ms BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_elapsed_ms BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS breakout_cooldown_ms BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS submit_origin TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS submit_side TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS submit_order_type TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pm_best_bid DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pm_best_ask DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pm_mid DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pm_spread_abs DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pm_spread_pct DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pm_depth_bid_1tick DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pm_depth_ask_1tick DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS order_type TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS limit_price_submitted DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS fill_price_avg DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS qty_requested DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS qty_filled DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS slippage_bps_vs_mid DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS fees_paid DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS decide_to_send_us BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS send_to_ack_us BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS decide_to_ack_us BIGINT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_downside DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_upside DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_skew_ratio DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_arb_triggered BOOLEAN NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_arb_edge_after_fees DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_t_into_s DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_price_bucket TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS maker_clip_bucket TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS updated_at TEXT NOT NULL DEFAULT '';
UPDATE trade_decisions SET created_at = COALESCE(NULLIF(trim(created_at), ''), '1970-01-01T00:00:00+00:00')
WHERE COALESCE(trim(created_at), '') = '';
UPDATE trade_decisions SET updated_at = COALESCE(NULLIF(trim(updated_at), ''), '1970-01-01T00:00:00+00:00')
WHERE COALESCE(trim(updated_at), '') = '';
"#,
        )
        .context("failed migrating configuration schema for PostgreSQL compatibility")?;
        Ok(())
    }

    pub fn upsert_bot(
        &self,
        bot_id: &str,
        bot_description: &str,
        account_name: &str,
        status: &str,
        configuration_id: &str,
    ) -> Result<()> {
        let now = now_iso_jakarta();
        let mut conn = open_conn(&self.engine)?;
        let existing = conn.query_opt("SELECT bot_id FROM bot WHERE bot_id = $1", &[&bot_id])?;
        if existing.is_some() {
            conn.execute(
                "UPDATE bot SET bot_description=$1, account_name=$2, status=$3, configuration_id=$4, updated_at=$5 WHERE bot_id=$6",
                &[&bot_description, &account_name, &status, &configuration_id, &now, &bot_id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO bot (bot_id, bot_description, account_name, status, configuration_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&bot_id, &bot_description, &account_name, &status, &configuration_id, &now, &now],
            )?;
        }
        Ok(())
    }

    pub fn get_bot(&self, bot_id: &str) -> Result<Option<BotRow>> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_opt(
            "SELECT bot_id, bot_description, account_name, status, configuration_id, created_at, updated_at FROM bot WHERE bot_id = $1",
            &[&bot_id],
        )?;

        Ok(row.map(|r| BotRow {
            bot_id: r.get(0),
            bot_description: r.get(1),
            account_name: r.get(2),
            status: r.get(3),
            configuration_id: r.get(4),
            created_at: r.get(5),
            updated_at: r.get(6),
        }))
    }

    pub fn upsert_configuration(&self, cfg: &BotConfig) -> Result<String> {
        let h = cfg_hash(cfg);
        let mut conn = open_conn(&self.engine)?;

        if let Some(row) = conn.query_opt(
            "SELECT configuration_id FROM configuration WHERE config_hash = $1",
            &[&h],
        )? {
            let existing_id: String = row.get(0);
            return Ok(existing_id);
        }

        let cid = new_uuid();
        let now = now_iso_jakarta();
        conn.execute(
            "INSERT INTO configuration (
                configuration_id, config_hash,
                clob_host, ws_base, chain_id, private_key, signature_type, funder,
                tick, min_shares, lock_profit_target,
                clip_shares, improve_bid_ticks, maker_buffer_ticks, replace_if_price_moves_ticks, stale_seconds,
                entry_edge_ticks, hedge_buffer_ticks, max_total_cost, reserve_usd,
                cancel_all_on_start, dry_run, log_every,
                market_data_stale_seconds, ws_reconnect_min, ws_reconnect_max,
                stop_buffer_seconds, created_at
            ) VALUES (
                $1, $2,
                $3, $4, $5, $6, $7, $8,
                $9, $10, $11,
                $12, $13, $14, $15, $16,
                $17, $18, $19, $20,
                $21, $22, $23,
                $24, $25, $26,
                $27, $28
            )",
            &[
                &cid,
                &h,
                &cfg.clob_host,
                &cfg.ws_base,
                &cfg.chain_id,
                &cfg.private_key,
                &cfg.signature_type,
                &cfg.funder,
                &cfg.tick,
                &cfg.min_shares,
                &cfg.lock_profit_target,
                &cfg.clip_shares,
                &cfg.improve_bid_ticks,
                &cfg.maker_buffer_ticks,
                &cfg.replace_if_price_moves_ticks,
                &cfg.stale_seconds,
                &cfg.entry_edge_ticks,
                &cfg.hedge_buffer_ticks,
                &cfg.max_total_cost,
                &cfg.reserve_usd,
                &cfg.cancel_all_on_start,
                &cfg.dry_run,
                &cfg.log_every,
                &cfg.market_data_stale_seconds,
                &cfg.ws_reconnect_min,
                &cfg.ws_reconnect_max,
                &cfg.stop_buffer_seconds,
                &now,
            ],
        )?;
        Ok(cid)
    }

    pub fn get_configuration(&self, configuration_id: &str) -> Result<Option<ConfigurationRow>> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_opt(
            "SELECT
                configuration_id, config_hash,
                clob_host, ws_base, chain_id, private_key, signature_type, funder,
                tick, min_shares, lock_profit_target,
                clip_shares, improve_bid_ticks, maker_buffer_ticks, replace_if_price_moves_ticks, stale_seconds,
                entry_edge_ticks, hedge_buffer_ticks, max_total_cost, reserve_usd,
                cancel_all_on_start, dry_run, log_every,
                market_data_stale_seconds, ws_reconnect_min, ws_reconnect_max,
                stop_buffer_seconds, created_at
             FROM configuration WHERE configuration_id = $1",
            &[&configuration_id],
        )?;

        Ok(row.map(|r| ConfigurationRow {
            configuration_id: r.get(0),
            config_hash: r.get(1),
            clob_host: r.get(2),
            ws_base: r.get(3),
            chain_id: r.get(4),
            private_key: r.get(5),
            signature_type: r.get(6),
            funder: r.get(7),
            tick: r.get(8),
            min_shares: r.get(9),
            lock_profit_target: r.get(10),
            clip_shares: r.get(11),
            improve_bid_ticks: r.get(12),
            maker_buffer_ticks: r.get(13),
            replace_if_price_moves_ticks: r.get(14),
            stale_seconds: r.get(15),
            entry_edge_ticks: r.get(16),
            hedge_buffer_ticks: r.get(17),
            max_total_cost: r.get(18),
            reserve_usd: r.get(19),
            cancel_all_on_start: r.get(20),
            dry_run: r.get(21),
            log_every: r.get(22),
            market_data_stale_seconds: r.get(23),
            ws_reconnect_min: r.get(24),
            ws_reconnect_max: r.get(25),
            stop_buffer_seconds: r.get(26),
            created_at: r.get(27),
        }))
    }

    pub fn pnl_and_trade_count_for_bot(
        &self,
        bot_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<(f64, i64)> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT COALESCE(SUM(lp), 0.0), COUNT(trade_id)
             FROM trade
             WHERE bot_id = $1
               AND date >= $2
               AND date <= $3
               AND status IN ('WON','LOSS')",
            &[&bot_id, &start_date, &end_date],
        )?;

        let pnl: f64 = row.get(0);
        let cnt: i64 = row.get(1);
        Ok((pnl, cnt))
    }

    pub fn pnl_and_trade_count_all_bots(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<(f64, i64)> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT COALESCE(SUM(lp), 0.0), COUNT(trade_id)
             FROM trade
             WHERE date >= $1
               AND date <= $2
               AND status IN ('WON','LOSS')",
            &[&start_date, &end_date],
        )?;

        let pnl: f64 = row.get(0);
        let cnt: i64 = row.get(1);
        Ok((pnl, cnt))
    }

    pub fn trade_stats_for_bot_period(
        &self,
        bot_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<BotTradeStats> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT
                COALESCE(SUM(lp), 0.0) AS net_pnl,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN lp ELSE 0.0 END), 0.0) AS total_profit,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN -lp ELSE 0.0 END), 0.0) AS total_loss,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN 1 ELSE 0 END), 0)::BIGINT AS win_count,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN 1 ELSE 0 END), 0)::BIGINT AS loss_count,
                COUNT(trade_id)::BIGINT AS total_count
             FROM trade
             WHERE bot_id = $1
               AND date >= $2
               AND date <= $3
               AND status IN ('WON','LOSS')",
            &[&bot_id, &start_date, &end_date],
        )?;
        Ok(BotTradeStats {
            net_pnl: row.get(0),
            total_profit: row.get(1),
            total_loss: row.get(2),
            win_count: row.get(3),
            loss_count: row.get(4),
            total_count: row.get(5),
        })
    }

    pub fn trade_stats_for_bot_recent_hours(
        &self,
        bot_id: &str,
        cutoff_iso: &str,
    ) -> Result<BotTradeStats> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT
                COALESCE(SUM(lp), 0.0) AS net_pnl,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN lp ELSE 0.0 END), 0.0) AS total_profit,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN -lp ELSE 0.0 END), 0.0) AS total_loss,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN 1 ELSE 0 END), 0)::BIGINT AS win_count,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN 1 ELSE 0 END), 0)::BIGINT AS loss_count,
                COUNT(trade_id)::BIGINT AS total_count
             FROM trade
             WHERE bot_id = $1
               AND end_trade >= $2
               AND status IN ('WON','LOSS')",
            &[&bot_id, &cutoff_iso],
        )?;
        Ok(BotTradeStats {
            net_pnl: row.get(0),
            total_profit: row.get(1),
            total_loss: row.get(2),
            win_count: row.get(3),
            loss_count: row.get(4),
            total_count: row.get(5),
        })
    }

    pub fn trade_stats_for_bot_all_time(&self, bot_id: &str) -> Result<BotTradeStats> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT
                COALESCE(SUM(lp), 0.0) AS net_pnl,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN lp ELSE 0.0 END), 0.0) AS total_profit,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN -lp ELSE 0.0 END), 0.0) AS total_loss,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN 1 ELSE 0 END), 0)::BIGINT AS win_count,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN 1 ELSE 0 END), 0)::BIGINT AS loss_count,
                COUNT(trade_id)::BIGINT AS total_count
             FROM trade
             WHERE bot_id = $1
               AND status IN ('WON','LOSS')",
            &[&bot_id],
        )?;
        Ok(BotTradeStats {
            net_pnl: row.get(0),
            total_profit: row.get(1),
            total_loss: row.get(2),
            win_count: row.get(3),
            loss_count: row.get(4),
            total_count: row.get(5),
        })
    }

    pub fn trade_stats_all_bots_period(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<BotTradeStats> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT
                COALESCE(SUM(lp), 0.0) AS net_pnl,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN lp ELSE 0.0 END), 0.0) AS total_profit,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN -lp ELSE 0.0 END), 0.0) AS total_loss,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN 1 ELSE 0 END), 0)::BIGINT AS win_count,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN 1 ELSE 0 END), 0)::BIGINT AS loss_count,
                COUNT(trade_id)::BIGINT AS total_count
             FROM trade
             WHERE date >= $1
               AND date <= $2
               AND status IN ('WON','LOSS')",
            &[&start_date, &end_date],
        )?;
        Ok(BotTradeStats {
            net_pnl: row.get(0),
            total_profit: row.get(1),
            total_loss: row.get(2),
            win_count: row.get(3),
            loss_count: row.get(4),
            total_count: row.get(5),
        })
    }

    pub fn trade_stats_all_bots_all_time(&self) -> Result<BotTradeStats> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT
                COALESCE(SUM(lp), 0.0) AS net_pnl,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN lp ELSE 0.0 END), 0.0) AS total_profit,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN -lp ELSE 0.0 END), 0.0) AS total_loss,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN 1 ELSE 0 END), 0)::BIGINT AS win_count,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN 1 ELSE 0 END), 0)::BIGINT AS loss_count,
                COUNT(trade_id)::BIGINT AS total_count
             FROM trade
             WHERE status IN ('WON','LOSS')",
            &[],
        )?;
        Ok(BotTradeStats {
            net_pnl: row.get(0),
            total_profit: row.get(1),
            total_loss: row.get(2),
            win_count: row.get(3),
            loss_count: row.get(4),
            total_count: row.get(5),
        })
    }

    pub fn trade_stats_all_bots_recent_hours(&self, cutoff_iso: &str) -> Result<BotTradeStats> {
        let mut conn = open_conn(&self.engine)?;
        let row = conn.query_one(
            "SELECT
                COALESCE(SUM(lp), 0.0) AS net_pnl,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN lp ELSE 0.0 END), 0.0) AS total_profit,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN -lp ELSE 0.0 END), 0.0) AS total_loss,
                COALESCE(SUM(CASE WHEN status = 'WON' THEN 1 ELSE 0 END), 0)::BIGINT AS win_count,
                COALESCE(SUM(CASE WHEN status = 'LOSS' THEN 1 ELSE 0 END), 0)::BIGINT AS loss_count,
                COUNT(trade_id)::BIGINT AS total_count
             FROM trade
             WHERE end_trade >= $1
               AND status IN ('WON','LOSS')",
            &[&cutoff_iso],
        )?;
        Ok(BotTradeStats {
            net_pnl: row.get(0),
            total_profit: row.get(1),
            total_loss: row.get(2),
            win_count: row.get(3),
            loss_count: row.get(4),
            total_count: row.get(5),
        })
    }

    pub fn list_all_bot_ids(&self) -> Result<Vec<String>> {
        let mut conn = open_conn(&self.engine)?;
        let has_bot_type = conn
            .query_one(
                "SELECT EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = current_schema()
                      AND table_name = 'bot'
                      AND column_name = 'bot_type'
                )",
                &[],
            )
            .map(|r| r.get::<usize, bool>(0))
            .unwrap_or(false);
        let rows = if has_bot_type {
            conn.query(
                "SELECT DISTINCT bot_id
                 FROM bot
                 WHERE bot_id IS NOT NULL
                   AND trim(bot_id) <> ''
                   AND upper(coalesce(bot_type, '')) = 'TRADING'
                 ORDER BY bot_id ASC",
                &[],
            )?
        } else {
            conn.query(
                "SELECT DISTINCT bot_id
                 FROM (
                    SELECT bot_id FROM bot
                    UNION
                    SELECT bot_id FROM trade
                 ) AS ids
                 WHERE bot_id IS NOT NULL
                   AND trim(bot_id) <> ''
                 ORDER BY bot_id ASC",
                &[],
            )?
        };
        Ok(rows
            .into_iter()
            .map(|r| r.get::<usize, String>(0))
            .collect())
    }

    pub fn create_pending_trade(
        &self,
        bot_id: &str,
        slug: &str,
        configuration_id: &str,
        start_trade_iso: &str,
    ) -> Result<(String, String)> {
        let mut conn = open_conn(&self.engine)?;
        if let Some(row) = conn.query_opt(
            "SELECT trade_id, status FROM trade WHERE bot_id = $1 AND slug = $2 LIMIT 1",
            &[&bot_id, &slug],
        )? {
            let trade_id: String = row.get(0);
            let status: Option<String> = row.get(1);
            return Ok((trade_id, status.unwrap_or_default()));
        }

        let tid = new_uuid();
        let empty = String::new();
        let running = "RUNNING".to_string();
        let initialized = "INITIALIZED".to_string();
        let none_claim: Option<String> = None;
        let none_meta: Option<String> = None;
        let none_duration: Option<f64> = None;
        let none_text: Option<String> = None;
        let none_price: Option<f64> = None;

        conn.execute(
            "INSERT INTO trade (
                trade_id, exit_reason, bot_id, slug, configuration_id,
                date, start_trade, end_trade,
                entry_time, holding_duration_seconds, entry_reason, exit_time, exit_reason_category,
                stop_loss_category, entry_price, exit_price,
                lp, total_cost, q_yes, q_no, cpp, status, claim_status, meta_data
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8,
                $9, $10, $11, $12, $13,
                $14, $15, $16,
                $17, $18, $19, $20, $21, $22, $23, $24
            )",
            &[
                &tid,
                &running,
                &bot_id,
                &slug,
                &configuration_id,
                &date_jakarta(),
                &start_trade_iso,
                &empty,
                &start_trade_iso,
                &none_duration,
                &initialized,
                &none_text,
                &none_text,
                &none_text,
                &none_price,
                &none_price,
                &0.0_f64,
                &0.0_f64,
                &0.0_f64,
                &0.0_f64,
                &0.0_f64,
                &initialized,
                &none_claim,
                &none_meta,
            ],
        )?;
        Ok((tid, initialized))
    }

    pub fn list_unvalidated_trades_for_bot(
        &self,
        bot_id: &str,
        start_date: &str,
        limit: i64,
    ) -> Result<Vec<UnvalidatedTradeRow>> {
        let mut conn = open_conn(&self.engine)?;
        let rows = conn.query(
            "SELECT trade_id, slug
             FROM trade
             WHERE bot_id = $1
               AND date >= $2
               AND status IN ('WON','LOSS','DRAW')
               AND NOT (
                    status = 'DRAW'
                    AND COALESCE(total_cost, 0.0) <= 1e-9
                    AND COALESCE(q_yes, 0.0) <= 1e-9
                    AND COALESCE(q_no, 0.0) <= 1e-9
               )
               AND COALESCE(validation_status, 'PENDING') <> 'VALIDATED'
             ORDER BY end_trade ASC, start_trade ASC
             LIMIT $3",
            &[&bot_id, &start_date, &limit],
        )?;
        Ok(rows
            .into_iter()
            .map(|r| UnvalidatedTradeRow {
                trade_id: r.get(0),
                slug: r.get(1),
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_trade_result(
        &self,
        trade_id: &str,
        end_trade_iso: &str,
        lp: f64,
        total_cost: f64,
        cpp: f64,
        q_yes: f64,
        q_no: f64,
        exit_reason: &str,
        entry_time_iso: Option<&str>,
        holding_duration_seconds: Option<f64>,
        entry_reason: Option<&str>,
        exit_reason_category: Option<&str>,
        stop_loss_category: Option<&str>,
        entry_price: Option<f64>,
        exit_price: Option<f64>,
    ) -> Result<()> {
        let status = if lp > 0.0 {
            "WON"
        } else if lp < 0.0 {
            "LOSS"
        } else {
            "DRAW"
        };
        let validation_pending = "PENDING".to_string();
        let validation_checked_at: Option<String> = None;
        let validation_validated_at: Option<String> = None;
        let validation_source: Option<String> = None;
        let exit_time_iso: Option<&str> = Some(end_trade_iso);
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "UPDATE trade SET
                end_trade = $1,
                exit_time = $2,
                entry_time = COALESCE($3, entry_time),
                holding_duration_seconds = COALESCE($4, holding_duration_seconds),
                entry_reason = COALESCE($5, entry_reason),
                exit_reason_category = COALESCE($6, exit_reason_category),
                stop_loss_category = COALESCE($7, stop_loss_category),
                entry_price = COALESCE($8, entry_price),
                exit_price = COALESCE($9, exit_price),
                lp = $10,
                total_cost = $11,
                cpp = $12,
                q_yes = $13,
                q_no = $14,
                exit_reason = $15,
                status = $16,
                validation_status = $17,
                validation_checked_at = $18,
                validation_validated_at = $19,
                validation_source = $20
             WHERE trade_id = $21",
            &[
                &end_trade_iso,
                &exit_time_iso,
                &entry_time_iso,
                &holding_duration_seconds,
                &entry_reason,
                &exit_reason_category,
                &stop_loss_category,
                &entry_price,
                &exit_price,
                &lp,
                &total_cost,
                &cpp,
                &q_yes,
                &q_no,
                &exit_reason,
                &status,
                &validation_pending,
                &validation_checked_at,
                &validation_validated_at,
                &validation_source,
                &trade_id,
            ],
        )?;
        Ok(())
    }

    pub fn touch_trade_validation_checked(
        &self,
        trade_id: &str,
        checked_at_iso: &str,
    ) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "UPDATE trade
             SET validation_checked_at = $1
             WHERE trade_id = $2",
            &[&checked_at_iso, &trade_id],
        )?;
        Ok(())
    }

    pub fn mark_trade_validated_from_polymarket(
        &self,
        trade_id: &str,
        lp: f64,
        checked_at_iso: &str,
        source: &str,
    ) -> Result<()> {
        let status = if lp > 0.0 {
            "WON"
        } else if lp < 0.0 {
            "LOSS"
        } else {
            "DRAW"
        };
        let validated = "VALIDATED".to_string();
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "UPDATE trade
             SET lp = $1,
                 status = $2,
                 validation_status = $3,
                 validation_checked_at = $4,
                 validation_validated_at = $5,
                 validation_source = $6
             WHERE trade_id = $7",
            &[
                &lp,
                &status,
                &validated,
                &checked_at_iso,
                &checked_at_iso,
                &source,
                &trade_id,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_trade_decision(
        &self,
        trade_id: &str,
        row: &TradeDecisionUpsert,
    ) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        let now = now_iso_jakarta();
        conn.execute(
            "INSERT INTO trade_decisions (
                trade_id,
                t_left_seconds, tick_age_ms,
                momentum_checks_passed, momentum_checks_required, momentum_trend_ok, momentum_slope_ok, momentum_candles_ok,
                momentum_ema_fast_last, momentum_ema_slow_last, momentum_ema_fast_prev, momentum_body_count,
                breakout_dir, breakout_triggered, breakout_reason, breakout_hk, breakout_lk, breakout_buf_up, breakout_buf_dn,
                breakout_persist_ms, breakout_elapsed_ms, breakout_cooldown_ms,
                submit_origin, submit_side, submit_order_type,
                pm_best_bid, pm_best_ask, pm_mid, pm_spread_abs, pm_spread_pct,
                pm_depth_bid_1tick, pm_depth_ask_1tick,
                order_type, limit_price_submitted, fill_price_avg, qty_requested, qty_filled,
                slippage_bps_vs_mid, fees_paid,
                decide_to_send_us, send_to_ack_us, decide_to_ack_us,
                maker_downside, maker_upside, maker_skew_ratio, maker_arb_triggered, maker_arb_edge_after_fees,
                maker_t_into_s, maker_price_bucket, maker_clip_bucket,
                created_at, updated_at
            ) VALUES (
                $1,
                $2, $3,
                $4, $5, $6, $7, $8,
                $9, $10, $11, $12,
                $13, $14, $15, $16, $17, $18, $19,
                $20, $21, $22,
                $23, $24, $25,
                $26, $27, $28, $29, $30,
                $31, $32,
                $33, $34, $35, $36, $37,
                $38, $39,
                $40, $41, $42,
                $43, $44, $45, $46, $47,
                $48, $49, $50,
                $51, $52
            )
            ON CONFLICT (trade_id) DO UPDATE SET
                t_left_seconds = EXCLUDED.t_left_seconds,
                tick_age_ms = EXCLUDED.tick_age_ms,
                momentum_checks_passed = EXCLUDED.momentum_checks_passed,
                momentum_checks_required = EXCLUDED.momentum_checks_required,
                momentum_trend_ok = EXCLUDED.momentum_trend_ok,
                momentum_slope_ok = EXCLUDED.momentum_slope_ok,
                momentum_candles_ok = EXCLUDED.momentum_candles_ok,
                momentum_ema_fast_last = EXCLUDED.momentum_ema_fast_last,
                momentum_ema_slow_last = EXCLUDED.momentum_ema_slow_last,
                momentum_ema_fast_prev = EXCLUDED.momentum_ema_fast_prev,
                momentum_body_count = EXCLUDED.momentum_body_count,
                breakout_dir = EXCLUDED.breakout_dir,
                breakout_triggered = EXCLUDED.breakout_triggered,
                breakout_reason = EXCLUDED.breakout_reason,
                breakout_hk = EXCLUDED.breakout_hk,
                breakout_lk = EXCLUDED.breakout_lk,
                breakout_buf_up = EXCLUDED.breakout_buf_up,
                breakout_buf_dn = EXCLUDED.breakout_buf_dn,
                breakout_persist_ms = EXCLUDED.breakout_persist_ms,
                breakout_elapsed_ms = EXCLUDED.breakout_elapsed_ms,
                breakout_cooldown_ms = EXCLUDED.breakout_cooldown_ms,
                submit_origin = EXCLUDED.submit_origin,
                submit_side = EXCLUDED.submit_side,
                submit_order_type = EXCLUDED.submit_order_type,
                pm_best_bid = EXCLUDED.pm_best_bid,
                pm_best_ask = EXCLUDED.pm_best_ask,
                pm_mid = EXCLUDED.pm_mid,
                pm_spread_abs = EXCLUDED.pm_spread_abs,
                pm_spread_pct = EXCLUDED.pm_spread_pct,
                pm_depth_bid_1tick = EXCLUDED.pm_depth_bid_1tick,
                pm_depth_ask_1tick = EXCLUDED.pm_depth_ask_1tick,
                order_type = EXCLUDED.order_type,
                limit_price_submitted = EXCLUDED.limit_price_submitted,
                fill_price_avg = EXCLUDED.fill_price_avg,
                qty_requested = EXCLUDED.qty_requested,
                qty_filled = EXCLUDED.qty_filled,
                slippage_bps_vs_mid = EXCLUDED.slippage_bps_vs_mid,
                fees_paid = EXCLUDED.fees_paid,
                decide_to_send_us = EXCLUDED.decide_to_send_us,
                send_to_ack_us = EXCLUDED.send_to_ack_us,
                decide_to_ack_us = EXCLUDED.decide_to_ack_us,
                maker_downside = EXCLUDED.maker_downside,
                maker_upside = EXCLUDED.maker_upside,
                maker_skew_ratio = EXCLUDED.maker_skew_ratio,
                maker_arb_triggered = EXCLUDED.maker_arb_triggered,
                maker_arb_edge_after_fees = EXCLUDED.maker_arb_edge_after_fees,
                maker_t_into_s = EXCLUDED.maker_t_into_s,
                maker_price_bucket = EXCLUDED.maker_price_bucket,
                maker_clip_bucket = EXCLUDED.maker_clip_bucket,
                updated_at = EXCLUDED.updated_at",
            &[
                &trade_id,
                &row.t_left_seconds,
                &row.tick_age_ms,
                &row.momentum_checks_passed,
                &row.momentum_checks_required,
                &row.momentum_trend_ok,
                &row.momentum_slope_ok,
                &row.momentum_candles_ok,
                &row.momentum_ema_fast_last,
                &row.momentum_ema_slow_last,
                &row.momentum_ema_fast_prev,
                &row.momentum_body_count,
                &row.breakout_dir,
                &row.breakout_triggered,
                &row.breakout_reason,
                &row.breakout_hk,
                &row.breakout_lk,
                &row.breakout_buf_up,
                &row.breakout_buf_dn,
                &row.breakout_persist_ms,
                &row.breakout_elapsed_ms,
                &row.breakout_cooldown_ms,
                &row.submit_origin,
                &row.submit_side,
                &row.submit_order_type,
                &row.pm_best_bid,
                &row.pm_best_ask,
                &row.pm_mid,
                &row.pm_spread_abs,
                &row.pm_spread_pct,
                &row.pm_depth_bid_1tick,
                &row.pm_depth_ask_1tick,
                &row.order_type,
                &row.limit_price_submitted,
                &row.fill_price_avg,
                &row.qty_requested,
                &row.qty_filled,
                &row.slippage_bps_vs_mid,
                &row.fees_paid,
                &row.decide_to_send_us,
                &row.send_to_ack_us,
                &row.decide_to_ack_us,
                &row.maker_downside,
                &row.maker_upside,
                &row.maker_skew_ratio,
                &row.maker_arb_triggered,
                &row.maker_arb_edge_after_fees,
                &row.maker_t_into_s,
                &row.maker_price_bucket,
                &row.maker_clip_bucket,
                &now,
                &now,
            ],
        )?;
        Ok(())
    }

    pub fn delete_trade(&self, trade_id: &str) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        conn.execute("DELETE FROM trade_decisions WHERE trade_id = $1", &[&trade_id])?;
        conn.execute("DELETE FROM trade WHERE trade_id = $1", &[&trade_id])?;
        Ok(())
    }
}
