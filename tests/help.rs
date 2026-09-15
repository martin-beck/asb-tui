// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    Capabilities,
    actions::{ActionRegistry, KeyChord},
    help::HelpModel,
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
fn contextual_entries_include_disabled_actions_and_search_is_global() {
    let mut help = HelpModel::new();
    let entries = help.entries(Route::Landing, Some(&capabilities()));
    assert!(entries.iter().any(|entry| entry.action.id() == "open_help"));
    help.set_query("compare").unwrap();
    assert_eq!(
        help.entries(Route::Landing, None)
            .iter()
            .map(|e| e.action.id())
            .collect::<Vec<_>>(),
        ["compare_runs"]
    );
}

#[test]
fn query_and_selection_are_bounded_and_deterministic() {
    let mut help = HelpModel::new();
    assert!(help.set_query("bad\nquery").is_err());
    assert!(help.set_query(&"x".repeat(129)).is_err());
    help.move_selection(1, 3);
    assert_eq!(help.selected(3), 1);
    help.move_selection(-2, 3);
    assert_eq!(help.selected(3), 2);
    help.move_selection(1, 0);
    assert_eq!(help.selected(0), 0);
}

#[test]
fn hotkey_labels_keep_space_and_unknown_keys_readable() {
    assert_eq!(KeyChord::Char(' ').label(), "Space");
    assert_eq!(KeyChord::Char('x').label(), "key");
    assert_eq!(KeyChord::Ctrl('x').label(), "Ctrl-key");
}

#[test]
fn contextual_help_explains_each_backend_capability_gate() {
    let mut caps = capabilities();
    caps.cancel = false;
    caps.events = false;
    caps.history = false;
    caps.analysis = false;
    caps.launch = false;
    caps.planning = false;
    for (route, action) in [
        (Route::RunControl, asb_tui::actions::UiAction::StartRun),
        (Route::RunControl, asb_tui::actions::UiAction::CancelRun),
        (Route::RecentRuns, asb_tui::actions::UiAction::RefreshRuns),
        (Route::Reports, asb_tui::actions::UiAction::CompareRuns),
        (
            Route::MeasurementSelection,
            asb_tui::actions::UiAction::ToggleMeasure,
        ),
    ] {
        let entries = ActionRegistry::for_context(route, Some(&caps));
        assert!(
            entries
                .iter()
                .any(|entry| entry.action == action && entry.disabled.is_some())
        );
    }
    // Exercise the more specific second-order gates after the primary
    // capability is present: cancellation needs event streaming, and report
    // comparison needs analysis in addition to history.
    let mut eventless = capabilities();
    eventless.events = false;
    assert!(
        ActionRegistry::for_context(Route::RunControl, Some(&eventless))
            .iter()
            .any(|entry| {
                entry.action == asb_tui::actions::UiAction::CancelRun && entry.disabled.is_some()
            })
    );
    let mut no_analysis = capabilities();
    no_analysis.analysis = false;
    assert!(
        ActionRegistry::for_context(Route::Reports, Some(&no_analysis))
            .iter()
            .any(|entry| {
                entry.action == asb_tui::actions::UiAction::CompareRuns && entry.disabled.is_some()
            })
    );
}
