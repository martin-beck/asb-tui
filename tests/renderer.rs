// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic TestBackend coverage for the Ratatui renderer.

use asb_tui::{
    app::{Action, AppState, ControlEvent, ControlEventKind},
    help::HelpModel,
    landing::{
        ActivityRecord, ActivityStatus, ConnectionStatus, Freshness, LandingInput, RunState,
        TrustStatus, project,
    },
    renderer,
    shell::Route,
    terminal::{CapabilityTier, RenderPolicy},
};
use ratatui::{Terminal, backend::TestBackend, style::Color};

fn policy(tier: CapabilityTier) -> RenderPolicy {
    RenderPolicy {
        tier,
        unicode: !matches!(tier, CapabilityTier::Plain),
        mouse: false,
        focus: false,
        bracketed_paste: false,
        synchronized_output: false,
        alternate_screen: !matches!(tier, CapabilityTier::Plain),
    }
}

fn snapshot(state: &AppState, tier: CapabilityTier, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| renderer::render(frame, state, policy(tier)))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn compact_standard_and_wide_snapshots_are_stable() {
    let compact = snapshot(
        &AppState::new(32, 8).unwrap(),
        CapabilityTier::BasicColor,
        32,
        8,
    );
    assert_eq!(snapshot_hash(&compact), 0x7032_fa60_ee3d_843e);
    assert!(compact.starts_with("┌"));
    assert!(compact.contains("Agent Systems Benchmark"));
    assert!(compact.contains("Connection: disconnected"));
    assert!(compact.contains("Last event: none"));

    let standard = snapshot(
        &AppState::new(80, 16).unwrap(),
        CapabilityTier::IndexedColor,
        80,
        16,
    );
    assert_eq!(snapshot_hash(&standard), 0x67c8_3165_e712_2d16);
    assert!(standard.contains("Layout: standard"));
    assert!(!standard.contains("Ownership"));

    let mut wide_state = AppState::new(120, 24).unwrap();
    wide_state.apply(Action::Connected { baseline: 4 }).unwrap();
    wide_state
        .apply(Action::Control(ControlEvent {
            revision: 5,
            kind: ControlEventKind::RunStarted,
        }))
        .unwrap();
    let wide = snapshot(&wide_state, CapabilityTier::TrueColor, 120, 24);
    assert_eq!(snapshot_hash(&wide), 0x9520_ae66_107b_a629);
    assert!(wide.contains("Connection: connected"));
    assert!(wide.contains("Last event: run_started"));
    assert!(wide.contains("Layout: wide"));
    assert!(wide.contains("Ownership"));
    assert!(wide.contains("Closing the UI never owns or cancels a runner."));
}

#[test]
fn tiny_resize_uses_a_bounded_unbordered_fallback() {
    let mut state = AppState::new(120, 24).unwrap();
    state
        .apply(Action::Resize {
            columns: 10,
            lines: 2,
        })
        .unwrap();
    assert_eq!(
        snapshot(&state, CapabilityTier::BasicColor, 10, 2),
        "ASB\nq quit"
    );
}

#[test]
fn plain_policy_has_no_color_and_plain_text_is_equivalent() {
    let state = AppState::new(80, 24).unwrap();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| renderer::render(frame, &state, policy(CapabilityTier::Plain)))
        .unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
    assert_eq!(
        state.plain_text(),
        concat!(
            "Agent Systems Benchmark\n",
            "connection: disconnected\n",
            "last event: none\n",
            "runner ownership remains external\n"
        )
    );
}

#[test]
fn landing_renderer_exposes_next_action_recent_activity_and_routes() {
    let projection = project(LandingInput {
        connection: ConnectionStatus::Connected,
        trust: TrustStatus::Trusted,
        freshness: Freshness::Fresh,
        run: RunState::None,
        activity: vec![ActivityRecord {
            run_id: "run-42".into(),
            label: "nightly benchmark".into(),
            status: ActivityStatus::Succeeded,
            observed_at: 42,
            cursor: 7,
        }],
        capabilities: None,
    })
    .unwrap();
    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            renderer::render_landing(frame, &projection, policy(CapabilityTier::IndexedColor))
        })
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(content.contains("Configure a benchmark"));
    assert!(content.contains("run-42"));
    assert!(content.contains("Recent runs"));
    assert!(content.contains("× Measures (not connected)"));
    assert!(content.contains("? help"));
    let plain = renderer::landing_plain_text(&projection);
    assert!(plain.contains("next: Configure a benchmark"));
    assert!(plain.is_ascii());
}

#[test]
fn landing_renderer_has_bounded_tiny_fallback() {
    let projection = project(LandingInput {
        connection: ConnectionStatus::Unavailable,
        trust: TrustStatus::Unknown,
        freshness: Freshness::Unknown,
        run: RunState::None,
        activity: Vec::new(),
        capabilities: None,
    })
    .unwrap();
    let backend = TestBackend::new(24, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| renderer::render_landing(frame, &projection, policy(CapabilityTier::Plain)))
        .unwrap();
    let rows: Vec<String> = (0..3)
        .map(|y| {
            (0..24)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    assert_eq!(rows, ["ASB", "next: Install or connect", "? help | q quit"]);
}

#[test]
fn help_renderer_shows_context_actions_and_disabled_reason() {
    let model = HelpModel::new();
    let backend = TestBackend::new(160, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            renderer::render_help(
                frame,
                &model,
                Route::Landing,
                None,
                policy(CapabilityTier::IndexedColor),
            )
        })
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(content.contains("Help and shortcuts"));
    assert!(content.contains("Search:"));
    assert!(content.contains("Home"));
    assert!(content.contains("Measures"));
    assert!(content.contains("connect to ASB first"));
    assert!(content.contains("Esc back"));
}

#[test]
fn help_renderer_tiny_fallback_is_bounded_and_plain() {
    let mut model = HelpModel::new();
    model.set_query("run").unwrap();
    let backend = TestBackend::new(20, 2);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            renderer::render_help(
                frame,
                &model,
                Route::Help,
                None,
                policy(CapabilityTier::Plain),
            )
        })
        .unwrap();
    let content: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(content.contains("? help"));
    assert!(content.contains("Esc back"));
    assert!(content.is_ascii());
}
