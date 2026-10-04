// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
use asb_tui::reports::*;
use std::collections::BTreeMap;

fn id(v: &str) -> RunId {
    RunId::new(v).unwrap()
}
fn run(name: &str, time: u64) -> RunRecord {
    RunRecord {
        id: id(name),
        started_at_ms: time,
        cursor: format!("c-{name}"),
        status: RunStatus::Succeeded,
        source: RunSource::LiveRecording,
        platform: "linux-x86_64".into(),
        agents: vec!["agent".into()],
        workload: "smoke".into(),
        integrity: Integrity::Verified,
        concise_result: Some("ok".into()),
    }
}
fn report(name: &str, provenance: &str, stale: bool) -> Report {
    let mut measures = BTreeMap::new();
    measures.insert(
        "latency".into(),
        MeasureResult {
            value: Some(1.0),
            unit: "ms".into(),
            status: MeasureStatus::Complete,
        },
    );
    Report {
        run_id: id(name),
        provenance: provenance.into(),
        measures,
        uncertainty: None,
        failures: vec![],
        artifacts: vec![],
        command_argv: vec!["asb".into(), "run".into(), "--name".into(), name.into()],
        stale,
        evidence: EvidenceKind::Live,
        status: ReportStatus::Complete,
        compatibility_key: "provider/model/catalog/config".into(),
    }
}

#[test]
fn newest_order_uses_id_tie_break_and_search_is_deterministic() {
    let mut state = RecentRuns::default();
    state
        .replace_page(
            "first",
            RecentPage {
                request_cursor: None,
                runs: vec![run("b", 20), run("a", 20), run("z", 10)],
                next_cursor: None,
                next_started_at_ms: None,
            },
        )
        .unwrap();
    assert_eq!(
        state
            .visible_runs()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "a", "z"]
    );
    state.set_filter("SMOKE").unwrap();
    assert_eq!(state.visible_runs().len(), 3);
}

#[test]
fn malformed_page_and_cursor_are_rejected() {
    let mut state = RecentRuns::default();
    let page = RecentPage {
        request_cursor: None,
        runs: vec![run("a", 1)],
        next_cursor: Some("next".into()),
        next_started_at_ms: Some(2),
    };
    assert_eq!(
        state.replace_page("x", page),
        Err(ValidationError::ContradictoryCursor)
    );
    let page = RecentPage {
        request_cursor: None,
        runs: vec![run("a", 1), run("b", 2)],
        next_cursor: None,
        next_started_at_ms: None,
    };
    assert_eq!(
        state.replace_page("x", page),
        Err(ValidationError::UnorderedPage)
    );
}

#[test]
fn selection_prunes_deleted_runs_and_compare_is_fail_closed() {
    let mut state = RecentRuns::default();
    state
        .replace_page(
            "x",
            RecentPage {
                request_cursor: None,
                runs: vec![run("a", 2), run("b", 1)],
                next_cursor: None,
                next_started_at_ms: None,
            },
        )
        .unwrap();
    state.select(&id("a"), true).unwrap();
    state
        .replace_page(
            "x",
            RecentPage {
                request_cursor: None,
                runs: vec![run("b", 1)],
                next_cursor: None,
                next_started_at_ms: None,
            },
        )
        .unwrap();
    assert!(state.selected.is_empty());
    assert_eq!(
        compare(&[report("a", "p", false)], &[id("a"), id("deleted")]),
        Err(CompareError::StaleOrDeleted)
    );
    assert_eq!(
        compare(
            &[report("a", "p", true), report("b", "p", false)],
            &[id("a"), id("b")]
        ),
        Err(CompareError::StaleOrDeleted)
    );
    assert_eq!(
        compare(
            &[report("a", "p", false), report("b", "p", false)],
            &[id("a"), id("a")]
        ),
        Err(CompareError::Duplicate)
    );
}

#[test]
fn comparison_reports_conflicts_and_command_is_quoted() {
    let a = report("a", "p", false);
    let mut b = report("b", "q", false);
    b.command_argv.push("--label=hello world".into());
    let comparison = compare(&[a.clone(), b.clone()], &[id("b"), id("a")]).unwrap();
    assert!(!comparison.compatible);
    assert_eq!(comparison.run_ids, vec![id("b"), id("a")]);
    assert!(b.shell_command().unwrap().contains("'--label=hello world'"));
}

fn comparison_selection() -> asb_tui::fanout_dispatch::FanoutSelection {
    asb_tui::fanout_dispatch::FanoutSelection {
        agent_ids: vec!["agent-a".into(), "agent-b".into()],
        workload_ids: vec!["smoke".into()],
        provider_id: "provider".into(),
        model_id: "model".into(),
        catalog_digest: "a".repeat(64),
        workload_revision: "b".repeat(64),
        scorer_revision: "c".repeat(64),
    }
}

#[test]
fn selected_agent_orchestration_joins_online_and_replay_terminal_refs() {
    let mut orchestration = SelectedAgentComparison::new(comparison_selection()).unwrap();
    orchestration
        .add_terminal(
            id("online"),
            "agent-a",
            "smoke",
            ComparisonTerminalSource::Online,
        )
        .unwrap();
    orchestration
        .add_terminal(
            id("replay"),
            "agent-b",
            "smoke",
            ComparisonTerminalSource::OfflineReplay,
        )
        .unwrap();
    let result = orchestration
        .orchestrate(&[
            report("online", "catalog", false),
            report("replay", "catalog", false),
        ])
        .unwrap();
    assert!(result.comparison.is_some());
    assert_eq!(
        result.terminal_sources,
        vec![
            ComparisonTerminalSource::Online,
            ComparisonTerminalSource::OfflineReplay,
        ]
    );
    assert!(SelectedAgentComparison::json(&result).contains("OfflineReplay"));
}

#[test]
fn selected_agent_orchestration_rejects_out_of_scope_terminal() {
    let mut orchestration = SelectedAgentComparison::new(comparison_selection()).unwrap();
    assert_eq!(
        orchestration.add_terminal(
            id("run"),
            "unselected-agent",
            "smoke",
            ComparisonTerminalSource::Online,
        ),
        Err(ComparisonOrchestrationError::InvalidTerminal(
            "terminal is outside the selected agent/workload scope"
        ))
    );
}

#[test]
fn assessment_marks_symmetric_runs_available_and_comparable() {
    let reports = vec![
        report("baseline", "catalog-a", false),
        report("candidate", "catalog-a", false),
    ];
    let assessment = assess_comparison(&reports, &[id("baseline"), id("candidate")]);
    assert_eq!(assessment.availability, ComparisonAvailability::Available);
    assert!(assessment.comparable);
    assert_eq!(assessment.sides.len(), 2);
    assert_eq!(assessment.sides[0].role, ComparisonRole::Baseline);
    assert_eq!(assessment.sides[1].role, ComparisonRole::Candidate);
    assert!(assessment.sides.iter().all(|side| side.available));
    assert!(assessment.unavailable_reasons.is_empty());
    assert!(assessment.confounders.is_empty());
}

#[test]
fn assessment_explains_asymmetric_missing_candidate_without_claiming_comparability() {
    let reports = vec![report("baseline", "catalog-a", false)];
    let assessment = assess_comparison(&reports, &[id("baseline"), id("candidate")]);
    assert_eq!(assessment.availability, ComparisonAvailability::Partial);
    assert!(!assessment.comparable);
    assert!(assessment.sides[0].available);
    assert_eq!(assessment.sides[1].unavailable_reasons, ["report_missing"]);
    assert_eq!(assessment.unavailable_reasons, ["report_missing"]);
    assert!(assessment.confounders.is_empty());
}

#[test]
fn assessment_preserves_multi_candidate_side_reasons_and_provenance_conflict() {
    let reports = vec![
        report("baseline", "catalog-a", false),
        report("candidate-a", "catalog-a", false),
        report("candidate-b", "catalog-b", false),
    ];
    let assessment = assess_comparison(
        &reports,
        &[id("baseline"), id("candidate-a"), id("candidate-b")],
    );
    assert_eq!(assessment.availability, ComparisonAvailability::Available);
    assert!(!assessment.comparable);
    assert!(
        assessment
            .confounders
            .contains(&"provenance_differs".into())
    );
    assert_eq!(assessment.sides[2].confounders, ["provenance_differs"]);
    assert!(assessment.sides.iter().all(|side| side.available));
}

#[test]
fn comparison_rejects_mixed_live_and_replay_evidence() {
    let a = report("a", "same", false);
    let mut b = report("b", "same", false);
    b.evidence = EvidenceKind::Replay;
    let comparison = compare(&[a, b], &[id("a"), id("b")]).unwrap();
    assert!(!comparison.compatible);
    assert!(
        comparison
            .confounders
            .iter()
            .any(|item| item == "evidence kind differs")
    );
}

#[test]
fn bounded_page_retention_is_enforced() {
    let mut state = RecentRuns::default();
    for n in 0..(MAX_RETAINED_PAGES + 2) {
        state
            .replace_page(
                format!("{n:02}"),
                RecentPage {
                    request_cursor: None,
                    runs: vec![run(&format!("r{n}"), n as u64)],
                    next_cursor: None,
                    next_started_at_ms: None,
                },
            )
            .unwrap();
    }
    assert_eq!(state.pages().count(), MAX_RETAINED_PAGES);
}

#[test]
fn command_projection_rejects_private_paths_credentials_and_environment_expansion() {
    for forbidden in [
        "/home/private/run.json",
        "$ASB_TOKEN",
        "--password",
        "super-secret",
        "--api-key",
        "--authorization",
        "Bearer abc123",
    ] {
        let mut candidate = report("a", "p", false);
        candidate.command_argv = vec!["asb".into(), forbidden.into()];
        assert_eq!(
            candidate.shell_command(),
            Err(ValidationError::SensitiveCommand)
        );
    }
}

#[test]
fn presentation_keeps_evidence_and_actionable_partial_measures() {
    let mut candidate = report("a", "live:runner", false);
    candidate.status = ReportStatus::Partial;
    candidate.evidence = EvidenceKind::Development;
    candidate.measures.insert(
        "tokens".into(),
        MeasureResult {
            value: None,
            unit: "tokens".into(),
            status: MeasureStatus::Missing,
        },
    );
    let view = candidate.presentation().unwrap();
    assert_eq!(view.evidence, EvidenceKind::Development);
    assert_eq!(view.status, ReportStatus::Partial);
    let missing = view.measures.iter().find(|m| m.name == "tokens").unwrap();
    assert_eq!(
        missing.next_action.as_deref(),
        Some("rerun with this measure selected")
    );
}

#[test]
fn comparison_rejects_different_provider_model_or_catalog_identity() {
    let a = report("a", "same", false);
    let mut b = report("b", "same", false);
    b.compatibility_key = "other-provider/model/catalog/config".into();
    let comparison = compare(&[a, b], &[id("a"), id("b")]).unwrap();
    assert!(!comparison.compatible);
    assert!(
        comparison
            .confounders
            .iter()
            .any(|item| item.contains("provider/model"))
    );
}

#[test]
fn non_finite_measure_values_are_rejected() {
    let mut candidate = report("a", "p", false);
    candidate.measures.get_mut("latency").unwrap().value = Some(f64::NAN);
    assert_eq!(candidate.validate(), Err(ValidationError::InvalidValue));
}
