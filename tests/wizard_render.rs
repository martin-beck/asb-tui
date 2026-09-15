// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    terminal::RenderPolicy,
    wizard::{
        FormalEvent, StartupRoute, Step, Wizard, WizardFormalState, element_id, plain_text, render,
        startup_route,
    },
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
fn setup_is_required_only_when_authoritative_readiness_is_false() {
    assert_eq!(startup_route(false), StartupRoute::Wizard);
    assert_eq!(startup_route(true), StartupRoute::Landing);
}

#[test]
fn wizard_render_has_stable_context_and_tiny_fallback() {
    let wizard = Wizard::default();
    assert_eq!(element_id(wizard.step()), "wizard.agent");
    assert!(plain_text(&wizard).contains("ASB setup wizard"));
    let mut terminal = Terminal::new(TestBackend::new(24, 6)).unwrap();
    terminal
        .draw(|frame| render(frame, &wizard, policy(false)))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("ASB setup wizard"));
}

#[test]
fn formal_wizard_flow_checks_documented_route_and_step_effects() {
    let mut state = WizardFormalState::new().unwrap();
    assert_eq!(
        state.apply(FormalEvent::Next),
        Err(asb_tui::wizard::WizardError::InvalidModel)
    );
    state.apply(FormalEvent::OpenWizard).unwrap();
    for value in [
        "agent",
        "provider",
        "model",
        "configuration",
        "auth",
        "record",
        "replay",
    ] {
        state.apply(FormalEvent::SetValue(value.into())).unwrap();
        state.apply(FormalEvent::Next).unwrap();
    }
    assert_eq!(state.step(), Step::Review);
    state.apply(FormalEvent::Complete).unwrap();
    assert_eq!(state.route(), StartupRoute::Landing);
}

#[test]
fn back_and_cancel_are_formal_wizard_transitions() {
    let mut state = WizardFormalState::new().unwrap();
    state.apply(FormalEvent::OpenWizard).unwrap();
    assert_eq!(
        state.apply(FormalEvent::Back),
        Err(asb_tui::wizard::WizardError::AtStart)
    );
    state.apply(FormalEvent::SetValue("agent".into())).unwrap();
    state.apply(FormalEvent::Next).unwrap();
    state.apply(FormalEvent::Back).unwrap();
    assert_eq!(state.step(), Step::Agent);
    state.apply(FormalEvent::Cancel).unwrap();
    assert_eq!(state.route(), StartupRoute::Landing);
}
