// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral recording campaign intent and lifecycle model.
//!
//! ASB remains authoritative for estimates, plans, capture, coverage,
//! reconciliation, and offline-default activation. This module only gates
//! frontend intents and applies validated observations; it never claims a
//! remote mutation succeeded.

/// Maximum selected workload identities held by the frontend draft.
pub const MAX_WORKLOADS: usize = 256;

/// Workload scope selected for a recording campaign.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkloadScope {
    /// All workloads exposed by ASB at estimate time.
    All,
    /// Exactly these workload identities.
    Selected(Vec<String>),
}

impl WorkloadScope {
    /// Validate and canonicalize selected workload identities.
    pub fn canonical(self) -> Result<Self, RecordingModelError> {
        match self {
            Self::All => Ok(Self::All),
            Self::Selected(mut values) => {
                if values.is_empty() || values.len() > MAX_WORKLOADS {
                    return Err(RecordingModelError::InvalidScope);
                }
                values.sort();
                if values.windows(2).any(|pair| pair[0] == pair[1])
                    || values.iter().any(|value| !valid_id(value))
                {
                    return Err(RecordingModelError::InvalidScope);
                }
                Ok(Self::Selected(values))
            }
        }
    }
}

/// Lifecycle state explicitly observed from ASB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CampaignPhase {
    None,
    Planned,
    Recording,
    NeedsReconciliation,
    Complete,
    Cancelled,
    Failed,
}

/// Verified tuple coverage supplied by ASB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Coverage {
    pub requested: u16,
    pub complete: u16,
}

impl Coverage {
    /// Reject empty or over-counted coverage.
    pub const fn validate(self) -> Result<(), RecordingModelError> {
        if self.requested == 0 || self.complete > self.requested {
            Err(RecordingModelError::InvalidCoverage)
        } else {
            Ok(())
        }
    }

    /// Return true only when every requested tuple is complete.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.requested > 0 && self.complete == self.requested
    }
}

/// Privacy-safe campaign observation returned by the control seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignObservation {
    pub generation: u64,
    pub campaign_id: Option<String>,
    pub phase: CampaignPhase,
    pub coverage: Coverage,
    pub offline_ready: bool,
}

impl TryFrom<&crate::control_codec::RecordingCampaignLifecycle> for CampaignObservation {
    type Error = RecordingModelError;

    /// Convert the authenticated ASB lifecycle projection without inferring
    /// coverage or readiness from a plan-only response.
    fn try_from(
        lifecycle: &crate::control_codec::RecordingCampaignLifecycle,
    ) -> Result<Self, Self::Error> {
        let phase = match lifecycle.state.as_str() {
            "recording" => CampaignPhase::Recording,
            "needs_reconciliation" => CampaignPhase::NeedsReconciliation,
            "complete" => CampaignPhase::Complete,
            "cancelled" => CampaignPhase::Cancelled,
            "failed" => CampaignPhase::Failed,
            _ => return Err(RecordingModelError::InvalidObservation),
        };
        let observation = Self {
            generation: lifecycle.generation.0,
            campaign_id: Some(lifecycle.campaign_id.clone()),
            phase,
            coverage: Coverage {
                requested: lifecycle.tuple_count,
                complete: lifecycle.covered_tuple_count,
            },
            offline_ready: lifecycle.offline_ready,
        };
        observation.validate()?;
        Ok(observation)
    }
}

impl TryFrom<&crate::control_codec::RecordingCampaignPlan> for CampaignObservation {
    type Error = RecordingModelError;

    fn try_from(plan: &crate::control_codec::RecordingCampaignPlan) -> Result<Self, Self::Error> {
        let observation = Self {
            generation: plan.generation.0,
            campaign_id: Some(plan.campaign_id.clone()),
            phase: match plan.state.as_str() {
                "planned" => CampaignPhase::Planned,
                "cancelled" => CampaignPhase::Cancelled,
                "failed" => CampaignPhase::Failed,
                _ => return Err(RecordingModelError::InvalidObservation),
            },
            coverage: Coverage {
                requested: plan.tuple_count,
                complete: 0,
            },
            offline_ready: plan.offline_ready,
        };
        observation.validate()?;
        Ok(observation)
    }
}

impl CampaignObservation {
    /// Validate lifecycle and offline-readiness invariants.
    pub fn validate(&self) -> Result<(), RecordingModelError> {
        if self.generation == 0 {
            return Err(RecordingModelError::InvalidGeneration);
        }
        if self.phase == CampaignPhase::None {
            if self.campaign_id.is_some()
                || self.coverage.requested != 0
                || self.coverage.complete != 0
            {
                return Err(RecordingModelError::InvalidObservation);
            }
        } else if self.campaign_id.as_deref().is_none_or(|id| !valid_id(id)) {
            return Err(RecordingModelError::InvalidObservation);
        }
        if self.phase != CampaignPhase::None {
            self.coverage.validate()?;
        }
        if self.offline_ready
            && (self.phase != CampaignPhase::Complete || !self.coverage.is_complete())
        {
            return Err(RecordingModelError::OfflineActivationBlocked);
        }
        Ok(())
    }
}

/// User actions exposed by the renderer-neutral campaign screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingAction {
    Estimate,
    Plan,
    ConfirmCapture,
    Progress,
    Cancel,
    Reconcile,
    ActivateOfflineDefault,
}

/// Validated intent for the authenticated transport adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordingIntent {
    pub action: RecordingAction,
    pub expected_generation: u64,
}

/// Frontend campaign model. It performs no remote effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingCampaignModel {
    scope: WorkloadScope,
    observation: CampaignObservation,
    capture_confirmation_required: bool,
}

impl RecordingCampaignModel {
    /// Construct a model from a validated ASB observation.
    pub fn new(
        scope: WorkloadScope,
        observation: CampaignObservation,
    ) -> Result<Self, RecordingModelError> {
        Ok(Self {
            scope: scope.canonical()?,
            observation: {
                observation.validate()?;
                observation
            },
            capture_confirmation_required: false,
        })
    }

    #[must_use]
    pub const fn scope(&self) -> &WorkloadScope {
        &self.scope
    }

    #[must_use]
    pub const fn observation(&self) -> &CampaignObservation {
        &self.observation
    }

    /// Apply a newer authoritative observation, rejecting stale state.
    pub fn apply(&mut self, observation: CampaignObservation) -> Result<(), RecordingModelError> {
        observation.validate()?;
        if observation.generation < self.observation.generation {
            return Err(RecordingModelError::StaleObservation);
        }
        self.observation = observation;
        self.capture_confirmation_required = false;
        Ok(())
    }

    /// Validate an action without claiming that ASB accepted it.
    pub fn request(
        &mut self,
        action: RecordingAction,
    ) -> Result<RecordingIntent, RecordingModelError> {
        let phase = self.observation.phase;
        match action {
            RecordingAction::Estimate | RecordingAction::Plan
                if matches!(
                    phase,
                    CampaignPhase::None | CampaignPhase::Cancelled | CampaignPhase::Failed
                ) => {}
            RecordingAction::ConfirmCapture if phase == CampaignPhase::Planned => {
                if !self.capture_confirmation_required {
                    return Err(RecordingModelError::CaptureConfirmationRequired);
                }
                self.capture_confirmation_required = false;
            }
            RecordingAction::Progress
                if matches!(
                    phase,
                    CampaignPhase::Recording | CampaignPhase::NeedsReconciliation
                ) => {}
            RecordingAction::Cancel
                if matches!(
                    phase,
                    CampaignPhase::Planned
                        | CampaignPhase::Recording
                        | CampaignPhase::NeedsReconciliation
                ) => {}
            RecordingAction::Reconcile if phase == CampaignPhase::NeedsReconciliation => {}
            RecordingAction::ActivateOfflineDefault
                if phase == CampaignPhase::Complete && self.observation.coverage.is_complete() => {}
            _ => return Err(RecordingModelError::ActionUnavailable),
        }
        Ok(RecordingIntent {
            action,
            expected_generation: self.observation.generation,
        })
    }

    /// Require a separate explicit confirmation before provider capture.
    pub fn arm_capture_confirmation(&mut self) -> Result<(), RecordingModelError> {
        if self.observation.phase != CampaignPhase::Planned {
            return Err(RecordingModelError::ActionUnavailable);
        }
        self.capture_confirmation_required = true;
        Ok(())
    }

    #[must_use]
    pub const fn capture_confirmation_required(&self) -> bool {
        self.capture_confirmation_required
    }
}

/// Fail-closed campaign model errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingModelError {
    InvalidScope,
    InvalidCoverage,
    InvalidGeneration,
    InvalidObservation,
    StaleObservation,
    CaptureConfirmationRequired,
    OfflineActivationBlocked,
    ActionUnavailable,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(phase: CampaignPhase, requested: u16, complete: u16) -> CampaignObservation {
        CampaignObservation {
            generation: 2,
            campaign_id: (phase != CampaignPhase::None).then(|| "campaign-1".into()),
            phase,
            coverage: Coverage {
                requested,
                complete,
            },
            offline_ready: false,
        }
    }

    #[test]
    fn scopes_support_all_and_canonical_selected_workloads() {
        assert_eq!(WorkloadScope::All.canonical().unwrap(), WorkloadScope::All);
        assert_eq!(
            WorkloadScope::Selected(vec!["workload-b".into(), "workload-a".into()])
                .canonical()
                .unwrap(),
            WorkloadScope::Selected(vec!["workload-a".into(), "workload-b".into()])
        );
        assert_eq!(
            WorkloadScope::Selected(vec!["workload-a".into(), "workload-a".into()]).canonical(),
            Err(RecordingModelError::InvalidScope)
        );
    }

    #[test]
    fn capture_requires_explicit_confirmation() {
        let mut model = RecordingCampaignModel::new(
            WorkloadScope::All,
            observation(CampaignPhase::Planned, 4, 0),
        )
        .unwrap();
        assert_eq!(
            model.request(RecordingAction::ConfirmCapture),
            Err(RecordingModelError::CaptureConfirmationRequired)
        );
        model.arm_capture_confirmation().unwrap();
        assert_eq!(
            model
                .request(RecordingAction::ConfirmCapture)
                .unwrap()
                .action,
            RecordingAction::ConfirmCapture
        );
    }

    #[test]
    fn offline_activation_requires_complete_coverage() {
        let mut model = RecordingCampaignModel::new(
            WorkloadScope::Selected(vec!["workload-a".into()]),
            observation(CampaignPhase::Recording, 2, 1),
        )
        .unwrap();
        assert_eq!(
            model.request(RecordingAction::ActivateOfflineDefault),
            Err(RecordingModelError::ActionUnavailable)
        );
        model
            .apply(observation(CampaignPhase::Complete, 2, 2))
            .unwrap();
        assert_eq!(
            model
                .request(RecordingAction::ActivateOfflineDefault)
                .unwrap()
                .action,
            RecordingAction::ActivateOfflineDefault
        );
    }

    #[test]
    fn interrupted_capture_requires_reconciliation() {
        let mut model = RecordingCampaignModel::new(
            WorkloadScope::All,
            observation(CampaignPhase::NeedsReconciliation, 3, 1),
        )
        .unwrap();
        assert_eq!(
            model.request(RecordingAction::Reconcile).unwrap().action,
            RecordingAction::Reconcile
        );
        model
            .apply(observation(CampaignPhase::Recording, 3, 1))
            .unwrap();
        assert_eq!(
            model.request(RecordingAction::Progress).unwrap().action,
            RecordingAction::Progress
        );
    }

    #[test]
    fn stale_observations_are_rejected() {
        let mut model = RecordingCampaignModel::new(
            WorkloadScope::All,
            observation(CampaignPhase::Recording, 1, 0),
        )
        .unwrap();
        let mut stale = observation(CampaignPhase::Complete, 1, 1);
        stale.generation = 1;
        assert_eq!(
            model.apply(stale),
            Err(RecordingModelError::StaleObservation)
        );
    }

    #[test]
    fn lifecycle_projection_is_converted_without_plan_inference() {
        let lifecycle = crate::control_codec::RecordingCampaignLifecycle {
            runner_instance_id: "runner-1".into(),
            generation: crate::control_codec::Revision(4),
            campaign_id: "campaign-1".into(),
            provider_id: "provider-1".into(),
            model_id: "model-1".into(),
            agent_ids: vec!["agent-1".into()],
            workload_ids: vec!["workload-1".into()],
            tuple_count: 2,
            covered_tuple_count: 1,
            state: "recording".into(),
            offline_ready: false,
            unavailable_reason: Some("recording-in-progress".into()),
        };
        let observation = CampaignObservation::try_from(&lifecycle).unwrap();
        assert_eq!(observation.phase, CampaignPhase::Recording);
        assert_eq!(
            observation.coverage,
            Coverage {
                requested: 2,
                complete: 1
            }
        );
        assert_eq!(observation.generation, 4);
    }

    #[test]
    fn invalid_observations_and_scopes_fail_closed() {
        assert_eq!(
            WorkloadScope::Selected(Vec::new()).canonical(),
            Err(RecordingModelError::InvalidScope)
        );
        assert_eq!(
            WorkloadScope::Selected(vec!["bad workload".into()]).canonical(),
            Err(RecordingModelError::InvalidScope)
        );
        let mut invalid = observation(CampaignPhase::None, 0, 1);
        assert_eq!(
            RecordingCampaignModel::new(WorkloadScope::All, invalid.clone()),
            Err(RecordingModelError::InvalidObservation)
        );
        invalid = observation(CampaignPhase::Recording, 1, 2);
        assert_eq!(
            RecordingCampaignModel::new(WorkloadScope::All, invalid),
            Err(RecordingModelError::InvalidCoverage)
        );
        let mut invalid = observation(CampaignPhase::Complete, 1, 1);
        invalid.offline_ready = true;
        assert!(RecordingCampaignModel::new(WorkloadScope::All, invalid).is_ok());
        let mut invalid = observation(CampaignPhase::Recording, 1, 1);
        invalid.offline_ready = true;
        assert_eq!(
            RecordingCampaignModel::new(WorkloadScope::All, invalid),
            Err(RecordingModelError::OfflineActivationBlocked)
        );
    }

    #[test]
    fn plan_estimate_cancel_and_reconcile_intents_are_generation_bound() {
        let mut model =
            RecordingCampaignModel::new(WorkloadScope::All, observation(CampaignPhase::None, 0, 0))
                .unwrap();
        for action in [RecordingAction::Estimate, RecordingAction::Plan] {
            assert_eq!(model.request(action).unwrap().expected_generation, 2);
        }
        assert_eq!(
            model.request(RecordingAction::Cancel),
            Err(RecordingModelError::ActionUnavailable)
        );
        model
            .apply(observation(CampaignPhase::Planned, 1, 0))
            .unwrap();
        assert_eq!(
            model.request(RecordingAction::Cancel).unwrap().action,
            RecordingAction::Cancel
        );
        assert_eq!(model.arm_capture_confirmation(), Ok(()));
    }
}
