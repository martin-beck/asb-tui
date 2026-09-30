// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    benchmark_route::{CampaignError, CampaignStage, GuidedCampaign, GuidedCatalog, ReplayMode},
    control_codec::Revision,
    reports::{Artifact, EvidenceKind, MeasureResult, MeasureStatus, Report, ReportStatus, RunId},
    selection::{
        BenchmarkCatalog, BenchmarkDefinition, BenchmarkGroup, BenchmarkMeasure, BenchmarkPool,
        BenchmarkSelection,
    },
};
use std::collections::BTreeMap;

#[test]
fn guided_route_selects_then_launches_and_comparisons_are_after_completion() {
    let mut route = GuidedCampaign::new("quality").unwrap();
    route.add_agent("agent-a").unwrap();
    route.add_agent("agent-b").unwrap();
    route.add_measure("quality.correctness").unwrap();
    route.review().unwrap();
    let intent = route.start().unwrap();
    assert_eq!(intent.agents, ["agent-a", "agent-b"]);
    assert_eq!(route.stage, CampaignStage::Running);
    route.finish(true).unwrap();
    assert_eq!(route.stage, CampaignStage::Complete);
}

#[test]
fn offline_replay_is_explicit_and_has_no_live_fallback() {
    let mut route = GuidedCampaign::new("quality").unwrap();
    route.add_agent("agent-a").unwrap();
    route.add_measure("quality.correctness").unwrap();
    route.set_replay_mode(ReplayMode::OfflineReplay).unwrap();
    route.review().unwrap();
    assert_eq!(route.start(), Err(CampaignError::OfflineRecordingRequired));
    let intent = route.start_offline_replay("cassette-1").unwrap();
    assert!(intent.offline_only);
    assert_eq!(intent.recording_id, "cassette-1");
}

#[test]
fn comparison_is_deferred_until_successful_completion_and_delegates_compatibility() {
    let mut route = GuidedCampaign::new("quality").unwrap();
    route.add_agent("agent-a").unwrap();
    route.add_measure("quality.correctness").unwrap();
    route.review().unwrap();
    let reports = vec![report("run-a", 1.0), report("run-b", 2.0)];
    assert_eq!(
        route.compare(&reports, &[id("run-a"), id("run-b")]),
        Err(CampaignError::InvalidTransition)
    );
    route.start().unwrap();
    route.finish(true).unwrap();
    let comparison = route
        .compare(&reports, &[id("run-a"), id("run-b")])
        .unwrap();
    assert!(comparison.compatible);
    assert_eq!(comparison.run_ids, vec![id("run-a"), id("run-b")]);
}

#[test]
fn catalog_backed_route_rejects_unsupported_choices() {
    let catalog = GuidedCatalog::new(
        vec!["quality".into()],
        vec!["agent-a".into()],
        vec!["quality.correctness".into()],
    )
    .unwrap();
    let mut route = GuidedCampaign::with_catalog("quality", catalog).unwrap();
    assert_eq!(
        route.add_agent("unlisted"),
        Err(CampaignError::MissingAgent)
    );
    assert_eq!(
        route.add_measure("unlisted"),
        Err(CampaignError::MissingMeasure)
    );
    route.add_agent("agent-a").unwrap();
    route.add_measure("quality.correctness").unwrap();
    route.review().unwrap();
}

#[test]
fn exact_nested_handoff_is_generation_bound_at_campaign_start() {
    let measure =
        BenchmarkMeasure::new("quality.correctness", "Correctness", "score", true).unwrap();
    let benchmark = BenchmarkDefinition::new("quality", "Quality", vec![measure]).unwrap();
    let group = BenchmarkGroup::new("core", "Core", vec![benchmark]).unwrap();
    let pool = BenchmarkPool::new("standard", "Standard", vec![group]).unwrap();
    let catalog = BenchmarkCatalog::new(Revision(7), "digest-7", vec![pool]).unwrap();
    let mut picker = BenchmarkSelection::new(catalog).unwrap();
    picker.set_benchmark_selected("quality", true).unwrap();
    let handoff = picker.campaign_handoff(Revision(7)).unwrap();
    let mut route = GuidedCampaign::new("quality").unwrap();
    route.add_agent("agent-a").unwrap();
    assert_eq!(
        route.start_with_handoff(handoff.clone(), Revision(6)),
        Err(CampaignError::StaleCatalog)
    );
    let intent = route.start_with_handoff(handoff, Revision(7)).unwrap();
    assert_eq!(intent.catalog_generation, Some(Revision(7)));
    assert_eq!(intent.catalog_digest.as_deref(), Some("digest-7"));
    assert_eq!(intent.benchmark_ids, ["quality"]);
    assert_eq!(intent.measures, ["quality.correctness"]);
}

fn id(value: &str) -> RunId {
    RunId::new(value).unwrap()
}

fn report(value: &str, measurement: f64) -> Report {
    Report {
        run_id: id(value),
        provenance: "development/mock/offline-replay".into(),
        measures: BTreeMap::from([(
            "quality.correctness".into(),
            MeasureResult {
                value: Some(measurement),
                unit: "score".into(),
                status: MeasureStatus::Complete,
            },
        )]),
        uncertainty: None,
        failures: Vec::new(),
        artifacts: vec![Artifact {
            name: "report.json".into(),
            available: true,
        }],
        command_argv: vec!["asb".into(), "replay".into(), "--offline".into()],
        stale: false,
        evidence: EvidenceKind::Replay,
        status: ReportStatus::Complete,
        compatibility_key: "development/provider/model/catalog/config".into(),
    }
}
