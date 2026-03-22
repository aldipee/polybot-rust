use polybot::replay::run_replay_scenario;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
enum CertificationKind {
    GoodOpen,
    OneSideLag,
    StaleHold,
    ReconnectMismatch,
    LateSettlement,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeExpectation {
    event_kind: &'static str,
    reason_code: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
struct ReplayCertificationCase {
    id: &'static str,
    coverage_label: &'static str,
    kind: CertificationKind,
    expected_owner: Option<&'static str>,
    expected_phase: Option<&'static str>,
    expected_safety_gate: Option<&'static str>,
    expected_exit_reason: Option<&'static str>,
    min_decision_events: usize,
    min_runtime_events: usize,
    require_resolution_snapshot: bool,
    required_runtime_events: &'static [RuntimeExpectation],
    forbidden_runtime_events: &'static [RuntimeExpectation],
}

#[derive(Debug, Deserialize)]
struct ReplayRuntimeEventRow {
    event_kind: String,
    reason_code: String,
    payload_json: String,
}

#[derive(Debug, Deserialize)]
struct ReplayFinalState {
    exit_reason: String,
    runtime_state: ReplayFinalRuntimeState,
    state: ReplayFinalBotState,
    trade_metrics: ReplayTradeMetrics,
}

#[derive(Debug, Deserialize)]
struct ReplayFinalRuntimeState {
    audit_decision_event_count: usize,
    audit_runtime_event_count: usize,
    owner: String,
    phase: String,
    safety_gate: String,
    safety_gate_reason: String,
}

#[derive(Debug, Deserialize)]
struct ReplayFinalBotState {
    q_yes: f64,
    q_no: f64,
    c_yes: f64,
    c_no: f64,
    open_orders: HashMap<String, Value>,
    pair_total_fill_events: usize,
    pair_total_fill_shares: f64,
}

#[derive(Debug, Deserialize)]
struct ReplayTradeMetrics {
    fill_count: usize,
    total_cost: f64,
}

const GOOD_OPEN_REQUIRED: &[RuntimeExpectation] = &[RuntimeExpectation {
    event_kind: "state_transition",
    reason_code: Some("both_sides_live"),
}];

const GOOD_OPEN_FORBIDDEN: &[RuntimeExpectation] = &[
    RuntimeExpectation {
        event_kind: "risk_block",
        reason_code: Some("dependency_pause:market_ws"),
    },
    RuntimeExpectation {
        event_kind: "reconciliation",
        reason_code: Some("reconnect_clean"),
    },
];

const ONE_SIDE_REQUIRED: &[RuntimeExpectation] = &[RuntimeExpectation {
    event_kind: "state_transition",
    reason_code: Some("startup_asymmetry"),
}];

const ONE_SIDE_FORBIDDEN: &[RuntimeExpectation] = &[RuntimeExpectation {
    event_kind: "state_transition",
    reason_code: Some("both_sides_live"),
}];

const STALE_REQUIRED: &[RuntimeExpectation] = &[
    RuntimeExpectation {
        event_kind: "risk_block",
        reason_code: Some("market_data_stale_add_block"),
    },
    RuntimeExpectation {
        event_kind: "risk_block",
        reason_code: Some("dependency_pause:market_data_stale"),
    },
];

const STALE_FORBIDDEN: &[RuntimeExpectation] = &[RuntimeExpectation {
    event_kind: "reconciliation",
    reason_code: Some("reconnect_clean"),
}];

const RECONNECT_REQUIRED: &[RuntimeExpectation] = &[
    RuntimeExpectation {
        event_kind: "risk_block",
        reason_code: Some("dependency_pause:market_ws"),
    },
    RuntimeExpectation {
        event_kind: "reconciliation",
        reason_code: Some("reconnect_clean"),
    },
];

const RECONNECT_FORBIDDEN: &[RuntimeExpectation] = &[];

const LATE_REQUIRED: &[RuntimeExpectation] = &[
    RuntimeExpectation {
        event_kind: "settlement",
        reason_code: Some("await_settlement_handoff"),
    },
    RuntimeExpectation {
        event_kind: "settlement",
        reason_code: Some("settled"),
    },
];

const LATE_FORBIDDEN: &[RuntimeExpectation] = &[RuntimeExpectation {
    event_kind: "settlement",
    reason_code: Some("resolution_snapshot_unavailable"),
}];

const CASES: &[ReplayCertificationCase] = &[
    ReplayCertificationCase {
        id: "good_open_paired_seed",
        coverage_label: "REQ-007 good open paired seed",
        kind: CertificationKind::GoodOpen,
        expected_owner: Some("PairBuild"),
        expected_phase: Some("OpenBoth"),
        expected_safety_gate: Some("healthy"),
        expected_exit_reason: Some("REPLAY_COMPLETE"),
        min_decision_events: 3,
        min_runtime_events: 19,
        require_resolution_snapshot: false,
        required_runtime_events: GOOD_OPEN_REQUIRED,
        forbidden_runtime_events: GOOD_OPEN_FORBIDDEN,
    },
    ReplayCertificationCase {
        id: "one_side_lag_await_second_fill",
        coverage_label: "REQ-007 one-side lag await-second-fill",
        kind: CertificationKind::OneSideLag,
        expected_owner: Some("AwaitSecondFill"),
        expected_phase: Some("OpenBoth"),
        expected_safety_gate: Some("healthy"),
        expected_exit_reason: Some("REPLAY_COMPLETE"),
        min_decision_events: 2,
        min_runtime_events: 17,
        require_resolution_snapshot: false,
        required_runtime_events: ONE_SIDE_REQUIRED,
        forbidden_runtime_events: ONE_SIDE_FORBIDDEN,
    },
    ReplayCertificationCase {
        id: "stale_data_hold_escalation",
        coverage_label: "REQ-013 stale-data hold escalation",
        kind: CertificationKind::StaleHold,
        expected_owner: Some("OpenBoth"),
        expected_phase: Some("OpenBoth"),
        expected_safety_gate: Some("dependency_paused"),
        expected_exit_reason: Some("REPLAY_COMPLETE"),
        min_decision_events: 1,
        min_runtime_events: 13,
        require_resolution_snapshot: false,
        required_runtime_events: STALE_REQUIRED,
        forbidden_runtime_events: STALE_FORBIDDEN,
    },
    ReplayCertificationCase {
        id: "reconnect_reconciliation_mismatch",
        coverage_label: "REQ-019 reconnect reconciliation mismatch",
        kind: CertificationKind::ReconnectMismatch,
        expected_owner: Some("OpenBoth"),
        expected_phase: Some("OpenBoth"),
        expected_safety_gate: Some("healthy"),
        expected_exit_reason: Some("REPLAY_COMPLETE"),
        min_decision_events: 2,
        min_runtime_events: 20,
        require_resolution_snapshot: false,
        required_runtime_events: RECONNECT_REQUIRED,
        forbidden_runtime_events: RECONNECT_FORBIDDEN,
    },
    ReplayCertificationCase {
        id: "late_settlement_handoff",
        coverage_label: "REQ-019 late settlement handoff",
        kind: CertificationKind::LateSettlement,
        expected_owner: Some("AwaitSettlement"),
        expected_phase: Some("AwaitSettlement"),
        expected_safety_gate: Some("startup_reconciliation_pending"),
        expected_exit_reason: Some("AWAIT_SETTLEMENT"),
        min_decision_events: 0,
        min_runtime_events: 2,
        require_resolution_snapshot: true,
        required_runtime_events: LATE_REQUIRED,
        forbidden_runtime_events: LATE_FORBIDDEN,
    },
];

fn scenario_root(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("replay")
        .join("scenarios")
        .join(id)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let text = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("failed reading {}: {err}", path.display());
    });
    serde_json::from_str(&text).unwrap_or_else(|err| {
        panic!("failed parsing {}: {err}", path.display());
    })
}

fn read_json_lines<T: for<'de> Deserialize<'de>>(path: &Path) -> Vec<T> {
    let text = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("failed reading {}: {err}", path.display());
    });
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<T>(line).unwrap_or_else(|err| {
                panic!("failed parsing jsonl row in {}: {err}", path.display());
            })
        })
        .collect()
}

fn runtime_event_matches(row: &ReplayRuntimeEventRow, expected: RuntimeExpectation) -> bool {
    if row.event_kind != expected.event_kind {
        return false;
    }
    expected
        .reason_code
        .map(|reason| row.reason_code == reason)
        .unwrap_or(true)
}

fn assert_case_semantics(
    case: &ReplayCertificationCase,
    final_state: &ReplayFinalState,
    runtime_events: &[ReplayRuntimeEventRow],
) {
    match case.kind {
        CertificationKind::GoodOpen => {
            assert!(
                final_state.trade_metrics.fill_count >= 2,
                "{} should finish with both sides filled",
                case.id
            );
            assert!(
                (final_state.state.q_yes - 1.0).abs() < 1e-9
                    && (final_state.state.q_no - 1.0).abs() < 1e-9,
                "{} should end with balanced paired inventory",
                case.id
            );
            assert!(
                final_state.state.open_orders.is_empty(),
                "{} should finish without lingering open orders",
                case.id
            );
        }
        CertificationKind::OneSideLag => {
            assert!(
                final_state.trade_metrics.fill_count >= 1,
                "{} should have at least one filled side",
                case.id
            );
            assert!(
                final_state.state.q_yes > 0.0 && final_state.state.q_no == 0.0,
                "{} should remain one-sided at the end of the fixture",
                case.id
            );
            assert!(
                final_state.state.open_orders.contains_key("no_asset_id"),
                "{} should keep the missing-side repair order live",
                case.id
            );
        }
        CertificationKind::StaleHold => {
            assert_eq!(
                final_state.trade_metrics.fill_count, 0,
                "{} should remain unfilled while stale holds escalate",
                case.id
            );
            assert!(
                runtime_events
                    .iter()
                    .filter(|row| row.event_kind == "risk_block")
                    .count()
                    >= 3,
                "{} should include multiple stale-hold runtime rows",
                case.id
            );
        }
        CertificationKind::ReconnectMismatch => {
            assert!(
                final_state.state.open_orders.len() >= 2,
                "{} should end with recovered shadow working orders after reconnect reconciliation",
                case.id
            );
        }
        CertificationKind::LateSettlement => {
            assert!(
                (final_state.state.q_yes - 5.0).abs() < 1e-9
                    && (final_state.state.q_no - 5.0).abs() < 1e-9,
                "{} should preserve the captured balanced position into settlement",
                case.id
            );
            assert!(
                (final_state.trade_metrics.total_cost - 4.55).abs() < 1e-9,
                "{} should preserve the captured total cost into settlement",
                case.id
            );
        }
    }
}

#[test]
fn replay_certification_fixture_integrity() {
    let scenarios_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("replay")
        .join("scenarios");

    let actual_dirs: BTreeSet<String> = fs::read_dir(&scenarios_root)
        .unwrap_or_else(|err| panic!("failed reading {}: {err}", scenarios_root.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|value| value.to_string())
        })
        .collect();
    let expected_dirs: BTreeSet<String> = CASES.iter().map(|case| case.id.to_string()).collect();

    assert_eq!(
        actual_dirs, expected_dirs,
        "scenario directories and certification case table must stay in sync"
    );

    for case in CASES {
        let root = scenario_root(case.id);
        for required in [
            "manifest.json",
            "resolved_config.json",
            "events.jsonl",
            "oracle_decisions.jsonl",
            "oracle_runtime_events.jsonl",
            "oracle_final_state.json",
            "README.md",
        ] {
            assert!(
                root.join(required).exists(),
                "{} missing required fixture file {}",
                case.id,
                required
            );
        }
        assert!(
            root.join("initial_state").is_dir(),
            "{} missing initial_state directory",
            case.id
        );
        if case.require_resolution_snapshot {
            assert!(
                root.join("resolution_snapshot.json").exists(),
                "{} requires a committed resolution_snapshot.json",
                case.id
            );
        }
    }
}

#[test]
fn replay_certification_suite_passes() {
    for case in CASES {
        let root = scenario_root(case.id);
        run_replay_scenario(&root).unwrap_or_else(|err| {
            panic!(
                "replay certification failed for {} ({}): {err:#}",
                case.id, case.coverage_label
            )
        });

        let final_state: ReplayFinalState = read_json(&root.join("oracle_final_state.json"));
        let decisions: Vec<Value> = read_json_lines(&root.join("oracle_decisions.jsonl"));
        let runtime_events: Vec<ReplayRuntimeEventRow> =
            read_json_lines(&root.join("oracle_runtime_events.jsonl"));

        if let Some(owner) = case.expected_owner {
            assert_eq!(
                final_state.runtime_state.owner, owner,
                "{} should end with owner {owner}",
                case.id
            );
        }
        if let Some(phase) = case.expected_phase {
            assert_eq!(
                final_state.runtime_state.phase, phase,
                "{} should end with phase {phase}",
                case.id
            );
        }
        if let Some(safety_gate) = case.expected_safety_gate {
            assert_eq!(
                final_state.runtime_state.safety_gate, safety_gate,
                "{} should end with safety gate {safety_gate}",
                case.id
            );
        }
        if let Some(exit_reason) = case.expected_exit_reason {
            assert_eq!(
                final_state.exit_reason, exit_reason,
                "{} should end with exit reason {exit_reason}",
                case.id
            );
        }

        assert!(
            final_state.runtime_state.audit_decision_event_count >= case.min_decision_events,
            "{} should record at least {} decision events, found {}",
            case.id,
            case.min_decision_events,
            final_state.runtime_state.audit_decision_event_count
        );
        assert!(
            final_state.runtime_state.audit_runtime_event_count >= case.min_runtime_events,
            "{} should record at least {} runtime events, found {}",
            case.id,
            case.min_runtime_events,
            final_state.runtime_state.audit_runtime_event_count
        );
        assert!(
            decisions.len() >= case.min_decision_events,
            "{} should commit at least {} oracle decisions, found {}",
            case.id,
            case.min_decision_events,
            decisions.len()
        );
        assert!(
            runtime_events.len() >= case.min_runtime_events,
            "{} should commit at least {} oracle runtime rows, found {}",
            case.id,
            case.min_runtime_events,
            runtime_events.len()
        );

        for required in case.required_runtime_events {
            assert!(
                runtime_events
                    .iter()
                    .any(|row| runtime_event_matches(row, *required)),
                "{} missing required runtime event {:?}",
                case.id,
                required
            );
        }
        for forbidden in case.forbidden_runtime_events {
            assert!(
                !runtime_events
                    .iter()
                    .any(|row| runtime_event_matches(row, *forbidden)),
                "{} unexpectedly contained forbidden runtime event {:?}",
                case.id,
                forbidden
            );
        }

        for row in &runtime_events {
            let _: Value = serde_json::from_str(&row.payload_json).unwrap_or_else(|err| {
                panic!(
                    "{} has invalid payload_json in runtime event {} / {}: {err}",
                    case.id, row.event_kind, row.reason_code
                )
            });
        }

        assert_case_semantics(case, &final_state, &runtime_events);
    }
}
