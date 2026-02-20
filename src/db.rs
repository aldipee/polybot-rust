use crate::config::BotConfig;
use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration, Utc};
use chrono_tz::Asia::Jakarta;
use postgres::{Client, NoTls};
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

    Client::connect(&engine.db_url, NoTls)
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
    pub lp: f64,
    pub total_cost: f64,
    pub q_yes: f64,
    pub q_no: f64,
    pub cpp: f64,
    pub status: Option<String>,
    pub claim_status: Option<String>,
    pub meta_data: Option<String>,
    pub exit_reason: String,
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
  lp DOUBLE PRECISION NOT NULL,
  total_cost DOUBLE PRECISION NOT NULL,
  q_yes DOUBLE PRECISION NOT NULL,
  q_no DOUBLE PRECISION NOT NULL,
  cpp DOUBLE PRECISION NOT NULL DEFAULT 0.0,
  status TEXT NULL,
  claim_status TEXT NULL,
  meta_data TEXT NULL
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
            "SELECT COALESCE(SUM(lp), 0.0), COUNT(trade_id) FROM trade WHERE bot_id = $1 AND date >= $2 AND date <= $3",
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
            "SELECT COALESCE(SUM(lp), 0.0), COUNT(trade_id) FROM trade WHERE date >= $1 AND date <= $2",
            &[&start_date, &end_date],
        )?;

        let pnl: f64 = row.get(0);
        let cnt: i64 = row.get(1);
        Ok((pnl, cnt))
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

        conn.execute(
            "INSERT INTO trade (
                trade_id, exit_reason, bot_id, slug, configuration_id,
                date, start_trade, end_trade,
                lp, total_cost, q_yes, q_no, cpp, status, claim_status, meta_data
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8,
                $9, $10, $11, $12, $13, $14, $15, $16
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
    ) -> Result<()> {
        let status = if lp > 0.0 {
            "WON"
        } else if lp < 0.0 {
            "LOSS"
        } else {
            "DRAW"
        };
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "UPDATE trade SET
                end_trade = $1,
                lp = $2,
                total_cost = $3,
                cpp = $4,
                q_yes = $5,
                q_no = $6,
                exit_reason = $7,
                status = $8
             WHERE trade_id = $9",
            &[
                &end_trade_iso,
                &lp,
                &total_cost,
                &cpp,
                &q_yes,
                &q_no,
                &exit_reason,
                &status,
                &trade_id,
            ],
        )?;
        Ok(())
    }
}
