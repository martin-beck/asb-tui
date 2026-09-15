// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Standalone setup/reconfiguration wizard.
//!
//! The draft is renderer-owned and has no provider, credential, filesystem,
//! or runner side effects.  Its transitions are checked against the authored
//! UI state model before state is committed.

use crate::terminal::RenderPolicy;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde::Deserialize;

const MODEL: &str = include_str!("../docs/ui-state-model.json");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupRoute {
    Wizard,
    Landing,
}

#[must_use]
pub fn startup_route(asb_setup_ready: bool) -> StartupRoute {
    if asb_setup_ready {
        StartupRoute::Landing
    } else {
        StartupRoute::Wizard
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    Agent,
    Provider,
    Model,
    Configuration,
    Authentication,
    Recording,
    Replay,
    Review,
}

#[must_use]
pub const fn element_id(step: Step) -> &'static str {
    match step {
        Step::Agent => "wizard.agent",
        Step::Provider => "wizard.provider",
        Step::Model => "wizard.model",
        Step::Configuration => "wizard.configuration",
        Step::Authentication => "wizard.authentication",
        Step::Recording => "wizard.recording",
        Step::Replay => "wizard.replay",
        Step::Review => "wizard.review",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WizardError {
    Missing,
    AtStart,
    AtEnd,
    InvalidValue,
    TooLong,
    InvalidModel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Wizard {
    step: Step,
    values: [String; 7],
    cancelled: bool,
}

impl Default for Wizard {
    fn default() -> Self {
        Self {
            step: Step::Agent,
            values: Default::default(),
            cancelled: false,
        }
    }
}

impl Wizard {
    #[must_use]
    pub const fn step(&self) -> Step {
        self.step
    }
    #[must_use]
    pub const fn cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn set_value(&mut self, value: impl Into<String>) -> Result<(), WizardError> {
        let value = value.into();
        if value.chars().count() > 256 {
            return Err(WizardError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(WizardError::InvalidValue);
        }
        if let Some(slot) = self.values.get_mut(self.step as usize) {
            *slot = value;
        }
        Ok(())
    }

    pub fn advance(&mut self) -> Result<(), WizardError> {
        if self.step != Step::Review && self.values[self.step as usize].trim().is_empty() {
            return Err(WizardError::Missing);
        }
        self.step = match self.step {
            Step::Agent => Step::Provider,
            Step::Provider => Step::Model,
            Step::Model => Step::Configuration,
            Step::Configuration => Step::Authentication,
            Step::Authentication => Step::Recording,
            Step::Recording => Step::Replay,
            Step::Replay => Step::Review,
            Step::Review => return Err(WizardError::AtEnd),
        };
        Ok(())
    }

    pub fn back(&mut self) -> Result<(), WizardError> {
        self.step = match self.step {
            Step::Agent => return Err(WizardError::AtStart),
            Step::Provider => Step::Agent,
            Step::Model => Step::Provider,
            Step::Configuration => Step::Model,
            Step::Authentication => Step::Configuration,
            Step::Recording => Step::Authentication,
            Step::Replay => Step::Recording,
            Step::Review => Step::Replay,
        };
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }
    pub fn complete(&self) -> Result<(), WizardError> {
        (self.step == Step::Review)
            .then_some(())
            .ok_or(WizardError::AtEnd)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormalEvent {
    OpenWizard,
    Next,
    Back,
    Complete,
    Cancel,
    SetValue(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WizardFormalState {
    route: StartupRoute,
    wizard: Wizard,
}

impl WizardFormalState {
    pub fn new() -> Result<Self, WizardError> {
        validate_model()?;
        Ok(Self {
            route: StartupRoute::Landing,
            wizard: Wizard::default(),
        })
    }
    pub fn apply(&mut self, event: FormalEvent) -> Result<(), WizardError> {
        let mut next = self.clone();
        let model: Model = serde_json::from_str(MODEL).map_err(|_| WizardError::InvalidModel)?;
        let (event_id, expected_to) = match &event {
            FormalEvent::OpenWizard => ("open_wizard", "wizard"),
            FormalEvent::Next => ("wizard_next", "wizard"),
            FormalEvent::Back => ("wizard_back", "wizard"),
            FormalEvent::Complete => ("complete_wizard", "landing"),
            FormalEvent::Cancel => ("cancel_wizard", "landing"),
            FormalEvent::SetValue(_) => ("wizard_set_value", "wizard"),
        };
        let from = match next.route {
            StartupRoute::Wizard => "wizard",
            StartupRoute::Landing => "landing",
        };
        let Some(transition) = model
            .transitions
            .iter()
            .find(|item| item.event == event_id && item.from == from && item.to == expected_to)
        else {
            return Err(WizardError::InvalidModel);
        };
        let expected_effects = match event_id {
            "wizard_next" | "wizard_back" => ["wizard_step_changed", "focus_reset"].as_slice(),
            "wizard_set_value" => ["wizard_draft_changed", "focus_reset"].as_slice(),
            _ => ["route_changed", "focus_reset"].as_slice(),
        };
        if transition
            .effects
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != expected_effects
        {
            return Err(WizardError::InvalidModel);
        }
        match event {
            FormalEvent::OpenWizard if next.route == StartupRoute::Landing => {
                next.route = StartupRoute::Wizard
            }
            FormalEvent::SetValue(value) if next.route == StartupRoute::Wizard => {
                next.wizard.set_value(value)?
            }
            FormalEvent::Next if next.route == StartupRoute::Wizard => next.wizard.advance()?,
            FormalEvent::Back if next.route == StartupRoute::Wizard => next.wizard.back()?,
            FormalEvent::Complete if next.route == StartupRoute::Wizard => {
                next.wizard.complete()?;
                next.route = StartupRoute::Landing;
            }
            FormalEvent::Cancel if next.route == StartupRoute::Wizard => {
                next.wizard.cancel();
                next.route = StartupRoute::Landing;
            }
            _ => return Err(WizardError::InvalidModel),
        }
        *self = next;
        Ok(())
    }
    #[must_use]
    pub const fn route(&self) -> StartupRoute {
        self.route
    }
    #[must_use]
    pub const fn step(&self) -> Step {
        self.wizard.step()
    }
    #[must_use]
    pub const fn wizard(&self) -> &Wizard {
        &self.wizard
    }
}

#[derive(Deserialize)]
struct Model {
    routes: Vec<Route>,
    transitions: Vec<Transition>,
}
#[derive(Deserialize)]
struct Route {
    id: String,
}
#[derive(Deserialize)]
struct Transition {
    event: String,
    from: String,
    to: String,
    effects: Vec<String>,
}

fn validate_model() -> Result<(), WizardError> {
    let model: Model = serde_json::from_str(MODEL).map_err(|_| WizardError::InvalidModel)?;
    let routes: std::collections::BTreeSet<_> =
        model.routes.iter().map(|r| r.id.as_str()).collect();
    let required = ["wizard", "landing"];
    if required.iter().any(|route| !routes.contains(route)) {
        return Err(WizardError::InvalidModel);
    }
    for transition in model.transitions {
        if transition.event.is_empty()
            || !routes.contains(transition.from.as_str())
            || !routes.contains(transition.to.as_str())
            || transition.effects.is_empty()
        {
            return Err(WizardError::InvalidModel);
        }
    }
    Ok(())
}

#[must_use]
pub fn plain_text(wizard: &Wizard) -> String {
    format!(
        "ASB setup wizard\nstep: {}\nfield: {}\ncontrols: Enter next | Esc back | q cancel\n",
        step_title(wizard.step),
        element_id(wizard.step)
    )
}

pub fn render(frame: &mut Frame<'_>, wizard: &Wizard, policy: RenderPolicy) {
    let area = frame.area();
    let accent = Style::default().fg(if policy.unicode {
        Color::Cyan
    } else {
        Color::White
    });
    if area.width < 30 || area.height < 8 {
        frame.render_widget(Paragraph::new(plain_text(wizard)).style(accent), area);
        return;
    }
    let regions = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("ASB", accent),
            Span::raw(" setup wizard"),
        ]))
        .block(Block::default().borders(Borders::ALL).title(" Setup ")),
        regions[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("Step: {}", step_title(wizard.step)),
                accent,
            )),
            Line::from(step_prompt(wizard.step)),
            Line::from(format!("Element: {}", element_id(wizard.step))),
        ])
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Current step "),
        ),
        regions[1],
    );
    frame.render_widget(
        Paragraph::new("Enter next | Esc back | ? help | q cancel").style(accent),
        regions[2],
    );
}

const fn step_title(step: Step) -> &'static str {
    match step {
        Step::Agent => "Agent",
        Step::Provider => "Provider",
        Step::Model => "Model",
        Step::Configuration => "Configuration",
        Step::Authentication => "Authentication",
        Step::Recording => "Recording",
        Step::Replay => "Offline replay",
        Step::Review => "Review",
    }
}
const fn step_prompt(step: Step) -> &'static str {
    match step {
        Step::Agent => "Choose the agent used for this benchmark.",
        Step::Provider => "Choose the model provider.",
        Step::Model => "Choose the provider model.",
        Step::Configuration => "Review benchmark configuration defaults.",
        Step::Authentication => "Select an existing authentication reference.",
        Step::Recording => "Choose whether to record benchmark activity.",
        Step::Replay => "Choose the offline replay policy.",
        Step::Review => "Review all choices before continuing.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_bound_wizard_flow_is_atomic() {
        let mut state = WizardFormalState::new().unwrap();
        assert!(state.apply(FormalEvent::Next).is_err());
        assert_eq!(state.route(), StartupRoute::Landing);
        state.apply(FormalEvent::OpenWizard).unwrap();
        for value in [
            "agent", "provider", "model", "config", "auth", "record", "replay",
        ] {
            state.apply(FormalEvent::SetValue(value.into())).unwrap();
            state.apply(FormalEvent::Next).unwrap();
        }
        assert_eq!(state.step(), Step::Review);
        state.apply(FormalEvent::Complete).unwrap();
        assert_eq!(state.route(), StartupRoute::Landing);
    }
    #[test]
    fn startup_route_is_fail_closed_and_manual_ready() {
        assert_eq!(startup_route(false), StartupRoute::Wizard);
        assert_eq!(startup_route(true), StartupRoute::Landing);
    }
}
