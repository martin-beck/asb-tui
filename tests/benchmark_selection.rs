// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
use asb_tui::control_codec::Revision;
use asb_tui::selection::{
    BenchmarkCatalog, BenchmarkDefinition, BenchmarkGroup, BenchmarkMeasure, BenchmarkPool,
    BenchmarkSelection, NodeSelection, SelectionError,
};

fn catalog(generation: u64) -> BenchmarkCatalog {
    let measure = |id: &str, available| BenchmarkMeasure::new(id, id, "score", available).unwrap();
    let benchmark = |id: &str| {
        BenchmarkDefinition::new(
            id,
            id,
            vec![
                measure(&format!("{id}.quality"), true),
                measure(&format!("{id}.latency"), true),
            ],
        )
        .unwrap()
    };
    BenchmarkCatalog::new(
        Revision(generation),
        format!("catalog-{generation}"),
        vec![
            BenchmarkPool::new(
                "standard",
                "Standard pool",
                vec![
                    BenchmarkGroup::new("quality", "Quality", vec![benchmark("assistant")])
                        .unwrap(),
                ],
            )
            .unwrap(),
            BenchmarkPool::new(
                "extended",
                "Extended pool",
                vec![
                    BenchmarkGroup::new(
                        "safety",
                        "Safety",
                        vec![
                            BenchmarkDefinition::new(
                                "guarded",
                                "Guarded",
                                vec![measure("guarded.policy", true)],
                            )
                            .unwrap(),
                        ],
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn nested_selection_search_and_exact_handoff_are_deterministic() {
    let mut selection = BenchmarkSelection::new(catalog(4)).unwrap();
    selection.set_query("latency").unwrap();
    assert_eq!(selection.visible_measure_ids(), ["assistant.latency"]);
    selection.set_benchmark_selected("assistant", true).unwrap();
    let handoff = selection.campaign_handoff(Revision(4)).unwrap();
    assert_eq!(handoff.pool_id, "standard");
    assert_eq!(handoff.group_ids, ["quality"]);
    assert_eq!(handoff.benchmark_ids, ["assistant"]);
    assert_eq!(
        handoff.measure_ids,
        ["assistant.quality", "assistant.latency"]
    );
}

#[test]
fn hidden_measurements_are_not_changed_and_generation_is_fenced() {
    let mut selection = BenchmarkSelection::new(catalog(4)).unwrap();
    selection.set_query("quality").unwrap();
    selection
        .set_measure_selected("assistant.quality", true)
        .unwrap();
    selection.set_query("latency").unwrap();
    assert_eq!(selection.visible_measure_ids(), ["assistant.latency"]);
    assert!(selection.is_measure_selected("assistant.quality"));
    assert_eq!(
        selection.campaign_handoff(Revision(3)),
        Err(SelectionError::StaleCatalog)
    );
    let dropped = selection.replace_catalog(catalog(5)).unwrap();
    assert!(dropped.is_empty());
    assert_eq!(
        selection.campaign_handoff(Revision(4)),
        Err(SelectionError::StaleCatalog)
    );
    assert_eq!(
        selection.campaign_handoff(Revision(5)).unwrap().measure_ids,
        ["assistant.quality"]
    );
}

#[test]
fn nested_group_and_benchmark_states_are_tri_state_and_pool_scoped() {
    let mut selection = BenchmarkSelection::new(catalog(4)).unwrap();
    assert_eq!(
        selection.group_state("quality").unwrap(),
        NodeSelection::None
    );
    assert_eq!(
        selection.benchmark_state("assistant").unwrap(),
        NodeSelection::None
    );
    selection
        .set_measure_selected("assistant.quality", true)
        .unwrap();
    assert_eq!(
        selection.group_state("quality").unwrap(),
        NodeSelection::Partial
    );
    assert_eq!(
        selection.benchmark_state("assistant").unwrap(),
        NodeSelection::Partial
    );
    selection.set_group_selected("quality", true).unwrap();
    assert_eq!(
        selection.group_state("quality").unwrap(),
        NodeSelection::All
    );
    assert_eq!(
        selection.benchmark_state("assistant").unwrap(),
        NodeSelection::All
    );
    selection.select_pool("extended").unwrap();
    assert_eq!(
        selection.set_measure_selected("assistant.quality", true),
        Err(SelectionError::UnknownMeasure)
    );
    assert!(selection.selected_measure_ids().is_empty());
}

#[test]
fn empty_and_unavailable_choices_fail_closed() {
    let unavailable = BenchmarkMeasure::new("x.measure", "Measure", "count", false).unwrap();
    let available = BenchmarkMeasure::new("x.other", "Other", "count", true).unwrap();
    let bench = BenchmarkDefinition::new("bench", "Bench", vec![unavailable, available]).unwrap();
    let cat = BenchmarkCatalog::new(
        Revision(1),
        "catalog",
        vec![
            BenchmarkPool::new(
                "pool",
                "Pool",
                vec![BenchmarkGroup::new("group", "Group", vec![bench]).unwrap()],
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let mut selection = BenchmarkSelection::new(cat).unwrap();
    assert_eq!(
        selection.set_benchmark_selected("bench", true),
        Err(SelectionError::UnavailableMeasure)
    );
    assert!(!selection.is_measure_selected("x.other"));
    assert_eq!(
        selection.campaign_handoff(Revision(1)),
        Err(SelectionError::NoSelection)
    );
}
