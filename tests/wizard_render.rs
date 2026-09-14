// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    terminal::RenderPolicy,
    wizard::{StartupRoute, Wizard, element_id, plain_text, render, startup_route},
};
use ratatui::{Terminal, backend::TestBackend};

fn policy(unicode: bool) -> RenderPolicy {
    RenderPolicy {
        tier: asb_tui::terminal::CapabilityTier::Plain,
        unicode,
        mouse: false,
        focus: false,
        bracketed_paste: false,
        synchronized_output: false,
        alternate_screen: false,
    }
}

#[test]
fn startup_requires_setup_until_plan_is_complete() {
    assert_eq!(startup_route(false), StartupRoute::Wizard);
    assert_eq!(startup_route(true), StartupRoute::Landing);
}

#[test]
fn wizard_projection_uses_stable_element_id_and_responsive_fallback() {
    let wizard = Wizard::default();
    assert_eq!(element_id(wizard.step()), "wizard.agent");
    assert!(plain_text(&wizard).contains("ASB setup wizard"));
    let backend = TestBackend::new(24, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render(frame, &wizard, policy(false)))
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("ASB setup wizard"));
}
