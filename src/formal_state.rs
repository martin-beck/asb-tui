// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Executable parity seam between the documented UI graph and wizard renderer.

use crate::wizard::{Step, Wizard, WizardError};
use serde::Deserialize;

const MODEL: &str = include_str!("../docs/ui-state-model.json");

#[derive(Clone, Debug, Deserialize)]
struct Document {
    transitions: Vec<Transition>,
}
#[derive(Clone, Debug, Deserialize)]
struct Transition {
    event: String,
    from: String,
    to: String,
    effects: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    OpenWizard,
    WizardNext,
    WizardBack,
    CompleteWizard,
    CancelWizard,
    SetWizardValue(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidModel(String),
    NotDocumented(String),
    Wizard(WizardError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormalState {
    route: String,
    wizard: Wizard,
}

impl FormalState {
    pub fn new() -> Result<Self, Error> {
        validate_document().map_err(Error::InvalidModel)?;
        Ok(Self {
            route: "landing".into(),
            wizard: Wizard::default(),
        })
    }

    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        let name = match &event {
            Event::OpenWizard => "open_wizard",
            Event::WizardNext => "wizard_next",
            Event::WizardBack => "wizard_back",
            Event::CompleteWizard => "complete_wizard",
            Event::CancelWizard => "cancel_wizard",
            Event::SetWizardValue(_) => return self.apply_value(event),
        };
        let document: Document =
            serde_json::from_str(MODEL).map_err(|e| Error::InvalidModel(e.to_string()))?;
        let transition = document
            .transitions
            .iter()
            .find(|item| item.event == name && item.from == self.route)
            .ok_or_else(|| Error::NotDocumented(name.into()))?;
        let expected = match name {
            "wizard_next" | "wizard_back" => ["wizard_step_changed", "focus_reset"].as_slice(),
            _ => ["route_changed", "focus_reset"].as_slice(),
        };
        if transition
            .effects
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != expected
        {
            return Err(Error::InvalidModel(format!(
                "effects for {name} do not match renderer semantics"
            )));
        }
        let mut next = self.clone();
        match event {
            Event::OpenWizard => next.route = transition.to.clone(),
            Event::WizardNext => {
                next.wizard.advance().map_err(Error::Wizard)?;
            }
            Event::WizardBack => {
                next.wizard.back().map_err(Error::Wizard)?;
            }
            Event::CompleteWizard => {
                next.wizard.complete().map_err(Error::Wizard)?;
                next.route = transition.to.clone();
            }
            Event::CancelWizard => {
                next.wizard.cancel();
                next.route = transition.to.clone();
            }
            Event::SetWizardValue(_) => unreachable!(),
        }
        *self = next;
        Ok(())
    }

    fn apply_value(&mut self, event: Event) -> Result<(), Error> {
        let Event::SetWizardValue(value) = event else {
            unreachable!()
        };
        let mut next = self.clone();
        next.wizard.set_value(value).map_err(Error::Wizard)?;
        *self = next;
        Ok(())
    }
    #[must_use]
    pub fn route(&self) -> &str {
        &self.route
    }
    #[must_use]
    pub const fn step(&self) -> Step {
        self.wizard.step()
    }
    #[must_use]
    pub fn wizard(&self) -> &Wizard {
        &self.wizard
    }
}

fn validate_document() -> Result<(), String> {
    let document: Document = serde_json::from_str(MODEL).map_err(|e| e.to_string())?;
    for transition in &document.transitions {
        if transition.effects.is_empty() {
            return Err(format!("transition {} has no effects", transition.event));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wizard_renderer_flow_is_executable_by_formal_state() {
        let mut state = FormalState::new().unwrap();
        state.apply(Event::OpenWizard).unwrap();
        for value in [
            "agent", "provider", "model", "config", "auth", "record", "replay",
        ] {
            state.apply(Event::SetWizardValue(value.into())).unwrap();
            state.apply(Event::WizardNext).unwrap();
        }
        assert_eq!(state.step(), Step::Review);
        state.apply(Event::CompleteWizard).unwrap();
        assert_eq!(state.route(), "landing");
    }
    #[test]
    fn invalid_wizard_transition_is_atomic() {
        let mut state = FormalState::new().unwrap();
        assert_eq!(
            state.apply(Event::WizardNext),
            Err(Error::NotDocumented("wizard_next".into()))
        );
        assert_eq!(state.route(), "landing");
    }
}
