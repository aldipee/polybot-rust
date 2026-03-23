use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::db::BotRepository;

const PAPER_TRADING_DAYS_MIN: usize = 10;
const PAPER_SETTLED_PAIRS_MIN: usize = 500;
const SHADOW_TRADING_DAYS_MIN: usize = 3;
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KpiStatus {
    Pass,
    Warn,
    Fail,
    InsufficientSample,
}

impl KpiStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::InsufficientSample => "INSUFFICIENT_SAMPLE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KpiProfile {
    Paper,
    Shadow,
}

impl KpiProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::Shadow => "shadow",
        }
    }

    pub fn from_arg(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "paper" => Some(Self::Paper),
            "shadow" => Some(Self::Shadow),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiTradeRow {
    pub trade_id: String,
    pub bot_id: String,
    pub pair_id: String,
    pub market_slug: String,
    pub date: String,
    pub start_trade: String,
    pub end_trade: String,
    pub entry_reason: Option<String>,
    pub exit_reason: String,
    pub lp: f64,
    pub total_cost: f64,
    pub q_yes: f64,
    pub q_no: f64,
    pub cpp: f64,
    pub status: Option<String>,
    pub claim_status: Option<String>,
    pub meta_data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiDecisionEventRow {
    pub decision_event_id: String,
    pub trade_id: String,
    pub decision_ts: String,
    pub approved: bool,
    pub reason_code: String,
    pub phase: Option<String>,
    pub owner: Option<String>,
    pub submit_origin: Option<String>,
    pub submit_side: Option<String>,
    pub payload_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiRuntimeEventRow {
    pub event_id: String,
    pub trade_id: String,
    pub event_kind: String,
    pub event_ts: String,
    pub reason_code: Option<String>,
    pub payload_json: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KpiSettlementDecomposition {
    pub paired_qty: f64,
    pub residual_qty: f64,
    pub paired_cost: f64,
    pub paired_realized_pnl: f64,
    pub residual_realized_pnl: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiRunRequest {
    pub bot_id: String,
    pub profile: KpiProfile,
    pub window_start: String,
    pub window_end: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KpiGateSampleCoverage {
    pub distinct_trading_days: usize,
    pub settled_pairs: usize,
    pub selected_trade_count: usize,
    pub participating_run_count: usize,
    pub sufficient_sample: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KpiGateSourceCounts {
    pub loaded_trades: usize,
    pub loaded_decision_events: usize,
    pub loaded_runtime_events: usize,
    pub selected_trades: usize,
    pub selected_decision_events: usize,
    pub selected_runtime_events: usize,
    pub missing_run_summary_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KpiGateMetricReport {
    pub status: String,
    pub details: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KpiGateEvaluation {
    pub failing_metrics: Vec<String>,
    pub warning_metrics: Vec<String>,
    pub insufficient_sample: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiGateReport {
    pub run_id: String,
    pub metadata: BTreeMap<String, Value>,
    pub sample_coverage: KpiGateSampleCoverage,
    pub source_counts: KpiGateSourceCounts,
    pub metrics: BTreeMap<String, KpiGateMetricReport>,
    pub evaluation: KpiGateEvaluation,
    pub overall_status: String,
}

pub trait KpiEventSource {
    fn load_trades(
        &self,
        bot_id: &str,
        window_start: &str,
        window_end: &str,
    ) -> Result<Vec<KpiTradeRow>>;
    fn load_decision_events(
        &self,
        bot_id: &str,
        window_start: &str,
        window_end: &str,
    ) -> Result<Vec<KpiDecisionEventRow>>;
    fn load_runtime_events(
        &self,
        bot_id: &str,
        window_start: &str,
        window_end: &str,
    ) -> Result<Vec<KpiRuntimeEventRow>>;
}

pub trait KpiReportSink {
    fn persist(&mut self, report: &KpiGateReport, summary_path: &Path) -> Result<()>;
}

pub struct NoopKpiGateSink;

impl KpiReportSink for NoopKpiGateSink {
    fn persist(&mut self, _report: &KpiGateReport, _summary_path: &Path) -> Result<()> {
        Ok(())
    }
}

pub struct PostgresKpiGateSink {
    repo: BotRepository,
}

impl PostgresKpiGateSink {
    pub fn new(repo: BotRepository) -> Self {
        Self { repo }
    }
}

impl KpiReportSink for PostgresKpiGateSink {
    fn persist(&mut self, report: &KpiGateReport, summary_path: &Path) -> Result<()> {
        self.repo.persist_kpi_gate_report(report, summary_path)
    }
}

impl KpiEventSource for BotRepository {
    fn load_trades(
        &self,
        bot_id: &str,
        window_start: &str,
        window_end: &str,
    ) -> Result<Vec<KpiTradeRow>> {
        self.load_kpi_trades(bot_id, window_start, window_end)
    }

    fn load_decision_events(
        &self,
        bot_id: &str,
        window_start: &str,
        window_end: &str,
    ) -> Result<Vec<KpiDecisionEventRow>> {
        self.load_kpi_decision_events(bot_id, window_start, window_end)
    }

    fn load_runtime_events(
        &self,
        bot_id: &str,
        window_start: &str,
        window_end: &str,
    ) -> Result<Vec<KpiRuntimeEventRow>> {
        self.load_kpi_runtime_events(bot_id, window_start, window_end)
    }
}

#[derive(Debug, Clone)]
struct ParsedDecisionEvent {
    row: KpiDecisionEventRow,
    payload: Value,
}

#[derive(Debug, Clone)]
struct ParsedRuntimeEvent {
    row: KpiRuntimeEventRow,
    payload: Value,
}

#[derive(Debug, Clone, Default)]
struct RunSummaryPayload {
    trade_id: String,
    event_ts: String,
    phase: String,
    owner: String,
    safety_gate: String,
    safety_gate_reason: String,
    configured_order_mode: String,
    effective_order_mode: String,
    live_order_mode_block_reason: Option<String>,
    fill_count: usize,
    market_participated: bool,
    entry_reason: Option<String>,
    exit_reason: String,
    settlement_status: String,
    settlement_reason: String,
    q_yes: f64,
    q_no: f64,
    total_cost: f64,
    cpp: f64,
    paired_size: f64,
    unmatched_size: f64,
    unmatched_fraction: f64,
    pair_taker_share: f64,
    daily_taker_share: f64,
    open_both_seed_by_deadline_met: bool,
    open_both_submit_delta_met: bool,
    open_both_first_submit_delta_ms: f64,
    second_side_by_15s: bool,
    second_side_by_30s: bool,
    first_fill_to_second_fill_ms: f64,
    await_second_fill_hard_paused: bool,
    startup_completion_blocked_count: u32,
    audit_decision_event_count: u32,
    audit_runtime_event_count: u32,
}

pub fn default_output_dir() -> PathBuf {
    PathBuf::from("output").join("kpi_gate")
}

pub fn settlement_pnl_decomposition(
    lp: f64,
    q_yes: f64,
    q_no: f64,
    cpp: f64,
) -> KpiSettlementDecomposition {
    let paired_qty = q_yes.max(0.0).min(q_no.max(0.0));
    let residual_qty = (q_yes.max(0.0) - q_no.max(0.0)).abs();
    let paired_cost = 2.0 * paired_qty * cpp.max(0.0);
    let paired_realized_pnl = paired_qty - paired_cost;
    let residual_realized_pnl = lp - paired_realized_pnl;
    KpiSettlementDecomposition {
        paired_qty,
        residual_qty,
        paired_cost,
        paired_realized_pnl,
        residual_realized_pnl,
    }
}

pub fn run_kpi_gate<S: KpiEventSource, K: KpiReportSink>(
    source: &S,
    sink: &mut K,
    request: &KpiRunRequest,
    output_dir: &Path,
) -> Result<(KpiGateReport, PathBuf)> {
    let window_start = parse_iso(request.window_start.as_str())
        .with_context(|| format!("invalid --start {}", request.window_start))?;
    let window_end = parse_iso(request.window_end.as_str())
        .with_context(|| format!("invalid --end {}", request.window_end))?;
    if window_end < window_start {
        return Err(anyhow!("window_end must be >= window_start"));
    }

    let trades = source.load_trades(
        request.bot_id.as_str(),
        request.window_start.as_str(),
        request.window_end.as_str(),
    )?;
    let decision_events = source.load_decision_events(
        request.bot_id.as_str(),
        request.window_start.as_str(),
        request.window_end.as_str(),
    )?;
    let runtime_events = source.load_runtime_events(
        request.bot_id.as_str(),
        request.window_start.as_str(),
        request.window_end.as_str(),
    )?;

    let report = build_kpi_gate_report(request, trades, decision_events, runtime_events)?;
    let summary_path = write_summary(&report, output_dir, request)?;
    sink.persist(&report, &summary_path)?;
    Ok((report, summary_path))
}

fn build_kpi_gate_report(
    request: &KpiRunRequest,
    trades: Vec<KpiTradeRow>,
    decision_events: Vec<KpiDecisionEventRow>,
    runtime_events: Vec<KpiRuntimeEventRow>,
) -> Result<KpiGateReport> {
    let loaded_trade_count = trades.len();
    let loaded_decision_count = decision_events.len();
    let loaded_runtime_count = runtime_events.len();

    let parsed_decisions = decision_events
        .into_iter()
        .map(|row| {
            let payload: Value = serde_json::from_str(&row.payload_json).with_context(|| {
                format!("failed parsing decision payload {}", row.decision_event_id)
            })?;
            Ok(ParsedDecisionEvent { row, payload })
        })
        .collect::<Result<Vec<_>>>()?;
    let parsed_runtime = runtime_events
        .into_iter()
        .map(|row| {
            let payload: Value = serde_json::from_str(&row.payload_json)
                .with_context(|| format!("failed parsing runtime payload {}", row.event_id))?;
            Ok(ParsedRuntimeEvent { row, payload })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut run_summaries = HashMap::<String, RunSummaryPayload>::new();
    let mut terminal_settlement_by_trade = HashMap::<String, String>::new();
    for event in &parsed_runtime {
        if event.row.event_kind == "run_summary" {
            let summary = parse_run_summary(event)?;
            run_summaries.insert(summary.trade_id.clone(), summary);
        }
        if event.row.event_kind == "settlement" {
            if let Some(reason) = event.row.reason_code.as_deref() {
                if matches!(reason, "settled" | "resolution_snapshot_unavailable") {
                    terminal_settlement_by_trade
                        .insert(event.row.trade_id.clone(), reason.to_string());
                }
            }
        }
    }

    let summarized_trade_ids: BTreeSet<String> = run_summaries.keys().cloned().collect();
    let selected_trade_ids: BTreeSet<String> = run_summaries
        .iter()
        .filter_map(|(trade_id, summary)| {
            if summary.effective_order_mode == request.profile.as_str() {
                Some(trade_id.clone())
            } else {
                None
            }
        })
        .collect();
    let stable_profile_signal_trade_ids =
        stable_profile_signal_trade_ids(&parsed_runtime, &parsed_decisions, request.profile);

    let missing_run_summary_count = stable_profile_signal_trade_ids
        .iter()
        .filter(|trade_id| !summarized_trade_ids.contains(*trade_id))
        .count();

    let selected_trades = trades
        .into_iter()
        .filter(|row| selected_trade_ids.contains(row.trade_id.as_str()))
        .collect::<Vec<_>>();
    let selected_decisions = parsed_decisions
        .into_iter()
        .filter(|row| selected_trade_ids.contains(row.row.trade_id.as_str()))
        .collect::<Vec<_>>();
    let selected_runtime = parsed_runtime
        .into_iter()
        .filter(|row| selected_trade_ids.contains(row.row.trade_id.as_str()))
        .collect::<Vec<_>>();

    let participating_runs = selected_trades
        .iter()
        .filter(|row| {
            run_summaries
                .get(row.trade_id.as_str())
                .map(|summary| summary.market_participated)
                .unwrap_or(false)
        })
        .count();
    let distinct_days = selected_trades
        .iter()
        .map(|row| row.date.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let settled_pairs = selected_trades
        .iter()
        .filter(|row| row.claim_status.as_deref() == Some("SETTLED"))
        .count();
    let sufficient_sample = match request.profile {
        KpiProfile::Paper => {
            distinct_days >= PAPER_TRADING_DAYS_MIN && settled_pairs >= PAPER_SETTLED_PAIRS_MIN
        }
        KpiProfile::Shadow => distinct_days >= SHADOW_TRADING_DAYS_MIN,
    };

    let sample_coverage = KpiGateSampleCoverage {
        distinct_trading_days: distinct_days,
        settled_pairs,
        selected_trade_count: selected_trades.len(),
        participating_run_count: participating_runs,
        sufficient_sample,
    };
    let source_counts = KpiGateSourceCounts {
        loaded_trades: loaded_trade_count,
        loaded_decision_events: loaded_decision_count,
        loaded_runtime_events: loaded_runtime_count,
        selected_trades: selected_trades.len(),
        selected_decision_events: selected_decisions.len(),
        selected_runtime_events: selected_runtime.len(),
        missing_run_summary_count,
    };

    let metrics = match request.profile {
        KpiProfile::Paper => evaluate_paper_metrics(
            &selected_trades,
            &selected_decisions,
            &selected_runtime,
            &run_summaries,
            &terminal_settlement_by_trade,
        )?,
        KpiProfile::Shadow => evaluate_shadow_metrics(
            &selected_trades,
            &selected_decisions,
            &selected_runtime,
            &run_summaries,
            &terminal_settlement_by_trade,
            missing_run_summary_count,
        )?,
    };

    let mut failing_metrics = Vec::new();
    let mut warning_metrics = Vec::new();
    for (name, metric) in &metrics {
        match metric.status.as_str() {
            "FAIL" => failing_metrics.push(name.clone()),
            "WARN" => warning_metrics.push(name.clone()),
            _ => {}
        }
    }

    let overall_status = if !failing_metrics.is_empty() {
        KpiStatus::Fail
    } else if !sufficient_sample {
        KpiStatus::InsufficientSample
    } else if !warning_metrics.is_empty() {
        KpiStatus::Warn
    } else {
        KpiStatus::Pass
    };

    let mut metadata = BTreeMap::new();
    metadata.insert("bot_id".to_string(), json!(request.bot_id));
    metadata.insert("profile".to_string(), json!(request.profile.as_str()));
    metadata.insert("window_start".to_string(), json!(request.window_start));
    metadata.insert("window_end".to_string(), json!(request.window_end));

    Ok(KpiGateReport {
        run_id: crate::db::new_uuid(),
        metadata,
        sample_coverage,
        source_counts,
        metrics,
        evaluation: KpiGateEvaluation {
            failing_metrics,
            warning_metrics,
            insufficient_sample: !sufficient_sample,
        },
        overall_status: overall_status.as_str().to_string(),
    })
}

fn evaluate_paper_metrics(
    trades: &[KpiTradeRow],
    decisions: &[ParsedDecisionEvent],
    runtime_events: &[ParsedRuntimeEvent],
    run_summaries: &HashMap<String, RunSummaryPayload>,
    terminal_settlement_by_trade: &HashMap<String, String>,
) -> Result<BTreeMap<String, KpiGateMetricReport>> {
    let mut out = BTreeMap::new();
    let participating = participating_summaries(trades, run_summaries);

    let compliant_seed_count = participating
        .iter()
        .filter(|summary| summary.open_both_submit_delta_met)
        .count();
    let deadline_miss_count = participating
        .iter()
        .filter(|summary| !summary.open_both_seed_by_deadline_met)
        .count();
    let worst_seed_delta_ms = participating
        .iter()
        .map(|summary| summary.open_both_first_submit_delta_ms.max(0.0))
        .fold(0.0_f64, f64::max);
    let seed_rate = ratio(compliant_seed_count, participating.len());
    out.insert(
        "seed_timing".to_string(),
        metric_report(
            if participating.is_empty() || seed_rate + EPS >= 0.99 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [
                ("entered_pairs", json!(participating.len())),
                ("compliant_pairs", json!(compliant_seed_count)),
                ("compliance_rate", json!(round6(seed_rate))),
                ("deadline_miss_count", json!(deadline_miss_count)),
                ("worst_seed_delta_ms", json!(round6(worst_seed_delta_ms))),
            ],
        ),
    );

    let no_scale_violations = decisions
        .iter()
        .filter(|event| {
            event.row.approved
                && event
                    .row
                    .owner
                    .as_deref()
                    .or_else(|| event.payload.get("owner").and_then(|value| value.as_str()))
                    == Some("AwaitSecondFill")
                && !decision_is_await_second_fill_rescue(event)
        })
        .count();
    out.insert(
        "no_scale_up_before_both_sides_filled".to_string(),
        metric_report(
            if no_scale_violations == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("violation_count", json!(no_scale_violations))],
        ),
    );

    let unmatched_values = participating
        .iter()
        .map(|summary| summary.unmatched_fraction.max(0.0))
        .collect::<Vec<_>>();
    let median = quantile(&unmatched_values, 0.5);
    let p95 = quantile(&unmatched_values, 0.95);
    let max_value = unmatched_values.iter().copied().fold(0.0_f64, f64::max);
    out.insert(
        "unmatched_fraction".to_string(),
        metric_report(
            if median < 0.07 - EPS && p95 < 0.12 - EPS && max_value < 0.20 - EPS {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [
                ("run_count", json!(unmatched_values.len())),
                ("median", json!(round6(median))),
                ("p95", json!(round6(p95))),
                ("max", json!(round6(max_value))),
            ],
        ),
    );

    let price_violations = decisions
        .iter()
        .filter(|event| event.row.approved && event_effective_pair_cost(event) + EPS >= 1.0)
        .count();
    out.insert(
        "price_discipline".to_string(),
        metric_report(
            if price_violations == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("violation_count", json!(price_violations))],
        ),
    );

    let underdog_violations = decisions
        .iter()
        .filter(|event| {
            event.row.approved
                && event
                    .payload
                    .get("increases_underdog_residual")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
        })
        .count();
    out.insert(
        "underdog_residual".to_string(),
        metric_report(
            if underdog_violations == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("violation_count", json!(underdog_violations))],
        ),
    );

    let taker_stats = fill_share_stats(runtime_events);
    let daily_max = daily_max_taker_share(runtime_events);
    let taker_status = if taker_stats.share + EPS >= 0.10 {
        KpiStatus::Fail
    } else if taker_stats.share + EPS >= 0.05 {
        KpiStatus::Warn
    } else {
        KpiStatus::Pass
    };
    out.insert(
        "taker_share".to_string(),
        metric_report(
            taker_status,
            [
                (
                    "maker_fill_shares",
                    json!(round6(taker_stats.maker_fill_shares)),
                ),
                (
                    "taker_fill_shares",
                    json!(round6(taker_stats.taker_fill_shares)),
                ),
                ("share", json!(round6(taker_stats.share))),
                ("max_daily_share", json!(round6(daily_max))),
            ],
        ),
    );

    let speculative_count = participating
        .iter()
        .filter(|summary| summary.fill_count > 0 && summary.paired_size <= EPS)
        .count();
    out.insert(
        "single_side_speculation".to_string(),
        metric_report(
            if speculative_count == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("violation_count", json!(speculative_count))],
        ),
    );

    let mut settlement_missing = 0usize;
    let mut settlement_mismatch = 0usize;
    let mut unresolved_count = 0usize;
    for trade in trades {
        if !run_summaries
            .get(trade.trade_id.as_str())
            .map(|summary| summary.market_participated)
            .unwrap_or(false)
        {
            continue;
        }
        let terminal = terminal_settlement_by_trade.get(trade.trade_id.as_str());
        if terminal.is_none() {
            settlement_missing += 1;
        }
        match trade.claim_status.as_deref() {
            Some("SETTLED") => {
                if terminal != Some(&"settled".to_string()) {
                    settlement_mismatch += 1;
                }
            }
            _ => {
                unresolved_count += 1;
            }
        }
    }
    out.insert(
        "settlement_reconciliation".to_string(),
        metric_report(
            if settlement_missing == 0 && settlement_mismatch == 0 && unresolved_count == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [
                ("missing_terminal_events", json!(settlement_missing)),
                ("settled_mismatch_count", json!(settlement_mismatch)),
                ("unresolved_count", json!(unresolved_count)),
            ],
        ),
    );

    let decompositions = trades
        .iter()
        .filter(|trade| trade.claim_status.as_deref() == Some("SETTLED"))
        .map(|trade| settlement_pnl_decomposition(trade.lp, trade.q_yes, trade.q_no, trade.cpp))
        .collect::<Vec<_>>();
    let paired_gain = decompositions
        .iter()
        .map(|value| value.paired_realized_pnl)
        .sum::<f64>();
    let residual_total = decompositions
        .iter()
        .map(|value| value.residual_realized_pnl)
        .sum::<f64>();
    let residual_loss = if residual_total < 0.0 {
        residual_total.abs()
    } else {
        0.0
    };
    out.insert(
        "pnl_decomposition".to_string(),
        metric_report(
            if paired_gain > EPS && residual_loss <= paired_gain * 0.5 + EPS {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [
                ("settled_trade_count", json!(decompositions.len())),
                ("paired_realized_pnl", json!(round6(paired_gain))),
                ("residual_realized_pnl", json!(round6(residual_total))),
                ("absolute_residual_loss", json!(round6(residual_loss))),
            ],
        ),
    );

    Ok(out)
}

fn evaluate_shadow_metrics(
    trades: &[KpiTradeRow],
    decisions: &[ParsedDecisionEvent],
    runtime_events: &[ParsedRuntimeEvent],
    run_summaries: &HashMap<String, RunSummaryPayload>,
    terminal_settlement_by_trade: &HashMap<String, String>,
    missing_run_summary_count: usize,
) -> Result<BTreeMap<String, KpiGateMetricReport>> {
    let mut out = BTreeMap::new();
    let events_by_trade = runtime_events_by_trade(runtime_events);

    let mut unrecovered_disconnects = 0usize;
    for (trade_id, events) in &events_by_trade {
        let mut saw_disconnect = false;
        let mut recovered = false;
        for event in events {
            if event.row.event_kind == "risk_block" {
                let reason = event.row.reason_code.as_deref().unwrap_or("");
                if reason.contains("market_ws") || reason.contains("user_ws") {
                    saw_disconnect = true;
                }
            }
            if saw_disconnect
                && event.row.event_kind == "reconciliation"
                && event
                    .payload
                    .get("reconcile_clean")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            {
                recovered = true;
            }
        }
        if saw_disconnect && !recovered && run_summaries.contains_key(trade_id.as_str()) {
            unrecovered_disconnects += 1;
        }
    }
    out.insert(
        "adapter_recovery".to_string(),
        metric_report(
            if unrecovered_disconnects == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [(
                "unrecovered_disconnect_runs",
                json!(unrecovered_disconnects),
            )],
        ),
    );

    let startup_failures = run_summaries
        .values()
        .filter(|summary| summary.effective_order_mode == "shadow")
        .filter(|summary| {
            matches!(
                summary.safety_gate.as_str(),
                "StartupReconPending" | "ReconnectReconPending"
            )
        })
        .count();
    out.insert(
        "startup_reconciliation".to_string(),
        metric_report(
            if startup_failures == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("unresolved_reconciliation_runs", json!(startup_failures))],
        ),
    );

    let audit_drop_count = runtime_events
        .iter()
        .filter(|event| event.row.event_kind == "audit_drop")
        .count();
    let mut count_mismatch = 0usize;
    for trade in trades {
        let Some(summary) = run_summaries.get(trade.trade_id.as_str()) else {
            count_mismatch += 1;
            continue;
        };
        let actual_decisions = decisions
            .iter()
            .filter(|event| event.row.trade_id == trade.trade_id)
            .count() as u32;
        let actual_runtime = runtime_events
            .iter()
            .filter(|event| event.row.trade_id == trade.trade_id)
            .count() as u32;
        if summary.audit_decision_event_count != actual_decisions
            || summary.audit_runtime_event_count != actual_runtime
        {
            count_mismatch += 1;
        }
    }
    out.insert(
        "decision_logging_integrity".to_string(),
        metric_report(
            if audit_drop_count == 0 && count_mismatch == 0 && missing_run_summary_count == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [
                ("audit_drop_count", json!(audit_drop_count)),
                ("count_mismatch_runs", json!(count_mismatch)),
                ("missing_run_summary_runs", json!(missing_run_summary_count)),
            ],
        ),
    );

    let mut deadlock_count = 0usize;
    for summary in participating_summaries(trades, run_summaries) {
        if summary.await_second_fill_hard_paused
            || summary.startup_completion_blocked_count > 0
            || summary.safety_gate != "Healthy"
            || matches!(
                summary.owner.as_str(),
                "OpenBoth" | "PairBuild" | "Taper" | "AwaitSecondFill"
            )
            || (!summary.second_side_by_30s && summary.first_fill_to_second_fill_ms > 30_000.0)
        {
            deadlock_count += 1;
        }
    }
    out.insert(
        "state_machine_progress".to_string(),
        metric_report(
            if deadlock_count == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("deadlock_like_runs", json!(deadlock_count))],
        ),
    );

    let price_or_imbalance_violations = decisions
        .iter()
        .filter(|event| {
            if !event.row.approved {
                return false;
            }
            let price_violation = event_effective_pair_cost(event) + EPS >= 1.0;
            let zone_violation = matches!(
                event
                    .payload
                    .get("price_zone")
                    .and_then(|value| value.as_str()),
                Some("stop_add") | Some("danger")
            );
            let imbalance_violation = event
                .payload
                .get("imbalance_state")
                .and_then(|value| value.as_str())
                == Some("HardDisable");
            price_violation || zone_violation || imbalance_violation
        })
        .count();
    out.insert(
        "hypothetical_price_and_imbalance_compliance".to_string(),
        metric_report(
            if price_or_imbalance_violations == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("violation_count", json!(price_or_imbalance_violations))],
        ),
    );

    let underdog_violations = decisions
        .iter()
        .filter(|event| {
            event.row.approved
                && event
                    .payload
                    .get("increases_underdog_residual")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
        })
        .count();
    out.insert(
        "hypothetical_underdog_residual".to_string(),
        metric_report(
            if underdog_violations == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [("violation_count", json!(underdog_violations))],
        ),
    );

    let mut settlement_missing = 0usize;
    let mut settlement_mismatch = 0usize;
    for trade in trades {
        if !run_summaries
            .get(trade.trade_id.as_str())
            .map(|summary| summary.market_participated)
            .unwrap_or(false)
        {
            continue;
        }
        let terminal = terminal_settlement_by_trade.get(trade.trade_id.as_str());
        if terminal.is_none() {
            settlement_missing += 1;
        }
        if trade.claim_status.as_deref() == Some("SETTLED")
            && terminal != Some(&"settled".to_string())
        {
            settlement_mismatch += 1;
        }
    }
    out.insert(
        "settlement_observation".to_string(),
        metric_report(
            if settlement_missing == 0 && settlement_mismatch == 0 {
                KpiStatus::Pass
            } else {
                KpiStatus::Fail
            },
            [
                ("missing_terminal_events", json!(settlement_missing)),
                ("settled_mismatch_count", json!(settlement_mismatch)),
            ],
        ),
    );

    Ok(out)
}

fn participating_summaries<'a>(
    trades: &'a [KpiTradeRow],
    run_summaries: &'a HashMap<String, RunSummaryPayload>,
) -> Vec<&'a RunSummaryPayload> {
    trades
        .iter()
        .filter_map(|trade| run_summaries.get(trade.trade_id.as_str()))
        .filter(|summary| summary.market_participated)
        .collect()
}

fn runtime_events_by_trade<'a>(
    runtime_events: &'a [ParsedRuntimeEvent],
) -> HashMap<String, Vec<&'a ParsedRuntimeEvent>> {
    let mut out = HashMap::<String, Vec<&ParsedRuntimeEvent>>::new();
    for event in runtime_events {
        out.entry(event.row.trade_id.clone())
            .or_default()
            .push(event);
    }
    for events in out.values_mut() {
        events.sort_by(|left, right| {
            compare_missing_summary_sort_keys(
                &missing_summary_sort_key_for_runtime_event(left),
                &missing_summary_sort_key_for_runtime_event(right),
            )
        });
    }
    out
}

#[derive(Default)]
struct MissingSummaryModeEvidence {
    effective_modes: BTreeSet<String>,
    configured_modes: BTreeSet<String>,
    last_effective_mode: Option<String>,
    last_effective_sort_key: Option<MissingSummarySortKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MissingSummarySortKey {
    event_ts: String,
    t_into_micros: Option<i64>,
    event_rank: u8,
    stable_tiebreak: String,
}

fn normalize_mode(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn missing_summary_t_into_micros(payload: &Value) -> Option<i64> {
    let value = payload
        .get("t_into_seconds")
        .and_then(|field| field.as_f64())?;
    if !value.is_finite() {
        return None;
    }
    Some((value.max(0.0) * 1_000_000.0).round() as i64)
}

fn missing_summary_runtime_event_rank(event_kind: &str) -> u8 {
    match event_kind {
        "state_transition" => 0,
        "fill" | "order_intent" | "order_ack" => 2,
        "risk_block" => 3,
        "reconciliation" => 4,
        "settlement" => 5,
        "audit_drop" => 6,
        "run_summary" => 7,
        _ => 2,
    }
}

fn compare_optional_t_into_micros(left: Option<i64>, right: Option<i64>) -> Ordering {
    match (left, right) {
        (Some(left_value), Some(right_value)) => left_value.cmp(&right_value),
        _ => Ordering::Equal,
    }
}

fn compare_missing_summary_sort_keys(
    left: &MissingSummarySortKey,
    right: &MissingSummarySortKey,
) -> Ordering {
    left.event_ts
        .cmp(&right.event_ts)
        .then_with(|| compare_optional_t_into_micros(left.t_into_micros, right.t_into_micros))
        .then(left.event_rank.cmp(&right.event_rank))
        .then(left.stable_tiebreak.cmp(&right.stable_tiebreak))
}

fn missing_summary_sort_key_for_runtime_event(event: &ParsedRuntimeEvent) -> MissingSummarySortKey {
    MissingSummarySortKey {
        event_ts: event.row.event_ts.clone(),
        t_into_micros: missing_summary_t_into_micros(&event.payload),
        event_rank: missing_summary_runtime_event_rank(event.row.event_kind.as_str()),
        stable_tiebreak: format!(
            "{}|{}|{}",
            event.row.event_kind,
            event.row.reason_code.as_deref().unwrap_or(""),
            event.row.payload_json
        ),
    }
}

fn missing_summary_sort_key_for_decision(decision: &ParsedDecisionEvent) -> MissingSummarySortKey {
    MissingSummarySortKey {
        event_ts: decision.row.decision_ts.clone(),
        t_into_micros: missing_summary_t_into_micros(&decision.payload),
        event_rank: 1,
        stable_tiebreak: format!(
            "{}|{}|{}|{}|{}",
            decision.row.reason_code,
            decision.row.owner.as_deref().unwrap_or(""),
            decision.row.submit_origin.as_deref().unwrap_or(""),
            decision.row.submit_side.as_deref().unwrap_or(""),
            decision.row.payload_json
        ),
    }
}

fn collect_missing_summary_mode_evidence(
    evidence: &mut HashMap<String, MissingSummaryModeEvidence>,
    trade_id: &str,
    payload: &Value,
    sort_key: MissingSummarySortKey,
) {
    let entry = evidence.entry(trade_id.to_string()).or_default();
    if let Some(value) = json_str(payload, "effective_order_mode") {
        let normalized = normalize_mode(value);
        entry.effective_modes.insert(normalized.clone());
        if entry
            .last_effective_sort_key
            .as_ref()
            .map(|existing| {
                compare_missing_summary_sort_keys(&sort_key, existing) != Ordering::Less
            })
            .unwrap_or(true)
        {
            entry.last_effective_mode = Some(normalized);
            entry.last_effective_sort_key = Some(sort_key.clone());
        }
    }
    if let Some(value) = json_str(payload, "configured_order_mode") {
        entry.configured_modes.insert(normalize_mode(value));
    }
}

fn stable_profile_signal_trade_ids(
    runtime_events: &[ParsedRuntimeEvent],
    decisions: &[ParsedDecisionEvent],
    profile: KpiProfile,
) -> BTreeSet<String> {
    let mut evidence = HashMap::<String, MissingSummaryModeEvidence>::new();
    for event in runtime_events {
        collect_missing_summary_mode_evidence(
            &mut evidence,
            &event.row.trade_id,
            &event.payload,
            missing_summary_sort_key_for_runtime_event(event),
        );
    }
    for decision in decisions {
        collect_missing_summary_mode_evidence(
            &mut evidence,
            &decision.row.trade_id,
            &decision.payload,
            missing_summary_sort_key_for_decision(decision),
        );
    }

    let requested = normalize_mode(profile.as_str());
    evidence
        .into_iter()
        .filter_map(|(trade_id, evidence)| {
            if let Some(last_effective_mode) = evidence.last_effective_mode.as_deref() {
                if last_effective_mode == requested {
                    Some(trade_id)
                } else {
                    None
                }
            } else if evidence.configured_modes.len() == 1
                && evidence.configured_modes.contains(requested.as_str())
            {
                Some(trade_id)
            } else {
                None
            }
        })
        .collect()
}

struct FillShareStats {
    maker_fill_shares: f64,
    taker_fill_shares: f64,
    share: f64,
}

fn fill_share_stats(runtime_events: &[ParsedRuntimeEvent]) -> FillShareStats {
    let mut maker_fill_shares = 0.0;
    let mut taker_fill_shares = 0.0;
    for event in runtime_events
        .iter()
        .filter(|event| event.row.event_kind == "fill")
    {
        let filled = event
            .payload
            .get("filled")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0)
            .max(0.0);
        if event
            .payload
            .get("is_maker")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            maker_fill_shares += filled;
        } else {
            taker_fill_shares += filled;
        }
    }
    let share = if maker_fill_shares + taker_fill_shares > EPS {
        taker_fill_shares / (maker_fill_shares + taker_fill_shares)
    } else {
        0.0
    };
    FillShareStats {
        maker_fill_shares,
        taker_fill_shares,
        share,
    }
}

fn daily_max_taker_share(runtime_events: &[ParsedRuntimeEvent]) -> f64 {
    let mut shares = HashMap::<String, (f64, f64)>::new();
    for event in runtime_events
        .iter()
        .filter(|event| event.row.event_kind == "fill")
    {
        let filled = event
            .payload
            .get("filled")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0)
            .max(0.0);
        let day = parse_iso(event.row.event_ts.as_str())
            .ok()
            .map(|value| value.with_timezone(&Utc).date_naive().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let entry = shares.entry(day).or_insert((0.0, 0.0));
        if event
            .payload
            .get("is_maker")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            entry.0 += filled;
        } else {
            entry.1 += filled;
        }
    }
    shares
        .into_values()
        .map(|(maker, taker)| {
            if maker + taker > EPS {
                taker / (maker + taker)
            } else {
                0.0
            }
        })
        .fold(0.0_f64, f64::max)
}

fn decision_is_await_second_fill_rescue(event: &ParsedDecisionEvent) -> bool {
    matches!(
        event
            .payload
            .get("one_side_exception_kind")
            .and_then(|value| value.as_str()),
        Some("AwaitSecondFillRescue")
    ) || event
        .row
        .submit_origin
        .as_deref()
        .map(|value| value.contains("AWAIT_SECOND_FILL_RESCUE"))
        .unwrap_or(false)
        || event.reason_code_contains("AWAIT_SECOND_FILL_RESCUE")
}

impl ParsedDecisionEvent {
    fn reason_code_contains(&self, needle: &str) -> bool {
        self.row.reason_code.contains(needle)
            || self
                .payload
                .get("reason_code")
                .and_then(|value| value.as_str())
                .map(|value| value.contains(needle))
                .unwrap_or(false)
    }
}

fn event_effective_pair_cost(event: &ParsedDecisionEvent) -> f64 {
    event
        .payload
        .get("effective_marginal_pair_cost")
        .and_then(|value| value.as_f64())
        .unwrap_or(f64::NAN)
}

fn parse_run_summary(event: &ParsedRuntimeEvent) -> Result<RunSummaryPayload> {
    Ok(RunSummaryPayload {
        trade_id: event.row.trade_id.clone(),
        event_ts: event.row.event_ts.clone(),
        phase: json_str(&event.payload, "phase")
            .unwrap_or_default()
            .to_string(),
        owner: json_str(&event.payload, "owner")
            .unwrap_or_default()
            .to_string(),
        safety_gate: json_str(&event.payload, "safety_gate")
            .unwrap_or_default()
            .to_string(),
        safety_gate_reason: json_str(&event.payload, "safety_gate_reason")
            .unwrap_or_default()
            .to_string(),
        configured_order_mode: json_str(&event.payload, "configured_order_mode")
            .unwrap_or_default()
            .to_string(),
        effective_order_mode: json_str(&event.payload, "effective_order_mode")
            .unwrap_or_default()
            .to_string(),
        live_order_mode_block_reason: json_str(&event.payload, "live_order_mode_block_reason")
            .map(|value| value.to_string()),
        fill_count: json_u64(&event.payload, "fill_count") as usize,
        market_participated: json_bool(&event.payload, "market_participated"),
        entry_reason: json_str(&event.payload, "entry_reason").map(|value| value.to_string()),
        exit_reason: json_str(&event.payload, "exit_reason")
            .unwrap_or_default()
            .to_string(),
        settlement_status: json_str(&event.payload, "settlement_status")
            .unwrap_or_default()
            .to_string(),
        settlement_reason: json_str(&event.payload, "settlement_reason")
            .unwrap_or_default()
            .to_string(),
        q_yes: json_f64(&event.payload, "q_yes"),
        q_no: json_f64(&event.payload, "q_no"),
        total_cost: json_f64(&event.payload, "total_cost"),
        cpp: json_f64(&event.payload, "cpp"),
        paired_size: json_f64(&event.payload, "paired_size"),
        unmatched_size: json_f64(&event.payload, "unmatched_size"),
        unmatched_fraction: json_f64(&event.payload, "unmatched_fraction"),
        pair_taker_share: json_f64(&event.payload, "pair_taker_share"),
        daily_taker_share: json_f64(&event.payload, "daily_taker_share"),
        open_both_seed_by_deadline_met: json_bool(&event.payload, "open_both_seed_by_deadline_met"),
        open_both_submit_delta_met: json_bool(&event.payload, "open_both_submit_delta_met"),
        open_both_first_submit_delta_ms: json_f64(
            &event.payload,
            "open_both_first_submit_delta_ms",
        ),
        second_side_by_15s: json_bool(&event.payload, "second_side_by_15s"),
        second_side_by_30s: json_bool(&event.payload, "second_side_by_30s"),
        first_fill_to_second_fill_ms: json_f64(&event.payload, "first_fill_to_second_fill_ms"),
        await_second_fill_hard_paused: json_bool(&event.payload, "await_second_fill_hard_paused"),
        startup_completion_blocked_count: json_u64(
            &event.payload,
            "startup_completion_blocked_count",
        ) as u32,
        audit_decision_event_count: json_u64(&event.payload, "audit_decision_event_count") as u32,
        audit_runtime_event_count: json_u64(&event.payload, "audit_runtime_event_count") as u32,
    })
}

fn write_summary(
    report: &KpiGateReport,
    output_dir: &Path,
    request: &KpiRunRequest,
) -> Result<PathBuf> {
    let window_dir = format!(
        "{}_{}",
        sanitize_path_segment(request.window_start.as_str()),
        sanitize_path_segment(request.window_end.as_str())
    );
    let dir = output_dir
        .join(request.bot_id.as_str())
        .join(request.profile.as_str())
        .join(window_dir);
    fs::create_dir_all(&dir).with_context(|| format!("failed creating {}", dir.display()))?;
    let path = dir.join("summary.json");
    fs::write(&path, serde_json::to_vec_pretty(report)?)
        .with_context(|| format!("failed writing {}", path.display()))?;
    Ok(path)
}

fn parse_iso(raw: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(raw).map_err(|err| anyhow!(err))
}

fn sanitize_path_segment(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

fn round6(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn quantile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut values = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize;
    values[idx]
}

fn metric_report<const N: usize>(
    status: KpiStatus,
    fields: [(&str, Value); N],
) -> KpiGateMetricReport {
    let mut details = BTreeMap::new();
    for (key, value) in fields {
        details.insert(key.to_string(), value);
    }
    KpiGateMetricReport {
        status: status.as_str().to_string(),
        details,
    }
}

fn json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(|field| field.as_str())
}

fn json_f64(value: &Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(|field| field.as_f64())
        .unwrap_or(0.0)
}

fn json_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(|field| field.as_u64()).unwrap_or(0)
}

fn json_bool(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|field| field.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct InMemorySource {
        trades: Vec<KpiTradeRow>,
        decisions: Vec<KpiDecisionEventRow>,
        runtime_events: Vec<KpiRuntimeEventRow>,
    }

    impl KpiEventSource for InMemorySource {
        fn load_trades(
            &self,
            _bot_id: &str,
            _window_start: &str,
            _window_end: &str,
        ) -> Result<Vec<KpiTradeRow>> {
            Ok(self.trades.clone())
        }

        fn load_decision_events(
            &self,
            _bot_id: &str,
            _window_start: &str,
            _window_end: &str,
        ) -> Result<Vec<KpiDecisionEventRow>> {
            Ok(self.decisions.clone())
        }

        fn load_runtime_events(
            &self,
            _bot_id: &str,
            _window_start: &str,
            _window_end: &str,
        ) -> Result<Vec<KpiRuntimeEventRow>> {
            Ok(self.runtime_events.clone())
        }
    }

    fn make_trade(
        trade_id: &str,
        date: &str,
        claim_status: &str,
        lp: f64,
        q_yes: f64,
        q_no: f64,
        cpp: f64,
    ) -> KpiTradeRow {
        KpiTradeRow {
            trade_id: trade_id.to_string(),
            bot_id: "bot".to_string(),
            pair_id: trade_id.to_string(),
            market_slug: trade_id.to_string(),
            date: date.to_string(),
            start_trade: format!("{date}T09:00:00+07:00"),
            end_trade: format!("{date}T09:05:00+07:00"),
            entry_reason: Some("BOT_OPEN_BOTH".to_string()),
            exit_reason: "DONE".to_string(),
            lp,
            total_cost: q_yes.min(q_no) * cpp * 2.0,
            q_yes,
            q_no,
            cpp,
            status: Some("WON".to_string()),
            claim_status: Some(claim_status.to_string()),
            meta_data: None,
        }
    }

    fn make_run_summary(
        trade_id: &str,
        profile: KpiProfile,
        date: &str,
        extras: Value,
    ) -> KpiRuntimeEventRow {
        let mut payload = json!({
            "trade_id": trade_id,
            "phase": "Open",
            "owner": "AwaitSettlement",
            "safety_gate": "Healthy",
            "safety_gate_reason": "",
            "configured_order_mode": profile.as_str(),
            "effective_order_mode": profile.as_str(),
            "live_order_mode_block_reason": null,
            "fill_count": 2,
            "market_participated": true,
            "entry_reason": "BOT_OPEN_BOTH",
            "exit_reason": "DONE",
            "settlement_status": "SETTLED",
            "settlement_reason": "settled",
            "q_yes": 10.0,
            "q_no": 10.0,
            "total_cost": 8.0,
            "cpp": 0.4,
            "paired_size": 10.0,
            "unmatched_size": 0.0,
            "unmatched_fraction": 0.0,
            "pair_taker_share": 0.0,
            "daily_taker_share": 0.0,
            "open_both_seed_by_deadline_met": true,
            "open_both_submit_delta_met": true,
            "open_both_first_submit_delta_ms": 250.0,
            "second_side_by_15s": true,
            "second_side_by_30s": true,
            "first_fill_to_second_fill_ms": 5000.0,
            "await_second_fill_hard_paused": false,
            "startup_completion_blocked_count": 0,
            "audit_decision_event_count": 0,
            "audit_runtime_event_count": 2
        });
        if let Some(map) = payload.as_object_mut() {
            if let Some(extra_map) = extras.as_object() {
                for (key, value) in extra_map {
                    map.insert(key.clone(), value.clone());
                }
            }
        }
        KpiRuntimeEventRow {
            event_id: format!("run-summary-{trade_id}"),
            trade_id: trade_id.to_string(),
            event_kind: "run_summary".to_string(),
            event_ts: format!("{date}T09:05:01+07:00"),
            reason_code: Some("completed".to_string()),
            payload_json: serde_json::to_string(&payload).expect("payload"),
        }
    }

    fn make_settlement(trade_id: &str, date: &str, reason_code: &str) -> KpiRuntimeEventRow {
        KpiRuntimeEventRow {
            event_id: format!("settlement-{trade_id}-{reason_code}"),
            trade_id: trade_id.to_string(),
            event_kind: "settlement".to_string(),
            event_ts: format!("{date}T09:05:00+07:00"),
            reason_code: Some(reason_code.to_string()),
            payload_json: serde_json::to_string(&json!({
                "trade_id": trade_id,
                "reason_code": reason_code
            }))
            .expect("payload"),
        }
    }

    fn make_fill(trade_id: &str, date: &str, is_maker: bool, filled: f64) -> KpiRuntimeEventRow {
        KpiRuntimeEventRow {
            event_id: format!("fill-{trade_id}-{is_maker}-{filled}"),
            trade_id: trade_id.to_string(),
            event_kind: "fill".to_string(),
            event_ts: format!("{date}T09:02:00+07:00"),
            reason_code: Some("fill".to_string()),
            payload_json: serde_json::to_string(&json!({
                "is_maker": is_maker,
                "filled": filled
            }))
            .expect("payload"),
        }
    }

    fn make_decision(
        trade_id: &str,
        approved: bool,
        owner: &str,
        effective_pair_cost: f64,
        price_zone: &str,
        imbalance_state: &str,
        increases_underdog_residual: bool,
        one_side_exception_kind: Option<&str>,
    ) -> KpiDecisionEventRow {
        KpiDecisionEventRow {
            decision_event_id: format!("decision-{trade_id}-{owner}"),
            trade_id: trade_id.to_string(),
            decision_ts: "2026-03-22T09:00:00+07:00".to_string(),
            approved,
            reason_code: one_side_exception_kind.unwrap_or("decision").to_string(),
            phase: Some("Open".to_string()),
            owner: Some(owner.to_string()),
            submit_origin: one_side_exception_kind.map(|value| value.to_string()),
            submit_side: Some("buy".to_string()),
            payload_json: serde_json::to_string(&json!({
                "owner": owner,
                "approved": approved,
                "effective_marginal_pair_cost": effective_pair_cost,
                "price_zone": price_zone,
                "imbalance_state": imbalance_state,
                "increases_underdog_residual": increases_underdog_residual,
                "one_side_exception_kind": one_side_exception_kind
            }))
            .expect("payload"),
        }
    }

    #[test]
    fn settlement_pnl_decomposition_splits_paired_and_residual() {
        let value = settlement_pnl_decomposition(1.0, 12.0, 7.0, 0.4);
        assert!((value.paired_qty - 7.0).abs() < 1e-9);
        assert!((value.residual_qty - 5.0).abs() < 1e-9);
        assert!((value.paired_cost - 5.6).abs() < 1e-9);
        assert!((value.paired_realized_pnl - 1.4).abs() < 1e-9);
        assert!((value.residual_realized_pnl + 0.4).abs() < 1e-9);
    }

    #[test]
    fn quantile_uses_sorted_rank() {
        let values = vec![0.2, 0.0, 0.5, 0.1, 0.4];
        assert!((quantile(&values, 0.5) - 0.2).abs() < 1e-9);
        assert!((quantile(&values, 0.95) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn kpi_gate_reports_insufficient_shadow_sample() -> Result<()> {
        let source = InMemorySource {
            trades: vec![
                make_trade("t1", "2026-03-20", "SETTLED", 1.0, 10.0, 10.0, 0.4),
                make_trade("t2", "2026-03-21", "SETTLED", 1.0, 10.0, 10.0, 0.4),
            ],
            decisions: vec![],
            runtime_events: vec![
                make_run_summary("t1", KpiProfile::Shadow, "2026-03-20", json!({})),
                make_run_summary("t2", KpiProfile::Shadow, "2026-03-21", json!({})),
                make_settlement("t1", "2026-03-20", "settled"),
                make_settlement("t2", "2026-03-21", "settled"),
            ],
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-insufficient-shadow");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.overall_status, "INSUFFICIENT_SAMPLE");
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn paper_gate_fails_price_violations() -> Result<()> {
        let trade = make_trade("t1", "2026-03-20", "SETTLED", 1.0, 10.0, 10.0, 0.4);
        let source = InMemorySource {
            trades: std::iter::repeat_with(|| trade.clone()).take(500).collect(),
            decisions: vec![make_decision(
                "t1",
                true,
                "PairBuild",
                1.0,
                "stop_add",
                "Normal",
                false,
                None,
            )],
            runtime_events: vec![
                make_run_summary(
                    "t1",
                    KpiProfile::Paper,
                    "2026-03-20",
                    json!({
                        "audit_decision_event_count": 1,
                        "audit_runtime_event_count": 3
                    }),
                ),
                make_settlement("t1", "2026-03-20", "settled"),
                make_fill("t1", "2026-03-20", true, 10.0),
            ],
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Paper,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-30T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-paper-price");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(
            report.metrics["price_discipline"].status,
            KpiStatus::Fail.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_fails_on_audit_drop() -> Result<()> {
        let trade = make_trade("t1", "2026-03-20", "SETTLED", 1.0, 10.0, 10.0, 0.4);
        let source = InMemorySource {
            trades: vec![
                trade.clone(),
                make_trade("t2", "2026-03-21", "SETTLED", 1.0, 10.0, 10.0, 0.4),
                make_trade("t3", "2026-03-22", "SETTLED", 1.0, 10.0, 10.0, 0.4),
            ],
            decisions: vec![],
            runtime_events: vec![
                make_run_summary("t1", KpiProfile::Shadow, "2026-03-20", json!({})),
                make_run_summary("t2", KpiProfile::Shadow, "2026-03-21", json!({})),
                make_run_summary("t3", KpiProfile::Shadow, "2026-03-22", json!({})),
                make_settlement("t1", "2026-03-20", "settled"),
                make_settlement("t2", "2026-03-21", "settled"),
                make_settlement("t3", "2026-03-22", "settled"),
                KpiRuntimeEventRow {
                    event_id: "audit-drop".to_string(),
                    trade_id: "t1".to_string(),
                    event_kind: "audit_drop".to_string(),
                    event_ts: "2026-03-20T09:05:02+07:00".to_string(),
                    reason_code: Some("runtime_insert_failed".to_string()),
                    payload_json: "{}".to_string(),
                },
            ],
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-23T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-shadow-audit-drop");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Fail.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn paper_gate_reports_insufficient_sample_until_day_and_pair_thresholds_are_met() -> Result<()>
    {
        let mut trades = Vec::new();
        let mut runtime_events = Vec::new();
        for day in 0..10 {
            let trade_id = format!("paper-{day}");
            let date = format!("2026-03-{:02}", 20 + day);
            trades.push(make_trade(
                trade_id.as_str(),
                date.as_str(),
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ));
            runtime_events.push(make_run_summary(
                trade_id.as_str(),
                KpiProfile::Paper,
                date.as_str(),
                json!({}),
            ));
            runtime_events.push(make_settlement(trade_id.as_str(), date.as_str(), "settled"));
        }
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Paper,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-31T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-paper-insufficient");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.sample_coverage.distinct_trading_days, 10);
        assert_eq!(report.sample_coverage.settled_pairs, 10);
        assert_eq!(
            report.overall_status,
            KpiStatus::InsufficientSample.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn paper_gate_does_not_count_missing_run_summary_rows_toward_sample_thresholds() -> Result<()> {
        let mut trades = Vec::new();
        let mut runtime_events = Vec::new();
        for idx in 0..500 {
            let trade_id = format!("paper-summary-gap-{idx}");
            let date = format!("2026-03-{:02}", 20 + (idx % 10));
            trades.push(make_trade(
                trade_id.as_str(),
                date.as_str(),
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ));
            runtime_events.push(make_settlement(trade_id.as_str(), date.as_str(), "settled"));
            if idx < 10 {
                runtime_events.push(make_run_summary(
                    trade_id.as_str(),
                    KpiProfile::Paper,
                    date.as_str(),
                    json!({}),
                ));
            } else {
                runtime_events.push(KpiRuntimeEventRow {
                    event_id: format!("fill-paper-summary-gap-{idx}"),
                    trade_id: trade_id.clone(),
                    event_kind: "fill".to_string(),
                    event_ts: format!("{date}T09:02:00+07:00"),
                    reason_code: Some("fill".to_string()),
                    payload_json: serde_json::to_string(&json!({
                        "filled": 1.0,
                        "is_maker": true,
                        "configured_order_mode": "paper",
                        "effective_order_mode": "paper"
                    }))
                    .expect("payload"),
                });
            }
        }
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Paper,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-31T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-paper-summary-gap");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 10);
        assert_eq!(report.source_counts.missing_run_summary_count, 490);
        assert_eq!(report.sample_coverage.distinct_trading_days, 10);
        assert_eq!(report.sample_coverage.settled_pairs, 10);
        assert!(!report.sample_coverage.sufficient_sample);
        assert_eq!(
            report.overall_status,
            KpiStatus::InsufficientSample.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn paper_gate_fails_unmatched_fraction_distribution_when_sample_is_sufficient() -> Result<()> {
        let mut trades = Vec::new();
        let mut runtime_events = Vec::new();
        for idx in 0..500 {
            let trade_id = format!("paper-unmatched-{idx}");
            let date = format!("2026-03-{:02}", 20 + (idx % 10));
            trades.push(make_trade(
                trade_id.as_str(),
                date.as_str(),
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ));
            runtime_events.push(make_run_summary(
                trade_id.as_str(),
                KpiProfile::Paper,
                date.as_str(),
                json!({
                    "unmatched_fraction": 0.08,
                    "audit_runtime_event_count": 2
                }),
            ));
            runtime_events.push(make_settlement(trade_id.as_str(), date.as_str(), "settled"));
        }
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Paper,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-31T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-paper-unmatched");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert!(report.sample_coverage.sufficient_sample);
        assert_eq!(
            report.metrics["unmatched_fraction"].status,
            KpiStatus::Fail.as_str()
        );
        assert_eq!(report.overall_status, KpiStatus::Fail.as_str());
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_passes_adapter_recovery_after_reconciliation() -> Result<()> {
        let trades = vec![
            make_trade("shadow-1", "2026-03-20", "SETTLED", 1.0, 10.0, 10.0, 0.4),
            make_trade("shadow-2", "2026-03-21", "SETTLED", 1.0, 10.0, 10.0, 0.4),
            make_trade("shadow-3", "2026-03-22", "SETTLED", 1.0, 10.0, 10.0, 0.4),
        ];
        let runtime_events = vec![
            make_run_summary("shadow-1", KpiProfile::Shadow, "2026-03-20", json!({})),
            make_run_summary("shadow-2", KpiProfile::Shadow, "2026-03-21", json!({})),
            make_run_summary("shadow-3", KpiProfile::Shadow, "2026-03-22", json!({})),
            make_settlement("shadow-1", "2026-03-20", "settled"),
            make_settlement("shadow-2", "2026-03-21", "settled"),
            make_settlement("shadow-3", "2026-03-22", "settled"),
            KpiRuntimeEventRow {
                event_id: "risk-block-1".to_string(),
                trade_id: "shadow-1".to_string(),
                event_kind: "risk_block".to_string(),
                event_ts: "2026-03-20T09:03:00+07:00".to_string(),
                reason_code: Some("dependency_pause:market_ws".to_string()),
                payload_json: "{}".to_string(),
            },
            KpiRuntimeEventRow {
                event_id: "reconciliation-1".to_string(),
                trade_id: "shadow-1".to_string(),
                event_kind: "reconciliation".to_string(),
                event_ts: "2026-03-20T09:04:00+07:00".to_string(),
                reason_code: Some("reconnect".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "reconcile_clean": true
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-23T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-shadow-recovery");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(
            report.metrics["adapter_recovery"].status,
            KpiStatus::Pass.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_same_second_reconciliation_after_disconnect_uses_t_into_not_event_id(
    ) -> Result<()> {
        let trades = vec![
            make_trade("shadow-1", "2026-03-20", "SETTLED", 1.0, 10.0, 10.0, 0.4),
            make_trade("shadow-2", "2026-03-21", "SETTLED", 1.0, 10.0, 10.0, 0.4),
            make_trade("shadow-3", "2026-03-22", "SETTLED", 1.0, 10.0, 10.0, 0.4),
        ];
        let runtime_events = vec![
            make_run_summary("shadow-1", KpiProfile::Shadow, "2026-03-20", json!({})),
            make_run_summary("shadow-2", KpiProfile::Shadow, "2026-03-21", json!({})),
            make_run_summary("shadow-3", KpiProfile::Shadow, "2026-03-22", json!({})),
            make_settlement("shadow-1", "2026-03-20", "settled"),
            make_settlement("shadow-2", "2026-03-21", "settled"),
            make_settlement("shadow-3", "2026-03-22", "settled"),
            KpiRuntimeEventRow {
                event_id: "zzzz-risk-block".to_string(),
                trade_id: "shadow-1".to_string(),
                event_kind: "risk_block".to_string(),
                event_ts: "2026-03-20T09:03:00+07:00".to_string(),
                reason_code: Some("dependency_pause:market_ws".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "shadow",
                    "effective_order_mode": "shadow",
                    "t_into_seconds": 100.10
                }))
                .expect("payload"),
            },
            KpiRuntimeEventRow {
                event_id: "aaaa-reconciliation".to_string(),
                trade_id: "shadow-1".to_string(),
                event_kind: "reconciliation".to_string(),
                event_ts: "2026-03-20T09:03:00+07:00".to_string(),
                reason_code: Some("reconnect_clean".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "shadow",
                    "effective_order_mode": "shadow",
                    "reconcile_clean": true,
                    "t_into_seconds": 100.60
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-23T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir =
            std::env::temp_dir().join("polybot-kpi-gate-shadow-recovery-same-second-order");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(
            report.metrics["adapter_recovery"].status,
            KpiStatus::Pass.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_fails_state_machine_progress_on_deadlock() -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-deadlock-1",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "shadow-deadlock-2",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "shadow-deadlock-3",
                "2026-03-22",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let runtime_events = vec![
            make_run_summary(
                "shadow-deadlock-1",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({
                    "owner": "AwaitSecondFill",
                    "await_second_fill_hard_paused": true,
                    "second_side_by_30s": false,
                    "first_fill_to_second_fill_ms": 45000.0
                }),
            ),
            make_run_summary(
                "shadow-deadlock-2",
                KpiProfile::Shadow,
                "2026-03-21",
                json!({}),
            ),
            make_run_summary(
                "shadow-deadlock-3",
                KpiProfile::Shadow,
                "2026-03-22",
                json!({}),
            ),
            make_settlement("shadow-deadlock-1", "2026-03-20", "settled"),
            make_settlement("shadow-deadlock-2", "2026-03-21", "settled"),
            make_settlement("shadow-deadlock-3", "2026-03-22", "settled"),
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-23T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-shadow-deadlock");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(
            report.metrics["state_machine_progress"].status,
            KpiStatus::Fail.as_str()
        );
        assert_eq!(report.overall_status, KpiStatus::Fail.as_str());
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_ignores_nonterminal_shadow_events_when_terminal_mode_is_live() -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "live-shadow-signal-1",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "live-shadow-signal-2",
                "2026-03-22",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_run_summary(
                "live-shadow-signal-1",
                KpiProfile::Shadow,
                "2026-03-21",
                json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "live"
                }),
            ),
            make_run_summary(
                "live-shadow-signal-2",
                KpiProfile::Shadow,
                "2026-03-22",
                json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "live"
                }),
            ),
            make_settlement("live-shadow-signal-1", "2026-03-21", "settled"),
            make_settlement("live-shadow-signal-2", "2026-03-22", "settled"),
            KpiRuntimeEventRow {
                event_id: "fill-live-shadow-signal-1".to_string(),
                trade_id: "live-shadow-signal-1".to_string(),
                event_kind: "fill".to_string(),
                event_ts: "2026-03-21T09:02:00+07:00".to_string(),
                reason_code: Some("fill".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "filled": 1.0,
                    "is_maker": true,
                    "effective_order_mode": "shadow"
                }))
                .expect("payload"),
            },
            KpiRuntimeEventRow {
                event_id: "fill-live-shadow-signal-2".to_string(),
                trade_id: "live-shadow-signal-2".to_string(),
                event_kind: "fill".to_string(),
                event_ts: "2026-03-22T09:02:00+07:00".to_string(),
                reason_code: Some("fill".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "filled": 1.0,
                    "is_maker": true,
                    "effective_order_mode": "shadow"
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-23T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-shadow-terminal-mode");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 0);
        assert_eq!(report.sample_coverage.distinct_trading_days, 1);
        assert!(!report.sample_coverage.sufficient_sample);
        assert_eq!(
            report.overall_status,
            KpiStatus::InsufficientSample.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_does_not_attribute_missing_summary_from_configured_live_shadow_events(
    ) -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "live-missing-summary",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_settlement("live-missing-summary", "2026-03-21", "settled"),
            KpiRuntimeEventRow {
                event_id: "fill-live-missing-summary".to_string(),
                trade_id: "live-missing-summary".to_string(),
                event_kind: "fill".to_string(),
                event_ts: "2026-03-21T09:02:00+07:00".to_string(),
                reason_code: Some("fill".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "filled": 1.0,
                    "is_maker": true,
                    "configured_order_mode": "live",
                    "effective_order_mode": "shadow"
                }))
                .expect("payload"),
            },
            KpiRuntimeEventRow {
                event_id: "risk-block-live-missing-summary".to_string(),
                trade_id: "live-missing-summary".to_string(),
                event_kind: "risk_block".to_string(),
                event_ts: "2026-03-21T09:04:00+07:00".to_string(),
                reason_code: Some("hold".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "live",
                    "safety_gate": "Healthy"
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir =
            std::env::temp_dir().join("polybot-kpi-gate-shadow-configured-live-missing-summary");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 0);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Pass.as_str()
        );
        assert_eq!(
            report.overall_status,
            KpiStatus::InsufficientSample.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_counts_missing_summary_for_configured_live_run_that_stayed_shadow() -> Result<()>
    {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "live-never-armed-shadow",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_settlement("live-never-armed-shadow", "2026-03-21", "settled"),
            KpiRuntimeEventRow {
                event_id: "audit-drop-live-never-armed-shadow".to_string(),
                trade_id: "live-never-armed-shadow".to_string(),
                event_kind: "audit_drop".to_string(),
                event_ts: "2026-03-21T09:05:02+07:00".to_string(),
                reason_code: Some("runtime_direct_insert_failed".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "shadow",
                    "live_order_mode_block_reason": "await_live_arm"
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir =
            std::env::temp_dir().join("polybot-kpi-gate-shadow-live-never-armed-summary-gap");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 1);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Fail.as_str()
        );
        assert_eq!(report.overall_status, KpiStatus::Fail.as_str());
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_counts_missing_summary_for_mixed_mode_run_that_last_observed_shadow(
    ) -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "live-then-shadow-missing-summary",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_settlement("live-then-shadow-missing-summary", "2026-03-21", "settled"),
            KpiRuntimeEventRow {
                event_id: "fill-live-then-shadow-1".to_string(),
                trade_id: "live-then-shadow-missing-summary".to_string(),
                event_kind: "fill".to_string(),
                event_ts: "2026-03-21T09:02:00+07:00".to_string(),
                reason_code: Some("fill".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "filled": 1.0,
                    "is_maker": true,
                    "configured_order_mode": "live",
                    "effective_order_mode": "live"
                }))
                .expect("payload"),
            },
            KpiRuntimeEventRow {
                event_id: "risk-block-live-then-shadow-2".to_string(),
                trade_id: "live-then-shadow-missing-summary".to_string(),
                event_kind: "risk_block".to_string(),
                event_ts: "2026-03-21T09:04:00+07:00".to_string(),
                reason_code: Some("hold".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "shadow",
                    "live_order_mode_block_reason": "market_data_stale"
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir =
            std::env::temp_dir().join("polybot-kpi-gate-shadow-live-then-shadow-summary-gap");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 1);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Fail.as_str()
        );
        assert_eq!(report.overall_status, KpiStatus::Fail.as_str());
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_same_second_runtime_shadow_beats_decision_live_without_uuid_tiebreak(
    ) -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "same-second-live-then-shadow-missing-summary",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let decisions = vec![KpiDecisionEventRow {
            decision_event_id: "zzzz-live-decision".to_string(),
            trade_id: "same-second-live-then-shadow-missing-summary".to_string(),
            decision_ts: "2026-03-21T09:04:00+07:00".to_string(),
            approved: true,
            reason_code: "decision".to_string(),
            phase: Some("Open".to_string()),
            owner: Some("OpenBoth".to_string()),
            submit_origin: Some("maker".to_string()),
            submit_side: Some("buy".to_string()),
            payload_json: serde_json::to_string(&json!({
                "configured_order_mode": "live",
                "effective_order_mode": "live",
                "owner": "OpenBoth",
                "approved": true,
                "t_into_seconds": 100.10
            }))
            .expect("payload"),
        }];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_settlement(
                "same-second-live-then-shadow-missing-summary",
                "2026-03-21",
                "settled",
            ),
            KpiRuntimeEventRow {
                event_id: "aaaa-shadow-risk-block".to_string(),
                trade_id: "same-second-live-then-shadow-missing-summary".to_string(),
                event_kind: "risk_block".to_string(),
                event_ts: "2026-03-21T09:04:00+07:00".to_string(),
                reason_code: Some("hold".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "shadow",
                    "live_order_mode_block_reason": "market_data_stale",
                    "t_into_seconds": 100.60
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions,
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir()
            .join("polybot-kpi-gate-shadow-same-second-live-shadow-summary-gap");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 1);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Fail.as_str()
        );
        assert_eq!(report.overall_status, KpiStatus::Fail.as_str());
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_same_second_live_decision_beats_earlier_shadow_state_transition() -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "same-second-shadow-then-live-missing-summary",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let decisions = vec![KpiDecisionEventRow {
            decision_event_id: "zzzz-live-decision".to_string(),
            trade_id: "same-second-shadow-then-live-missing-summary".to_string(),
            decision_ts: "2026-03-21T09:04:00+07:00".to_string(),
            approved: true,
            reason_code: "decision".to_string(),
            phase: Some("Open".to_string()),
            owner: Some("OpenBoth".to_string()),
            submit_origin: Some("maker".to_string()),
            submit_side: Some("buy".to_string()),
            payload_json: serde_json::to_string(&json!({
                "configured_order_mode": "live",
                "effective_order_mode": "live",
                "owner": "OpenBoth",
                "approved": true,
                "t_into_seconds": 100.60
            }))
            .expect("payload"),
        }];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_settlement(
                "same-second-shadow-then-live-missing-summary",
                "2026-03-21",
                "settled",
            ),
            KpiRuntimeEventRow {
                event_id: "aaaa-shadow-state-transition".to_string(),
                trade_id: "same-second-shadow-then-live-missing-summary".to_string(),
                event_kind: "state_transition".to_string(),
                event_ts: "2026-03-21T09:04:00+07:00".to_string(),
                reason_code: Some("await_live_arm".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "shadow",
                    "live_order_mode_block_reason": "await_live_arm",
                    "t_into_seconds": 100.10
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions,
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir()
            .join("polybot-kpi-gate-shadow-same-second-shadow-live-summary-gap");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 0);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Pass.as_str()
        );
        assert_eq!(
            report.overall_status,
            KpiStatus::InsufficientSample.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_same_second_live_decision_beats_earlier_shadow_risk_block_by_t_into(
    ) -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "same-second-risk-block-then-live-missing-summary",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let decisions = vec![KpiDecisionEventRow {
            decision_event_id: "zzzz-live-decision".to_string(),
            trade_id: "same-second-risk-block-then-live-missing-summary".to_string(),
            decision_ts: "2026-03-21T09:04:00+07:00".to_string(),
            approved: true,
            reason_code: "decision".to_string(),
            phase: Some("Open".to_string()),
            owner: Some("OpenBoth".to_string()),
            submit_origin: Some("maker".to_string()),
            submit_side: Some("buy".to_string()),
            payload_json: serde_json::to_string(&json!({
                "configured_order_mode": "live",
                "effective_order_mode": "live",
                "owner": "OpenBoth",
                "approved": true,
                "t_into_seconds": 100.60
            }))
            .expect("payload"),
        }];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_settlement(
                "same-second-risk-block-then-live-missing-summary",
                "2026-03-21",
                "settled",
            ),
            KpiRuntimeEventRow {
                event_id: "aaaa-shadow-risk-block".to_string(),
                trade_id: "same-second-risk-block-then-live-missing-summary".to_string(),
                event_kind: "risk_block".to_string(),
                event_ts: "2026-03-21T09:04:00+07:00".to_string(),
                reason_code: Some("hold".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "shadow",
                    "live_order_mode_block_reason": "market_data_stale",
                    "t_into_seconds": 100.10
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions,
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir()
            .join("polybot-kpi-gate-shadow-same-second-risk-block-live-summary-gap");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 0);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Pass.as_str()
        );
        assert_eq!(
            report.overall_status,
            KpiStatus::InsufficientSample.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_same_second_live_decision_then_shadow_risk_block_without_t_into_counts_missing_summary(
    ) -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-selected",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "same-second-live-then-shadow-no-t-into",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let decisions = vec![KpiDecisionEventRow {
            decision_event_id: "zzzz-live-decision".to_string(),
            trade_id: "same-second-live-then-shadow-no-t-into".to_string(),
            decision_ts: "2026-03-21T09:04:00+07:00".to_string(),
            approved: true,
            reason_code: "decision".to_string(),
            phase: Some("Open".to_string()),
            owner: Some("OpenBoth".to_string()),
            submit_origin: Some("maker".to_string()),
            submit_side: Some("buy".to_string()),
            payload_json: serde_json::to_string(&json!({
                "configured_order_mode": "live",
                "effective_order_mode": "live",
                "owner": "OpenBoth",
                "approved": true,
                "t_into_seconds": 100.10
            }))
            .expect("payload"),
        }];
        let runtime_events = vec![
            make_run_summary(
                "shadow-selected",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_settlement("shadow-selected", "2026-03-20", "settled"),
            make_settlement(
                "same-second-live-then-shadow-no-t-into",
                "2026-03-21",
                "settled",
            ),
            KpiRuntimeEventRow {
                event_id: "aaaa-shadow-risk-block".to_string(),
                trade_id: "same-second-live-then-shadow-no-t-into".to_string(),
                event_kind: "risk_block".to_string(),
                event_ts: "2026-03-21T09:04:00+07:00".to_string(),
                reason_code: Some("hold".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "live",
                    "effective_order_mode": "shadow",
                    "live_order_mode_block_reason": "market_data_stale"
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions,
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-22T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir()
            .join("polybot-kpi-gate-shadow-same-second-live-shadow-no-t-into-summary-gap");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 1);
        assert_eq!(report.source_counts.missing_run_summary_count, 1);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Fail.as_str()
        );
        assert_eq!(report.overall_status, KpiStatus::Fail.as_str());
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }

    #[test]
    fn shadow_gate_detects_missing_run_summary_without_counting_it_as_sample() -> Result<()> {
        let trades = vec![
            make_trade(
                "shadow-missing-summary-1",
                "2026-03-20",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "shadow-missing-summary-2",
                "2026-03-21",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
            make_trade(
                "shadow-missing-summary-3",
                "2026-03-22",
                "SETTLED",
                1.0,
                10.0,
                10.0,
                0.4,
            ),
        ];
        let runtime_events = vec![
            make_run_summary(
                "shadow-missing-summary-1",
                KpiProfile::Shadow,
                "2026-03-20",
                json!({}),
            ),
            make_run_summary(
                "shadow-missing-summary-2",
                KpiProfile::Shadow,
                "2026-03-21",
                json!({}),
            ),
            make_settlement("shadow-missing-summary-1", "2026-03-20", "settled"),
            make_settlement("shadow-missing-summary-2", "2026-03-21", "settled"),
            make_settlement("shadow-missing-summary-3", "2026-03-22", "settled"),
            KpiRuntimeEventRow {
                event_id: "audit-drop-shadow-3".to_string(),
                trade_id: "shadow-missing-summary-3".to_string(),
                event_kind: "audit_drop".to_string(),
                event_ts: "2026-03-22T09:05:02+07:00".to_string(),
                reason_code: Some("runtime_direct_insert_failed".to_string()),
                payload_json: serde_json::to_string(&json!({
                    "configured_order_mode": "shadow",
                    "effective_order_mode": "shadow",
                    "live_order_mode_block_reason": null
                }))
                .expect("payload"),
            },
        ];
        let source = InMemorySource {
            trades,
            decisions: vec![],
            runtime_events,
        };
        let request = KpiRunRequest {
            bot_id: "bot".to_string(),
            profile: KpiProfile::Shadow,
            window_start: "2026-03-20T00:00:00+07:00".to_string(),
            window_end: "2026-03-23T00:00:00+07:00".to_string(),
        };
        let mut sink = NoopKpiGateSink;
        let out_dir = std::env::temp_dir().join("polybot-kpi-gate-shadow-missing-summary");
        let (report, _) = run_kpi_gate(&source, &mut sink, &request, &out_dir)?;
        assert_eq!(report.source_counts.selected_trades, 2);
        assert_eq!(report.source_counts.missing_run_summary_count, 1);
        assert_eq!(report.sample_coverage.distinct_trading_days, 2);
        assert!(!report.sample_coverage.sufficient_sample);
        assert_eq!(
            report.metrics["decision_logging_integrity"].status,
            KpiStatus::Fail.as_str()
        );
        fs::remove_dir_all(&out_dir).ok();
        Ok(())
    }
}
