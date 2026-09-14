// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{Capabilities, help::HelpModel, shell::Route};

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
