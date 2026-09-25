// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    Capabilities,
    formal_state::{FormalEvent, FormalUiState},
    shell::Route,
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
fn recording_dispatch_actions_are_declared_in_the_formal_model() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    let fields = model["state_fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|field| field == "recording_dispatch_action")
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
