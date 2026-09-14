// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
use asb_tui::selection::{GroupSelection, Measurement, MeasurementSelection, SelectionError};

fn catalog() -> MeasurementSelection {
    MeasurementSelection::new(vec![
        Measurement::new("cpu.user", "CPU", "User time", "ns").unwrap(),
        Measurement::new("memory.rss", "Memory", "Resident set", "bytes").unwrap(),
        Measurement::new("cpu.system", "CPU", "System time", "ns").unwrap(),
    ])
    .unwrap()
}

#[test]
fn canonical_order_and_group_tri_state_are_deterministic() {
    let mut state = catalog();
    assert_eq!(
        state
            .measurements()
            .iter()
            .map(Measurement::id)
            .collect::<Vec<_>>(),
        ["cpu.system", "cpu.user", "memory.rss"]
    );
    assert_eq!(state.visible_groups()[0].state(), GroupSelection::None);
    state.set_measure_selected("cpu.system", true).unwrap();
    assert_eq!(state.visible_groups()[0].state(), GroupSelection::Partial);
    state.set_group_selected("CPU", true).unwrap();
    assert_eq!(state.visible_groups()[0].state(), GroupSelection::All);
    assert_eq!(state.selected_ids(), ["cpu.system", "cpu.user"]);
}

#[test]
fn search_filters_groups_without_destroying_selection() {
    let mut state = catalog();
    state.set_group_selected("CPU", true).unwrap();
    state.set_query("rss").unwrap();
    assert_eq!(
        state
            .visible_measurements()
            .iter()
            .map(|m| m.id())
            .collect::<Vec<_>>(),
        ["memory.rss"]
    );
    assert_eq!(state.visible_groups()[0].state(), GroupSelection::None);
    state.set_query("CPU").unwrap();
    assert_eq!(state.visible_groups()[0].state(), GroupSelection::All);
}

#[test]
fn filtered_group_toggle_changes_only_visible_members() {
    let mut state = catalog();
    state.set_group_selected("CPU", true).unwrap();
    state.set_query("user").unwrap();
    state.set_visible_group_selected("CPU", false).unwrap();
    assert!(!state.is_selected("cpu.user"));
    assert!(state.is_selected("cpu.system"));
    state.set_group_selected("CPU", false).unwrap();
    assert_eq!(state.selected_ids(), Vec::<&str>::new());
}

#[test]
fn query_accepts_bounded_unicode_scalars_and_rejects_controls_or_overflow() {
    let mut state = catalog();
    state.set_query("温度").unwrap();
    assert_eq!(state.query(), "温度");
    state
        .set_query("温".repeat(asb_tui::selection::MAX_QUERY_BYTES))
        .unwrap();
    assert!(matches!(
        state.set_query("温".repeat(asb_tui::selection::MAX_QUERY_BYTES + 1)),
        Err(SelectionError::FieldTooLong("query"))
    ));
    assert!(matches!(
        state.set_query("温\u{7f}度"),
        Err(SelectionError::NonPublicText("query"))
    ));
}

#[test]
fn invalid_and_unknown_inputs_fail_closed() {
    assert_eq!(
        Measurement::new("", "CPU", "name", "ns"),
        Err(SelectionError::EmptyField("id"))
    );
    let duplicate = vec![
        Measurement::new("same", "a", "one", "u").unwrap(),
        Measurement::new("same", "b", "two", "u").unwrap(),
    ];
    assert_eq!(
        MeasurementSelection::new(duplicate),
        Err(SelectionError::DuplicateId)
    );
    let non_adjacent_duplicate = vec![
        Measurement::new("cross-group", "a", "one", "u").unwrap(),
        Measurement::new("other", "a", "two", "u").unwrap(),
        Measurement::new("cross-group", "b", "three", "u").unwrap(),
    ];
    assert_eq!(
        MeasurementSelection::new(non_adjacent_duplicate),
        Err(SelectionError::DuplicateId)
    );
    let mut state = catalog();
    assert_eq!(
        state.set_measure_selected("nope", true),
        Err(SelectionError::UnknownMeasure)
    );
    assert_eq!(
        state.set_group_selected("nope", true),
        Err(SelectionError::UnknownGroup)
    );
    assert!(matches!(
        state.set_query("\n"),
        Err(SelectionError::NonPublicText("query"))
    ));
}
