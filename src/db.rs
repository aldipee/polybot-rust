use crate::config::{BotConfig, ResolvedVersionedConfigBundle};
use crate::helpers::canonical_pair_id_from_slug;
use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration, Utc};
use chrono_tz::Asia::Jakarta;
use native_tls::TlsConnector;
use polybot::analysis_import::AnalysisImportResult;
use postgres::Client;
use postgres_native_tls::MakeTlsConnector;
use serde::{Deserialize, Serialize};
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
    pub config_version: String,
    pub config_text: String,
    pub loaded_at: String,
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
    pub pair_id: String,
    pub condition_id: Option<String>,
    pub yes_asset_id: Option<String>,
    pub no_asset_id: Option<String>,
    pub configuration_id: String,
    pub config_version: String,
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
pub struct TradePairMetadata {
    pub pair_id: String,
    pub market_slug: String,
    pub condition_id: Option<String>,
    pub yes_asset_id: Option<String>,
    pub no_asset_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TradeDecisionUpsert {
    pub config_version: Option<String>,
    pub pair_id: Option<String>,
    pub market_slug: Option<String>,
    pub condition_id: Option<String>,
    pub yes_asset_id: Option<String>,
    pub no_asset_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDecisionEventInsert {
    pub decision_event_id: String,
    pub trade_id: String,
    pub pair_id: String,
    pub market_slug: String,
    pub condition_id: Option<String>,
    pub yes_asset_id: Option<String>,
    pub no_asset_id: Option<String>,
    pub config_version: String,
    pub decision_scope: String,
    pub decision_ts: String,
    pub phase: Option<String>,
    pub owner: Option<String>,
    pub approved: bool,
    pub reason_code: String,
    pub submit_origin: Option<String>,
    pub submit_side: Option<String>,
    pub payload_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRuntimeEventInsert {
    pub event_id: String,
    pub trade_id: String,
    pub pair_id: String,
    pub market_slug: String,
    pub condition_id: Option<String>,
    pub yes_asset_id: Option<String>,
    pub no_asset_id: Option<String>,
    pub config_version: String,
    pub event_kind: String,
    pub event_ts: String,
    pub decision_event_id: Option<String>,
    pub order_id: Option<String>,
    pub asset_id: Option<String>,
    pub side: Option<String>,
    pub reason_code: Option<String>,
    pub payload_json: String,
}

pub fn now_iso_jakarta() -> String {
    crate::replay::runtime_now_iso_jakarta()
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
    crate::replay::replay_runtime_new_uuid().unwrap_or_else(|| Uuid::new_v4().to_string())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn legacy_config_version_from_hash(config_hash: &str) -> String {
    let suffix = config_hash.chars().take(12).collect::<String>();
    format!("legacy_{suffix}")
}

fn normalized_trade_pair_metadata(pair: &TradePairMetadata) -> TradePairMetadata {
    let market_slug = pair.market_slug.trim().to_string();
    let pair_id = canonical_pair_id_from_slug(if pair.pair_id.trim().is_empty() {
        market_slug.as_str()
    } else {
        pair.pair_id.as_str()
    });
    TradePairMetadata {
        pair_id,
        market_slug,
        condition_id: normalize_optional_text(pair.condition_id.as_deref()),
        yes_asset_id: normalize_optional_text(pair.yes_asset_id.as_deref()),
        no_asset_id: normalize_optional_text(pair.no_asset_id.as_deref()),
    }
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
  config_version TEXT NOT NULL,
  config_text TEXT NOT NULL,
  loaded_at TEXT NOT NULL,
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
  pair_id TEXT NOT NULL,
  condition_id TEXT NULL,
  yes_asset_id TEXT NULL,
  no_asset_id TEXT NULL,
  configuration_id TEXT NOT NULL,
  config_version TEXT NOT NULL DEFAULT '',
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
  config_version TEXT NULL,
  pair_id TEXT NULL,
  market_slug TEXT NULL,
  condition_id TEXT NULL,
  yes_asset_id TEXT NULL,
  no_asset_id TEXT NULL,
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

CREATE TABLE IF NOT EXISTS trade_decision_events (
  decision_event_id TEXT PRIMARY KEY,
  trade_id TEXT NOT NULL,
  pair_id TEXT NOT NULL,
  market_slug TEXT NOT NULL,
  condition_id TEXT NULL,
  yes_asset_id TEXT NULL,
  no_asset_id TEXT NULL,
  config_version TEXT NOT NULL,
  decision_scope TEXT NOT NULL,
  decision_ts TEXT NOT NULL,
  phase TEXT NULL,
  owner TEXT NULL,
  approved BOOLEAN NOT NULL,
  reason_code TEXT NOT NULL,
  submit_origin TEXT NULL,
  submit_side TEXT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS trade_runtime_events (
  event_id TEXT PRIMARY KEY,
  trade_id TEXT NOT NULL,
  pair_id TEXT NOT NULL,
  market_slug TEXT NOT NULL,
  condition_id TEXT NULL,
  yes_asset_id TEXT NULL,
  no_asset_id TEXT NULL,
  config_version TEXT NOT NULL,
  event_kind TEXT NOT NULL,
  event_ts TEXT NOT NULL,
  decision_event_id TEXT NULL,
  order_id TEXT NULL,
  asset_id TEXT NULL,
  side TEXT NULL,
  reason_code TEXT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS analysis_import_run (
  import_run_id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  dataset_dir TEXT NOT NULL,
  trade_parquet_path TEXT NOT NULL,
  close_csv_path TEXT NOT NULL,
  schema_doc_path TEXT NOT NULL,
  trade_parquet_sha256 TEXT NOT NULL,
  close_csv_sha256 TEXT NOT NULL,
  schema_doc_sha256 TEXT NOT NULL,
  trade_parquet_mtime TEXT NULL,
  close_csv_mtime TEXT NULL,
  schema_doc_mtime TEXT NULL,
  started_at TEXT NOT NULL,
  completed_at TEXT NOT NULL,
  summary_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS analysis_trade_row (
  import_run_id TEXT NOT NULL,
  row_ordinal BIGINT NOT NULL,
  "trade_identity_key" TEXT NOT NULL,
  "proxyWallet" TEXT NOT NULL,
  "side" TEXT NOT NULL,
  "asset" TEXT NOT NULL,
  "conditionId" TEXT NOT NULL,
  "size" DOUBLE PRECISION NOT NULL,
  "price" DOUBLE PRECISION NOT NULL,
  "timestamp" BIGINT NOT NULL,
  "title" TEXT NOT NULL,
  "slug" TEXT NULL,
  "eventSlug" TEXT NOT NULL,
  "outcome" TEXT NOT NULL,
  "outcomeIndex" BIGINT NOT NULL,
  "transactionHash" TEXT NULL,
  "is_taker" BOOLEAN NOT NULL,
  "window_start" BIGINT NULL,
  "window_end" BIGINT NULL,
  "t_remain_s" DOUBLE PRECISION NULL,
  "t_into_s" DOUBLE PRECISION NULL,
  "trade_time_utc" TEXT NULL,
  "binance_btc_trade_px" DOUBLE PRECISION NULL,
  "binance_btc_start_px" DOUBLE PRECISION NULL,
  "binance_delta_from_start" DOUBLE PRECISION NULL,
  "binance_rsi14_at_trade" DOUBLE PRECISION NULL,
  "binance_vol30m_1m_at_trade" DOUBLE PRECISION NULL,
  "binance_up_model" DOUBLE PRECISION NULL,
  "binance_down_model" DOUBLE PRECISION NULL,
  "edge_model_minus_price" DOUBLE PRECISION NULL,
  "final_outcome" TEXT NULL,
  "snapshot_status" TEXT NOT NULL,
  "snapshot_requested_ts_ms" BIGINT NULL,
  "snapshot_market_id" BIGINT NULL,
  "snapshot_time" TEXT NULL,
  "snapshot_match_delta_ms" DOUBLE PRECISION NULL,
  "snapshot_id" DOUBLE PRECISION NULL,
  "snapsot_market_btc_price" DOUBLE PRECISION NULL,
  "snapshot_price_up" DOUBLE PRECISION NULL,
  "snapshot_price_down" DOUBLE PRECISION NULL,
  "snapshot_last_trade_price_up" DOUBLE PRECISION NULL,
  "snapshot_last_trade_price_down" DOUBLE PRECISION NULL,
  "snapshot_min_order_size_up" DOUBLE PRECISION NULL,
  "snapshot_min_order_size_down" DOUBLE PRECISION NULL,
  "snapshot_tick_size_up" DOUBLE PRECISION NULL,
  "snapshot_tick_size_down" DOUBLE PRECISION NULL,
  "snapshot_orderbook_up_bid_count" DOUBLE PRECISION NULL,
  "snapshot_orderbook_up_ask_count" DOUBLE PRECISION NULL,
  "snapshot_orderbook_up_spread" DOUBLE PRECISION NULL,
  "snapshot_orderbook_up_bid_1_price" DOUBLE PRECISION NULL,
  "snapshot_orderbook_up_bid_1_size" DOUBLE PRECISION NULL,
  "snapshot_orderbook_up_ask_1_price" DOUBLE PRECISION NULL,
  "snapshot_orderbook_up_ask_1_size" DOUBLE PRECISION NULL,
  "snapshot_orderbook_down_bid_count" DOUBLE PRECISION NULL,
  "snapshot_orderbook_down_ask_count" DOUBLE PRECISION NULL,
  "snapshot_orderbook_down_spread" DOUBLE PRECISION NULL,
  "snapshot_orderbook_down_bid_1_price" DOUBLE PRECISION NULL,
  "snapshot_orderbook_down_bid_1_size" DOUBLE PRECISION NULL,
  "snapshot_orderbook_down_ask_1_price" DOUBLE PRECISION NULL,
  "snapshot_orderbook_down_ask_1_size" DOUBLE PRECISION NULL,
  "snapsot_market_btc_price_to_beat" DOUBLE PRECISION NULL,
  "snapsot_btc_price_delta" DOUBLE PRECISION NULL,
  row_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (import_run_id, "trade_identity_key")
);

CREATE TABLE IF NOT EXISTS analysis_close_position_row (
  import_run_id TEXT NOT NULL,
  row_ordinal BIGINT NOT NULL,
  "proxyWallet" TEXT NOT NULL,
  "asset" TEXT NOT NULL,
  "conditionId" TEXT NOT NULL,
  "avgPrice" DOUBLE PRECISION NOT NULL,
  "totalBought" DOUBLE PRECISION NOT NULL,
  "realizedPnl" DOUBLE PRECISION NOT NULL,
  "curPrice" DOUBLE PRECISION NOT NULL,
  "title" TEXT NOT NULL,
  "slug" TEXT NOT NULL,
  "icon" TEXT NOT NULL,
  "eventSlug" TEXT NOT NULL,
  "outcome" TEXT NOT NULL,
  "outcomeIndex" BIGINT NOT NULL,
  "oppositeOutcome" TEXT NOT NULL,
  "oppositeAsset" TEXT NOT NULL,
  "endDate" TEXT NOT NULL,
  "timestamp" BIGINT NOT NULL,
  row_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (import_run_id, row_ordinal)
);

CREATE TABLE IF NOT EXISTS analysis_pair_rollup (
  import_run_id TEXT NOT NULL,
  condition_id TEXT NOT NULL,
  event_slug TEXT NOT NULL,
  trade_outcomes_csv TEXT NOT NULL,
  close_outcomes_csv TEXT NOT NULL,
  both_sided_close BOOLEAN NOT NULL,
  total_trade_count BIGINT NOT NULL,
  taker_trade_count BIGINT NOT NULL,
  total_notional DOUBLE PRECISION NOT NULL,
  taker_notional DOUBLE PRECISION NOT NULL,
  up_avg_price DOUBLE PRECISION NULL,
  down_avg_price DOUBLE PRECISION NULL,
  up_total_bought DOUBLE PRECISION NULL,
  down_total_bought DOUBLE PRECISION NULL,
  up_realized_pnl DOUBLE PRECISION NULL,
  down_realized_pnl DOUBLE PRECISION NULL,
  up_cur_price DOUBLE PRECISION NULL,
  down_cur_price DOUBLE PRECISION NULL,
  rollup_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (import_run_id, condition_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_trade_bot_pair_id_unique
  ON trade (bot_id, pair_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_configuration_config_version_unique
  ON configuration (config_version);
CREATE INDEX IF NOT EXISTS idx_trade_decision_events_trade_ts
  ON trade_decision_events (trade_id, decision_ts);
CREATE INDEX IF NOT EXISTS idx_trade_runtime_events_trade_ts
  ON trade_runtime_events (trade_id, event_ts);
CREATE INDEX IF NOT EXISTS idx_trade_runtime_events_decision
  ON trade_runtime_events (decision_event_id);
CREATE INDEX IF NOT EXISTS idx_analysis_trade_row_import_condition
  ON analysis_trade_row (import_run_id, "conditionId");
CREATE INDEX IF NOT EXISTS idx_analysis_close_position_import_condition
  ON analysis_close_position_row (import_run_id, "conditionId");
CREATE INDEX IF NOT EXISTS idx_analysis_pair_rollup_import
  ON analysis_pair_rollup (import_run_id);
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
ALTER TABLE configuration ADD COLUMN IF NOT EXISTS config_version TEXT NOT NULL DEFAULT '';
ALTER TABLE configuration ADD COLUMN IF NOT EXISTS config_text TEXT NOT NULL DEFAULT '';
ALTER TABLE configuration ADD COLUMN IF NOT EXISTS loaded_at TEXT NOT NULL DEFAULT '';
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
UPDATE configuration
SET loaded_at = COALESCE(NULLIF(trim(loaded_at), ''), created_at);
UPDATE configuration
SET config_version = COALESCE(NULLIF(trim(config_version), ''), 'legacy_' || LEFT(config_hash, 12));
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_status TEXT NOT NULL DEFAULT 'PENDING';
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_checked_at TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_validated_at TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS validation_source TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS pair_id TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS condition_id TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS yes_asset_id TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS no_asset_id TEXT NULL;
ALTER TABLE trade ADD COLUMN IF NOT EXISTS config_version TEXT NOT NULL DEFAULT '';
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
SET pair_id = lower(trim(slug))
WHERE pair_id IS NULL OR trim(pair_id) = '';
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
ALTER TABLE trade ALTER COLUMN pair_id SET NOT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS t_left_seconds DOUBLE PRECISION NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS config_version TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS pair_id TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS market_slug TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS condition_id TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS yes_asset_id TEXT NULL;
ALTER TABLE trade_decisions ADD COLUMN IF NOT EXISTS no_asset_id TEXT NULL;
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
CREATE UNIQUE INDEX IF NOT EXISTS idx_trade_bot_pair_id_unique
  ON trade (bot_id, pair_id);
"#,
        )
        .context("failed migrating configuration schema for PostgreSQL compatibility")?;
        Ok(())
    }

    pub fn persist_analysis_import(&self, result: &AnalysisImportResult) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        let now = now_iso_jakarta();
        let mut tx = conn
            .transaction()
            .context("failed starting analysis import transaction")?;

        let summary_json = serde_json::to_string(&result.summary)
            .context("failed serializing analysis summary json")?;
        tx.execute(
            "INSERT INTO analysis_import_run (
                import_run_id, status, dataset_dir, trade_parquet_path, close_csv_path, schema_doc_path,
                trade_parquet_sha256, close_csv_sha256, schema_doc_sha256,
                trade_parquet_mtime, close_csv_mtime, schema_doc_mtime,
                started_at, completed_at, summary_json, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9,
                $10, $11, $12,
                $13, $14, $15, $16
            )",
            &[
                &result.source.import_run_id,
                &"COMPLETED",
                &result.source.dataset_dir.to_string_lossy().to_string(),
                &result.source.trade_parquet_path.to_string_lossy().to_string(),
                &result.source.close_csv_path.to_string_lossy().to_string(),
                &result.source.schema_doc_path.to_string_lossy().to_string(),
                &result.source.trade_parquet_sha256,
                &result.source.close_csv_sha256,
                &result.source.schema_doc_sha256,
                &result.source.trade_parquet_mtime,
                &result.source.close_csv_mtime,
                &result.source.schema_doc_mtime,
                &result.source.started_at,
                &result.source.completed_at,
                &summary_json,
                &now,
            ],
        )
        .context("failed inserting analysis_import_run")?;

        let trade_stmt = tx.prepare(
            "INSERT INTO analysis_trade_row (
                import_run_id, row_ordinal, \"trade_identity_key\", \"proxyWallet\", \"side\", \"asset\", \"conditionId\", \"size\",
                \"price\", \"timestamp\", \"title\", \"slug\", \"eventSlug\", \"outcome\", \"outcomeIndex\", \"transactionHash\",
                \"is_taker\", \"window_start\", \"window_end\", \"t_remain_s\", \"t_into_s\", \"trade_time_utc\",
                \"binance_btc_trade_px\", \"binance_btc_start_px\", \"binance_delta_from_start\", \"binance_rsi14_at_trade\",
                \"binance_vol30m_1m_at_trade\", \"binance_up_model\", \"binance_down_model\", \"edge_model_minus_price\",
                \"final_outcome\", \"snapshot_status\", \"snapshot_requested_ts_ms\", \"snapshot_market_id\", \"snapshot_time\",
                \"snapshot_match_delta_ms\", \"snapshot_id\", \"snapsot_market_btc_price\", \"snapshot_price_up\", \"snapshot_price_down\",
                \"snapshot_last_trade_price_up\", \"snapshot_last_trade_price_down\", \"snapshot_min_order_size_up\",
                \"snapshot_min_order_size_down\", \"snapshot_tick_size_up\", \"snapshot_tick_size_down\",
                \"snapshot_orderbook_up_bid_count\", \"snapshot_orderbook_up_ask_count\", \"snapshot_orderbook_up_spread\",
                \"snapshot_orderbook_up_bid_1_price\", \"snapshot_orderbook_up_bid_1_size\", \"snapshot_orderbook_up_ask_1_price\",
                \"snapshot_orderbook_up_ask_1_size\", \"snapshot_orderbook_down_bid_count\", \"snapshot_orderbook_down_ask_count\",
                \"snapshot_orderbook_down_spread\", \"snapshot_orderbook_down_bid_1_price\", \"snapshot_orderbook_down_bid_1_size\",
                \"snapshot_orderbook_down_ask_1_price\", \"snapshot_orderbook_down_ask_1_size\", \"snapsot_market_btc_price_to_beat\",
                \"snapsot_btc_price_delta\", row_json, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8,
                $9, $10, $11, $12, $13, $14, $15, $16,
                $17, $18, $19, $20, $21, $22,
                $23, $24, $25, $26,
                $27, $28, $29, $30,
                $31, $32, $33, $34, $35,
                $36, $37, $38, $39, $40,
                $41, $42, $43,
                $44, $45, $46,
                $47, $48, $49,
                $50, $51, $52,
                $53, $54, $55,
                $56, $57, $58,
                $59, $60, $61,
                $62, $63, $64
            )",
        )
        .context("failed preparing analysis_trade_row insert")?;

        for (idx, row) in result.trade_rows.iter().enumerate() {
            let row_json =
                serde_json::to_string(row).context("failed serializing analysis trade row")?;
            tx.execute(
                &trade_stmt,
                &[
                    &result.source.import_run_id,
                    &(idx as i64),
                    &row.trade_identity_key,
                    &row.proxyWallet,
                    &row.side,
                    &row.asset,
                    &row.conditionId,
                    &row.size,
                    &row.price,
                    &row.timestamp,
                    &row.title,
                    &row.slug,
                    &row.eventSlug,
                    &row.outcome,
                    &row.outcomeIndex,
                    &row.transactionHash,
                    &row.is_taker,
                    &row.window_start,
                    &row.window_end,
                    &row.t_remain_s,
                    &row.t_into_s,
                    &row.trade_time_utc,
                    &row.binance_btc_trade_px,
                    &row.binance_btc_start_px,
                    &row.binance_delta_from_start,
                    &row.binance_rsi14_at_trade,
                    &row.binance_vol30m_1m_at_trade,
                    &row.binance_up_model,
                    &row.binance_down_model,
                    &row.edge_model_minus_price,
                    &row.final_outcome,
                    &row.snapshot_status,
                    &row.snapshot_requested_ts_ms,
                    &row.snapshot_market_id,
                    &row.snapshot_time,
                    &row.snapshot_match_delta_ms,
                    &row.snapshot_id,
                    &row.snapsot_market_btc_price,
                    &row.snapshot_price_up,
                    &row.snapshot_price_down,
                    &row.snapshot_last_trade_price_up,
                    &row.snapshot_last_trade_price_down,
                    &row.snapshot_min_order_size_up,
                    &row.snapshot_min_order_size_down,
                    &row.snapshot_tick_size_up,
                    &row.snapshot_tick_size_down,
                    &row.snapshot_orderbook_up_bid_count,
                    &row.snapshot_orderbook_up_ask_count,
                    &row.snapshot_orderbook_up_spread,
                    &row.snapshot_orderbook_up_bid_1_price,
                    &row.snapshot_orderbook_up_bid_1_size,
                    &row.snapshot_orderbook_up_ask_1_price,
                    &row.snapshot_orderbook_up_ask_1_size,
                    &row.snapshot_orderbook_down_bid_count,
                    &row.snapshot_orderbook_down_ask_count,
                    &row.snapshot_orderbook_down_spread,
                    &row.snapshot_orderbook_down_bid_1_price,
                    &row.snapshot_orderbook_down_bid_1_size,
                    &row.snapshot_orderbook_down_ask_1_price,
                    &row.snapshot_orderbook_down_ask_1_size,
                    &row.snapsot_market_btc_price_to_beat,
                    &row.snapsot_btc_price_delta,
                    &row_json,
                    &now,
                ],
            )
            .context("failed inserting analysis_trade_row")?;
        }

        let close_stmt = tx.prepare(
            "INSERT INTO analysis_close_position_row (
                import_run_id, row_ordinal, \"proxyWallet\", \"asset\", \"conditionId\", \"avgPrice\", \"totalBought\",
                \"realizedPnl\", \"curPrice\", \"title\", \"slug\", \"icon\", \"eventSlug\", \"outcome\", \"outcomeIndex\",
                \"oppositeOutcome\", \"oppositeAsset\", \"endDate\", \"timestamp\", row_json, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12, $13, $14, $15,
                $16, $17, $18, $19, $20, $21
            )",
        )
        .context("failed preparing analysis_close_position_row insert")?;

        for (idx, row) in result.close_rows.iter().enumerate() {
            let row_json = serde_json::to_string(row)
                .context("failed serializing analysis close-position row")?;
            tx.execute(
                &close_stmt,
                &[
                    &result.source.import_run_id,
                    &(idx as i64),
                    &row.proxyWallet,
                    &row.asset,
                    &row.conditionId,
                    &row.avgPrice,
                    &row.totalBought,
                    &row.realizedPnl,
                    &row.curPrice,
                    &row.title,
                    &row.slug,
                    &row.icon,
                    &row.eventSlug,
                    &row.outcome,
                    &row.outcomeIndex,
                    &row.oppositeOutcome,
                    &row.oppositeAsset,
                    &row.endDate,
                    &row.timestamp,
                    &row_json,
                    &now,
                ],
            )
            .context("failed inserting analysis_close_position_row")?;
        }

        let rollup_stmt = tx.prepare(
            "INSERT INTO analysis_pair_rollup (
                import_run_id, condition_id, event_slug, trade_outcomes_csv, close_outcomes_csv, both_sided_close,
                total_trade_count, taker_trade_count, total_notional, taker_notional, up_avg_price, down_avg_price,
                up_total_bought, down_total_bought, up_realized_pnl, down_realized_pnl, up_cur_price, down_cur_price,
                rollup_json, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6,
                $7, $8, $9, $10, $11, $12,
                $13, $14, $15, $16, $17, $18,
                $19, $20
            )",
        )
        .context("failed preparing analysis_pair_rollup insert")?;

        for row in &result.pair_rollups {
            let rollup_json =
                serde_json::to_string(row).context("failed serializing analysis pair rollup")?;
            tx.execute(
                &rollup_stmt,
                &[
                    &result.source.import_run_id,
                    &row.condition_id,
                    &row.event_slug,
                    &row.trade_outcomes_csv,
                    &row.close_outcomes_csv,
                    &row.both_sided_close,
                    &row.total_trade_count,
                    &row.taker_trade_count,
                    &row.total_notional,
                    &row.taker_notional,
                    &row.up_avg_price,
                    &row.down_avg_price,
                    &row.up_total_bought,
                    &row.down_total_bought,
                    &row.up_realized_pnl,
                    &row.down_realized_pnl,
                    &row.up_cur_price,
                    &row.down_cur_price,
                    &rollup_json,
                    &now,
                ],
            )
            .context("failed inserting analysis_pair_rollup")?;
        }

        tx.commit()
            .context("failed committing analysis import transaction")?;
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

    pub fn upsert_configuration(&self, bundle: &ResolvedVersionedConfigBundle) -> Result<String> {
        let h = bundle.config_hash().to_string();
        let cfg = &bundle.effective_bot_config;
        let config_text = bundle.config_text()?;
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
                configuration_id, config_hash, config_version, config_text, loaded_at,
                clob_host, ws_base, chain_id, private_key, signature_type, funder,
                tick, min_shares, lock_profit_target,
                clip_shares, improve_bid_ticks, maker_buffer_ticks, replace_if_price_moves_ticks, stale_seconds,
                entry_edge_ticks, hedge_buffer_ticks, max_total_cost, reserve_usd,
                cancel_all_on_start, dry_run, log_every,
                market_data_stale_seconds, ws_reconnect_min, ws_reconnect_max,
                stop_buffer_seconds, created_at
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8, $9, $10, $11,
                $12, $13, $14,
                $15, $16, $17, $18, $19,
                $20, $21, $22, $23,
                $24, $25, $26,
                $27, $28, $29,
                $30, $31
            )",
            &[
                &cid,
                &h,
                &bundle.snapshot.config_version,
                &config_text,
                &bundle.snapshot.loaded_at,
                &cfg.clob_host,
                &cfg.ws_base,
                &cfg.chain_id,
                &bundle.snapshot.bot_config.private_key,
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
                configuration_id, config_hash, config_version, config_text, loaded_at,
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

        Ok(row.map(|r| {
            let config_hash: String = r.get(1);
            let raw_config_version: String = r.get(2);
            let created_at: String = r.get(30);
            let loaded_at: String = r.get(4);
            ConfigurationRow {
                configuration_id: r.get(0),
                config_hash: config_hash.clone(),
                config_version: if raw_config_version.trim().is_empty() {
                    legacy_config_version_from_hash(config_hash.as_str())
                } else {
                    raw_config_version
                },
                config_text: r.get(3),
                loaded_at: if loaded_at.trim().is_empty() {
                    created_at.clone()
                } else {
                    loaded_at
                },
                clob_host: r.get(5),
                ws_base: r.get(6),
                chain_id: r.get(7),
                private_key: r.get(8),
                signature_type: r.get(9),
                funder: r.get(10),
                tick: r.get(11),
                min_shares: r.get(12),
                lock_profit_target: r.get(13),
                clip_shares: r.get(14),
                improve_bid_ticks: r.get(15),
                maker_buffer_ticks: r.get(16),
                replace_if_price_moves_ticks: r.get(17),
                stale_seconds: r.get(18),
                entry_edge_ticks: r.get(19),
                hedge_buffer_ticks: r.get(20),
                max_total_cost: r.get(21),
                reserve_usd: r.get(22),
                cancel_all_on_start: r.get(23),
                dry_run: r.get(24),
                log_every: r.get(25),
                market_data_stale_seconds: r.get(26),
                ws_reconnect_min: r.get(27),
                ws_reconnect_max: r.get(28),
                stop_buffer_seconds: r.get(29),
                created_at,
            }
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
        pair: &TradePairMetadata,
        configuration_id: &str,
        config_version: &str,
        start_trade_iso: &str,
    ) -> Result<(String, String)> {
        let pair = normalized_trade_pair_metadata(pair);
        if pair.pair_id.trim().is_empty() || pair.market_slug.trim().is_empty() {
            return Err(anyhow!("missing pair metadata for pending trade creation"));
        }
        let mut conn = open_conn(&self.engine)?;
        if let Some(row) = conn.query_opt(
            "SELECT trade_id, status FROM trade WHERE bot_id = $1 AND pair_id = $2 LIMIT 1",
            &[&bot_id, &pair.pair_id],
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
                trade_id, exit_reason, bot_id, slug, pair_id, condition_id, yes_asset_id, no_asset_id,
                configuration_id, config_version,
                date, start_trade, end_trade,
                entry_time, holding_duration_seconds, entry_reason, exit_time, exit_reason_category,
                stop_loss_category, entry_price, exit_price,
                lp, total_cost, q_yes, q_no, cpp, status, claim_status, meta_data
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8,
                $9, $10,
                $11, $12, $13,
                $14, $15, $16, $17, $18,
                $19, $20, $21,
                $22, $23, $24, $25, $26, $27, $28, $29
            )",
            &[
                &tid,
                &running,
                &bot_id,
                &pair.market_slug,
                &pair.pair_id,
                &pair.condition_id,
                &pair.yes_asset_id,
                &pair.no_asset_id,
                &configuration_id,
                &config_version,
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
               AND (
                    (
                        status IN ('WON','LOSS','DRAW')
                        AND NOT (
                            status = 'DRAW'
                            AND COALESCE(total_cost, 0.0) <= 1e-9
                            AND COALESCE(q_yes, 0.0) <= 1e-9
                            AND COALESCE(q_no, 0.0) <= 1e-9
                        )
                    )
                    OR COALESCE(claim_status, '') IN ('AWAIT_SETTLEMENT', 'SETTLED')
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

    pub fn update_trade_settlement_fields(
        &self,
        trade_id: &str,
        claim_status: Option<&str>,
        meta_data: Option<&str>,
    ) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "UPDATE trade
             SET claim_status = $1,
                 meta_data = $2
             WHERE trade_id = $3",
            &[&claim_status, &meta_data, &trade_id],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_trade_await_settlement_snapshot(
        &self,
        trade_id: &str,
        end_trade_iso: &str,
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
    ) -> Result<()> {
        let await_settlement = "AWAIT_SETTLEMENT".to_string();
        let validation_pending = "PENDING".to_string();
        let validation_checked_at: Option<String> = None;
        let validation_validated_at: Option<String> = None;
        let validation_source: Option<String> = None;
        let exit_time_iso: Option<&str> = Some(end_trade_iso);
        let exit_price: Option<f64> = None;
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
                total_cost = $10,
                cpp = $11,
                q_yes = $12,
                q_no = $13,
                exit_reason = $14,
                status = $15,
                claim_status = $16,
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
                &total_cost,
                &cpp,
                &q_yes,
                &q_no,
                &exit_reason,
                &await_settlement,
                &await_settlement,
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
                 validation_source = $6,
                 claim_status = CASE
                    WHEN COALESCE(claim_status, '') IN ('', 'AWAIT_SETTLEMENT')
                        THEN 'SETTLED'
                    ELSE claim_status
                 END,
                 end_trade = CASE
                    WHEN COALESCE(trim(end_trade), '') = ''
                        THEN $4
                    ELSE end_trade
                 END,
                 exit_time = COALESCE(exit_time, $4),
                 exit_price = COALESCE(
                    exit_price,
                    CASE
                        WHEN COALESCE(q_yes, 0.0) + COALESCE(q_no, 0.0) > 1e-9
                            THEN (COALESCE(total_cost, 0.0) + $1)
                                / (COALESCE(q_yes, 0.0) + COALESCE(q_no, 0.0))
                        ELSE NULL
                    END
                 )
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

    pub fn upsert_trade_decision(&self, trade_id: &str, row: &TradeDecisionUpsert) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        let now = now_iso_jakarta();
        let pair_id = row
            .pair_id
            .as_deref()
            .map(canonical_pair_id_from_slug)
            .or_else(|| row.market_slug.as_deref().map(canonical_pair_id_from_slug));
        let market_slug = normalize_optional_text(row.market_slug.as_deref());
        let condition_id = normalize_optional_text(row.condition_id.as_deref());
        let yes_asset_id = normalize_optional_text(row.yes_asset_id.as_deref());
        let no_asset_id = normalize_optional_text(row.no_asset_id.as_deref());
        conn.execute(
            "INSERT INTO trade_decisions (
                trade_id,
                config_version,
                pair_id, market_slug, condition_id, yes_asset_id, no_asset_id,
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
                $2,
                $3, $4, $5, $6, $7,
                $8, $9,
                $10, $11, $12, $13, $14,
                $15, $16, $17, $18,
                $19, $20, $21, $22, $23, $24, $25,
                $26, $27, $28,
                $29, $30, $31,
                $32, $33, $34, $35, $36,
                $37, $38,
                $39, $40, $41, $42, $43,
                $44, $45,
                $46, $47, $48,
                $49, $50, $51, $52, $53,
                $54, $55, $56,
                $57, $58
            )
            ON CONFLICT (trade_id) DO UPDATE SET
                config_version = EXCLUDED.config_version,
                pair_id = EXCLUDED.pair_id,
                market_slug = EXCLUDED.market_slug,
                condition_id = EXCLUDED.condition_id,
                yes_asset_id = EXCLUDED.yes_asset_id,
                no_asset_id = EXCLUDED.no_asset_id,
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
                &row.config_version,
                &pair_id,
                &market_slug,
                &condition_id,
                &yes_asset_id,
                &no_asset_id,
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

    pub fn insert_trade_decision_event(&self, row: &TradeDecisionEventInsert) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "INSERT INTO trade_decision_events (
                decision_event_id,
                trade_id,
                pair_id,
                market_slug,
                condition_id,
                yes_asset_id,
                no_asset_id,
                config_version,
                decision_scope,
                decision_ts,
                phase,
                owner,
                approved,
                reason_code,
                submit_origin,
                submit_side,
                payload_json,
                created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18
            )",
            &[
                &row.decision_event_id,
                &row.trade_id,
                &row.pair_id,
                &row.market_slug,
                &row.condition_id,
                &row.yes_asset_id,
                &row.no_asset_id,
                &row.config_version,
                &row.decision_scope,
                &row.decision_ts,
                &row.phase,
                &row.owner,
                &row.approved,
                &row.reason_code,
                &row.submit_origin,
                &row.submit_side,
                &row.payload_json,
                &now_iso_jakarta(),
            ],
        )?;
        Ok(())
    }

    pub fn insert_trade_runtime_event(&self, row: &TradeRuntimeEventInsert) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "INSERT INTO trade_runtime_events (
                event_id,
                trade_id,
                pair_id,
                market_slug,
                condition_id,
                yes_asset_id,
                no_asset_id,
                config_version,
                event_kind,
                event_ts,
                decision_event_id,
                order_id,
                asset_id,
                side,
                reason_code,
                payload_json,
                created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17
            )",
            &[
                &row.event_id,
                &row.trade_id,
                &row.pair_id,
                &row.market_slug,
                &row.condition_id,
                &row.yes_asset_id,
                &row.no_asset_id,
                &row.config_version,
                &row.event_kind,
                &row.event_ts,
                &row.decision_event_id,
                &row.order_id,
                &row.asset_id,
                &row.side,
                &row.reason_code,
                &row.payload_json,
                &now_iso_jakarta(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_trade(&self, trade_id: &str) -> Result<()> {
        let mut conn = open_conn(&self.engine)?;
        conn.execute(
            "DELETE FROM trade_decisions WHERE trade_id = $1",
            &[&trade_id],
        )?;
        conn.execute("DELETE FROM trade WHERE trade_id = $1", &[&trade_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_trade_pair_metadata_canonicalizes_slug_variants_to_one_pair_id() {
        let upper = normalized_trade_pair_metadata(&TradePairMetadata {
            market_slug: "  BTC-Up-15M  ".to_string(),
            ..TradePairMetadata::default()
        });
        let lower = normalized_trade_pair_metadata(&TradePairMetadata {
            market_slug: "btc-up-15m".to_string(),
            ..TradePairMetadata::default()
        });
        assert_eq!(upper.pair_id, "btc-up-15m");
        assert_eq!(upper.pair_id, lower.pair_id);
        assert_eq!(upper.market_slug, "BTC-Up-15M");
    }

    #[test]
    fn normalized_trade_pair_metadata_preserves_pair_metadata_fields() {
        let pair = normalized_trade_pair_metadata(&TradePairMetadata {
            pair_id: "  custom-pair  ".to_string(),
            market_slug: " Custom-Pair ".to_string(),
            condition_id: Some(" cond ".to_string()),
            yes_asset_id: Some(" yes ".to_string()),
            no_asset_id: Some(" no ".to_string()),
        });
        assert_eq!(pair.pair_id, "custom-pair");
        assert_eq!(pair.market_slug, "Custom-Pair");
        assert_eq!(pair.condition_id.as_deref(), Some("cond"));
        assert_eq!(pair.yes_asset_id.as_deref(), Some("yes"));
        assert_eq!(pair.no_asset_id.as_deref(), Some("no"));
    }
}
