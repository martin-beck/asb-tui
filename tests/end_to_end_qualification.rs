// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! AR-1338's bounded, credential-free end-to-end qualification.
//!
//! This is an in-process development fixture. The transcript represents the
//! visible user actions while each stage exercises the renderer-neutral seam
//! used by the application. No network, credential, or ASB process is used.

use asb_tui::{
    benchmark_route::{GuidedCampaign, GuidedCatalog},
    configuration_materialization::{
        MaterializationInput, MaterializedBundle, MaterializedBundleStore,
    },
    control_codec::{ConfigurationSelection, ProviderAuthMethod, Revision},
    development_journey::{FixtureRequest, evaluate_fixture},
    development_onboarding,
    launch_statistics::{LaunchState, LiveRunEvent, StatisticsProvenance},
    provider_setup::ProviderSetupDraft,
    reports::{
        Artifact, EvidenceKind, Integrity, MeasureResult, MeasureStatus, RecentPage, RecentRuns,
        Report, ReportStatus, RunId, RunRecord, RunSource, RunStatus, compare,
    },
    selection::{
        BenchmarkCatalog, BenchmarkDefinition, BenchmarkGroup, BenchmarkMeasure, BenchmarkPool,
        BenchmarkSelection, CampaignSelection,
    },
    top_level::{self, TuiCommand},
    ui::WorkspaceState,
    wizard::{Step, Wizard},
    wizard_catalog::WizardCatalog,
};
use std::{
    collections::BTreeMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const CATALOG_DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn disposable_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "asb-tui-ar1338-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir(&root).expect("create disposable root");
    root
}

fn benchmark_catalog() -> BenchmarkCatalog {
    let measures = vec![
        BenchmarkMeasure::new("latency.first", "First response", "ms", true).unwrap(),
        BenchmarkMeasure::new("tokens.total", "Total tokens", "count", true).unwrap(),
    ];
    let benchmark = BenchmarkDefinition::new("chat", "Chat quality", measures).unwrap();
    let group = BenchmarkGroup::new("quality", "Quality", vec![benchmark]).unwrap();
    let pool = BenchmarkPool::new("core", "Core pool", vec![group]).unwrap();
    BenchmarkCatalog::new(Revision(7), CATALOG_DIGEST, vec![pool]).unwrap()
}

#[test]
fn persisted_live_openrouter_setup_survives_tui_restart_without_mock_fallback() {
    let root = disposable_root();
    let config_path = root.join("config.json");
    let materialized_root = root.join("materialized");
    let selection = ConfigurationSelection {
        agent_ids: vec!["opencode".into(), "opendesk".into()],
        provider_id: "openrouter".into(),
        model_id: "cohere/north-mini-code:free".into(),
        auth_method: ProviderAuthMethod::CredentialReference,
        credential_reference_sha256: Some("a".repeat(64)),
    };
    let draft = ProviderSetupDraft::from_selection_for_development(selection.clone()).unwrap();
    let catalog = benchmark_catalog();
    let benchmark = CampaignSelection {
        generation: Revision(7),
        catalog_digest: CATALOG_DIGEST.into(),
        pool_id: "core".into(),
        group_ids: vec!["quality".into()],
        benchmark_ids: vec!["chat".into()],
        measure_ids: vec!["latency.first".into()],
    };
    let bundle = MaterializedBundle::build(
        MaterializationInput {
            provider: selection,
            provider_catalog_generation: Revision(1),
            provider_catalog_digest: ZERO_DIGEST.into(),
            benchmark,
            asb_protocol: "asb-control".into(),
            asb_version: "development".into(),
            execution_mode: asb_tui::configuration_materialization::ExecutionMode::Live,
        },
        &draft,
        &catalog,
        Revision(1),
        Revision(7),
    )
    .unwrap();
    let store = MaterializedBundleStore::new(&materialized_root);
    store.apply(&bundle).unwrap();

    let restarted = WorkspaceState::with_configuration_store(&config_path).unwrap();
    let values = restarted.wizard.values();
    assert_eq!(values[0], "opencode,opendesk");
    assert_eq!(values[1], "openrouter");
    assert_eq!(values[2], "cohere/north-mini-code:free");
    assert_eq!(values[3], "all defaults");
    assert_eq!(values[5], "live");
    assert!(!values[5].contains("mock"));
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(
        loaded.document.provider.execution_mode,
        asb_tui::configuration_materialization::ExecutionMode::Live
    );
    assert_eq!(loaded.document.provider.provider_id, "openrouter");
    assert_eq!(
        loaded.document.provider.model_id,
        "cohere/north-mini-code:free"
    );
    assert!(!loaded.canonical_json.contains("api_key"));
    fs::remove_dir_all(root).unwrap();
}

fn development_report(run_id: &str, latency: f64) -> Report {
    Report {
        run_id: RunId::new(run_id).unwrap(),
        provenance: "development/mock".into(),
        measures: BTreeMap::from([(
            "latency.first".into(),
            MeasureResult {
                value: Some(latency),
                unit: "ms".into(),
                status: MeasureStatus::Complete,
            },
        )]),
        uncertainty: None,
        failures: Vec::new(),
        artifacts: vec![Artifact {
            name: "fixture-result".into(),
            available: true,
        }],
        command_argv: vec!["asb".into(), "benchmark".into(), "--development".into()],
        stale: false,
        evidence: EvidenceKind::Development,
        status: ReportStatus::Complete,
        compatibility_key: "development/provider/model/catalog/config".into(),
    }
}

#[test]
fn complete_development_journey_has_bounded_visible_transcript() {
    let mut transcript = Vec::new();

    // Install and launch are explicit top-level contracts. The fixture path
    // is selected by the surrounding ASB router; the standalone adapter never
    // accepts a caller-supplied production trust mode.
    assert!(matches!(
        top_level::parse(&[
            "install".into(),
            "--development".into(),
            "--format".into(),
            "json".into()
        ]),
        Ok(TuiCommand::Lifecycle { .. })
    ));
    assert_eq!(top_level::parse(&[]), Ok(TuiCommand::LaunchUi));
    transcript.push("install: development_onboarding_ready".to_owned());
    transcript.push("launch: landing_or_wizard_visible".to_owned());

    let onboarding = development_onboarding::execute_input(
        br#"{"schema_version":1,"profile":"development","bundle_available":true,"broker_available":true,"protocol_version":1}"#.as_slice(),
    );
    assert!(onboarding.ready && onboarding.development_only);

    let fixture = evaluate_fixture(FixtureRequest {
        schema_version: 1,
        profile: "development".into(),
        asb_version: "fixture-1".into(),
        expected_asb_version: "fixture-1".into(),
        catalog_version: 7,
        expected_catalog_version: 7,
        bundle_available: true,
        broker_available: true,
        protocol_version: Some(1),
        expected_protocol_version: 1,
    });
    assert!(fixture.ready && fixture.development_only);
    fn mismatch_version(request: &mut FixtureRequest) {
        request.asb_version = "fixture-0".into();
    }
    fn mismatch_catalog(request: &mut FixtureRequest) {
        request.catalog_version = 6;
    }
    fn mismatch_bundle(request: &mut FixtureRequest) {
        request.bundle_available = false;
    }
    fn mismatch_protocol(request: &mut FixtureRequest) {
        request.protocol_version = Some(2);
    }
    for (code, recovery, mutate) in [
        (
            "asb_version_mismatch",
            "refresh_version",
            mismatch_version as fn(&mut FixtureRequest),
        ),
        (
            "catalog_mismatch",
            "refresh_catalog",
            mismatch_catalog as fn(&mut FixtureRequest),
        ),
        (
            "development_bundle_unavailable",
            "repair_bundle",
            mismatch_bundle as fn(&mut FixtureRequest),
        ),
        (
            "development_protocol_mismatch",
            "retry_broker",
            mismatch_protocol as fn(&mut FixtureRequest),
        ),
    ] {
        let mut mismatch = FixtureRequest {
            schema_version: 1,
            profile: "development".into(),
            asb_version: "fixture-1".into(),
            expected_asb_version: "fixture-1".into(),
            catalog_version: 7,
            expected_catalog_version: 7,
            bundle_available: true,
            broker_available: true,
            protocol_version: Some(1),
            expected_protocol_version: 1,
        };
        mutate(&mut mismatch);
        let response = evaluate_fixture(mismatch);
        assert!(!response.ready);
        assert_eq!((response.code, response.recovery), (code, recovery));
    }

    let mut wizard = Wizard::with_catalog(WizardCatalog::development().unwrap());
    assert_eq!(wizard.step(), Step::Agent);
    wizard.select_all_agents().unwrap();
    for value in [
        "development",
        "fixture-model",
        "all defaults",
        "none",
        "local-mock",
        "strict offline replay",
    ] {
        wizard.advance().unwrap();
        wizard.set_value(value).unwrap();
    }
    wizard.advance().unwrap();
    wizard.complete().unwrap();
    assert!(wizard.development_auth().development_only);
    transcript.push("wizard: agent provider model review".to_owned());

    let catalog = benchmark_catalog();
    let mut picker = BenchmarkSelection::new(catalog.clone()).unwrap();
    picker.set_query("first").unwrap();
    picker.select_pool("core").unwrap();
    picker.set_benchmark_selected("chat", true).unwrap();
    picker.set_measure_selected("tokens.total", false).unwrap();
    assert_eq!(picker.selected_measure_ids(), vec!["latency.first"]);
    let handoff = picker.campaign_handoff(Revision(7)).unwrap();
    assert_eq!(handoff.measure_ids, vec!["latency.first"]);
    transcript.push("benchmark_selection: core/quality/chat -> latency.first".to_owned());

    let provider_selection = ConfigurationSelection {
        agent_ids: vec!["fake-alpha".into(), "fake-beta".into()],
        provider_id: "development".into(),
        model_id: "fixture-model".into(),
        auth_method: ProviderAuthMethod::None,
        credential_reference_sha256: None,
    };
    let provider_draft =
        ProviderSetupDraft::from_selection_for_development(provider_selection.clone()).unwrap();
    let bundle = MaterializedBundle::build(
        MaterializationInput {
            provider: provider_selection,
            provider_catalog_generation: Revision(1),
            provider_catalog_digest: ZERO_DIGEST.into(),
            benchmark: handoff,
            asb_protocol: "asb-control".into(),
            asb_version: "fixture-1".into(),
            execution_mode: asb_tui::configuration_materialization::ExecutionMode::LocalMock,
        },
        &provider_draft,
        &catalog,
        Revision(1),
        Revision(7),
    )
    .unwrap();
    bundle.validate_integrity().unwrap();
    transcript.push(format!(
        "materialization: configuration_digest={}",
        bundle.digest_sha256
    ));
    transcript.push(format!(
        "preflight: {}",
        bundle.preflight_summary().digest_sha256
    ));

    let root = disposable_root();
    let store = MaterializedBundleStore::new(&root);
    store.apply(&bundle).unwrap();
    let loaded = store.load().unwrap().expect("materialized bundle");
    assert_eq!(loaded.digest_sha256, bundle.digest_sha256);

    let mut campaign = GuidedCampaign::with_catalog(
        "chat",
        GuidedCatalog::new(
            vec!["chat".into()],
            vec!["fake-alpha".into(), "fake-beta".into()],
            vec!["latency.first".into()],
        )
        .unwrap(),
    )
    .unwrap();
    campaign.add_agent("fake-alpha").unwrap();
    campaign.add_agent("fake-beta").unwrap();
    campaign.add_measure("latency.first").unwrap();
    campaign.review().unwrap();
    let launch = campaign.start().unwrap();
    assert_eq!(launch.agents.len(), 2);
    transcript.push("launch_run: development_run_started".to_owned());

    let mut run = LaunchState::new(loaded.launch_binding());
    run.begin_launch().unwrap();
    run.apply_event(LiveRunEvent {
        revision: Revision(1),
        run_id: asb_tui::control_codec::RunId("fixture-run".into()),
        attempt_id: asb_tui::control_codec::AttemptId("fixture-attempt".into()),
        generation: Revision(7),
        state: asb_tui::control_codec::PublicRunState::Running,
        completed_measures: 0,
        total_measures: 1,
        failed_measures: 0,
        throughput_per_second: Some(2),
        latency_millis: Some(12),
        provenance: StatisticsProvenance::DevelopmentMock,
        unavailable_reason: None,
    })
    .unwrap();
    assert_eq!(run.statistics().unwrap().bounded_progress_percent(), 0);
    run.apply_event(LiveRunEvent {
        revision: Revision(2),
        run_id: asb_tui::control_codec::RunId("fixture-run".into()),
        attempt_id: asb_tui::control_codec::AttemptId("fixture-attempt".into()),
        generation: Revision(7),
        state: asb_tui::control_codec::PublicRunState::Completed,
        completed_measures: 1,
        total_measures: 1,
        failed_measures: 0,
        throughput_per_second: Some(2),
        latency_millis: Some(10),
        provenance: StatisticsProvenance::DevelopmentMock,
        unavailable_reason: None,
    })
    .unwrap();
    assert_eq!(run.statistics().unwrap().bounded_progress_percent(), 100);
    transcript.push("live_statistics: 100% development/mock".to_owned());

    campaign.finish(true).unwrap();
    let reports = [
        development_report("fixture-run-a", 10.0),
        development_report("fixture-run-b", 11.0),
    ];
    assert!(reports.iter().all(|report| report.is_conclusive()));
    transcript.push("final_results: complete performance measures".to_owned());
    let comparison = compare(
        &reports,
        &[reports[0].run_id.clone(), reports[1].run_id.clone()],
    )
    .unwrap();
    assert!(comparison.compatible);

    let mut recent = RecentRuns::default();
    recent
        .replace_page(
            "fixture-page",
            RecentPage {
                request_cursor: None,
                runs: vec![RunRecord {
                    id: RunId::new("fixture-run-a").unwrap(),
                    started_at_ms: 2,
                    cursor: "fixture-cursor".into(),
                    status: RunStatus::Succeeded,
                    source: RunSource::LiveRecording,
                    platform: "development".into(),
                    agents: vec!["fake-alpha".into(), "fake-beta".into()],
                    workload: "chat".into(),
                    integrity: Integrity::Verified,
                    concise_result: Some("10 ms".into()),
                }],
                next_cursor: None,
                next_started_at_ms: None,
            },
        )
        .unwrap();
    recent.set_filter("fixture").unwrap();
    assert_eq!(recent.visible_runs().len(), 1);
    transcript.push("history_comparison: compatible recent runs".to_owned());

    assert_eq!(transcript.len(), 10);
    assert!(transcript.iter().all(|line| !line.contains("api_key")));
    fs::remove_dir_all(root).unwrap();
}
