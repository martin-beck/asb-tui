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

impl RecordingBackend for AuthenticatedBrokerSession {
    fn refresh_provider_catalog(
        &mut self,
        p: &mut ControlProjection,
    ) -> Result<(), TransportError> {
        self.refresh_provider_catalog(p)
    }
    fn estimate_recording_campaign(
        &mut self,
        p: &mut ControlProjection,
        a: String,
        b: String,
        c: Vec<String>,
        d: Vec<String>,
    ) -> Result<(), TransportError> {
        self.estimate_recording_campaign(p, a, b, c, d)
    }
    fn plan_recording_campaign(
        &mut self,
        p: &mut ControlProjection,
        a: String,
        b: String,
        c: Vec<String>,
        d: Vec<String>,
        e: String,
    ) -> Result<(), TransportError> {
        self.plan_recording_campaign(p, a, b, c, d, e)
    }
    fn execute_recording_campaign(
        &mut self,
        p: &mut ControlProjection,
        a: String,
        b: String,
    ) -> Result<(), TransportError> {
        self.execute_recording_campaign(p, a, b)
    }
    fn progress_recording_campaign(
        &mut self,
        p: &mut ControlProjection,
        a: String,
    ) -> Result<(), TransportError> {
        self.progress_recording_campaign(p, a)
    }
    fn cancel_recording_campaign(
        &mut self,
        p: &mut ControlProjection,
        a: String,
        b: String,
    ) -> Result<(), TransportError> {
        self.cancel_recording_campaign(p, a, b)
    }
    fn reconcile_recording_campaign(
        &mut self,
        p: &mut ControlProjection,
        a: String,
        b: String,
    ) -> Result<(), TransportError> {
        self.reconcile_recording_campaign(p, a, b)
    }
    fn set_recording_campaign_offline_default(
        &mut self,
        p: &mut ControlProjection,
        a: String,
        b: String,
    ) -> Result<(), TransportError> {
        self.set_recording_campaign_offline_default(p, a, b)
    }
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

/// Dispatch one renderer-neutral recording/provider action. Remote success is
/// never inferred: each transport call must return and validate an
/// authoritative projection before the caller renders the next state.
pub fn dispatch(
    action: UiAction,
    state: &mut RecordingDispatchState,
    session: &mut AuthenticatedBrokerSession,
    projection: &mut ControlProjection,
    idempotency_key: String,
) -> Result<RecordingDispatchOutcome, RecordingDispatchError> {
    dispatch_with_backend(action, state, session, projection, idempotency_key)
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
            session.estimate_recording_campaign(
                projection,
                state.provider_id.clone(),
                state.model_id.clone(),
                state.agent_ids.clone(),
                workloads,
            )?;
            Ok(RecordingDispatchOutcome::EstimateRequested)
        }
        UiAction::PlanRecording => {
            model.request(RecordingAction::Plan)?;
            session.plan_recording_campaign(
                projection,
                state.provider_id.clone(),
                state.model_id.clone(),
                state.agent_ids.clone(),
                workloads,
                idempotency_key,
            )?;
            Ok(RecordingDispatchOutcome::PlanRequested)
        }
        UiAction::ConfirmRecordingCapture => {
            if !state.capture_armed {
                model.arm_capture_confirmation()?;
                state.capture_armed = true;
                return Ok(RecordingDispatchOutcome::CaptureConfirmationRequired);
            }
            model.request(RecordingAction::ConfirmCapture)?;
            let campaign_id = snapshot
                .recording_campaign
                .as_ref()
                .map(|plan| plan.campaign_id.clone())
                .ok_or(RecordingDispatchError::MissingCampaign)?;
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
            session.set_recording_campaign_offline_default(
                projection,
                campaign_id,
                idempotency_key,
            )?;
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
