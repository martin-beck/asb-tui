use asb_tui::reports::*;
use std::collections::BTreeMap;

fn id(v: &str) -> RunId {
    RunId::new(v).unwrap()
}
fn run(name: &str, time: u64) -> RunRecord {
    RunRecord {
        id: id(name),
        started_at_ms: time,
        cursor: format!("c-{name}"),
        status: RunStatus::Succeeded,
        source: RunSource::LiveRecording,
        platform: "linux-x86_64".into(),
        agents: vec!["agent".into()],
        workload: "smoke".into(),
        integrity: Integrity::Verified,
        concise_result: Some("ok".into()),
    }
}
fn report(name: &str, provenance: &str, stale: bool) -> Report {
    let mut measures = BTreeMap::new();
    measures.insert(
        "latency".into(),
        MeasureResult {
            value: Some(1.0),
            unit: "ms".into(),
            status: MeasureStatus::Complete,
        },
    );
    Report {
        run_id: id(name),
        provenance: provenance.into(),
        measures,
        uncertainty: None,
        failures: vec![],
        artifacts: vec![],
        command_argv: vec!["asb".into(), "run".into(), "--name".into(), name.into()],
        stale,
    }
}

#[test]
fn newest_order_uses_id_tie_break_and_search_is_deterministic() {
    let mut state = RecentRuns::default();
    state
        .replace_page(
            "first",
            RecentPage {
                request_cursor: None,
                runs: vec![run("b", 20), run("a", 20), run("z", 10)],
                next_cursor: None,
                next_started_at_ms: None,
            },
        )
        .unwrap();
    assert_eq!(
        state
            .visible_runs()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "a", "z"]
    );
    state.set_filter("SMOKE").unwrap();
    assert_eq!(state.visible_runs().len(), 3);
}

#[test]
fn malformed_page_and_cursor_are_rejected() {
    let mut state = RecentRuns::default();
    let page = RecentPage {
        request_cursor: None,
        runs: vec![run("a", 1)],
        next_cursor: Some("next".into()),
        next_started_at_ms: Some(2),
    };
    assert_eq!(
        state.replace_page("x", page),
        Err(ValidationError::ContradictoryCursor)
    );
    let page = RecentPage {
        request_cursor: None,
        runs: vec![run("a", 1), run("b", 2)],
        next_cursor: None,
        next_started_at_ms: None,
    };
    assert_eq!(
        state.replace_page("x", page),
        Err(ValidationError::UnorderedPage)
    );
}

#[test]
fn selection_prunes_deleted_runs_and_compare_is_fail_closed() {
    let mut state = RecentRuns::default();
    state
        .replace_page(
            "x",
            RecentPage {
                request_cursor: None,
                runs: vec![run("a", 2), run("b", 1)],
                next_cursor: None,
                next_started_at_ms: None,
            },
        )
        .unwrap();
    state.select(&id("a"), true).unwrap();
    state
        .replace_page(
            "x",
            RecentPage {
                request_cursor: None,
                runs: vec![run("b", 1)],
                next_cursor: None,
                next_started_at_ms: None,
            },
        )
        .unwrap();
    assert!(state.selected.is_empty());
    assert_eq!(
        compare(&[report("a", "p", false)], &[id("a"), id("deleted")]),
        Err(CompareError::StaleOrDeleted)
    );
    assert_eq!(
        compare(
            &[report("a", "p", true), report("b", "p", false)],
            &[id("a"), id("b")]
        ),
        Err(CompareError::StaleOrDeleted)
    );
    assert_eq!(
        compare(
            &[report("a", "p", false), report("b", "p", false)],
            &[id("a"), id("a")]
        ),
        Err(CompareError::Duplicate)
    );
}

#[test]
fn comparison_reports_conflicts_and_command_is_quoted() {
    let a = report("a", "p", false);
    let mut b = report("b", "q", false);
    b.command_argv.push("--label=hello world".into());
    let comparison = compare(&[a.clone(), b.clone()], &[id("b"), id("a")]).unwrap();
    assert!(!comparison.compatible);
    assert_eq!(comparison.run_ids, vec![id("b"), id("a")]);
    assert!(b.shell_command().unwrap().contains("'--label=hello world'"));
}

#[test]
fn bounded_page_retention_is_enforced() {
    let mut state = RecentRuns::default();
    for n in 0..(MAX_RETAINED_PAGES + 2) {
        state
            .replace_page(
                format!("{n:02}"),
                RecentPage {
                    request_cursor: None,
                    runs: vec![run(&format!("r{n}"), n as u64)],
                    next_cursor: None,
                    next_started_at_ms: None,
                },
            )
            .unwrap();
    }
    assert_eq!(state.pages().count(), MAX_RETAINED_PAGES);
}

#[test]
fn command_projection_rejects_private_paths_credentials_and_environment_expansion() {
    for forbidden in [
        "/home/private/run.json",
        "$ASB_TOKEN",
        "--password",
        "super-secret",
    ] {
        let mut candidate = report("a", "p", false);
        candidate.command_argv = vec!["asb".into(), forbidden.into()];
        assert_eq!(
            candidate.shell_command(),
            Err(ValidationError::SensitiveCommand)
        );
    }
}
