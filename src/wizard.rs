// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral first-run and reconfiguration wizard state machine.
//!
//! This module validates local draft input and emits contextual events only;
//! it performs no provider, credential, filesystem, or runner effects.

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    Entered(Step),
    Back(Step),
    Cancelled,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WizardError {
    Missing(&'static str),
    AtStart,
    AtEnd,
    InvalidValue,
    TooLong,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Wizard {
    step: Step,
    agent: String,
    provider: String,
    model: String,
    configuration: String,
    authentication_ref: String,
    recording: String,
    replay: String,
    cancelled: bool,
}

impl Default for Wizard {
    fn default() -> Self {
        Self {
            step: Step::Agent,
            agent: String::new(),
            provider: String::new(),
            model: String::new(),
            configuration: String::new(),
            authentication_ref: String::new(),
            recording: String::new(),
            replay: String::new(),
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
        match self.step {
            Step::Agent => self.agent = value,
            Step::Provider => self.provider = value,
            Step::Model => self.model = value,
            Step::Configuration => self.configuration = value,
            Step::Authentication => self.authentication_ref = value,
            Step::Recording => self.recording = value,
            Step::Replay => self.replay = value,
            Step::Review => {}
        }
        Ok(())
    }

    pub fn advance(&mut self) -> Result<Event, WizardError> {
        self.validate_current()?;
        let Some(next) = next_step(self.step) else {
            return Err(WizardError::AtEnd);
        };
        self.step = next;
        Ok(Event::Entered(next))
    }

    pub fn back(&mut self) -> Result<Event, WizardError> {
        let Some(previous) = previous_step(self.step) else {
            return Err(WizardError::AtStart);
        };
        self.step = previous;
        Ok(Event::Back(previous))
    }

    pub fn cancel(&mut self) -> Event {
        self.cancelled = true;
        Event::Cancelled
    }

    pub fn complete(&mut self) -> Result<Event, WizardError> {
        if self.step != Step::Review {
            return Err(WizardError::AtEnd);
        }
        Ok(Event::Completed)
    }

    fn validate_current(&self) -> Result<(), WizardError> {
        let (value, field) = match self.step {
            Step::Agent => (&self.agent, "agent"),
            Step::Provider => (&self.provider, "provider"),
            Step::Model => (&self.model, "model"),
            Step::Configuration => (&self.configuration, "configuration"),
            Step::Authentication => (&self.authentication_ref, "authentication_ref"),
            Step::Recording => (&self.recording, "recording"),
            Step::Replay => (&self.replay, "replay"),
            Step::Review => return Ok(()),
        };
        (!value.trim().is_empty())
            .then_some(())
            .ok_or(WizardError::Missing(field))
    }
}

const fn next_step(step: Step) -> Option<Step> {
    match step {
        Step::Agent => Some(Step::Provider),
        Step::Provider => Some(Step::Model),
        Step::Model => Some(Step::Configuration),
        Step::Configuration => Some(Step::Authentication),
        Step::Authentication => Some(Step::Recording),
        Step::Recording => Some(Step::Replay),
        Step::Replay => Some(Step::Review),
        Step::Review => None,
    }
}
const fn previous_step(step: Step) -> Option<Step> {
    match step {
        Step::Agent => None,
        Step::Provider => Some(Step::Agent),
        Step::Model => Some(Step::Provider),
        Step::Configuration => Some(Step::Model),
        Step::Authentication => Some(Step::Configuration),
        Step::Recording => Some(Step::Authentication),
        Step::Replay => Some(Step::Recording),
        Step::Review => Some(Step::Replay),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn next_requires_each_step_and_emits_contextual_events() {
        let mut wizard = Wizard::default();
        assert_eq!(wizard.advance(), Err(WizardError::Missing("agent")));
        wizard.set_value("agent").unwrap();
        assert_eq!(wizard.advance(), Ok(Event::Entered(Step::Provider)));
        assert_eq!(wizard.back(), Ok(Event::Back(Step::Agent)));
    }
    #[test]
    fn full_flow_reaches_review_and_back_cancel_are_deterministic() {
        let mut wizard = Wizard::default();
        for value in ["a", "p", "m", "c", "auth", "record", "replay"] {
            wizard.set_value(value).unwrap();
            assert!(wizard.advance().is_ok());
        }
        assert_eq!(wizard.step(), Step::Review);
        assert_eq!(wizard.complete(), Ok(Event::Completed));
        assert_eq!(wizard.cancel(), Event::Cancelled);
        assert!(wizard.cancelled());
    }

    #[test]
    fn invalid_input_is_atomic_and_does_not_advance() {
        let mut wizard = Wizard::default();
        assert_eq!(wizard.complete(), Err(WizardError::AtEnd));
        assert_eq!(wizard.step(), Step::Agent);
        assert_eq!(wizard.set_value("\n"), Err(WizardError::InvalidValue));
        assert_eq!(wizard.advance(), Err(WizardError::Missing("agent")));
        assert_eq!(wizard.set_value("x".repeat(257)), Err(WizardError::TooLong));
        wizard.set_value("agent").unwrap();
        assert_eq!(wizard.advance(), Ok(Event::Entered(Step::Provider)));
    }
}
