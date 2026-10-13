// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral, truthful progress projection for the guided journey.
//!
//! The wizard owns its local draft steps; later runner-owned stages remain
//! visible here as unavailable rather than being implied as completed.  This
//! module deliberately has no terminal or control-plane effects.

use crate::wizard::Step;

/// Every user-visible stage in the guided benchmark journey.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JourneyStage {
    Agent,
    Provider,
    Model,
    Configuration,
    CredentialReference,
    Workload,
    Measures,
    Recording,
    Replay,
    PlanReview,
    Execution,
    Results,
    Comparison,
}

impl JourneyStage {
    /// Short, stable label suitable for a horizontal terminal header.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Provider => "Provider",
            Self::Model => "Model",
            Self::Configuration => "Setup",
            Self::CredentialReference => "Credentials",
            Self::Workload => "Workloads",
            Self::Measures => "Measures",
            Self::Recording => "Recording",
            Self::Replay => "Replay",
            Self::PlanReview => "Review",
            Self::Execution => "Run",
            Self::Results => "Results",
            Self::Comparison => "Compare",
        }
    }

    /// Stable model element used by help and source-parity checks.
    pub const fn element_id(self) -> &'static str {
        match self {
            Self::Agent => "wizard.agent",
            Self::Provider => "wizard.provider",
            Self::Model => "wizard.model",
            Self::Configuration => "wizard.configuration",
            Self::CredentialReference => "wizard.authentication",
            Self::Workload => "wizard.workload_stage",
            Self::Measures => "wizard.measure_stage",
            Self::Recording => "wizard.recording",
            Self::Replay => "wizard.replay",
            Self::PlanReview => "wizard.review",
            Self::Execution => "wizard.execution_stage",
            Self::Results => "wizard.results_stage",
            Self::Comparison => "wizard.comparison_stage",
        }
    }
}

/// Authoritative order of the complete journey.  The unavailable stages are
/// intentional: the standalone TUI never claims that runner-owned work has
/// completed merely because a local setup draft is complete.
pub const JOURNEY_STAGES: [JourneyStage; 13] = [
    JourneyStage::Agent,
    JourneyStage::Provider,
    JourneyStage::Model,
    JourneyStage::Configuration,
    JourneyStage::CredentialReference,
    JourneyStage::Workload,
    JourneyStage::Measures,
    JourneyStage::Recording,
    JourneyStage::Replay,
    JourneyStage::PlanReview,
    JourneyStage::Execution,
    JourneyStage::Results,
    JourneyStage::Comparison,
];

/// Non-colour progress distinction.  A renderer can add semantic colour, but
/// the marker and text must remain sufficient in monochrome terminals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepState {
    Complete,
    Current,
    Future,
    Unavailable,
}

impl StepState {
    /// ASCII marker that remains meaningful with `NO_COLOR` or plain output.
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Complete => "[x]",
            Self::Current => "[>]",
            Self::Future => "[ ]",
            Self::Unavailable => "[-]",
        }
    }
}

/// One renderable stage and an optional reason for unavailable work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepPresentation {
    pub stage: JourneyStage,
    pub state: StepState,
    pub unavailable_reason: Option<&'static str>,
}

/// Responsive representation selected from the current authoritative frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepperLayout {
    Horizontal,
    Compact,
    Tiny,
}

/// The keyboard-focusable regions in the wizard shell.  Cycling regions never
/// changes a draft value or advances a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // This renderer-neutral module is also included directly by its contract test.
pub enum WizardPane {
    Progress,
    Choices,
    Summary,
    Controls,
}

impl WizardPane {
    #[allow(dead_code)] // The direct contract test does not drive pane focus.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Progress => Self::Choices,
            Self::Choices => Self::Summary,
            Self::Summary => Self::Controls,
            Self::Controls => Self::Progress,
        }
    }
}

impl StepperLayout {
    #[must_use]
    pub const fn from_dimensions(columns: u16, lines: u16) -> Self {
        if columns < 40 || lines < 10 {
            Self::Tiny
        } else if columns < 176 || lines < 16 {
            Self::Compact
        } else {
            Self::Horizontal
        }
    }
}

/// Deterministic projection of the current local wizard step.
#[must_use]
pub fn wizard_progress(step: Step) -> Vec<StepPresentation> {
    let current = match step {
        Step::Agent => JourneyStage::Agent,
        Step::Provider => JourneyStage::Provider,
        Step::Model => JourneyStage::Model,
        Step::Configuration => JourneyStage::Configuration,
        Step::Authentication => JourneyStage::CredentialReference,
        Step::Recording => JourneyStage::Recording,
        Step::Replay => JourneyStage::Replay,
        Step::Review => JourneyStage::PlanReview,
    };
    let current_index = JOURNEY_STAGES
        .iter()
        .position(|stage| *stage == current)
        .expect("current wizard stage is included in JOURNEY_STAGES");
    JOURNEY_STAGES
        .iter()
        .enumerate()
        .map(|(index, stage)| {
            let unavailable_reason = runner_owned_reason(*stage);
            let state = if unavailable_reason.is_some() {
                StepState::Unavailable
            } else if index < current_index {
                StepState::Complete
            } else if index == current_index {
                StepState::Current
            } else {
                StepState::Future
            };
            StepPresentation {
                stage: *stage,
                state,
                unavailable_reason,
            }
        })
        .collect()
}

fn runner_owned_reason(stage: JourneyStage) -> Option<&'static str> {
    match stage {
        JourneyStage::Workload | JourneyStage::Measures => {
            Some("requires an authoritative ASB catalog")
        }
        JourneyStage::Execution => Some("requires an authoritative ASB plan and launch"),
        JourneyStage::Results | JourneyStage::Comparison => {
            Some("requires completed authoritative ASB runs")
        }
        _ => None,
    }
}

/// Stable textual form for the header.  Horizontal output intentionally shows
/// every stage; narrow terminals preserve the exact current ordinal instead
/// of truncating into a misleading partial route.
#[must_use]
pub fn render_text(stages: &[StepPresentation], layout: StepperLayout) -> String {
    let current = stages
        .iter()
        .position(|item| item.state == StepState::Current)
        .expect("wizard progress has exactly one current local stage");
    match layout {
        StepperLayout::Horizontal => stages
            .iter()
            .map(|item| format!("{} {}", item.state.marker(), item.stage.label()))
            .collect::<Vec<_>>()
            .join(" ─ "),
        StepperLayout::Compact => {
            let item = &stages[current];
            format!(
                "Step {}/{}: {} {} · [-] ASB catalog/plan/completed runs",
                current + 1,
                stages.len(),
                item.state.marker(),
                item.stage.label()
            )
        }
        StepperLayout::Tiny => {
            let item = &stages[current];
            format!("{}/{} {}", current + 1, stages.len(), item.stage.label())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_journey_has_stable_non_colour_markers() {
        assert_eq!(JOURNEY_STAGES.len(), 13);
        assert_eq!(StepState::Complete.marker(), "[x]");
        assert_eq!(StepState::Current.marker(), "[>]");
        assert_eq!(StepState::Future.marker(), "[ ]");
        assert_eq!(StepState::Unavailable.marker(), "[-]");
    }

    #[test]
    fn wizard_projection_never_claims_runner_owned_stages_are_complete() {
        let stages = wizard_progress(Step::Review);
        assert!(
            stages
                .iter()
                .filter(|item| item.unavailable_reason.is_some())
                .all(|item| item.state == StepState::Unavailable)
        );
        assert_eq!(
            stages
                .iter()
                .find(|item| item.stage == JourneyStage::PlanReview)
                .expect("review stage")
                .state,
            StepState::Current
        );
    }

    #[test]
    fn layout_fallback_is_truthful_and_deterministic() {
        let stages = wizard_progress(Step::Agent);
        assert_eq!(
            StepperLayout::from_dimensions(200, 24),
            StepperLayout::Horizontal
        );
        assert_eq!(
            StepperLayout::from_dimensions(80, 24),
            StepperLayout::Compact
        );
        assert_eq!(StepperLayout::from_dimensions(30, 8), StepperLayout::Tiny);
        assert!(render_text(&stages, StepperLayout::Horizontal).contains("[-] Workloads"));
        assert!(render_text(&stages, StepperLayout::Compact).contains("Step 1/13"));
        assert_eq!(render_text(&stages, StepperLayout::Tiny), "1/13 Agent");
    }
}
