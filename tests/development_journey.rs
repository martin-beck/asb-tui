// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Executable, credential-free qualification for the standalone development
//! journey.  This is local/mock evidence only; it does not claim ASB runtime
//! or provider reachability.

use asb_tui::{
    recording_campaign::{
        CampaignObservation, CampaignPhase, RecordingAction, RecordingCampaignModel, WorkloadScope,
    },
    reports::{
        Artifact, EvidenceKind, MeasureResult, MeasureStatus, Report, ReportStatus, RunId, compare,
    },
    wizard::{StartupRoute, Step, Wizard, startup_route},
    wizard_catalog::{OptionKind, WizardCatalog},
};
use std::collections::BTreeMap;

fn report(run_id: &str, value: f64) -> Report {
    Report {
        run_id: RunId::new(run_id).unwrap(),
        provenance: "development/mock/offline-replay".into(),
        measures: BTreeMap::from([(
            "fixture.latency".into(),
            MeasureResult {
                value: Some(value),
                unit: "ms".into(),
                status: MeasureStatus::Complete,
            },
        )]),
        uncertainty: None,
        failures: Vec::new(),
        artifacts: vec![Artifact {
            name: "strict-replay-cassette".into(),
            available: true,
        }],
        command_argv: vec!["asb".into(), "replay".into(), "--offline".into()],
        stale: false,
        evidence: EvidenceKind::Replay,
        status: ReportStatus::Complete,
        compatibility_key: "development/provider/model/catalog/config".into(),
    }
}

#[test]
fn development_journey_runs_without_credentials_or_network() {
    assert_eq!(startup_route(false), StartupRoute::Wizard);

    let mut wizard = Wizard::with_catalog(WizardCatalog::development().unwrap());
    assert_eq!(wizard.step(), Step::Agent);
    wizard.select_all_agents().unwrap();
    assert_eq!(
        wizard.values()[Step::Agent as usize],
        "fake-alpha,fake-beta"
    );

    wizard.advance().unwrap();
    assert_eq!(wizard.catalog().unwrap().kind(), OptionKind::Provider);
    wizard.select_catalog_cursor().unwrap();
    assert_eq!(wizard.values()[Step::Provider as usize], "development");

    wizard.advance().unwrap();
    assert_eq!(wizard.catalog().unwrap().kind(), OptionKind::Model);
    wizard.select_catalog_cursor().unwrap();
    assert_eq!(wizard.values()[Step::Model as usize], "fixture-model");

    wizard.advance().unwrap();
    wizard.set_value("all defaults").unwrap();
    wizard.advance().unwrap();
    wizard.select_development_fixture().unwrap();
    wizard.enroll_development_credential().unwrap();
    wizard.test_development_credential().unwrap();
    let auth = wizard.development_auth();
    assert!(auth.development_only);
    assert!(auth.warning.contains("not production guarantees"));
    assert!(auth.credential_locator_sha256.is_some());
    assert!(!wizard.values()[Step::Authentication as usize].contains("api_key"));
    wizard.advance().unwrap();

    for (step, value) in [
        (Step::Recording, "local-mock"),
        (Step::Replay, "strict offline replay"),
    ] {
        assert_eq!(wizard.step(), step);
        wizard.set_value(value).unwrap();
        wizard.advance().unwrap();
    }
    assert_eq!(wizard.step(), Step::Review);
    wizard.complete().unwrap();
}

#[test]
fn development_capture_replay_and_comparison_are_renderer_neutral() {
    let planned = CampaignObservation {
        generation: 1,
        campaign_id: Some("development-campaign".into()),
        phase: CampaignPhase::Planned,
        coverage: asb_tui::recording_campaign::Coverage {
            requested: 2,
            complete: 0,
        },
        offline_ready: false,
    };
    let selected_scope =
        WorkloadScope::Selected(vec!["workload-alpha".into(), "workload-beta".into()])
            .canonical()
            .unwrap();
    let mut campaign = RecordingCampaignModel::new(selected_scope.clone(), planned).unwrap();
    assert_eq!(campaign.scope(), &selected_scope);
    campaign.arm_capture_confirmation().unwrap();
    assert_eq!(
        campaign
            .request(RecordingAction::ConfirmCapture)
            .unwrap()
            .action,
        RecordingAction::ConfirmCapture
    );
    campaign
        .apply(CampaignObservation {
            generation: 2,
            campaign_id: Some("development-campaign".into()),
            phase: CampaignPhase::Complete,
            coverage: asb_tui::recording_campaign::Coverage {
                requested: 2,
                complete: 2,
            },
            offline_ready: true,
        })
        .unwrap();
    assert_eq!(
        campaign
            .request(RecordingAction::ActivateOfflineDefault)
            .unwrap()
            .action,
        RecordingAction::ActivateOfflineDefault
    );
    assert!(campaign.observation().offline_ready);

    let reports = [
        report("development-replay-001", 10.0),
        report("development-replay-002", 11.0),
    ];
    let comparison = compare(
        &reports,
        &[reports[0].run_id.clone(), reports[1].run_id.clone()],
    )
    .unwrap();
    assert!(comparison.compatible);
    assert!(comparison.excluded_measures.is_empty());
}
