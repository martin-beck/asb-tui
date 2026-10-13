// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Reviewed, privacy-safe text-grid and capability regression fixtures.

use asb_tui::{
    control_codec::{AttemptId, PublicRunState, Revision, RunId, RunSummary},
    live_projection::{Connection, LiveSnapshot},
    terminal::{CapabilityTier, RenderPolicy},
    ui::{Screen, WorkspaceState},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, style::Color};

#[derive(Clone, Copy)]
struct GoldenCase {
    id: &'static str,
    width: u16,
    height: u16,
    policy: RenderPolicy,
    state: fn() -> WorkspaceState,
}

type StateFactory = fn() -> WorkspaceState;

const fn policy(tier: CapabilityTier, unicode: bool) -> RenderPolicy {
    RenderPolicy {
        tier,
        unicode,
        mouse: false,
        focus: false,
        bracketed_paste: false,
        synchronized_output: false,
        alternate_screen: true,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn report_snapshot() -> LiveSnapshot {
    LiveSnapshot {
        connection: Connection::Negotiated,
        runner_instance_id: Some("runner-visual".into()),
        latest_revision: Some(Revision(2)),
        capabilities: None,
        measurement_catalog: None,
        benchmark_catalog: None,
        agent_catalog: None,
        agent_lifecycle: None,
        provider_catalog: None,
        dynamic_provider_catalog: None,
        configuration: None,
        auth_status: None,
        auth_unavailable: false,
        auth_development_only: false,
        auth_unavailable_reason: None,
        recording_campaign: None,
        recording_estimate: None,
        recording_campaign_lifecycle: None,
        fanout: None,
        runs: vec![
            RunSummary {
                run_id: RunId("run-a".into()),
                attempt_id: AttemptId("attempt-a".into()),
                state: PublicRunState::Completed,
                created_revision: Revision(1),
                revision: Revision(1),
                plan_sha256: "a".repeat(64),
            },
            RunSummary {
                run_id: RunId("run-b".into()),
                attempt_id: AttemptId("attempt-b".into()),
                state: PublicRunState::Failed,
                created_revision: Revision(2),
                revision: Revision(2),
                plan_sha256: "b".repeat(64),
            },
        ],
        analysis: None,
    }
}

fn landing() -> WorkspaceState {
    WorkspaceState::default()
}

fn wizard() -> WorkspaceState {
    let mut state = WorkspaceState::for_startup(false);
    state.handle_key(key(KeyCode::Char('c')));
    state.handle_key(key(KeyCode::Char('o')));
    state
}

fn measures() -> WorkspaceState {
    let mut state = WorkspaceState::default();
    state.screen = Screen::Measures;
    state.handle_key(key(KeyCode::Char('/')));
    for character in "latency".chars() {
        state.handle_key(key(KeyCode::Char(character)));
    }
    state.handle_key(key(KeyCode::Enter));
    state
}

fn configuration() -> WorkspaceState {
    let mut state = WorkspaceState::default();
    state.screen = Screen::Configuration;
    state.handle_key(key(KeyCode::Enter));
    state.handle_key(key(KeyCode::Char('-')));
    state.handle_key(key(KeyCode::Char('v')));
    state.handle_key(key(KeyCode::Char('2')));
    state
}

fn run_control() -> WorkspaceState {
    let mut state = WorkspaceState::default();
    state.screen = Screen::RunControl;
    state
}

fn reports() -> WorkspaceState {
    let mut state = WorkspaceState::default();
    state.screen = Screen::Reports;
    state.apply_live_snapshot(report_snapshot());
    state
}

fn help() -> WorkspaceState {
    let mut state = WorkspaceState::default();
    state.screen = Screen::Help;
    state.handle_key(key(KeyCode::F(1)));
    state
}

fn narrow_overlay() -> WorkspaceState {
    let mut state = configuration();
    state.handle_key(key(KeyCode::F(1)));
    state
}

const CASES: &[GoldenCase] = &[
    GoldenCase {
        id: "landing-wide-color",
        width: 140,
        height: 36,
        policy: policy(CapabilityTier::TrueColor, true),
        state: landing,
    },
    GoldenCase {
        id: "wizard-ordinary-mono",
        width: 100,
        height: 24,
        policy: policy(CapabilityTier::Plain, true),
        state: wizard,
    },
    GoldenCase {
        id: "measures-narrow-ascii",
        width: 60,
        height: 16,
        policy: policy(CapabilityTier::Plain, false),
        state: measures,
    },
    GoldenCase {
        id: "configuration-ordinary-color",
        width: 100,
        height: 24,
        policy: policy(CapabilityTier::TrueColor, true),
        state: configuration,
    },
    GoldenCase {
        id: "run-minimum-ascii",
        width: 32,
        height: 8,
        policy: policy(CapabilityTier::Plain, false),
        state: run_control,
    },
    GoldenCase {
        id: "reports-wide-color",
        width: 140,
        height: 36,
        policy: policy(CapabilityTier::TrueColor, true),
        state: reports,
    },
    GoldenCase {
        id: "help-ordinary-mono",
        width: 100,
        height: 24,
        policy: policy(CapabilityTier::Plain, true),
        state: help,
    },
    GoldenCase {
        id: "overlay-minimum-ascii",
        width: 32,
        height: 8,
        policy: policy(CapabilityTier::Plain, false),
        state: narrow_overlay,
    },
];

fn render(case: GoldenCase) -> (String, Vec<Color>) {
    let mut terminal = Terminal::new(TestBackend::new(case.width, case.height)).unwrap();
    let state = (case.state)();
    terminal
        .draw(|frame| asb_tui::ui::render(frame, &state, case.policy))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut rows = (0..case.height)
        .map(|y| {
            (0..case.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    let colors = buffer.content().iter().map(|cell| cell.fg).collect();
    (format!("{}\n", rows.join("\n")), colors)
}

#[test]
fn reviewed_text_grid_goldens_are_exact_bounded_and_private() {
    let print = std::env::var_os("ASB_TUI_PRINT_VISUAL_GOLDENS").is_some();
    for case in CASES {
        let (actual, _) = render(*case);
        if print {
            println!(
                "--- ASB_VISUAL_GOLDEN:{} ---\n{}--- ASB_VISUAL_END:{} ---",
                case.id, actual, case.id
            );
            continue;
        }
        let path = format!(
            "{}/tests/goldens/visual/{}.txt",
            env!("CARGO_MANIFEST_DIR"),
            case.id
        );
        let expected = std::fs::read_to_string(path).unwrap();
        assert_eq!(actual, expected, "visual golden {} drifted", case.id);
        assert!(actual.len() <= 32_768);
        assert!(!actual.contains("/home/") && !actual.contains("api_key"));
        assert!(!actual.contains('\u{1b}') && !actual.contains('\0'));
        if !case.policy.unicode {
            assert!(
                actual.is_ascii(),
                "ASCII fixture {} emitted Unicode",
                case.id
            );
        }
    }
}

#[test]
fn color_monochrome_and_ascii_keep_semantic_selection_and_focus() {
    for capability in [
        policy(CapabilityTier::TrueColor, true),
        policy(CapabilityTier::Plain, true),
        policy(CapabilityTier::Plain, false),
    ] {
        let case = GoldenCase {
            id: "capability",
            width: 100,
            height: 24,
            policy: capability,
            state: measures,
        };
        let (text, colors) = render(case);
        assert!(text.contains("[x]"));
        assert!(text.contains('>'));
        assert!(text.contains("Selection summary"));
        assert!(text.contains("Measure:"));
        if matches!(capability.tier, CapabilityTier::Plain) {
            assert!(colors.into_iter().all(|color| color == Color::Reset));
        } else {
            assert!(colors.into_iter().any(|color| color != Color::Reset));
        }
        if !capability.unicode {
            assert!(text.is_ascii());
        }
    }
}

#[test]
fn route_dimension_capability_matrix_renders_without_losing_escape_or_help() {
    let state_factories: &[(&str, StateFactory)] = &[
        ("landing", landing),
        ("wizard", wizard),
        ("measures", measures),
        ("configuration", configuration),
        ("run_control", run_control),
        ("reports", reports),
        ("help", help),
    ];
    for (route, state) in state_factories {
        for (width, height) in [(140, 36), (100, 24), (60, 16), (32, 8)] {
            for capability in [
                policy(CapabilityTier::TrueColor, true),
                policy(CapabilityTier::Plain, true),
                policy(CapabilityTier::Plain, false),
            ] {
                let case = GoldenCase {
                    id: "matrix",
                    width,
                    height,
                    policy: capability,
                    state: *state,
                };
                let (text, _) = render(case);
                assert!(!text.is_empty());
                assert!(
                    text.contains("F1")
                        || text.contains("help")
                        || text.contains("Help")
                        || text.contains("ASB setup wizard"),
                    "route {route} lost help at {width}x{height} under {capability:?}: {text:?}"
                );
                assert!(!text.contains("/home/") && !text.contains("Bearer "));
                if !capability.unicode {
                    assert!(text.is_ascii(), "ASCII matrix fixture emitted Unicode");
                }
            }
        }
    }
}

#[test]
fn bounded_resize_property_covers_threshold_edges_without_overflow() {
    let widths = [1, 2, 31, 32, 39, 40, 59, 60, 79, 80, 99, 100, 119, 120, 140];
    let heights = [1, 2, 7, 8, 9, 15, 16, 19, 20, 23, 24, 36];
    for state in [landing as StateFactory, wizard, measures] {
        for width in widths {
            for height in heights {
                for capability in [
                    policy(CapabilityTier::TrueColor, true),
                    policy(CapabilityTier::Plain, false),
                ] {
                    let case = GoldenCase {
                        id: "bounded-property",
                        width,
                        height,
                        policy: capability,
                        state,
                    };
                    let (text, _) = render(case);
                    let rows = text.trim_end_matches('\n').split('\n').collect::<Vec<_>>();
                    assert!(rows.len() <= usize::from(height.max(1)));
                    assert!(
                        rows.iter()
                            .all(|row| row.chars().count() <= usize::from(width.max(1)))
                    );
                    if !capability.unicode {
                        assert!(text.is_ascii());
                    }
                }
            }
        }
    }
}
