// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Isolated contract coverage for the stepper before it is joined to the
//! overlapping wizard renderer work.  Keep this test independent from the
//! library module registry so it can validate the renderer-neutral contract
//! without modifying AR-1732-owned integration files.

mod wizard {
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
}

#[path = "../src/wizard_stepper.rs"]
mod wizard_stepper;

use wizard::Step;
use wizard_stepper::{
    JOURNEY_STAGES, JourneyStage, StepState, StepperLayout, render_text, wizard_progress,
};

#[test]
fn every_local_wizard_step_has_exactly_one_truthful_current_stage() {
    for (step, expected) in [
        (Step::Agent, JourneyStage::Agent),
        (Step::Provider, JourneyStage::Provider),
        (Step::Model, JourneyStage::Model),
        (Step::Configuration, JourneyStage::Configuration),
        (Step::Authentication, JourneyStage::CredentialReference),
        (Step::Recording, JourneyStage::Recording),
        (Step::Replay, JourneyStage::Replay),
        (Step::Review, JourneyStage::PlanReview),
    ] {
        let stages = wizard_progress(step);
        assert_eq!(stages.len(), JOURNEY_STAGES.len());
        let current = stages
            .iter()
            .filter(|stage| stage.state == StepState::Current)
            .collect::<Vec<_>>();
        assert_eq!(current.len(), 1, "{step:?}");
        assert_eq!(current[0].stage, expected, "{step:?}");
    }
    assert_eq!(JourneyStage::Workload.element_id(), "wizard.workload_stage");
    assert_eq!(
        JourneyStage::Comparison.element_id(),
        "wizard.comparison_stage"
    );
}

#[test]
fn completed_local_steps_and_runner_owned_stages_are_never_conflated() {
    let stages = wizard_progress(Step::Review);
    let completed = stages
        .iter()
        .find(|stage| stage.stage == JourneyStage::CredentialReference)
        .expect("credential stage");
    assert_eq!(completed.state, StepState::Complete);

    for stage in [
        JourneyStage::Workload,
        JourneyStage::Measures,
        JourneyStage::Execution,
        JourneyStage::Results,
        JourneyStage::Comparison,
    ] {
        let presentation = stages
            .iter()
            .find(|item| item.stage == stage)
            .expect("runner-owned stage");
        assert_eq!(presentation.state, StepState::Unavailable, "{stage:?}");
        assert!(presentation.unavailable_reason.is_some(), "{stage:?}");
    }
}

#[test]
fn compact_and_tiny_resize_fallbacks_preserve_current_location() {
    let stages = wizard_progress(Step::Authentication);
    assert_eq!(
        StepperLayout::from_dimensions(175, 16),
        StepperLayout::Compact
    );
    assert_eq!(
        StepperLayout::from_dimensions(176, 16),
        StepperLayout::Horizontal
    );
    assert_eq!(StepperLayout::from_dimensions(39, 16), StepperLayout::Tiny);

    let compact = render_text(&stages, StepperLayout::Compact);
    assert!(compact.contains("Step 5/13: [>] Credentials"));
    assert!(compact.contains("[-]"));
    assert_eq!(
        render_text(&stages, StepperLayout::Tiny),
        "5/13 Credentials"
    );
}
