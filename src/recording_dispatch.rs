// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral dispatch for provider refresh and recording campaigns.
//!
//! This is the single runtime seam between action descriptors and the typed
//! authenticated control transport. It carries identifiers only; credentials
//! and readiness claims remain runner-owned.

use crate::{
    agent_catalog::AgentCatalog,
    actions::UiAction,
    control_transport::{AuthenticatedBrokerSession, TransportError},
    live_projection::{ControlProjection, LiveSnapshot},
    recording_campaign::{
        CampaignObservation, CampaignPhase, RecordingAction, RecordingCampaignModel,
        RecordingModelError, WorkloadScope,
    },
    provider_catalog::AgentScope,
};

pub trait RecordingBackend {
    fn repeat_run(
        &mut self,
        projection: &mut ControlProjection,
        run_id: crate::control_codec::RunId,
        key: String,
    ) -> Result<(), TransportError>;
    fn compare_live_offline(
        &mut self,
        projection: &mut ControlProjection,
        run_ids: Vec<crate::control_codec::RunId>,
    ) -> Result<(), TransportError>;
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
    fn seal_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
        key: String,
    ) -> Result<(), TransportError>;
    fn reopen_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
        key: String,
    ) -> Result<(), TransportError>;
    fn remove_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign: String,
        key: String,
        cassette_sha256: String,
    ) -> Result<(), TransportError>;
    fn retry_recording_campaign(
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
    forward_backend!(repeat_run, repeat_run, (run_id: crate::control_codec::RunId, key: String), (run_id, key));
    forward_backend!(compare_live_offline, analyze_runs, (run_ids: Vec<crate::control_codec::RunId>), (run_ids));
    forward_backend!(refresh_provider_catalog, refresh_provider_catalog, (), ());
    forward_backend!(estimate_recording_campaign, estimate_recording_campaign, (provider: String, model: String, agents: Vec<String>, workloads: Vec<String>), (provider, model, agents, workloads));
    forward_backend!(plan_recording_campaign, plan_recording_campaign, (provider: String, model: String, agents: Vec<String>, workloads: Vec<String>, key: String), (provider, model, agents, workloads, key));
    forward_backend!(execute_recording_campaign, execute_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(progress_recording_campaign, progress_recording_campaign, (campaign: String), (campaign));
    forward_backend!(cancel_recording_campaign, cancel_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(reconcile_recording_campaign, reconcile_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(set_recording_campaign_offline_default, set_recording_campaign_offline_default, (campaign: String, key: String), (campaign, key));
    forward_backend!(seal_recording_campaign, seal_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(reopen_recording_campaign, reopen_recording_campaign, (campaign: String, key: String), (campaign, key));
    forward_backend!(remove_recording_campaign, remove_recording_campaign, (campaign: String, key: String, cassette_sha256: String), (campaign, key, cassette_sha256));
    forward_backend!(retry_recording_campaign, retry_recording_campaign, (campaign: String, key: String), (campaign, key));
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingDispatchState {
    /// Stable coding-agent adapter identity carried alongside recording and
    /// replay requests; it is never inferred from a display label.
    pub adapter_id: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub agent_ids: Vec<String>,
    pub workload_scope: WorkloadScope,
    capture_armed: bool,
    /// The last authenticated digest-only catalog fetched after activation.
    /// Cassette contents never enter this state.
    pub authenticated_catalog: Option<crate::benchmark_route::AuthenticatedCassetteCatalog>,
    /// Explicit cassette selected by the user from the authenticated catalog.
    /// A replay action never guesses or falls back to the first entry.
    pub selected_cassette_sha256: Option<String>,
    /// Number of successful bounded progress observations for this campaign.
    /// A frontend must not turn a stalled runner into an unbounded poll loop.
    progress_requests: u16,
    progress_campaign_id: Option<String>,
    pub selected_run_id: Option<crate::control_codec::RunId>,
    pub comparison_run_ids: Vec<crate::control_codec::RunId>,
    remove_confirmation_required: bool,
}

impl RecordingDispatchState {
    pub fn new(
        provider_id: String,
        model_id: String,
        agent_ids: Vec<String>,
        workload_scope: WorkloadScope,
    ) -> Result<Self, RecordingModelError> {
        Ok(Self {
            adapter_id: None,
            provider_id,
            model_id,
            agent_ids,
            workload_scope: workload_scope.canonical()?,
            capture_armed: false,
            authenticated_catalog: None,
            selected_cassette_sha256: None,
            progress_requests: 0,
            progress_campaign_id: None,
            selected_run_id: None,
            comparison_run_ids: Vec::new(),
            remove_confirmation_required: false,
        })
    }

    /// Construct a recording state from the explicit selected/all agent scope.
    /// `All` is resolved against the authenticated catalog before it reaches
    /// the wire, so the backend keeps its non-empty, digest-bound agent list
    /// contract while the frontend still exposes an honest all-agents choice.
    pub fn with_agent_scope(
        provider_id: String,
        model_id: String,
        scope: AgentScope,
        agents: &AgentCatalog,
        workload_scope: WorkloadScope,
    ) -> Result<Self, RecordingModelError> {
        let agent_ids = scope
            .resolve(agents)
            .map_err(|_| RecordingModelError::InvalidScope)?;
        Self::new(provider_id, model_id, agent_ids, workload_scope)
    }

    #[must_use]
    pub const fn capture_armed(&self) -> bool {
        self.capture_armed
    }

    #[must_use]
    pub const fn progress_requests(&self) -> u16 {
        self.progress_requests
    }

    pub fn select_cassette(&mut self, cassette_sha256: impl Into<String>) {
        self.selected_cassette_sha256 = Some(cassette_sha256.into());
    }

    pub fn bind_adapter_id(&mut self, adapter_id: impl Into<String>) -> Result<(), String> {
        let adapter_id = adapter_id.into();
        if crate::adapter_catalog::AdapterCatalog::development()
            .get(&adapter_id)
            .is_none()
        {
            return Err("unknown coding-agent adapter".into());
        }
        self.adapter_id = Some(adapter_id);
        Ok(())
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
    SealRequested,
    ReopenRequested,
    RemoveRequested,
    RemovalConfirmationRequired,
    RetryRequested,
    ComparisonRequested,
    ReplayDispatched,
    CassetteSelected,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RecordingDispatchError {
    Model(RecordingModelError),
    Transport(TransportError),
    MissingCampaign,
    InvalidWorkloadScope,
    WorkloadCatalogUnavailable,
    ProgressLimitReached,
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

const MAX_PROGRESS_REQUESTS: u16 = 256;

/// Expand the explicit `All` marker against the authenticated benchmark
/// catalog.  An empty vector is never a valid wire representation of `All`:
/// the control contract treats it as an empty campaign.
fn workload_ids(
    scope: &WorkloadScope,
    catalog: Option<&crate::live_projection::LiveBenchmarkCatalog>,
) -> Result<Vec<String>, RecordingDispatchError> {
    match scope {
        WorkloadScope::Selected(ids) => Ok(ids.clone()),
        WorkloadScope::All => {
            let catalog = catalog.ok_or(RecordingDispatchError::WorkloadCatalogUnavailable)?;
            let mut ids = Vec::new();
            for pool in &catalog.pools {
                for group in &pool.groups {
                    for benchmark in &group.benchmarks {
                        if !ids.iter().any(|id| id == &benchmark.id) {
                            ids.push(benchmark.id.clone());
                        }
                    }
                }
            }
            if ids.is_empty() || ids.len() > crate::recording_campaign::MAX_WORKLOADS {
                return Err(RecordingDispatchError::WorkloadCatalogUnavailable);
            }
            ids.sort();
            Ok(ids)
        }
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
    match action {
        UiAction::EstimateRecording => {
            let workloads =
                workload_ids(&state.workload_scope, snapshot.benchmark_catalog.as_ref())?;
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
            let workloads =
                workload_ids(&state.workload_scope, snapshot.benchmark_catalog.as_ref())?;
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
            let requests = state
                .progress_campaign_id
                .as_deref()
                .filter(|known| *known == campaign_id)
                .map_or(0, |_| state.progress_requests);
            if requests >= MAX_PROGRESS_REQUESTS {
                return Err(RecordingDispatchError::ProgressLimitReached);
            }
            session.progress_recording_campaign(projection, campaign_id.clone())?;
            if state.progress_campaign_id.as_deref() != Some(campaign_id.as_str()) {
                state.progress_campaign_id = Some(campaign_id);
                state.progress_requests = 1;
            } else {
                state.progress_requests = requests.saturating_add(1);
            }
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
        UiAction::SealRecording => {
            model.request(RecordingAction::Seal)?;
            let campaign_id = campaign_id(&snapshot)?;
            session.seal_recording_campaign(projection, campaign_id, idempotency_key)?;
            Ok(RecordingDispatchOutcome::SealRequested)
        }
        UiAction::ReopenRecording => {
            model.request(RecordingAction::Reopen)?;
            let campaign_id = campaign_id(&snapshot)?;
            session.reopen_recording_campaign(projection, campaign_id, idempotency_key)?;
            Ok(RecordingDispatchOutcome::ReopenRequested)
        }
        UiAction::RemoveRecordingCassette => {
            model.request(RecordingAction::Remove)?;
            let selected = state
                .selected_cassette_sha256
                .as_deref()
                .ok_or(RecordingDispatchError::InvalidWorkloadScope)?;
            if state.authenticated_catalog.as_ref().is_none_or(|catalog| {
                !catalog
                    .entries
                    .iter()
                    .any(|entry| entry.cassette_sha256 == selected)
            }) {
                return Err(RecordingDispatchError::InvalidWorkloadScope);
            }
            if !state.remove_confirmation_required {
                state.remove_confirmation_required = true;
                return Ok(RecordingDispatchOutcome::RemovalConfirmationRequired);
            }
            state.remove_confirmation_required = false;
            let campaign_id = campaign_id(&snapshot)?;
            session.remove_recording_campaign(
                projection,
                campaign_id,
                idempotency_key,
                selected.to_owned(),
            )?;
            Ok(RecordingDispatchOutcome::RemoveRequested)
        }
        UiAction::RetryRun => {
            let run_id = state
                .selected_run_id
                .clone()
                .ok_or(RecordingDispatchError::MissingCampaign)?;
            session.repeat_run(projection, run_id, idempotency_key)?;
            Ok(RecordingDispatchOutcome::RetryRequested)
        }
        UiAction::CompareLiveOffline => {
            if state.comparison_run_ids.len() != 2 {
                return Err(RecordingDispatchError::InvalidWorkloadScope);
            }
            session.compare_live_offline(projection, state.comparison_run_ids.clone())?;
            Ok(RecordingDispatchOutcome::ComparisonRequested)
        }
        UiAction::ReplaySelected | UiAction::SelectOfflineCassette => {
            Err(RecordingDispatchError::InvalidWorkloadScope)
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
        repeated_run: Option<String>,
        compared_runs: Vec<String>,
    }

    impl RecordingBackend for FakeBackend {
        fn repeat_run(
            &mut self,
            _: &mut ControlProjection,
            run: crate::control_codec::RunId,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("repeat");
            self.repeated_run = Some(run.0);
            Ok(())
        }
        fn compare_live_offline(
            &mut self,
            _: &mut ControlProjection,
            runs: Vec<crate::control_codec::RunId>,
        ) -> Result<(), TransportError> {
            self.calls.push("compare");
            self.compared_runs = runs.into_iter().map(|run| run.0).collect();
            Ok(())
        }
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
        fn seal_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("seal");
            Ok(())
        }
        fn reopen_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("reopen");
            Ok(())
        }
        fn remove_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("remove");
            Ok(())
        }
        fn retry_recording_campaign(
            &mut self,
            _: &mut ControlProjection,
            _: String,
            _: String,
        ) -> Result<(), TransportError> {
            self.calls.push("retry");
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
        lifecycle_projection(RecordingCampaignLifecycle {
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

    fn lifecycle_projection(lifecycle: RecordingCampaignLifecycle) -> ControlProjection {
        ControlProjection::test_recording_lifecycle(lifecycle)
    }

    fn projection_without_catalog(
        phase: &str,
        covered: u16,
        offline_ready: bool,
    ) -> ControlProjection {
        ControlProjection::test_recording_lifecycle_without_benchmark_catalog(
            RecordingCampaignLifecycle {
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
            },
        )
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
        state.selected_run_id = Some(crate::control_codec::RunId("run-1".into()));
        state.selected_cassette_sha256 = Some("a".repeat(64));
        state.authenticated_catalog = Some(crate::benchmark_route::AuthenticatedCassetteCatalog {
            runner_instance_id: "runner".into(),
            generation: crate::control_codec::Revision(2),
            campaign_id: "campaign".into(),
            entries: vec![crate::benchmark_route::AuthenticatedCassetteEntry {
                cassette_id: "cassette".into(),
                cassette_sha256: "a".repeat(64),
                provider_profile_sha256: "b".repeat(64),
                agent_id: "agent".into(),
                workload_id: "workload".into(),
                scorer_revision: "scorer".into(),
            }],
        });
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
        assert_eq!(
            workload_ids(&WorkloadScope::All, None),
            Err(RecordingDispatchError::WorkloadCatalogUnavailable)
        );
        let catalog = projection("planned", 0, false).snapshot().benchmark_catalog;
        assert_eq!(
            workload_ids(&WorkloadScope::All, catalog.as_ref()).unwrap(),
            vec!["workload"]
        );
        assert!(!state(WorkloadScope::All).capture_armed());
        assert_eq!(
            UiAction::RefreshProviderCatalog,
            UiAction::RefreshProviderCatalog
        );
        assert_eq!(UiAction::EstimateRecording, UiAction::EstimateRecording);
        assert_eq!(UiAction::PlanRecording, UiAction::PlanRecording);
    }

    #[test]
    fn recording_dispatch_carries_selected_adapter_identity() {
        let mut state = state(WorkloadScope::All);
        assert!(state.bind_adapter_id("opendesk").is_ok());
        assert_eq!(state.adapter_id.as_deref(), Some("opendesk"));
        assert!(state.bind_adapter_id("unknown").is_err());
        assert_eq!(state.adapter_id.as_deref(), Some("opendesk"));
    }

    #[test]
    fn lifecycle_actions_do_not_require_workload_catalog() {
        let mut backend = FakeBackend::default();
        let mut state = state(WorkloadScope::All);
        let mut recording = projection_without_catalog("recording", 0, false);
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
        let mut planned = projection_without_catalog("planned", 0, false);
        assert_eq!(
            dispatch_with_backend(
                UiAction::CancelRecording,
                &mut state,
                &mut backend,
                &mut planned,
                "c".into()
            )
            .unwrap(),
            RecordingDispatchOutcome::CancelRequested
        );
    }

    #[test]
    fn progress_limit_is_per_campaign_and_failed_polls_do_not_consume_budget() {
        let mut backend = FakeBackend::default();
        let mut state = state(WorkloadScope::Selected(vec!["workload".into()]));
        state.progress_campaign_id = Some("campaign".into());
        state.progress_requests = MAX_PROGRESS_REQUESTS;
        let mut exhausted = projection("recording", 0, false);
        assert_eq!(
            dispatch_with_backend(
                UiAction::ProgressRecording,
                &mut state,
                &mut backend,
                &mut exhausted,
                "p".into()
            ),
            Err(RecordingDispatchError::ProgressLimitReached)
        );
        let mut next_campaign = projection("recording", 0, false);
        // A fresh campaign identity resets the bounded poll budget.
        let snapshot = next_campaign.snapshot();
        let mut lifecycle = snapshot.recording_campaign_lifecycle.unwrap();
        lifecycle.campaign_id = "campaign-next".into();
        next_campaign = ControlProjection::test_recording_lifecycle(lifecycle);
        assert_eq!(
            dispatch_with_backend(
                UiAction::ProgressRecording,
                &mut state,
                &mut backend,
                &mut next_campaign,
                "p".into()
            ),
            Ok(RecordingDispatchOutcome::ProgressRequested)
        );
        assert_eq!(state.progress_requests(), 1);
    }

    #[test]
    fn repair_actions_dispatch_typed_backend_operations() {
        let mut backend = FakeBackend::default();
        let mut state = state(WorkloadScope::All);
        state.selected_run_id = Some(crate::control_codec::RunId("run-1".into()));
        state.selected_cassette_sha256 = Some("a".repeat(64));
        state.authenticated_catalog = Some(crate::benchmark_route::AuthenticatedCassetteCatalog {
            runner_instance_id: "runner".into(),
            generation: crate::control_codec::Revision(2),
            campaign_id: "campaign".into(),
            entries: vec![crate::benchmark_route::AuthenticatedCassetteEntry {
                cassette_id: "cassette".into(),
                cassette_sha256: "a".repeat(64),
                provider_profile_sha256: "b".repeat(64),
                agent_id: "agent".into(),
                workload_id: "workload".into(),
                scorer_revision: "scorer".into(),
            }],
        });
        state.selected_run_id = Some(crate::control_codec::RunId("run-1".into()));
        state.selected_cassette_sha256 = Some("a".repeat(64));
        for (action, expected, phase, covered) in [
            (
                UiAction::SealRecording,
                RecordingDispatchOutcome::SealRequested,
                "complete",
                2,
            ),
            (
                UiAction::ReopenRecording,
                RecordingDispatchOutcome::ReopenRequested,
                "needs_reconciliation",
                1,
            ),
            (
                UiAction::RemoveRecordingCassette,
                RecordingDispatchOutcome::RemoveRequested,
                "complete",
                2,
            ),
            (
                UiAction::RetryRun,
                RecordingDispatchOutcome::RetryRequested,
                "needs_reconciliation",
                1,
            ),
        ] {
            let mut projection = projection(phase, covered, phase == "complete");
            if action == UiAction::RemoveRecordingCassette {
                assert_eq!(
                    dispatch_with_backend(
                        action,
                        &mut state,
                        &mut backend,
                        &mut projection,
                        "confirm".into()
                    )
                    .unwrap(),
                    RecordingDispatchOutcome::RemovalConfirmationRequired
                );
            }
            assert_eq!(
                dispatch_with_backend(
                    action,
                    &mut state,
                    &mut backend,
                    &mut projection,
                    "repair".into()
                )
                .unwrap(),
                expected
            );
        }
        assert_eq!(backend.calls, ["seal", "reopen", "remove", "repeat"]);
        assert_eq!(backend.repeated_run.as_deref(), Some("run-1"));
        state.comparison_run_ids = vec![
            crate::control_codec::RunId("run-a".into()),
            crate::control_codec::RunId("run-b".into()),
        ];
        let mut comparison_projection = ControlProjection::default();
        assert_eq!(
            dispatch_with_backend(
                UiAction::CompareLiveOffline,
                &mut state,
                &mut backend,
                &mut comparison_projection,
                "compare".into()
            ),
            Ok(RecordingDispatchOutcome::ComparisonRequested)
        );
        assert_eq!(backend.compared_runs, ["run-a", "run-b"]);
    }
}
