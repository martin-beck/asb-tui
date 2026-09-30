// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! AR-1327's executable local qualification harness.  Every operation here is
//! an in-process fixture: no credentials are read and no network is opened.

use asb_tui::{
    benchmark_route::{GuidedCampaign, GuidedCatalog, ReplayMode},
    development_journey::{FixtureRequest, evaluate_fixture, execute_input},
    development_onboarding, development_router,
    reports::{
        Artifact, EvidenceKind, MeasureResult, MeasureStatus, Report, ReportStatus, RunId, compare,
    },
    wizard::{Step, Wizard},
    wizard_catalog::WizardCatalog,
};
use std::{
    collections::BTreeMap,
    fs,
    io::Cursor,
    time::{SystemTime, UNIX_EPOCH},
};

fn fixture_request() -> FixtureRequest {
    FixtureRequest {
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
    }
}

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

struct MismatchCase {
    code: &'static str,
    recovery: &'static str,
    mutate: fn(&mut FixtureRequest),
}

fn temporary_install_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "asb-tui-ar1327-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("versions")).unwrap();
    fs::write(root.join(".lifecycle.lock"), b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(root.join("versions"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(
            root.join(".lifecycle.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    root
}

#[test]
fn clean_fixture_composes_install_wizard_benchmark_replay_and_comparison() {
    let response = execute_input(Cursor::new(serde_json::to_vec(&fixture_request()).unwrap()));
    assert!(response.ready);
    assert!(response.development_only);
    assert_eq!(
        (response.code, response.recovery),
        ("development_journey_ready", "continue")
    );

    let onboarding = development_onboarding::execute_input(
        &br#"{"schema_version":1,"profile":"development","bundle_available":true,"broker_available":true,"protocol_version":1}"#[..],
    );
    assert!(onboarding.ready);
    assert!(onboarding.development_only);

    let root = temporary_install_root();
    let router_request = format!(
        r#"{{"router_version":1,"profile":"development","request":{{"operation":"status","schema_version":1,"install_root":{root:?}}}}}"#,
        root = root.to_string_lossy()
    );
    let router = development_router::execute_input(router_request.as_bytes());
    assert!(router.development_only);
    assert_eq!(router.lifecycle.code, "extension_not_installed");
    fs::remove_dir_all(root).unwrap();

    let mut wizard = Wizard::with_catalog(WizardCatalog::development().unwrap());
    assert_eq!(wizard.step(), Step::Agent);
    wizard.select_all_agents().unwrap();
    for value in [
        "development",
        "fixture-model",
        "development defaults",
        "development fixture",
        "mock capture",
        "strict offline replay",
    ] {
        wizard.advance().unwrap();
        wizard.set_value(value).unwrap();
    }
    wizard.advance().unwrap();
    assert_eq!(wizard.step(), Step::Review);
    wizard.complete().unwrap();

    let catalog = GuidedCatalog::new(
        vec!["fixture-workload".into()],
        vec!["fake-alpha".into(), "fake-beta".into()],
        vec!["fixture.latency".into()],
    )
    .unwrap();
    let mut campaign = GuidedCampaign::with_catalog("fixture-workload", catalog.clone()).unwrap();
    campaign.add_agent("fake-alpha").unwrap();
    campaign.add_agent("fake-beta").unwrap();
    campaign.add_measure("fixture.latency").unwrap();
    campaign.review().unwrap();
    let launch = campaign.start().unwrap();
    assert_eq!(launch.agents, ["fake-alpha", "fake-beta"]);
    campaign.finish(true).unwrap();

    let mut replay = GuidedCampaign::with_catalog("fixture-workload", catalog).unwrap();
    replay.add_agent("fake-alpha").unwrap();
    replay.add_agent("fake-beta").unwrap();
    replay.add_measure("fixture.latency").unwrap();
    replay.set_replay_mode(ReplayMode::OfflineReplay).unwrap();
    replay.review().unwrap();
    let replay_intent = replay.start_offline_replay("fixture-cassette").unwrap();
    assert!(replay_intent.offline_only);
    replay.finish(true).unwrap();

    let reports = [report("fixture-run-a", 10.0), report("fixture-run-b", 11.0)];
    let comparison = compare(
        &reports,
        &[reports[0].run_id.clone(), reports[1].run_id.clone()],
    )
    .unwrap();
    assert!(comparison.compatible);
    assert_eq!(
        comparison.run_ids,
        [reports[0].run_id.clone(), reports[1].run_id.clone()]
    );
    assert!(
        reports
            .iter()
            .all(|item| item.provenance == "development/mock/offline-replay")
    );
}

#[test]
fn mismatch_fixtures_are_non_executable_and_offer_one_recovery() {
    let cases = [
        MismatchCase {
            code: "asb_version_mismatch",
            recovery: "refresh_version",
            mutate: |value| value.asb_version = "fixture-0".into(),
        },
        MismatchCase {
            code: "catalog_mismatch",
            recovery: "refresh_catalog",
            mutate: |value| value.catalog_version = 6,
        },
        MismatchCase {
            code: "development_bundle_unavailable",
            recovery: "repair_bundle",
            mutate: |value| value.bundle_available = false,
        },
        MismatchCase {
            code: "development_protocol_mismatch",
            recovery: "retry_broker",
            mutate: |value| value.protocol_version = Some(2),
        },
    ];
    for MismatchCase {
        code,
        recovery,
        mutate,
    } in cases
    {
        let mut request = fixture_request();
        mutate(&mut request);
        let response = evaluate_fixture(request);
        assert!(!response.ready);
        assert_eq!((response.code, response.recovery), (code, recovery));
    }
}
