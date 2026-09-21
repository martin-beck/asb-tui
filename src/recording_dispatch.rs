// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral dispatch for provider refresh and recording campaigns.
//!
//! This is the single runtime seam between action descriptors and the typed
//! authenticated control transport. It carries identifiers only; credentials
//! and readiness claims remain runner-owned.

use crate::{
    actions::UiAction,
    control_transport::{AuthenticatedBrokerSession, TransportError},
    live_projection::{ControlProjection, LiveSnapshot},
    recording_campaign::{
        CampaignObservation, CampaignPhase, RecordingAction, RecordingCampaignModel,
        RecordingModelError, WorkloadScope,
    },
};

pub trait RecordingBackend {
    fn refresh_provider_catalog(
        &mut self,
        projection: &mut ControlProjection,
    ) -> Result<(), TransportError>;
    fn estimate_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        provider: String,
        model: String,
        agents: Vec<String>,
        workloads: Vec<String>,
    ) -> Result<(), TransportError>;
    fn plan_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        provider: String,
        model: String,
        agents: Vec<String>,
        workloads: Vec<String>,
        key: String,
    ) -> Result<(), TransportError>;
    fn execute_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
        key: String,
    ) -> Result<(), TransportError>;
    fn progress_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
    ) -> Result<(), TransportError>;
    fn cancel_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
        key: String,
    ) -> Result<(), TransportError>;
    fn reconcile_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
        key: String,
    ) -> Result<(), TransportError>;
    fn set_recording_campaign_offline_default(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
        key: String,
    ) -> Result<(), TransportError>;
}

macro_rules! forward_backend {
    ($name:ident, $target:ident, ($($arg:ident : $ty:ty),*), ($($call:expr),*)) => {
        fn $name(&mut self, projection: &mut ControlProjection, $($arg: $ty),*) -> Result<(), TransportError> {
            self.$target(projection, $($call),*)
        }
    };
}

impl RecordingBackend for AuthenticatedBrokerSession {
    forward_backend!(refresh_provider_catalog, refresh_provider_catalog, (), ());
    forward_backend!(estimate_recording_campaign, estimate_recording_campaign, (provider: String, model: String, agents: Vec<String>, workloads: Vec<String>), (provider, model, agents, workloads));
    forward_backend!(plan_recording_campaign, plan_recording_campaign, (provider: String, model: String, agents: Vec<String>, workloads: Vec<String>, key: String), (provider, model, agents, workloads, key));
    forward_backend!(execute_recording_campaign, execute_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(progress_recording_campaign, progress_recording_campaign, (campaign: String), (campaign));
    forward_backend!(cancel_recording_campaign, cancel_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(reconcile_recording_campaign, reconcile_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(set_recording_campaign_offline_default, set_recording_campaign_offline_default, (campaign: String, key: String), (campaign, key));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingDispatchState {
    pub provider_id: String,
    pub model_id: String,
    pub agent_ids: Vec<String>,
    pub workload_scope: WorkloadScope,
    capture_armed: bool,
}

impl RecordingDispatchState {
    pub fn new(
        provider_id: String,
        model_id: String,
        agent_ids: Vec<String>,
        workload_scope: WorkloadScope,
    ) -> Result<Self, RecordingModelError> {
        Ok(Self {
            provider_id,
            model_id,
            agent_ids,
            workload_scope: workload_scope.canonical()?,
            capture_armed: false,
        })
    }

    #[must_use]
    pub const fn capture_armed(&self) -> bool {
        self.capture_armed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingDispatchOutcome {
    ProviderCatalogRefreshed,
    EstimateRequested,
    PlanRequested,
    CaptureConfirmationRequired,
    CaptureRequested,
    ProgressRequested,
    CancelRequested,
    ReconcileRequested,
    OfflineDefaultRequested,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RecordingDispatchError {
    Model(RecordingModelError),
    Transport(TransportError),
    MissingCampaign,
    InvalidWorkloadScope,
}

impl From<RecordingModelError> for RecordingDispatchError {
    fn from(value: RecordingModelError) -> Self {
        Self::Model(value)
    }
}

impl From<TransportError> for RecordingDispatchError {
    fn from(value: TransportError) -> Self {
        Self::Transport(value)
    }
}

fn workload_ids(scope: &WorkloadScope) -> Vec<String> {
    match scope {
        WorkloadScope::All => Vec::new(),
        WorkloadScope::Selected(ids) => ids.clone(),
    }
}

fn model_for_snapshot(
    state: &RecordingDispatchState,
    snapshot: &LiveSnapshot,
) -> Result<RecordingCampaignModel, RecordingDispatchError> {
    let observation = if let Some(lifecycle) = snapshot.recording_campaign_lifecycle.as_ref() {
        CampaignObservation::try_from(lifecycle)?
    } else if let Some(plan) = snapshot.recording_campaign.as_ref() {
        CampaignObservation::try_from(plan)?
    } else {
        CampaignObservation {
            generation: snapshot
                .configuration
                .as_ref()
                .map_or(1, |configuration| configuration.generation.0),
            campaign_id: None,
            phase: CampaignPhase::None,
            coverage: crate::recording_campaign::Coverage {
                requested: 0,
                complete: 0,
            },
            offline_ready: false,
        }
    };
    RecordingCampaignModel::new(state.workload_scope.clone(), observation).map_err(Into::into)
}

pub fn dispatch_with_backend<B: RecordingBackend>(
    action: UiAction,
    state: &mut RecordingDispatchState,
    session: &mut B,
    projection: &mut ControlProjection,
    idempotency_key: String,
) -> Result<RecordingDispatchOutcome, RecordingDispatchError> {
    if action == UiAction::RefreshProviderCatalog {
        session.refresh_provider_catalog(projection)?;
        return Ok(RecordingDispatchOutcome::ProviderCatalogRefreshed);
    }
    let snapshot = projection.snapshot();
    let mut model = model_for_snapshot(state, &snapshot)?;
    let workloads = workload_ids(&state.workload_scope);
    match action {
        UiAction::EstimateRecording => {
            model.request(RecordingAction::Estimate)?;
            session
                .estimate_recording_campaign(
                    projection,
                    state.provider_id.clone(),
                    state.model_id.clone(),
                    state.agent_ids.clone(),
                    workloads,
                )
                .map_err(RecordingDispatchError::from)?;
            Ok(RecordingDispatchOutcome::EstimateRequested)
        }
        UiAction::PlanRecording => {
            model.request(RecordingAction::Plan)?;
            session
                .plan_recording_campaign(
                    projection,
                    state.provider_id.clone(),
                    state.model_id.clone(),
                    state.agent_ids.clone(),
                    workloads,
                    idempotency_key,
                )
                .map_err(RecordingDispatchError::from)?;
            Ok(RecordingDispatchOutcome::PlanRequested)
        }
        UiAction::ConfirmRecordingCapture => {
            if !state.capture_armed {
                model.arm_capture_confirmation()?;
                state.capture_armed = true;
                return Ok(RecordingDispatchOutcome::CaptureConfirmationRequired);
            }
            model.arm_capture_confirmation()?;
            model.request(RecordingAction::ConfirmCapture)?;
            let campaign_id = campaign_id(&snapshot)?;
            session.execute_recording_campaign(projection, campaign_id, idempotency_key)?;
            state.capture_armed = false;
            Ok(RecordingDispatchOutcome::CaptureRequested)
        }
        UiAction::ProgressRecording => {
            model.request(RecordingAction::Progress)?;
            let campaign_id = campaign_id(&snapshot)?;
            session.progress_recording_campaign(projection, campaign_id)?;
            Ok(RecordingDispatchOutcome::ProgressRequested)
        }
        UiAction::CancelRecording => {
            model.request(RecordingAction::Cancel)?;
            let campaign_id = campaign_id(&snapshot)?;
            session.cancel_recording_campaign(projection, campaign_id, idempotency_key)?;
            Ok(RecordingDispatchOutcome::CancelRequested)
        }
        UiAction::ReconcileRecording => {
            model.request(RecordingAction::Reconcile)?;
            let campaign_id = campaign_id(&snapshot)?;
            session.reconcile_recording_campaign(projection, campaign_id, idempotency_key)?;
            Ok(RecordingDispatchOutcome::ReconcileRequested)
        }
        UiAction::ActivateOfflineDefault => {
            model.request(RecordingAction::ActivateOfflineDefault)?;
            let campaign_id = campaign_id(&snapshot)?;
            session
                .set_recording_campaign_offline_default(projection, campaign_id, idempotency_key)
                .map_err(RecordingDispatchError::from)?;
            Ok(RecordingDispatchOutcome::OfflineDefaultRequested)
        }
        _ => Err(RecordingDispatchError::InvalidWorkloadScope),
    }
}

fn campaign_id(snapshot: &LiveSnapshot) -> Result<String, RecordingDispatchError> {
    snapshot
        .recording_campaign_lifecycle
        .as_ref()
        .map(|campaign| campaign.campaign_id.clone())
        .or_else(|| {
            snapshot
                .recording_campaign
                .as_ref()
                .map(|campaign| campaign.campaign_id.clone())
        })
        .ok_or(RecordingDispatchError::MissingCampaign)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_codec::{RecordingCampaignLifecycle, Revision};

    #[derive(Default)]
    struct FakeBackend {
        calls: Vec<&'static str>,
        workloads: Vec<String>,
    }

    impl RecordingBackend for FakeBackend {
        fn refresh_provider_catalog(
            &mut self,
            _: &mut ControlProjection,
        ) -> Result<(), TransportError> {
            self.calls.push("refresh");
            Ok(())
        }
        fn estimate_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
            _: Vec<String>,
            workloads: Vec<String>,
        ) -> Result<(), TransportError> {
            self.calls.push("estimate");
            self.workloads = workloads;
            Ok(())
        }
        fn plan_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
            _: Vec<String>,
            workloads: Vec<String>,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("plan");
            self.workloads = workloads;
            Ok(())
        }
        fn execute_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("execute");
            Ok(())
        }
        fn progress_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("progress");
            Ok(())
        }
        fn cancel_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("cancel");
            Ok(())
        }
        fn reconcile_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("reconcile");
            Ok(())
        }
        fn set_recording_campaign_offline_default(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("offline");
            Ok(())
        }
    }

    fn state(scope: WorkloadScope) -> RecordingDispatchState {
        RecordingDispatchState::new(
            "provider".into(),
            "model".into(),
            vec!["agent".into()],
            scope,
        )
        .unwrap()
    }

    fn projection(phase: &str, covered: u16, offline_ready: bool) -> ControlProjection {
        ControlProjection::test_recording_lifecycle(RecordingCampaignLifecycle {
            runner_instance_id: "runner".into(),
            generation: Revision(2),
            campaign_id: "campaign".into(),
            provider_id: "provider".into(),
            model_id: "model".into(),
            agent_ids: vec!["agent".into()],
            workload_ids: vec!["workload".into()],
            tuple_count: 2,
            covered_tuple_count: covered,
            state: phase.into(),
            offline_ready,
            unavailable_reason: None,
        })
    }

    #[test]
    fn dispatches_refresh_estimate_and_selected_plan_without_credentials() {
        let mut backend = FakeBackend::default();
        let mut projection = ControlProjection::default();
        let mut state = state(WorkloadScope::Selected(vec![
            "workload-b".into(),
            "workload-a".into(),
        ]));
        assert_eq!(
            dispatch_with_backend(
                UiAction::RefreshProviderCatalog,
                &mut state,
                &mut backend,
                &mut projection,
                "k".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::ProviderCatalogRefreshed
        );
        assert_eq!(
            dispatch_with_backend(
                UiAction::EstimateRecording,
                &mut state,
                &mut backend,
                &mut projection,
                "k".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::EstimateRequested
        );
        assert_eq!(
            dispatch_with_backend(
                UiAction::PlanRecording,
                &mut state,
                &mut backend,
                &mut projection,
                "k".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::PlanRequested
        );
        assert_eq!(backend.calls, ["refresh", "estimate", "plan"]);
        assert_eq!(backend.workloads, ["workload-a", "workload-b"]);
    }

    #[test]
    fn capture_requires_two_explicit_dispatches_and_lifecycle_actions_are_gated() {
        let mut backend = FakeBackend::default();
        let mut state = state(WorkloadScope::All);
        let mut planned = projection("planned", 0, false);
        assert_eq!(
            dispatch_with_backend(
                UiAction::ConfirmRecordingCapture,
                &mut state,
                &mut backend,
                &mut planned,
                "capture".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::CaptureConfirmationRequired
        );
        assert!(state.capture_armed());
        assert_eq!(
            dispatch_with_backend(
                UiAction::ConfirmRecordingCapture,
                &mut state,
                &mut backend,
                &mut planned,
                "capture".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::CaptureRequested
        );
        let mut recording = projection("recording", 0, false);
        assert_eq!(
            dispatch_with_backend(
                UiAction::ProgressRecording,
                &mut state,
                &mut backend,
                &mut recording,
                "p".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::ProgressRequested
        );
        let mut needs_reconcile = projection("needs_reconciliation", 1, false);
        assert_eq!(
            dispatch_with_backend(
                UiAction::ReconcileRecording,
                &mut state,
                &mut backend,
                &mut needs_reconcile,
                "r".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::ReconcileRequested
        );
        let mut complete = projection("complete", 2, true);
        assert_eq!(
            dispatch_with_backend(
                UiAction::ActivateOfflineDefault,
                &mut state,
                &mut backend,
                &mut complete,
                "o".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::OfflineDefaultRequested
        );
        assert_eq!(
            backend.calls,
            ["execute", "progress", "reconcile", "offline"]
        );
    }

    #[test]
    fn offline_activation_and_wrong_phase_fail_closed() {
        let mut backend = FakeBackend::default();
        let mut state = state(WorkloadScope::All);
        let mut incomplete = projection("complete", 1, false);
        assert!(matches!(
            dispatch_with_backend(
                UiAction::ActivateOfflineDefault,
                &mut state,
                &mut backend,
                &mut incomplete,
                "o".into()
            ),
            Err(RecordingDispatchError::Model(
                RecordingModelError::ActionUnavailable
            ))
        ));
        let mut planned = projection("planned", 0, false);
        assert!(matches!(
            dispatch_with_backend(
                UiAction::CancelRecording,
                &mut state,
                &mut backend,
                &mut planned,
                "c".into()
            ),
            Ok(RecordingDispatchOutcome::CancelRequested)
        ));
        assert_eq!(backend.calls, ["cancel"]);
    }

    #[test]
    fn plan_projection_and_unavailable_actions_fail_closed() {
        let mut backend = FakeBackend::default();
        let mut state = state(WorkloadScope::All);
        let plan = crate::control_codec::RecordingCampaignPlan {
            runner_instance_id: "runner".into(),
            generation: Revision(2),
            campaign_id: "campaign".into(),
            provider_id: "provider".into(),
            model_id: "model".into(),
            agent_ids: vec!["agent".into()],
            workload_ids: vec!["workload".into()],
            tuple_count: 2,
            state: "planned".into(),
            offline_ready: false,
            unavailable_reason: None,
        };
        let mut planned = ControlProjection::test_recording_plan(plan);
        assert_eq!(
            dispatch_with_backend(
                UiAction::ProgressRecording,
                &mut state,
                &mut backend,
                &mut planned,
                "p".into()
            ),
            Err(RecordingDispatchError::Model(
                RecordingModelError::ActionUnavailable
            ))
        );
        assert_eq!(
            dispatch_with_backend(
                UiAction::ConfirmRecordingCapture,
                &mut state,
                &mut backend,
                &mut planned,
                "c".into()
            ),
            Ok(RecordingDispatchOutcome::CaptureConfirmationRequired)
        );
        assert_eq!(
            dispatch_with_backend(
                UiAction::ConfirmRecordingCapture,
                &mut state,
                &mut backend,
                &mut planned,
                "c".into()
            ),
            Ok(RecordingDispatchOutcome::CaptureRequested)
        );
        assert_eq!(backend.calls, ["execute"]);
        assert_eq!(
            dispatch_with_backend(
                UiAction::OpenHelp,
                &mut state,
                &mut backend,
                &mut planned,
                "x".into()
            ),
            Err(RecordingDispatchError::InvalidWorkloadScope)
        );
    }

    #[test]
    fn transport_errors_remain_typed_and_content_free() {
        assert_eq!(
            RecordingDispatchError::from(TransportError::RemoteFailure),
            RecordingDispatchError::Transport(TransportError::RemoteFailure)
        );
    }

    #[test]
    fn action_identifiers_are_stable_for_recordings() {
        assert_eq!(
            UiAction::RefreshProviderCatalog.id(),
            "refresh_provider_catalog"
        );
        assert_eq!(UiAction::EstimateRecording.id(), "estimate_recording");
        assert_eq!(UiAction::PlanRecording.id(), "plan_recording");
        assert_eq!(
            UiAction::ConfirmRecordingCapture.id(),
            "confirm_recording_capture"
        );
        assert_eq!(UiAction::ProgressRecording.id(), "progress_recording");
        assert_eq!(UiAction::CancelRecording.id(), "cancel_recording");
        assert_eq!(UiAction::ReconcileRecording.id(), "reconcile_recording");
        assert_eq!(
            UiAction::ActivateOfflineDefault.id(),
            "activate_offline_default"
        );
        assert_eq!(workload_ids(&WorkloadScope::All), Vec::<String>::new());
        assert!(!state(WorkloadScope::All).capture_armed());
        assert_eq!(
            UiAction::RefreshProviderCatalog,
            UiAction::RefreshProviderCatalog
        );
        assert_eq!(UiAction::EstimateRecording, UiAction::EstimateRecording);
        assert_eq!(UiAction::PlanRecording, UiAction::PlanRecording);
    }
}
