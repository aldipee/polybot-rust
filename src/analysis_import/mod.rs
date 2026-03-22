use crate::db::BotRepository;
use anyhow::{anyhow, Context, Result};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::DataType;
use chrono::{DateTime, Utc};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const TRADE_PARQUET_FILENAME: &str = "vidarx_trade_profitable.parquet";
pub const CLOSE_CSV_FILENAME: &str = "vidarx_close_position_profitable.csv";
pub const SCHEMA_DOC_FILENAME: &str = "dataset_schema.md";

const TRADE_PARQUET_COLUMNS: [(&str, ColumnKind); 60] = [
    ("trade_identity_key", ColumnKind::String),
    ("proxyWallet", ColumnKind::String),
    ("side", ColumnKind::String),
    ("asset", ColumnKind::String),
    ("conditionId", ColumnKind::String),
    ("size", ColumnKind::Float64),
    ("price", ColumnKind::Float64),
    ("timestamp", ColumnKind::Int64),
    ("title", ColumnKind::String),
    ("slug", ColumnKind::String),
    ("eventSlug", ColumnKind::String),
    ("outcome", ColumnKind::String),
    ("outcomeIndex", ColumnKind::Int64),
    ("transactionHash", ColumnKind::String),
    ("is_taker", ColumnKind::Boolean),
    ("window_start", ColumnKind::Int64),
    ("window_end", ColumnKind::Int64),
    ("t_remain_s", ColumnKind::Float64),
    ("t_into_s", ColumnKind::Float64),
    ("trade_time_utc", ColumnKind::String),
    ("binance_btc_trade_px", ColumnKind::Float64),
    ("binance_btc_start_px", ColumnKind::Float64),
    ("binance_delta_from_start", ColumnKind::Float64),
    ("binance_rsi14_at_trade", ColumnKind::Float64),
    ("binance_vol30m_1m_at_trade", ColumnKind::Float64),
    ("binance_up_model", ColumnKind::Float64),
    ("binance_down_model", ColumnKind::Float64),
    ("edge_model_minus_price", ColumnKind::Float64),
    ("final_outcome", ColumnKind::String),
    ("snapshot_status", ColumnKind::String),
    ("snapshot_requested_ts_ms", ColumnKind::Int64),
    ("snapshot_market_id", ColumnKind::Int64),
    ("snapshot_time", ColumnKind::String),
    ("snapshot_match_delta_ms", ColumnKind::Float64),
    ("snapshot_id", ColumnKind::Float64),
    ("snapsot_market_btc_price", ColumnKind::Float64),
    ("snapshot_price_up", ColumnKind::Float64),
    ("snapshot_price_down", ColumnKind::Float64),
    ("snapshot_last_trade_price_up", ColumnKind::Float64),
    ("snapshot_last_trade_price_down", ColumnKind::Float64),
    ("snapshot_min_order_size_up", ColumnKind::Float64),
    ("snapshot_min_order_size_down", ColumnKind::Float64),
    ("snapshot_tick_size_up", ColumnKind::Float64),
    ("snapshot_tick_size_down", ColumnKind::Float64),
    ("snapshot_orderbook_up_bid_count", ColumnKind::Float64),
    ("snapshot_orderbook_up_ask_count", ColumnKind::Float64),
    ("snapshot_orderbook_up_spread", ColumnKind::Float64),
    ("snapshot_orderbook_up_bid_1_price", ColumnKind::Float64),
    ("snapshot_orderbook_up_bid_1_size", ColumnKind::Float64),
    ("snapshot_orderbook_up_ask_1_price", ColumnKind::Float64),
    ("snapshot_orderbook_up_ask_1_size", ColumnKind::Float64),
    ("snapshot_orderbook_down_bid_count", ColumnKind::Float64),
    ("snapshot_orderbook_down_ask_count", ColumnKind::Float64),
    ("snapshot_orderbook_down_spread", ColumnKind::Float64),
    ("snapshot_orderbook_down_bid_1_price", ColumnKind::Float64),
    ("snapshot_orderbook_down_bid_1_size", ColumnKind::Float64),
    ("snapshot_orderbook_down_ask_1_price", ColumnKind::Float64),
    ("snapshot_orderbook_down_ask_1_size", ColumnKind::Float64),
    ("snapsot_market_btc_price_to_beat", ColumnKind::Float64),
    ("snapsot_btc_price_delta", ColumnKind::Float64),
];

const CLOSE_CSV_HEADERS: [&str; 17] = [
    "proxyWallet",
    "asset",
    "conditionId",
    "avgPrice",
    "totalBought",
    "realizedPnl",
    "curPrice",
    "title",
    "slug",
    "icon",
    "eventSlug",
    "outcome",
    "outcomeIndex",
    "oppositeOutcome",
    "oppositeAsset",
    "endDate",
    "timestamp",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnKind {
    String,
    Float64,
    Int64,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceZone {
    Preferred,
    Acceptable,
    Caution,
    StopAdd,
    Danger,
}

impl PriceZone {
    const THRESHOLD_EPSILON: f64 = 1e-9;

    fn as_str(self) -> &'static str {
        match self {
            Self::Preferred => "preferred",
            Self::Acceptable => "acceptable",
            Self::Caution => "caution",
            Self::StopAdd => "stop_add",
            Self::Danger => "danger",
        }
    }

    fn classify(value: f64) -> Self {
        if !value.is_finite() || value >= 1.03 - Self::THRESHOLD_EPSILON {
            Self::Danger
        } else if value >= 1.0 - Self::THRESHOLD_EPSILON {
            Self::StopAdd
        } else if value >= 0.97 - Self::THRESHOLD_EPSILON {
            Self::Caution
        } else if value >= 0.94 - Self::THRESHOLD_EPSILON {
            Self::Acceptable
        } else {
            Self::Preferred
        }
    }
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisTradeRow {
    pub trade_identity_key: String,
    pub proxyWallet: String,
    pub side: String,
    pub asset: String,
    pub conditionId: String,
    pub size: f64,
    pub price: f64,
    pub timestamp: i64,
    pub title: String,
    pub slug: Option<String>,
    pub eventSlug: String,
    pub outcome: String,
    pub outcomeIndex: i64,
    pub transactionHash: Option<String>,
    pub is_taker: bool,
    pub window_start: Option<i64>,
    pub window_end: Option<i64>,
    pub t_remain_s: Option<f64>,
    pub t_into_s: Option<f64>,
    pub trade_time_utc: Option<String>,
    pub binance_btc_trade_px: Option<f64>,
    pub binance_btc_start_px: Option<f64>,
    pub binance_delta_from_start: Option<f64>,
    pub binance_rsi14_at_trade: Option<f64>,
    pub binance_vol30m_1m_at_trade: Option<f64>,
    pub binance_up_model: Option<f64>,
    pub binance_down_model: Option<f64>,
    pub edge_model_minus_price: Option<f64>,
    pub final_outcome: Option<String>,
    pub snapshot_status: String,
    pub snapshot_requested_ts_ms: Option<i64>,
    pub snapshot_market_id: Option<i64>,
    pub snapshot_time: Option<String>,
    pub snapshot_match_delta_ms: Option<f64>,
    pub snapshot_id: Option<f64>,
    pub snapsot_market_btc_price: Option<f64>,
    pub snapshot_price_up: Option<f64>,
    pub snapshot_price_down: Option<f64>,
    pub snapshot_last_trade_price_up: Option<f64>,
    pub snapshot_last_trade_price_down: Option<f64>,
    pub snapshot_min_order_size_up: Option<f64>,
    pub snapshot_min_order_size_down: Option<f64>,
    pub snapshot_tick_size_up: Option<f64>,
    pub snapshot_tick_size_down: Option<f64>,
    pub snapshot_orderbook_up_bid_count: Option<f64>,
    pub snapshot_orderbook_up_ask_count: Option<f64>,
    pub snapshot_orderbook_up_spread: Option<f64>,
    pub snapshot_orderbook_up_bid_1_price: Option<f64>,
    pub snapshot_orderbook_up_bid_1_size: Option<f64>,
    pub snapshot_orderbook_up_ask_1_price: Option<f64>,
    pub snapshot_orderbook_up_ask_1_size: Option<f64>,
    pub snapshot_orderbook_down_bid_count: Option<f64>,
    pub snapshot_orderbook_down_ask_count: Option<f64>,
    pub snapshot_orderbook_down_spread: Option<f64>,
    pub snapshot_orderbook_down_bid_1_price: Option<f64>,
    pub snapshot_orderbook_down_bid_1_size: Option<f64>,
    pub snapshot_orderbook_down_ask_1_price: Option<f64>,
    pub snapshot_orderbook_down_ask_1_size: Option<f64>,
    pub snapsot_market_btc_price_to_beat: Option<f64>,
    pub snapsot_btc_price_delta: Option<f64>,
}

impl AnalysisTradeRow {
    pub fn notional(&self) -> f64 {
        self.price * self.size
    }

    pub fn historical_effective_pair_cost(&self) -> Option<f64> {
        let opposite = match self.outcome.as_str() {
            "Up" => self
                .snapshot_last_trade_price_down
                .or(self.snapshot_price_down),
            "Down" => self.snapshot_last_trade_price_up.or(self.snapshot_price_up),
            _ => None,
        }?;
        Some(self.price + opposite)
    }

    pub fn resolved_outcome_present(&self) -> bool {
        self.final_outcome
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn winner_alignment(&self) -> Option<bool> {
        self.final_outcome
            .as_deref()
            .map(|value| value == self.outcome.as_str())
    }
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisClosePositionRow {
    pub proxyWallet: String,
    pub asset: String,
    pub conditionId: String,
    pub avgPrice: f64,
    pub totalBought: f64,
    pub realizedPnl: f64,
    pub curPrice: f64,
    pub title: String,
    pub slug: String,
    pub icon: String,
    pub eventSlug: String,
    pub outcome: String,
    pub outcomeIndex: i64,
    pub oppositeOutcome: String,
    pub oppositeAsset: String,
    pub endDate: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisPairRollup {
    pub condition_id: String,
    pub event_slug: String,
    pub trade_outcomes_csv: String,
    pub close_outcomes_csv: String,
    pub both_sided_close: bool,
    pub total_trade_count: i64,
    pub taker_trade_count: i64,
    pub total_notional: f64,
    pub taker_notional: f64,
    pub up_avg_price: Option<f64>,
    pub down_avg_price: Option<f64>,
    pub up_total_bought: Option<f64>,
    pub down_total_bought: Option<f64>,
    pub up_realized_pnl: Option<f64>,
    pub down_realized_pnl: Option<f64>,
    pub up_cur_price: Option<f64>,
    pub down_cur_price: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AnalysisImportPaths {
    pub dataset_dir: PathBuf,
    pub trade_parquet: PathBuf,
    pub close_csv: PathBuf,
    pub schema_doc: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisImportSummary {
    pub dataset: AnalysisImportDatasetSummary,
    pub counts: AnalysisImportCounts,
    pub metrics: AnalysisImportMetrics,
    pub coverage: AnalysisImportCoverage,
    pub price_zone_summary: AnalysisImportPriceZoneSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisImportDatasetSummary {
    pub trade_parquet: String,
    pub close_csv: String,
    pub schema_doc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisImportCounts {
    pub parquet_rows: usize,
    pub close_rows: usize,
    pub filtered_market_count: usize,
    pub closed_position_pair_count: usize,
    pub two_sided_pairs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisImportMetrics {
    pub two_sided_participation_rate: String,
    pub taker_share: String,
    pub taker_share_notional: String,
    pub weighted_pair_sum_median: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisImportCoverage {
    pub skipped_pair_cost_count: usize,
    pub skipped_outcome_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisImportPriceZoneSummary {
    pub preferred: PriceZoneSummaryRow,
    pub acceptable: PriceZoneSummaryRow,
    pub caution: PriceZoneSummaryRow,
    pub stop_add: PriceZoneSummaryRow,
    pub danger: PriceZoneSummaryRow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PriceZoneSummaryRow {
    pub trade_count: usize,
    pub trade_notional: String,
    pub taker_trade_rate: String,
    pub resolved_trade_count: usize,
    pub winner_alignment_rate: String,
    pub skipped_pair_cost_count: usize,
    pub skipped_outcome_count: usize,
}

#[derive(Debug, Clone)]
pub struct AnalysisImportSourceMetadata {
    pub import_run_id: String,
    pub dataset_dir: PathBuf,
    pub trade_parquet_path: PathBuf,
    pub close_csv_path: PathBuf,
    pub schema_doc_path: PathBuf,
    pub trade_parquet_sha256: String,
    pub close_csv_sha256: String,
    pub schema_doc_sha256: String,
    pub trade_parquet_mtime: Option<String>,
    pub close_csv_mtime: Option<String>,
    pub schema_doc_mtime: Option<String>,
    pub started_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone)]
pub struct AnalysisImportResult {
    pub source: AnalysisImportSourceMetadata,
    pub trade_rows: Vec<AnalysisTradeRow>,
    pub close_rows: Vec<AnalysisClosePositionRow>,
    pub pair_rollups: Vec<AnalysisPairRollup>,
    pub summary: AnalysisImportSummary,
}

pub trait AnalysisImportSink {
    fn persist(&mut self, result: &AnalysisImportResult) -> Result<()>;
}

pub struct NoopAnalysisImportSink;

impl AnalysisImportSink for NoopAnalysisImportSink {
    fn persist(&mut self, _result: &AnalysisImportResult) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct MemoryAnalysisImportSink {
    pub persisted_trade_rows: usize,
    pub persisted_close_rows: usize,
    pub persisted_pair_rollups: usize,
    pub last_summary: Option<AnalysisImportSummary>,
}

impl AnalysisImportSink for MemoryAnalysisImportSink {
    fn persist(&mut self, result: &AnalysisImportResult) -> Result<()> {
        self.persisted_trade_rows = result.trade_rows.len();
        self.persisted_close_rows = result.close_rows.len();
        self.persisted_pair_rollups = result.pair_rollups.len();
        self.last_summary = Some(result.summary.clone());
        Ok(())
    }
}

pub struct PostgresAnalysisImportSink {
    repo: BotRepository,
}

impl PostgresAnalysisImportSink {
    pub fn new(repo: BotRepository) -> Self {
        Self { repo }
    }
}

impl AnalysisImportSink for PostgresAnalysisImportSink {
    fn persist(&mut self, result: &AnalysisImportResult) -> Result<()> {
        self.repo.persist_analysis_import(result)
    }
}

#[derive(Debug, Default, Clone)]
struct PairRollupAccumulator {
    event_slug: String,
    trade_outcomes: BTreeSet<String>,
    close_outcomes: BTreeSet<String>,
    total_trade_count: i64,
    taker_trade_count: i64,
    total_notional: f64,
    taker_notional: f64,
    up_avg_price: Option<f64>,
    down_avg_price: Option<f64>,
    up_total_bought: Option<f64>,
    down_total_bought: Option<f64>,
    up_realized_pnl: Option<f64>,
    down_realized_pnl: Option<f64>,
    up_cur_price: Option<f64>,
    down_cur_price: Option<f64>,
}

#[derive(Debug, Default, Clone)]
struct PriceZoneAccumulator {
    trade_count: usize,
    trade_notional: f64,
    taker_trade_count: usize,
    resolved_trade_count: usize,
    winner_alignment_count: usize,
    skipped_outcome_count: usize,
}

pub fn default_output_dir() -> PathBuf {
    PathBuf::from("output").join("analysis_import")
}

pub fn resolve_dataset_paths(dataset_dir: &Path) -> Result<AnalysisImportPaths> {
    let dataset_dir = dataset_dir.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize dataset dir {}",
            dataset_dir.display()
        )
    })?;
    let trade_parquet = dataset_dir.join(TRADE_PARQUET_FILENAME);
    let close_csv = dataset_dir.join(CLOSE_CSV_FILENAME);
    let schema_doc = dataset_dir.join(SCHEMA_DOC_FILENAME);
    for path in [&trade_parquet, &close_csv, &schema_doc] {
        if !path.is_file() {
            return Err(anyhow!("missing required dataset file {}", path.display()));
        }
    }
    Ok(AnalysisImportPaths {
        dataset_dir,
        trade_parquet,
        close_csv,
        schema_doc,
    })
}

pub fn build_analysis_import_result(dataset_dir: &Path) -> Result<AnalysisImportResult> {
    let started_at = now_iso_utc();
    let paths = resolve_dataset_paths(dataset_dir)?;
    let trade_rows = load_trade_rows(&paths.trade_parquet)?;
    let close_rows = load_close_rows(&paths.close_csv)?;
    let pair_rollups = build_pair_rollups(&trade_rows, &close_rows);
    let summary = build_summary(
        &paths.trade_parquet,
        &paths.close_csv,
        &paths.schema_doc,
        &trade_rows,
        &close_rows,
    );
    let completed_at = now_iso_utc();
    let source = AnalysisImportSourceMetadata {
        import_run_id: Uuid::new_v4().to_string(),
        dataset_dir: paths.dataset_dir.clone(),
        trade_parquet_path: paths.trade_parquet.clone(),
        close_csv_path: paths.close_csv.clone(),
        schema_doc_path: paths.schema_doc.clone(),
        trade_parquet_sha256: sha256_file(&paths.trade_parquet)?,
        close_csv_sha256: sha256_file(&paths.close_csv)?,
        schema_doc_sha256: sha256_file(&paths.schema_doc)?,
        trade_parquet_mtime: file_mtime_iso(&paths.trade_parquet)?,
        close_csv_mtime: file_mtime_iso(&paths.close_csv)?,
        schema_doc_mtime: file_mtime_iso(&paths.schema_doc)?,
        started_at,
        completed_at,
    };
    Ok(AnalysisImportResult {
        source,
        trade_rows,
        close_rows,
        pair_rollups,
        summary,
    })
}

pub fn write_summary(output_dir: &Path, summary: &AnalysisImportSummary) -> Result<PathBuf> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed creating output dir {}", output_dir.display()))?;
    let summary_path = output_dir.join("summary.json");
    let payload =
        serde_json::to_string_pretty(summary).context("failed serializing analysis summary")?;
    fs::write(&summary_path, payload)
        .with_context(|| format!("failed writing {}", summary_path.display()))?;
    Ok(summary_path)
}

pub fn run_analysis_import(
    dataset_dir: &Path,
    output_dir: &Path,
    sink: &mut dyn AnalysisImportSink,
) -> Result<(AnalysisImportResult, PathBuf)> {
    let result = build_analysis_import_result(dataset_dir)?;
    let summary_path = write_summary(output_dir, &result.summary)?;
    sink.persist(&result)?;
    Ok((result, summary_path))
}

fn load_trade_rows(path: &Path) -> Result<Vec<AnalysisTradeRow>> {
    let file = File::open(path).with_context(|| format!("failed opening {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("failed reading parquet metadata for {}", path.display()))?;
    let schema_fields = builder
        .schema()
        .fields()
        .iter()
        .map(|field| (field.name().as_str(), field.data_type()))
        .collect::<Vec<_>>();
    validate_trade_schema(schema_fields)?;
    let mut reader = builder.build().with_context(|| {
        format!(
            "failed building parquet batch reader for {}",
            path.display()
        )
    })?;
    let mut rows = Vec::new();
    while let Some(batch) = reader.next() {
        let batch = batch
            .with_context(|| format!("failed reading parquet batch from {}", path.display()))?;
        let access = BatchAccess::new(&batch)?;
        for row_index in 0..batch.num_rows() {
            rows.push(build_trade_row(&access, row_index)?);
        }
    }
    Ok(rows)
}

fn build_trade_row(access: &BatchAccess<'_>, row_index: usize) -> Result<AnalysisTradeRow> {
    Ok(AnalysisTradeRow {
        trade_identity_key: access.required_string("trade_identity_key", row_index)?,
        proxyWallet: access.required_string("proxyWallet", row_index)?,
        side: access.required_string("side", row_index)?,
        asset: access.required_string("asset", row_index)?,
        conditionId: access.required_string("conditionId", row_index)?,
        size: access.required_f64("size", row_index)?,
        price: access.required_f64("price", row_index)?,
        timestamp: access.required_i64("timestamp", row_index)?,
        title: access.required_string("title", row_index)?,
        slug: access.optional_string("slug", row_index)?,
        eventSlug: access.required_string("eventSlug", row_index)?,
        outcome: access.required_string("outcome", row_index)?,
        outcomeIndex: access.required_i64("outcomeIndex", row_index)?,
        transactionHash: access.optional_string("transactionHash", row_index)?,
        is_taker: access.required_bool("is_taker", row_index)?,
        window_start: access.optional_i64("window_start", row_index)?,
        window_end: access.optional_i64("window_end", row_index)?,
        t_remain_s: access.optional_f64("t_remain_s", row_index)?,
        t_into_s: access.optional_f64("t_into_s", row_index)?,
        trade_time_utc: access.optional_string("trade_time_utc", row_index)?,
        binance_btc_trade_px: access.optional_f64("binance_btc_trade_px", row_index)?,
        binance_btc_start_px: access.optional_f64("binance_btc_start_px", row_index)?,
        binance_delta_from_start: access.optional_f64("binance_delta_from_start", row_index)?,
        binance_rsi14_at_trade: access.optional_f64("binance_rsi14_at_trade", row_index)?,
        binance_vol30m_1m_at_trade: access.optional_f64("binance_vol30m_1m_at_trade", row_index)?,
        binance_up_model: access.optional_f64("binance_up_model", row_index)?,
        binance_down_model: access.optional_f64("binance_down_model", row_index)?,
        edge_model_minus_price: access.optional_f64("edge_model_minus_price", row_index)?,
        final_outcome: access.optional_string("final_outcome", row_index)?,
        snapshot_status: access.required_string("snapshot_status", row_index)?,
        snapshot_requested_ts_ms: access.optional_i64("snapshot_requested_ts_ms", row_index)?,
        snapshot_market_id: access.optional_i64("snapshot_market_id", row_index)?,
        snapshot_time: access.optional_string("snapshot_time", row_index)?,
        snapshot_match_delta_ms: access.optional_f64("snapshot_match_delta_ms", row_index)?,
        snapshot_id: access.optional_f64("snapshot_id", row_index)?,
        snapsot_market_btc_price: access.optional_f64("snapsot_market_btc_price", row_index)?,
        snapshot_price_up: access.optional_f64("snapshot_price_up", row_index)?,
        snapshot_price_down: access.optional_f64("snapshot_price_down", row_index)?,
        snapshot_last_trade_price_up: access
            .optional_f64("snapshot_last_trade_price_up", row_index)?,
        snapshot_last_trade_price_down: access
            .optional_f64("snapshot_last_trade_price_down", row_index)?,
        snapshot_min_order_size_up: access.optional_f64("snapshot_min_order_size_up", row_index)?,
        snapshot_min_order_size_down: access
            .optional_f64("snapshot_min_order_size_down", row_index)?,
        snapshot_tick_size_up: access.optional_f64("snapshot_tick_size_up", row_index)?,
        snapshot_tick_size_down: access.optional_f64("snapshot_tick_size_down", row_index)?,
        snapshot_orderbook_up_bid_count: access
            .optional_f64("snapshot_orderbook_up_bid_count", row_index)?,
        snapshot_orderbook_up_ask_count: access
            .optional_f64("snapshot_orderbook_up_ask_count", row_index)?,
        snapshot_orderbook_up_spread: access
            .optional_f64("snapshot_orderbook_up_spread", row_index)?,
        snapshot_orderbook_up_bid_1_price: access
            .optional_f64("snapshot_orderbook_up_bid_1_price", row_index)?,
        snapshot_orderbook_up_bid_1_size: access
            .optional_f64("snapshot_orderbook_up_bid_1_size", row_index)?,
        snapshot_orderbook_up_ask_1_price: access
            .optional_f64("snapshot_orderbook_up_ask_1_price", row_index)?,
        snapshot_orderbook_up_ask_1_size: access
            .optional_f64("snapshot_orderbook_up_ask_1_size", row_index)?,
        snapshot_orderbook_down_bid_count: access
            .optional_f64("snapshot_orderbook_down_bid_count", row_index)?,
        snapshot_orderbook_down_ask_count: access
            .optional_f64("snapshot_orderbook_down_ask_count", row_index)?,
        snapshot_orderbook_down_spread: access
            .optional_f64("snapshot_orderbook_down_spread", row_index)?,
        snapshot_orderbook_down_bid_1_price: access
            .optional_f64("snapshot_orderbook_down_bid_1_price", row_index)?,
        snapshot_orderbook_down_bid_1_size: access
            .optional_f64("snapshot_orderbook_down_bid_1_size", row_index)?,
        snapshot_orderbook_down_ask_1_price: access
            .optional_f64("snapshot_orderbook_down_ask_1_price", row_index)?,
        snapshot_orderbook_down_ask_1_size: access
            .optional_f64("snapshot_orderbook_down_ask_1_size", row_index)?,
        snapsot_market_btc_price_to_beat: access
            .optional_f64("snapsot_market_btc_price_to_beat", row_index)?,
        snapsot_btc_price_delta: access.optional_f64("snapsot_btc_price_delta", row_index)?,
    })
}

fn load_close_rows(path: &Path) -> Result<Vec<AnalysisClosePositionRow>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .with_context(|| format!("failed opening {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("failed reading headers from {}", path.display()))?
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    let expected_headers = CLOSE_CSV_HEADERS
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    if headers != expected_headers {
        return Err(anyhow!(
            "csv header mismatch for {}: expected {:?}, got {:?}",
            path.display(),
            expected_headers,
            headers
        ));
    }
    let mut rows = Vec::new();
    for row in reader.deserialize() {
        let row: AnalysisClosePositionRow =
            row.with_context(|| format!("failed parsing csv row in {}", path.display()))?;
        rows.push(row);
    }
    Ok(rows)
}

fn build_pair_rollups(
    trade_rows: &[AnalysisTradeRow],
    close_rows: &[AnalysisClosePositionRow],
) -> Vec<AnalysisPairRollup> {
    let mut by_condition: BTreeMap<String, PairRollupAccumulator> = BTreeMap::new();
    for row in trade_rows {
        let entry = by_condition.entry(row.conditionId.clone()).or_default();
        if entry.event_slug.is_empty() {
            entry.event_slug = row.eventSlug.clone();
        }
        entry.trade_outcomes.insert(row.outcome.clone());
        entry.total_trade_count += 1;
        entry.total_notional += row.notional();
        if row.is_taker {
            entry.taker_trade_count += 1;
            entry.taker_notional += row.notional();
        }
    }
    for row in close_rows {
        let entry = by_condition.entry(row.conditionId.clone()).or_default();
        if entry.event_slug.is_empty() {
            entry.event_slug = row.eventSlug.clone();
        }
        entry.close_outcomes.insert(row.outcome.clone());
        match row.outcome.as_str() {
            "Up" => {
                entry.up_avg_price = Some(row.avgPrice);
                entry.up_total_bought = Some(row.totalBought);
                entry.up_realized_pnl = Some(row.realizedPnl);
                entry.up_cur_price = Some(row.curPrice);
            }
            "Down" => {
                entry.down_avg_price = Some(row.avgPrice);
                entry.down_total_bought = Some(row.totalBought);
                entry.down_realized_pnl = Some(row.realizedPnl);
                entry.down_cur_price = Some(row.curPrice);
            }
            _ => {}
        }
    }
    by_condition
        .into_iter()
        .map(|(condition_id, acc)| AnalysisPairRollup {
            condition_id,
            event_slug: acc.event_slug,
            trade_outcomes_csv: join_set(&acc.trade_outcomes),
            close_outcomes_csv: join_set(&acc.close_outcomes),
            both_sided_close: acc.close_outcomes.len() == 2,
            total_trade_count: acc.total_trade_count,
            taker_trade_count: acc.taker_trade_count,
            total_notional: acc.total_notional,
            taker_notional: acc.taker_notional,
            up_avg_price: acc.up_avg_price,
            down_avg_price: acc.down_avg_price,
            up_total_bought: acc.up_total_bought,
            down_total_bought: acc.down_total_bought,
            up_realized_pnl: acc.up_realized_pnl,
            down_realized_pnl: acc.down_realized_pnl,
            up_cur_price: acc.up_cur_price,
            down_cur_price: acc.down_cur_price,
        })
        .collect()
}

fn build_summary(
    trade_parquet: &Path,
    close_csv: &Path,
    schema_doc: &Path,
    trade_rows: &[AnalysisTradeRow],
    close_rows: &[AnalysisClosePositionRow],
) -> AnalysisImportSummary {
    let filtered_market_count = trade_rows
        .iter()
        .map(|row| row.conditionId.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let close_condition_ids = close_rows
        .iter()
        .map(|row| row.conditionId.as_str())
        .collect::<BTreeSet<_>>();
    let closed_position_pair_count = close_condition_ids.len();
    let mut close_outcomes_by_condition: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for row in close_rows {
        close_outcomes_by_condition
            .entry(row.conditionId.as_str())
            .or_default()
            .insert(row.outcome.as_str());
    }
    let two_sided_pairs = close_outcomes_by_condition
        .values()
        .filter(|values| values.len() == 2)
        .count();

    let mut pair_cost_values = Vec::new();
    let mut pair_cost_weights = Vec::new();
    let mut skipped_pair_cost_count = 0usize;
    let mut skipped_outcome_count = 0usize;
    let mut taker_trade_count = 0usize;
    let mut taker_notional = 0.0f64;
    let mut total_notional = 0.0f64;
    let mut zone_accumulators: BTreeMap<&'static str, PriceZoneAccumulator> = BTreeMap::new();
    for zone in [
        PriceZone::Preferred,
        PriceZone::Acceptable,
        PriceZone::Caution,
        PriceZone::StopAdd,
        PriceZone::Danger,
    ] {
        zone_accumulators.insert(zone.as_str(), PriceZoneAccumulator::default());
    }

    for row in trade_rows {
        let notional = row.notional();
        total_notional += notional;
        if row.is_taker {
            taker_trade_count += 1;
            taker_notional += notional;
        }
        if !row.resolved_outcome_present() {
            skipped_outcome_count += 1;
        }
        let Some(pair_cost) = row.historical_effective_pair_cost() else {
            skipped_pair_cost_count += 1;
            continue;
        };
        pair_cost_values.push(pair_cost);
        pair_cost_weights.push(notional);
        let zone = PriceZone::classify(pair_cost).as_str();
        let acc = zone_accumulators
            .get_mut(zone)
            .expect("zone accumulator exists");
        acc.trade_count += 1;
        acc.trade_notional += notional;
        if row.is_taker {
            acc.taker_trade_count += 1;
        }
        if let Some(aligned) = row.winner_alignment() {
            acc.resolved_trade_count += 1;
            if aligned {
                acc.winner_alignment_count += 1;
            }
        } else {
            acc.skipped_outcome_count += 1;
        }
    }

    let weighted_pair_sum_median =
        weighted_median(&pair_cost_values, &pair_cost_weights).unwrap_or(0.0);
    AnalysisImportSummary {
        dataset: AnalysisImportDatasetSummary {
            trade_parquet: trade_parquet
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            close_csv: close_csv
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            schema_doc: schema_doc
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        },
        counts: AnalysisImportCounts {
            parquet_rows: trade_rows.len(),
            close_rows: close_rows.len(),
            filtered_market_count,
            closed_position_pair_count,
            two_sided_pairs,
        },
        metrics: AnalysisImportMetrics {
            two_sided_participation_rate: format_fixed6(if closed_position_pair_count == 0 {
                0.0
            } else {
                two_sided_pairs as f64 / closed_position_pair_count as f64
            }),
            taker_share: format_fixed6(if trade_rows.is_empty() {
                0.0
            } else {
                taker_trade_count as f64 / trade_rows.len() as f64
            }),
            taker_share_notional: format_fixed6(if total_notional <= 0.0 {
                0.0
            } else {
                taker_notional / total_notional
            }),
            weighted_pair_sum_median: format_fixed6(weighted_pair_sum_median),
        },
        coverage: AnalysisImportCoverage {
            skipped_pair_cost_count,
            skipped_outcome_count,
        },
        price_zone_summary: AnalysisImportPriceZoneSummary {
            preferred: zone_summary_row(zone_accumulators.get("preferred").unwrap()),
            acceptable: zone_summary_row(zone_accumulators.get("acceptable").unwrap()),
            caution: zone_summary_row(zone_accumulators.get("caution").unwrap()),
            stop_add: zone_summary_row(zone_accumulators.get("stop_add").unwrap()),
            danger: zone_summary_row(zone_accumulators.get("danger").unwrap()),
        },
    }
}

fn zone_summary_row(acc: &PriceZoneAccumulator) -> PriceZoneSummaryRow {
    PriceZoneSummaryRow {
        trade_count: acc.trade_count,
        trade_notional: format_fixed6(acc.trade_notional),
        taker_trade_rate: format_fixed6(if acc.trade_count == 0 {
            0.0
        } else {
            acc.taker_trade_count as f64 / acc.trade_count as f64
        }),
        resolved_trade_count: acc.resolved_trade_count,
        winner_alignment_rate: format_fixed6(if acc.resolved_trade_count == 0 {
            0.0
        } else {
            acc.winner_alignment_count as f64 / acc.resolved_trade_count as f64
        }),
        skipped_pair_cost_count: 0,
        skipped_outcome_count: acc.skipped_outcome_count,
    }
}

fn join_set(values: &BTreeSet<String>) -> String {
    values.iter().cloned().collect::<Vec<_>>().join(",")
}

fn format_fixed6(value: f64) -> String {
    format!("{value:.6}")
}

fn weighted_median(values: &[f64], weights: &[f64]) -> Option<f64> {
    if values.len() != weights.len() || values.is_empty() {
        return None;
    }
    let mut pairs = values
        .iter()
        .copied()
        .zip(weights.iter().copied())
        .filter(|(_, weight)| *weight > 0.0)
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return None;
    }
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let total_weight: f64 = pairs.iter().map(|(_, weight)| *weight).sum();
    let mut accumulated = 0.0;
    for (value, weight) in pairs {
        accumulated += weight;
        if accumulated >= total_weight / 2.0 {
            return Some(value);
        }
    }
    None
}

fn validate_trade_schema(fields: Vec<(&str, &DataType)>) -> Result<()> {
    if fields.len() != TRADE_PARQUET_COLUMNS.len() {
        return Err(anyhow!(
            "parquet schema column count mismatch: expected {}, got {}",
            TRADE_PARQUET_COLUMNS.len(),
            fields.len()
        ));
    }
    for (index, ((expected_name, expected_kind), (actual_name, actual_type))) in
        TRADE_PARQUET_COLUMNS.iter().zip(fields.iter()).enumerate()
    {
        if expected_name != actual_name {
            return Err(anyhow!(
                "parquet schema mismatch at column {}: expected {}, got {}",
                index,
                expected_name,
                actual_name
            ));
        }
        if !matches_column_kind(expected_kind, actual_type) {
            return Err(anyhow!(
                "parquet type mismatch for {}: expected {:?}, got {:?}",
                expected_name,
                expected_kind,
                actual_type
            ));
        }
    }
    Ok(())
}

fn matches_column_kind(expected: &ColumnKind, actual: &DataType) -> bool {
    matches!(
        (expected, actual),
        (ColumnKind::String, DataType::Utf8)
            | (ColumnKind::Float64, DataType::Float64)
            | (ColumnKind::Int64, DataType::Int64)
            | (ColumnKind::Boolean, DataType::Boolean)
    )
}

fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("failed opening {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = reader
            .read(&mut buf)
            .with_context(|| format!("failed hashing {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_mtime_iso(path: &Path) -> Result<Option<String>> {
    let meta = fs::metadata(path)
        .with_context(|| format!("failed reading metadata for {}", path.display()))?;
    let modified = match meta.modified() {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let dt: DateTime<Utc> = modified.into();
    Ok(Some(dt.to_rfc3339()))
}

fn now_iso_utc() -> String {
    Utc::now().to_rfc3339()
}

struct BatchAccess<'a> {
    batch: &'a RecordBatch,
    indexes: BTreeMap<&'static str, usize>,
}

impl<'a> BatchAccess<'a> {
    fn new(batch: &'a RecordBatch) -> Result<Self> {
        let mut indexes = BTreeMap::new();
        for (name, _) in TRADE_PARQUET_COLUMNS {
            let index = batch
                .schema()
                .index_of(name)
                .with_context(|| format!("missing parquet column {}", name))?;
            indexes.insert(name, index);
        }
        Ok(Self { batch, indexes })
    }

    fn column(&self, name: &'static str) -> Result<&ArrayRef> {
        let index = *self
            .indexes
            .get(name)
            .ok_or_else(|| anyhow!("missing batch index for {}", name))?;
        Ok(self.batch.column(index))
    }

    fn required_string(&self, name: &'static str, row: usize) -> Result<String> {
        self.optional_string(name, row)?
            .ok_or_else(|| anyhow!("required parquet string {} is null at row {}", name, row))
    }

    fn optional_string(&self, name: &'static str, row: usize) -> Result<Option<String>> {
        let array = self
            .column(name)?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow!("parquet column {} is not Utf8", name))?;
        if array.is_null(row) {
            Ok(None)
        } else {
            Ok(Some(array.value(row).to_string()))
        }
    }

    fn required_f64(&self, name: &'static str, row: usize) -> Result<f64> {
        self.optional_f64(name, row)?
            .ok_or_else(|| anyhow!("required parquet float {} is null at row {}", name, row))
    }

    fn optional_f64(&self, name: &'static str, row: usize) -> Result<Option<f64>> {
        let array = self
            .column(name)?
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| anyhow!("parquet column {} is not Float64", name))?;
        if array.is_null(row) {
            Ok(None)
        } else {
            Ok(Some(array.value(row)))
        }
    }

    fn required_i64(&self, name: &'static str, row: usize) -> Result<i64> {
        self.optional_i64(name, row)?
            .ok_or_else(|| anyhow!("required parquet int {} is null at row {}", name, row))
    }

    fn optional_i64(&self, name: &'static str, row: usize) -> Result<Option<i64>> {
        let array = self
            .column(name)?
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| anyhow!("parquet column {} is not Int64", name))?;
        if array.is_null(row) {
            Ok(None)
        } else {
            Ok(Some(array.value(row)))
        }
    }

    fn required_bool(&self, name: &'static str, row: usize) -> Result<bool> {
        self.optional_bool(name, row)?
            .ok_or_else(|| anyhow!("required parquet bool {} is null at row {}", name, row))
    }

    fn optional_bool(&self, name: &'static str, row: usize) -> Result<Option<bool>> {
        let array = self
            .column(name)?
            .as_any()
            .downcast_ref::<BooleanArray>()
            .ok_or_else(|| anyhow!("parquet column {} is not Boolean", name))?;
        if array.is_null(row) {
            Ok(None)
        } else {
            Ok(Some(array.value(row)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[derive(Default)]
    struct CountingSink {
        persist_calls: usize,
    }

    impl AnalysisImportSink for CountingSink {
        fn persist(&mut self, _result: &AnalysisImportResult) -> Result<()> {
            self.persist_calls += 1;
            Ok(())
        }
    }

    fn dataset_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dataset")
    }

    #[test]
    fn parquet_column_list_matches_dataset() {
        let paths = resolve_dataset_paths(&dataset_dir()).expect("dataset paths");
        let file = File::open(paths.trade_parquet).expect("open parquet");
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("builder");
        let actual = builder
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .collect::<Vec<_>>();
        let expected = TRADE_PARQUET_COLUMNS
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn csv_header_list_matches_dataset() {
        let paths = resolve_dataset_paths(&dataset_dir()).expect("dataset paths");
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(paths.close_csv)
            .expect("open csv");
        let actual = reader
            .headers()
            .expect("headers")
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let expected = CLOSE_CSV_HEADERS
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn pair_cost_fallback_uses_snapshot_last_then_snapshot_price() {
        let mut row = AnalysisTradeRow {
            trade_identity_key: "id".to_string(),
            proxyWallet: "wallet".to_string(),
            side: "BUY".to_string(),
            asset: "asset".to_string(),
            conditionId: "condition".to_string(),
            size: 10.0,
            price: 0.41,
            timestamp: 1,
            title: "title".to_string(),
            slug: None,
            eventSlug: "slug".to_string(),
            outcome: "Up".to_string(),
            outcomeIndex: 0,
            transactionHash: None,
            is_taker: false,
            window_start: None,
            window_end: None,
            t_remain_s: None,
            t_into_s: None,
            trade_time_utc: None,
            binance_btc_trade_px: None,
            binance_btc_start_px: None,
            binance_delta_from_start: None,
            binance_rsi14_at_trade: None,
            binance_vol30m_1m_at_trade: None,
            binance_up_model: None,
            binance_down_model: None,
            edge_model_minus_price: None,
            final_outcome: None,
            snapshot_status: "matched".to_string(),
            snapshot_requested_ts_ms: None,
            snapshot_market_id: None,
            snapshot_time: None,
            snapshot_match_delta_ms: None,
            snapshot_id: None,
            snapsot_market_btc_price: None,
            snapshot_price_up: None,
            snapshot_price_down: Some(0.56),
            snapshot_last_trade_price_up: None,
            snapshot_last_trade_price_down: Some(0.51),
            snapshot_min_order_size_up: None,
            snapshot_min_order_size_down: None,
            snapshot_tick_size_up: None,
            snapshot_tick_size_down: None,
            snapshot_orderbook_up_bid_count: None,
            snapshot_orderbook_up_ask_count: None,
            snapshot_orderbook_up_spread: None,
            snapshot_orderbook_up_bid_1_price: None,
            snapshot_orderbook_up_bid_1_size: None,
            snapshot_orderbook_up_ask_1_price: None,
            snapshot_orderbook_up_ask_1_size: None,
            snapshot_orderbook_down_bid_count: None,
            snapshot_orderbook_down_ask_count: None,
            snapshot_orderbook_down_spread: None,
            snapshot_orderbook_down_bid_1_price: None,
            snapshot_orderbook_down_bid_1_size: None,
            snapshot_orderbook_down_ask_1_price: None,
            snapshot_orderbook_down_ask_1_size: None,
            snapsot_market_btc_price_to_beat: None,
            snapsot_btc_price_delta: None,
        };
        let pair_cost = row
            .historical_effective_pair_cost()
            .expect("pair cost with opposite last trade");
        assert!((pair_cost - 0.92).abs() < 1e-9);
        row.snapshot_last_trade_price_down = None;
        let fallback_cost = row
            .historical_effective_pair_cost()
            .expect("pair cost with opposite snapshot price");
        assert!((fallback_cost - 0.97).abs() < 1e-9);
    }

    #[test]
    fn price_zone_boundaries_match_requirement_thresholds() {
        assert_eq!(PriceZone::classify(0.939999).as_str(), "preferred");
        assert_eq!(PriceZone::classify(0.94).as_str(), "acceptable");
        assert_eq!(PriceZone::classify(0.97).as_str(), "caution");
        assert_eq!(PriceZone::classify(1.0).as_str(), "stop_add");
        assert_eq!(PriceZone::classify(1.03).as_str(), "danger");
        assert_eq!(
            PriceZone::classify(0.94 - (PriceZone::THRESHOLD_EPSILON / 2.0)).as_str(),
            "acceptable"
        );
        assert_eq!(
            PriceZone::classify(0.97 - (PriceZone::THRESHOLD_EPSILON / 2.0)).as_str(),
            "caution"
        );
        assert_eq!(
            PriceZone::classify(1.0 - (PriceZone::THRESHOLD_EPSILON / 2.0)).as_str(),
            "stop_add"
        );
        assert_eq!(
            PriceZone::classify(1.03 - (PriceZone::THRESHOLD_EPSILON / 2.0)).as_str(),
            "danger"
        );
    }

    #[test]
    fn weighted_median_helper_returns_expected_value() {
        let values = [0.91, 0.95, 1.02];
        let weights = [1.0, 3.0, 1.0];
        assert_eq!(weighted_median(&values, &weights), Some(0.95));
    }

    #[test]
    fn in_memory_sink_captures_import_counts() {
        let mut sink = MemoryAnalysisImportSink::default();
        let result = build_analysis_import_result(&dataset_dir()).expect("build import result");
        sink.persist(&result).expect("persist to memory sink");
        assert_eq!(sink.persisted_trade_rows, 29342);
        assert_eq!(sink.persisted_close_rows, 209);
        assert_eq!(sink.persisted_pair_rollups, 105);
        assert_eq!(
            sink.last_summary.expect("summary").counts.two_sided_pairs,
            104
        );
    }

    #[test]
    fn run_analysis_import_does_not_persist_when_summary_write_fails() {
        let temp_root = std::env::temp_dir().join(format!(
            "polybot-analysis-import-summary-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_root).expect("create temp root");
        let blocked_output = temp_root.join("occupied-output-path");
        fs::write(&blocked_output, "occupied").expect("seed blocking output file");

        let mut sink = CountingSink::default();
        let err = run_analysis_import(&dataset_dir(), &blocked_output, &mut sink)
            .expect_err("summary write should fail when output path is a file");

        assert!(
            err.to_string().contains("failed creating output dir"),
            "actual_error={err:#}"
        );
        assert_eq!(sink.persist_calls, 0);

        fs::remove_file(&blocked_output).ok();
        fs::remove_dir_all(&temp_root).ok();
    }
}
