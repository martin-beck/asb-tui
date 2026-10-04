// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    Capabilities,
    fanout_dispatch::{FanoutDispatchState, FanoutSelection, FanoutSelectionError},
    formal_state::{FormalEvent, FormalUiState},
    shell::Route,
    wizard::{FormalEvent as WizardEvent, WizardFormalState},
};

fn capabilities() -> Capabilities {
    Capabilities {
        analysis: true,
        artifacts: true,
        cancel: true,
        events: true,
        history: true,
        launch: true,
        planning: true,
        repeat: true,
    }
}

#[test]
fn authored_model_declares_result_analysis_projection_state() {
    let model = include_str!("../docs/ui-state-model.json");
    assert!(model.contains("\"analysis_summary\""));
}

#[test]
fn documented_transition_sequence_is_executable() {
    let all = capabilities();
    let mut state = FormalUiState::new(100, 30).unwrap();
    state.apply(FormalEvent::OpenConfiguration, None).unwrap();
    state
        .apply(FormalEvent::OpenMeasurementSelection, Some(&all))
        .unwrap();
    state
        .apply(FormalEvent::OpenRunControl, Some(&all))
        .unwrap();
    state
        .apply(FormalEvent::OpenRecentRuns, Some(&all))
        .unwrap();
    state.apply(FormalEvent::OpenReports, Some(&all)).unwrap();
    state.apply(FormalEvent::OpenHelp, Some(&all)).unwrap();
    assert_eq!(state.route(), Route::Help);
    state.apply(FormalEvent::GoBack, Some(&all)).unwrap();
    assert_eq!(state.route(), Route::Reports);
}

#[test]
fn rendered_element_bindings_are_stable_and_model_visible() {
    let model = include_str!("../docs/ui-state-model.json");
    for element in asb_tui::formal_state::RENDERED_ELEMENT_BINDINGS {
        assert!(
            model.contains(element),
            "missing model element binding: {element}"
        );
    }
}

#[test]
fn development_catalog_fallback_is_declared_in_the_formal_model() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    assert!(
        model["state_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "development_catalog_fallback")
    );
}

#[test]
fn development_handoff_actions_are_formally_executable_and_bounded() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    assert!(
        model["state_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "development_handoff")
    );
    let bindings = model["bindings"].as_array().unwrap();
    for action in [
        "open_development_handoff",
        "materialize_development",
        "cancel_development",
        "retry_development",
    ] {
        assert!(bindings.iter().any(|binding| binding["action"] == action));
    }
    let mut state = FormalUiState::new(100, 30).unwrap();
    state
        .apply(FormalEvent::OpenDevelopmentHandoff, None)
        .unwrap();
    state
        .apply(FormalEvent::MaterializeDevelopment, None)
        .unwrap();
    state.apply(FormalEvent::CancelDevelopment, None).unwrap();
    state.apply(FormalEvent::RetryDevelopment, None).unwrap();
    assert_eq!(state.route(), Route::Landing);
}

#[test]
fn stale_live_catalog_publications_are_declared_in_the_formal_model() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    let contract = model["catalog_contract"].as_str().unwrap();
    assert!(contract.contains("digest-validated"));
    assert!(contract.contains("generation-fenced"));
}

#[test]
fn auth_unavailability_provenance_is_declared_in_the_formal_model() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    for field in [
        "auth_unavailable",
        "auth_development_only",
        "auth_unavailable_reason",
    ] {
        assert!(
            model["state_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == field)
        );
    }
}

#[test]
fn provider_setup_review_and_single_apply_are_declared_in_the_formal_model() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    assert!(
        model["state_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "provider_setup_review")
    );
    let catalog_contract = model["catalog_contract"].as_str().unwrap();
    assert!(catalog_contract.contains("catalog-bound"));
    assert!(catalog_contract.contains("at most once"));
    let mut gate = asb_tui::provider_setup::AtomicProviderSetup::default();
    assert_eq!(gate.applied_generation(), None);
    gate.restart();
    assert_eq!(gate.applied_generation(), None);
}

#[test]
fn materialized_campaigns_remain_bound_to_catalog_membership() {
    let measure = asb_tui::selection::BenchmarkMeasure::new("m", "Measure", "count", true).unwrap();
    let benchmark =
        asb_tui::selection::BenchmarkDefinition::new("b", "Benchmark", vec![measure]).unwrap();
    let group = asb_tui::selection::BenchmarkGroup::new("g", "Group", vec![benchmark]).unwrap();
    let pool = asb_tui::selection::BenchmarkPool::new("p", "Pool", vec![group]).unwrap();
    let catalog = asb_tui::selection::BenchmarkCatalog::new(
        asb_tui::control_codec::Revision(1),
        "a".repeat(64),
        vec![pool],
    )
    .unwrap();
    let valid = asb_tui::selection::CampaignSelection {
        generation: asb_tui::control_codec::Revision(1),
        catalog_digest: "a".repeat(64),
        pool_id: "p".into(),
        group_ids: vec!["g".into()],
        benchmark_ids: vec!["b".into()],
        measure_ids: vec!["m".into()],
    };
    assert!(catalog.validate_campaign(&valid).is_ok());
    let mut invalid = valid;
    invalid.measure_ids = vec!["unknown".into()];
    assert!(catalog.validate_campaign(&invalid).is_err());
}

#[test]
fn focus_and_resize_are_bounded_and_atomic() {
    let mut state = FormalUiState::new(100, 30).unwrap();
    state
        .apply(FormalEvent::Focus("landing.primary"), None)
        .unwrap();
    assert_eq!(state.focus(), Some("landing.primary"));
    let before = state.size();
    assert!(
        state
            .apply(
                FormalEvent::Resize {
                    columns: 0,
                    lines: 30
                },
                None
            )
            .is_err()
    );
    assert_eq!(state.size(), before);
}

#[test]
fn every_route_declares_an_executable_resize_transition() {
    let all = capabilities();
    let mut state = FormalUiState::new(100, 30).unwrap();
    for event in [
        FormalEvent::OpenConfiguration,
        FormalEvent::OpenMeasurementSelection,
        FormalEvent::OpenRunControl,
        FormalEvent::OpenRecentRuns,
        FormalEvent::OpenReports,
        FormalEvent::OpenHelp,
    ] {
        state.apply(event, Some(&all)).unwrap();
        let route = state.route();
        state
            .apply(
                FormalEvent::Resize {
                    columns: 120,
                    lines: 40,
                },
                Some(&all),
            )
            .unwrap();
        assert_eq!(state.route(), route);
        assert_eq!(state.size(), (120, 40));
    }
}

#[test]
fn run_control_lifecycle_actions_are_formally_executable() {
    let all = capabilities();
    let mut state = FormalUiState::new(100, 30).unwrap();
    state
        .apply(FormalEvent::OpenMeasurementSelection, Some(&all))
        .unwrap();
    state
        .apply(FormalEvent::OpenRunControl, Some(&all))
        .unwrap();
    state.apply(FormalEvent::StartRun, Some(&all)).unwrap();
    state.apply(FormalEvent::CancelRun, Some(&all)).unwrap();
    state.apply(FormalEvent::Reconnect, Some(&all)).unwrap();
    assert_eq!(state.route(), Route::RunControl);
}

#[test]
fn recording_dispatch_actions_are_declared_in_the_formal_model() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    let fields = model["state_fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|field| field == "recording_dispatch_action")
    );
    assert!(
        fields
            .iter()
            .any(|field| field == "recording_progress_requests")
    );
    let bindings = model["bindings"].as_array().unwrap();
    for action in [
        "refresh_provider_catalog",
        "estimate_recording",
        "plan_recording",
        "confirm_recording_capture",
        "progress_recording",
        "cancel_recording",
        "reconcile_recording",
        "activate_offline_default",
    ] {
        assert!(
            bindings.iter().any(|binding| binding["action"] == action),
            "missing formal binding for {action}"
        );
    }
}

#[test]
fn fanout_control_actions_are_declared_in_the_formal_model() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    assert!(
        model["state_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "fanout_projection")
    );
    let bindings = model["bindings"].as_array().unwrap();
    for action in ["admit_fanout", "cancel_fanout"] {
        assert!(
            bindings.iter().any(|binding| binding["action"] == action),
            "missing formal binding for {action}"
        );
    }
}

#[test]
fn fanout_selection_and_status_projection_are_formally_bounded() {
    let selection = FanoutSelection {
        agent_ids: vec!["agent-b".into(), "agent-a".into(), "agent-a".into()],
        workload_ids: vec!["workload-b".into(), "workload-a".into()],
        provider_id: "provider".into(),
        model_id: "model".into(),
        catalog_digest: "0".repeat(64),
        workload_revision: "0".repeat(64),
        scorer_revision: "0".repeat(64),
    }
    .canonicalize()
    .unwrap();
    assert_eq!(selection.agent_ids, ["agent-a", "agent-b"]);
    assert_eq!(selection.workload_ids, ["workload-a", "workload-b"]);
    assert_eq!(
        FanoutDispatchState::default().report().status,
        "unavailable"
    );
    assert_eq!(
        FanoutSelection {
            agent_ids: Vec::new(),
            workload_ids: vec!["workload".into()],
            ..Default::default()
        }
        .canonicalize(),
        Err(FanoutSelectionError::Empty("agents"))
    );
}

#[test]
fn fanout_behavior_coverage_is_bound_to_the_formal_ui_contract() {
    let model = std::fs::read_to_string("docs/ui-state-model.json").unwrap();
    assert!(model.contains("fanout_request_contract"));
    assert!(model.contains("fanout_live_admission_contract"));
    assert!(model.contains("64-character hexadecimal credential reference"));
    assert!(model.contains("applied-configuration gates are behavior-covered"));
    let inventory = std::fs::read_to_string("docs/ui-module-inventory.json").unwrap();
    assert!(inventory.contains("src/fanout_dispatch.rs"));
    assert!(inventory.contains("bounded identity/recovery coverage"));
    assert!(inventory.contains("applied-configuration gate coverage"));
}

#[test]
fn development_authentication_actions_are_formally_executable() {
    let mut wizard = WizardFormalState::new().unwrap();
    wizard.apply(WizardEvent::OpenWizard).unwrap();
    for value in ["agent", "provider", "model", "configuration"] {
        wizard.apply(WizardEvent::SetValue(value.into())).unwrap();
        wizard.apply(WizardEvent::Next).unwrap();
    }
    wizard.apply(WizardEvent::DevelopmentEnroll).unwrap();
    wizard.apply(WizardEvent::DevelopmentTest).unwrap();
    wizard.apply(WizardEvent::DevelopmentRotate).unwrap();
    wizard.apply(WizardEvent::DevelopmentReset).unwrap();
    wizard.apply(WizardEvent::DevelopmentSelectNone).unwrap();
    assert_eq!(wizard.wizard().values()[4], "none");
    wizard.apply(WizardEvent::DevelopmentSelectFixture).unwrap();
    wizard.apply(WizardEvent::DevelopmentRestart).unwrap();
    assert!(
        wizard
            .wizard()
            .development_auth()
            .credential_locator_sha256
            .is_none()
    );
}
