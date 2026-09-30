// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Generation-bound benchmark launch and truthful live-statistics state.
//!
//! This module is deliberately independent of Ratatui and sockets.  The
//! authenticated transport validates a [`LaunchRequest`] before its first
//! control write, while this state machine validates every subsequent event.
//! A reconnect therefore preserves the last known state instead of inventing
//! progress or silently starting a second attempt.

use crate::{
    configuration_materialization::{LaunchBinding, MaterializedBundle},
    control_codec::{AttemptId, PublicRunState, Revision, RunId},
    live_projection::{Connection, LiveSnapshot},
};

const MAX_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchValidationError {
    EmptyId(&'static str),
    InvalidId(&'static str),
    BindingDigestMismatch,
    ProviderCatalogUnavailable,
    ProviderCatalogGenerationMismatch,
    ProviderCatalogDigestMismatch,
    ConfigurationUnavailable,
    ConfigurationMismatch,
    Disconnected,
    InvalidBundle,
    InvalidBenchmarkSelection,
    BenchmarkCatalogUnavailable,
    BenchmarkCatalogGenerationMismatch,
}

/// A plan launch request bound to exactly one reviewed materialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchRequest {
    pub binding: LaunchBinding,
    pub idempotency_key: String,
}

impl LaunchRequest {
    pub fn new(
        bundle: &MaterializedBundle,
        snapshot: &LiveSnapshot,
        idempotency_key: impl Into<String>,
    ) -> Result<Self, LaunchValidationError> {
        let key = idempotency_key.into();
        validate_id(&key, "idempotency key")?;
        validate_binding(bundle, snapshot)?;
        Ok(Self {
            binding: bundle.launch_binding(),
            idempotency_key: key,
        })
    }
}

/// Validate all externally observable generation and selection fences before
/// any control I/O.  A missing live catalog/configuration is unavailable, not
/// permission to use a stale or synthetic value.
pub fn validate_binding(
    bundle: &MaterializedBundle,
    snapshot: &LiveSnapshot,
) -> Result<(), LaunchValidationError> {
    bundle
        .validate_integrity()
        .map_err(|_| LaunchValidationError::InvalidBundle)?;
    let binding = bundle.launch_binding();
    if binding.materialization_digest_sha256 != bundle.digest_sha256
        || bundle.document.configuration_digest_sha256 != bundle.digest_sha256
    {
        return Err(LaunchValidationError::BindingDigestMismatch);
    }
    let provider = snapshot
        .provider_catalog
        .as_ref()
        .ok_or(LaunchValidationError::ProviderCatalogUnavailable)?;
    if provider.generation != binding.provider_catalog_generation {
        return Err(LaunchValidationError::ProviderCatalogGenerationMismatch);
    }
    if provider.catalog_sha256 != binding.provider_catalog_digest {
        return Err(LaunchValidationError::ProviderCatalogDigestMismatch);
    }
    if binding.benchmark_catalog_generation.0 == 0
        || binding.benchmark_catalog_digest.len() != 64
        || binding.pool_id.is_empty()
        || binding.group_ids.is_empty()
        || binding.benchmark_ids.is_empty()
        || binding.measure_ids.is_empty()
        || has_duplicate(&binding.group_ids)
        || has_duplicate(&binding.benchmark_ids)
        || has_duplicate(&binding.measure_ids)
    {
        return Err(LaunchValidationError::InvalidBenchmarkSelection);
    }
    let catalog = snapshot
        .benchmark_catalog
        .as_ref()
        .ok_or(LaunchValidationError::BenchmarkCatalogUnavailable)?;
    if catalog.generation != binding.benchmark_catalog_generation {
        return Err(LaunchValidationError::BenchmarkCatalogGenerationMismatch);
    }
    if catalog.catalog_sha256 != binding.benchmark_catalog_digest {
        return Err(LaunchValidationError::InvalidBenchmarkSelection);
    }
    let Some(pool) = catalog.pools.iter().find(|pool| pool.id == binding.pool_id) else {
        return Err(LaunchValidationError::InvalidBenchmarkSelection);
    };
    if binding.group_ids.iter().any(|group_id| {
        let Some(group) = pool.groups.iter().find(|group| &group.id == group_id) else {
            return true;
        };
        binding.benchmark_ids.iter().any(|benchmark_id| {
            let Some(benchmark) = group
                .benchmarks
                .iter()
                .find(|benchmark| &benchmark.id == benchmark_id)
            else {
                return true;
            };
            binding
                .measure_ids
                .iter()
                .any(|measure_id| !benchmark.measure_ids.contains(measure_id))
        })
    }) {
        return Err(LaunchValidationError::InvalidBenchmarkSelection);
    }
    if snapshot.connection != Connection::Negotiated {
        return Err(LaunchValidationError::Disconnected);
    }
    let configuration = snapshot
        .configuration
        .as_ref()
        .ok_or(LaunchValidationError::ConfigurationUnavailable)?;
    if !configuration.configured
        || configuration.provider_id.as_deref() != Some(binding.provider_id.as_str())
        || configuration.model_id.as_deref() != Some(binding.model_id.as_str())
        || configuration.agent_ids != binding.agent_ids
    {
        return Err(LaunchValidationError::ConfigurationMismatch);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatisticsProvenance {
    DevelopmentMock,
    Replayed,
    Live,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveStatistics {
    pub run_id: RunId,
    pub attempt_id: AttemptId,
    pub generation: Revision,
    pub state: PublicRunState,
    pub completed_measures: u16,
    pub total_measures: u16,
    pub failed_measures: u16,
    pub throughput_per_second: Option<u32>,
    pub latency_millis: Option<u32>,
    pub provenance: StatisticsProvenance,
    pub unavailable_reason: Option<String>,
}

impl LiveStatistics {
    #[must_use]
    pub fn bounded_progress_percent(&self) -> u8 {
        if self.total_measures == 0 {
            return 0;
        }
        let completed = u32::from(self.completed_measures.min(self.total_measures));
        ((completed * 100) / u32::from(self.total_measures)) as u8
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveRunEvent {
    pub revision: Revision,
    pub run_id: RunId,
    pub attempt_id: AttemptId,
    pub generation: Revision,
    pub state: PublicRunState,
    pub completed_measures: u16,
    pub total_measures: u16,
    pub failed_measures: u16,
    pub throughput_per_second: Option<u32>,
    pub latency_millis: Option<u32>,
    pub provenance: StatisticsProvenance,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunConnection {
    Connected,
    Reconnecting,
    Disconnected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunState {
    Idle,
    Launching,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunEventError {
    WrongRun,
    WrongAttempt,
    StaleRevision,
    StaleGeneration,
    ProgressOutOfBounds,
    InvalidUnavailableState,
    TerminalRegression,
}

/// Single-writer state for the run-control screen.  It retains partial
/// statistics during cancellation, failure, and reconnect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchState {
    binding: LaunchBinding,
    run_id: Option<RunId>,
    attempt_id: Option<AttemptId>,
    generation: Revision,
    connection: RunConnection,
    state: RunState,
    last_revision: Option<Revision>,
    statistics: Option<LiveStatistics>,
}

impl LaunchState {
    #[must_use]
    pub fn new(binding: LaunchBinding) -> Self {
        Self {
            generation: binding.benchmark_catalog_generation,
            binding,
            run_id: None,
            attempt_id: None,
            connection: RunConnection::Connected,
            state: RunState::Idle,
            last_revision: None,
            statistics: None,
        }
    }

    pub fn begin_launch(&mut self) -> Result<(), RunEventError> {
        if self.connection != RunConnection::Connected || self.state != RunState::Idle {
            return Err(RunEventError::StaleRevision);
        }
        self.state = RunState::Launching;
        Ok(())
    }

    pub fn request_cancel(&mut self) -> Result<(), RunEventError> {
        if !matches!(self.state, RunState::Launching | RunState::Running) {
            return Err(RunEventError::StaleRevision);
        }
        self.state = RunState::Cancelling;
        Ok(())
    }

    pub fn reconnect_started(&mut self) {
        self.connection = RunConnection::Reconnecting;
    }

    pub fn reconnect_complete(&mut self, latest_revision: Option<Revision>) {
        self.connection = RunConnection::Connected;
        if let Some(revision) = latest_revision
            && self.last_revision.is_none_or(|known| revision > known)
        {
            self.last_revision = Some(revision);
        }
    }

    pub fn disconnected(&mut self) {
        self.connection = RunConnection::Disconnected;
    }

    pub fn apply_event(&mut self, event: LiveRunEvent) -> Result<(), RunEventError> {
        if self.connection == RunConnection::Disconnected {
            return Err(RunEventError::StaleRevision);
        }
        if self
            .last_revision
            .is_some_and(|known| event.revision <= known)
        {
            return Err(RunEventError::StaleRevision);
        }
        if event.generation != self.generation {
            return Err(RunEventError::StaleGeneration);
        }
        let next_state = state_for_public(event.state);
        if is_terminal(&self.state) && self.state != next_state {
            return Err(RunEventError::TerminalRegression);
        }
        if let Some(run_id) = &self.run_id {
            if run_id != &event.run_id {
                return Err(RunEventError::WrongRun);
            }
        } else {
            self.run_id = Some(event.run_id.clone());
        }
        if let Some(attempt_id) = &self.attempt_id {
            if attempt_id != &event.attempt_id {
                return Err(RunEventError::WrongAttempt);
            }
        } else {
            self.attempt_id = Some(event.attempt_id.clone());
        }
        if event.completed_measures > event.total_measures
            || event.failed_measures > event.total_measures
        {
            return Err(RunEventError::ProgressOutOfBounds);
        }
        if matches!(event.provenance, StatisticsProvenance::Unavailable)
            && event.unavailable_reason.is_none()
        {
            return Err(RunEventError::InvalidUnavailableState);
        }
        self.last_revision = Some(event.revision);
        self.state = next_state;
        self.statistics = Some(LiveStatistics {
            run_id: event.run_id,
            attempt_id: event.attempt_id,
            generation: event.generation,
            state: event.state,
            completed_measures: event.completed_measures,
            total_measures: event.total_measures,
            failed_measures: event.failed_measures,
            throughput_per_second: event.throughput_per_second,
            latency_millis: event.latency_millis,
            provenance: event.provenance,
            unavailable_reason: event.unavailable_reason,
        });
        Ok(())
    }

    /// Project the runner's coarse run summary when a detailed event stream
    /// is unavailable. The view remains explicit about unavailable measures
    /// instead of presenting a fabricated completion percentage.
    pub fn observe_summary(
        &mut self,
        summary: &crate::control_codec::RunSummary,
    ) -> Result<(), RunEventError> {
        let total = u16::try_from(self.binding.measure_ids.len()).unwrap_or(u16::MAX);
        let terminal = matches!(
            summary.state,
            PublicRunState::Completed | PublicRunState::Failed | PublicRunState::Cancelled
        );
        self.apply_event(LiveRunEvent {
            revision: summary.revision,
            run_id: summary.run_id.clone(),
            attempt_id: summary.attempt_id.clone(),
            generation: self.generation,
            state: summary.state,
            completed_measures: if terminal { total } else { 0 },
            total_measures: total,
            failed_measures: if matches!(summary.state, PublicRunState::Failed) {
                1
            } else {
                0
            },
            throughput_per_second: None,
            latency_millis: None,
            provenance: StatisticsProvenance::Unavailable,
            unavailable_reason: Some("runner did not publish detailed live statistics".into()),
        })
    }

    #[must_use]
    pub fn binding(&self) -> &LaunchBinding {
        &self.binding
    }
    #[must_use]
    pub fn active_run_attempt(&self) -> Option<(&RunId, &AttemptId)> {
        self.run_id.as_ref().zip(self.attempt_id.as_ref())
    }
    #[must_use]
    pub const fn state(&self) -> &RunState {
        &self.state
    }
    #[must_use]
    pub const fn connection(&self) -> RunConnection {
        self.connection
    }
    #[must_use]
    pub fn statistics(&self) -> Option<&LiveStatistics> {
        self.statistics.as_ref()
    }
}

fn state_for_public(state: PublicRunState) -> RunState {
    match state {
        PublicRunState::Planned | PublicRunState::Prepared => RunState::Launching,
        PublicRunState::Running | PublicRunState::Collecting => RunState::Running,
        PublicRunState::Completed => RunState::Completed,
        PublicRunState::Failed => RunState::Failed,
        PublicRunState::Cancelled => RunState::Cancelled,
        PublicRunState::NeedsReconciliation => RunState::Unavailable,
    }
}

fn is_terminal(state: &RunState) -> bool {
    matches!(
        state,
        RunState::Completed | RunState::Failed | RunState::Cancelled | RunState::Unavailable
    )
}

fn has_duplicate(values: &[String]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn validate_id(value: &str, field: &'static str) -> Result<(), LaunchValidationError> {
    if value.is_empty() {
        return Err(LaunchValidationError::EmptyId(field));
    }
    if value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(LaunchValidationError::InvalidId(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_codec::{ConfigurationSnapshot, ProviderAuthMethod, ProviderCatalog};

    fn binding() -> LaunchBinding {
        LaunchBinding {
            materialization_digest_sha256: "a".repeat(64),
            provider_catalog_generation: Revision(4),
            provider_catalog_digest: "b".repeat(64),
            benchmark_catalog_generation: Revision(7),
            benchmark_catalog_digest: "c".repeat(64),
            agent_ids: vec!["agent".into()],
            provider_id: "provider".into(),
            model_id: "model".into(),
            pool_id: "pool".into(),
            group_ids: vec!["latency".into()],
            benchmark_ids: vec!["latency".into()],
            measure_ids: vec!["latency.first_response".into()],
            development_only: true,
        }
    }

    fn event(revision: u64, state: PublicRunState) -> LiveRunEvent {
        LiveRunEvent {
            revision: Revision(revision),
            run_id: RunId("run".into()),
            attempt_id: AttemptId("attempt".into()),
            generation: Revision(7),
            state,
            completed_measures: 1,
            total_measures: 2,
            failed_measures: 0,
            throughput_per_second: Some(3),
            latency_millis: Some(10),
            provenance: StatisticsProvenance::DevelopmentMock,
            unavailable_reason: None,
        }
    }

    fn reviewed_bundle() -> MaterializedBundle {
        let mut document = crate::configuration_materialization::MaterializedConfiguration {
            schema_version: 1,
            asb_protocol: "asb-control".into(),
            asb_version: "development".into(),
            provider: crate::configuration_materialization::MaterializedProvider {
                agent_ids: vec!["agent".into()],
                provider_id: "provider".into(),
                model_id: "model".into(),
                auth_method: ProviderAuthMethod::None,
                credential_reference_sha256: None,
                catalog_generation: Revision(4),
                catalog_digest: "b".repeat(64),
            },
            benchmark: crate::configuration_materialization::MaterializedBenchmark {
                generation: Revision(7),
                catalog_digest: "c".repeat(64),
                pool_id: "pool".into(),
                group_ids: vec!["latency".into()],
                benchmark_ids: vec!["latency".into()],
                measure_ids: vec!["latency.first_response".into()],
            },
            configuration_digest_sha256: String::new(),
        };
        let unsigned = serde_json::to_string(&document).unwrap();
        let digest =
            crate::sha256::digest_hex(format!("asb-tui.materialized.v1\0{unsigned}").as_bytes());
        document.configuration_digest_sha256 = digest.clone();
        MaterializedBundle {
            canonical_json: serde_json::to_string(&document).unwrap(),
            document,
            digest_sha256: digest,
        }
    }

    fn snapshot() -> LiveSnapshot {
        let mut measurement_catalog = crate::control_codec::MeasurementCatalog {
            schema_version: 1,
            catalog_sha256: String::new(),
            groups: vec![crate::control_codec::MeasurementGroup {
                id: crate::control_codec::MeasurementGroupId::Latency,
                label: "Latency".into(),
                description: "timing".into(),
            }],
            measurements: vec![crate::control_codec::MeasurementDefinition {
                id: "latency.first_response".into(),
                name: "First response".into(),
                description: "time until first response".into(),
                group: crate::control_codec::MeasurementGroupId::Latency,
                quantity: crate::control_codec::MeasurementQuantity::Time,
                unit: "ns".into(),
                aggregation: crate::control_codec::MeasurementAggregation::Gauge,
                scope: crate::control_codec::MeasurementScope::Attempt,
                provenance: crate::control_codec::MeasurementProvenance {
                    source: crate::control_codec::MeasurementSource::AsbRunnerJournal,
                    qualification: crate::control_codec::MeasurementQualification::Implemented,
                },
                source_identity: crate::control_codec::MeasurementSourceIdentity::ProcfsProcessStat,
                resolution_ns: 1,
                overhead: crate::control_codec::MeasurementOverhead {
                    class: crate::control_codec::MeasurementOverheadClass::Low,
                    minimum_interval_ns: 1,
                    requires_privilege: false,
                },
                live: crate::control_codec::MeasurementModeSupport::Supported,
                replay: crate::control_codec::MeasurementModeSupport::Supported,
                platforms: vec![crate::control_codec::MeasurementPlatform {
                    operating_system: crate::control_codec::MeasurementOperatingSystem::Linux,
                    architectures: vec![crate::control_codec::MeasurementArchitecture::X86_64],
                    required_features: vec![
                        crate::control_codec::MeasurementPlatformFeature::Procfs,
                    ],
                }],
                evidence_limits: vec![
                    crate::control_codec::MeasurementEvidenceLimit::CollectorOverheadRecorded,
                ],
            }],
        };
        measurement_catalog.catalog_sha256 = measurement_catalog.computed_digest().unwrap();
        LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner".into()),
            latest_revision: Some(Revision(7)),
            capabilities: None,
            measurement_catalog: Some(measurement_catalog),
            benchmark_catalog: Some(crate::live_projection::LiveBenchmarkCatalog {
                generation: Revision(7),
                catalog_sha256: "c".repeat(64),
                pools: vec![crate::live_projection::LiveBenchmarkPool {
                    id: "pool".into(),
                    groups: vec![crate::live_projection::LiveBenchmarkGroup {
                        id: "latency".into(),
                        benchmarks: vec![crate::live_projection::LiveBenchmarkEntry {
                            id: "latency".into(),
                            measure_ids: vec!["latency.first_response".into()],
                        }],
                    }],
                }],
            }),
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: Some(ProviderCatalog {
                runner_instance_id: "runner".into(),
                generation: Revision(4),
                catalog_sha256: "b".repeat(64),
                providers: vec![],
                refreshed: true,
            }),
            configuration: Some(ConfigurationSnapshot {
                runner_instance_id: "runner".into(),
                generation: Revision(3),
                configured: true,
                agent_ids: vec!["agent".into()],
                provider_id: Some("provider".into()),
                model_id: Some("model".into()),
                auth_method: Some(ProviderAuthMethod::None),
                credential_reference_sha256: None,
            }),
            auth_status: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            runs: vec![],
        }
    }

    #[test]
    fn launch_request_fences_generation_and_configuration_before_io() {
        let mut bundle = reviewed_bundle();
        let live = snapshot();
        bundle.document.benchmark.catalog_digest = live
            .benchmark_catalog
            .as_ref()
            .unwrap()
            .catalog_sha256
            .clone();
        bundle.document.configuration_digest_sha256.clear();
        let unsigned = serde_json::to_string(&bundle.document).unwrap();
        bundle.digest_sha256 =
            crate::sha256::digest_hex(format!("asb-tui.materialized.v1\0{unsigned}").as_bytes());
        bundle.document.configuration_digest_sha256 = bundle.digest_sha256.clone();
        bundle.canonical_json = serde_json::to_string(&bundle.document).unwrap();
        let request = LaunchRequest::new(&bundle, &live, "launch-key").unwrap();
        assert_eq!(
            request.binding.materialization_digest_sha256,
            bundle.digest_sha256
        );
        let mut stale = snapshot();
        stale.provider_catalog.as_mut().unwrap().generation = Revision(5);
        assert_eq!(
            LaunchRequest::new(&bundle, &stale, "launch-key"),
            Err(LaunchValidationError::ProviderCatalogGenerationMismatch)
        );
        let mut wrong = snapshot();
        wrong.configuration.as_mut().unwrap().model_id = Some("other".into());
        assert_eq!(
            LaunchRequest::new(&bundle, &wrong, "launch-key"),
            Err(LaunchValidationError::ConfigurationMismatch)
        );
    }

    #[test]
    fn launch_request_rejects_tampered_canonical_bundle_before_io() {
        let mut bundle = reviewed_bundle();
        bundle.canonical_json = "{}".into();
        assert_eq!(
            LaunchRequest::new(&bundle, &snapshot(), "launch-key"),
            Err(LaunchValidationError::InvalidBundle)
        );
    }

    #[test]
    fn state_preserves_partial_statistics_and_rejects_stale_events() {
        let mut state = LaunchState::new(binding());
        state.begin_launch().unwrap();
        state
            .apply_event(event(1, PublicRunState::Running))
            .unwrap();
        assert_eq!(state.statistics().unwrap().bounded_progress_percent(), 50);
        assert_eq!(
            state.apply_event(event(1, PublicRunState::Completed)),
            Err(RunEventError::StaleRevision)
        );
        state.request_cancel().unwrap();
        state
            .apply_event(event(2, PublicRunState::Cancelled))
            .unwrap();
        assert_eq!(state.state(), &RunState::Cancelled);
    }

    #[test]
    fn reconnect_does_not_reset_run_or_allow_wrong_generation() {
        let mut state = LaunchState::new(binding());
        state.begin_launch().unwrap();
        state
            .apply_event(event(2, PublicRunState::Running))
            .unwrap();
        state.reconnect_started();
        state.reconnect_complete(Some(Revision(8)));
        let mut wrong = event(9, PublicRunState::Completed);
        wrong.generation = Revision(8);
        assert_eq!(
            state.apply_event(wrong),
            Err(RunEventError::StaleGeneration)
        );
        assert_eq!(state.statistics().unwrap().state, PublicRunState::Running);
    }

    #[test]
    fn terminal_run_cannot_regress_to_nonterminal_state() {
        let mut state = LaunchState::new(binding());
        state.begin_launch().unwrap();
        state
            .apply_event(event(1, PublicRunState::Completed))
            .unwrap();
        assert_eq!(
            state.apply_event(event(2, PublicRunState::Running)),
            Err(RunEventError::TerminalRegression)
        );
        assert_eq!(state.state(), &RunState::Completed);
    }

    #[test]
    fn terminal_outcome_is_immutable_without_reconciliation() {
        let mut state = LaunchState::new(binding());
        state.begin_launch().unwrap();
        state
            .apply_event(event(1, PublicRunState::Completed))
            .unwrap();
        assert_eq!(
            state.apply_event(event(2, PublicRunState::Failed)),
            Err(RunEventError::TerminalRegression)
        );
        assert_eq!(state.state(), &RunState::Completed);
    }

    #[test]
    fn unavailable_statistics_require_an_explanation() {
        let mut state = LaunchState::new(binding());
        state.begin_launch().unwrap();
        let mut event = event(1, PublicRunState::NeedsReconciliation);
        event.provenance = StatisticsProvenance::Unavailable;
        assert_eq!(
            state.apply_event(event),
            Err(RunEventError::InvalidUnavailableState)
        );
        assert!(state.statistics().is_none());
    }

    #[test]
    fn launch_validation_reports_each_live_catalog_boundary() {
        let bundle = reviewed_bundle();
        let mut missing = snapshot();
        missing.benchmark_catalog = None;
        assert_eq!(
            LaunchRequest::new(&bundle, &missing, "launch-key"),
            Err(LaunchValidationError::BenchmarkCatalogUnavailable)
        );
        let mut stale = snapshot();
        stale.benchmark_catalog.as_mut().unwrap().generation = Revision(8);
        assert_eq!(
            LaunchRequest::new(&bundle, &stale, "launch-key"),
            Err(LaunchValidationError::BenchmarkCatalogGenerationMismatch)
        );
        let mut wrong_pool = snapshot();
        wrong_pool.benchmark_catalog.as_mut().unwrap().pools[0].id = "other".into();
        assert_eq!(
            LaunchRequest::new(&bundle, &wrong_pool, "launch-key"),
            Err(LaunchValidationError::InvalidBenchmarkSelection)
        );
    }

    #[test]
    fn launch_request_rejects_empty_or_invalid_idempotency_keys() {
        let bundle = reviewed_bundle();
        assert_eq!(
            LaunchRequest::new(&bundle, &snapshot(), ""),
            Err(LaunchValidationError::EmptyId("idempotency key"))
        );
        assert!(matches!(
            LaunchRequest::new(&bundle, &snapshot(), "bad key"),
            Err(LaunchValidationError::InvalidId("idempotency key"))
        ));
    }

    #[test]
    fn launch_validation_fails_closed_for_every_selection_and_connection_fence() {
        let bundle = reviewed_bundle();
        let mut cases = Vec::new();
        let mut invalid_generation = bundle.clone();
        invalid_generation.document.benchmark.generation = Revision(0);
        cases.push((invalid_generation, snapshot(), LaunchValidationError::InvalidBundle));

        let mut provider_digest = snapshot();
        provider_digest.provider_catalog.as_mut().unwrap().catalog_sha256 = "d".repeat(64);
        cases.push((bundle.clone(), provider_digest, LaunchValidationError::ProviderCatalogDigestMismatch));

        let mut benchmark_digest = snapshot();
        benchmark_digest.benchmark_catalog.as_mut().unwrap().catalog_sha256 = "d".repeat(64);
        cases.push((bundle.clone(), benchmark_digest, LaunchValidationError::InvalidBenchmarkSelection));

        let mut disconnected = snapshot();
        disconnected.connection = Connection::Disconnected;
        cases.push((bundle.clone(), disconnected, LaunchValidationError::Disconnected));

        let mut missing_config = snapshot();
        missing_config.configuration = None;
        cases.push((bundle.clone(), missing_config, LaunchValidationError::ConfigurationUnavailable));

        let mut unconfigured = snapshot();
        unconfigured.configuration.as_mut().unwrap().configured = false;
        cases.push((bundle.clone(), unconfigured, LaunchValidationError::ConfigurationMismatch));

        for (candidate, live, expected) in cases {
            assert_eq!(validate_binding(&candidate, &live), Err(expected));
        }

        let mut duplicate = bundle.clone();
        duplicate.document.benchmark.measure_ids.push("latency.first_response".into());
        assert_eq!(validate_binding(&duplicate, &snapshot()), Err(LaunchValidationError::InvalidBundle));
        let mut empty = bundle.clone();
        empty.document.benchmark.group_ids.clear();
        assert_eq!(validate_binding(&empty, &snapshot()), Err(LaunchValidationError::InvalidBundle));
        let mut wrong_group = snapshot();
        wrong_group.benchmark_catalog.as_mut().unwrap().pools[0].groups[0].id = "other".into();
        assert_eq!(validate_binding(&bundle, &wrong_group), Err(LaunchValidationError::InvalidBenchmarkSelection));
        let mut wrong_benchmark = snapshot();
        wrong_benchmark.benchmark_catalog.as_mut().unwrap().pools[0].groups[0].benchmarks[0].id = "other".into();
        assert_eq!(validate_binding(&bundle, &wrong_benchmark), Err(LaunchValidationError::InvalidBenchmarkSelection));
        let mut wrong_measure = snapshot();
        wrong_measure.benchmark_catalog.as_mut().unwrap().pools[0].groups[0].benchmarks[0].measure_ids = vec!["other".into()];
        assert_eq!(validate_binding(&bundle, &wrong_measure), Err(LaunchValidationError::InvalidBenchmarkSelection));
    }

    #[test]
    fn launch_state_covers_reconnect_attempt_identity_and_summary_projection() {
        let mut state = LaunchState::new(binding());
        assert_eq!(state.connection(), RunConnection::Connected);
        assert_eq!(state.active_run_attempt(), None);
        assert_eq!(state.request_cancel(), Err(RunEventError::StaleRevision));
        state.reconnect_started();
        assert_eq!(state.begin_launch(), Err(RunEventError::StaleRevision));
        state.reconnect_complete(Some(Revision(3)));
        state.begin_launch().unwrap();

        let mut prepared = event(4, PublicRunState::Prepared);
        state.apply_event(prepared.clone()).unwrap();
        prepared.revision = Revision(5);
        prepared.run_id = RunId("other".into());
        assert_eq!(state.apply_event(prepared), Err(RunEventError::WrongRun));
        let mut wrong_attempt = event(6, PublicRunState::Running);
        wrong_attempt.attempt_id = AttemptId("other".into());
        assert_eq!(state.apply_event(wrong_attempt), Err(RunEventError::WrongAttempt));

        let mut out_of_bounds = event(7, PublicRunState::Running);
        out_of_bounds.completed_measures = 3;
        assert_eq!(state.apply_event(out_of_bounds), Err(RunEventError::ProgressOutOfBounds));
        let mut failed_bounds = event(8, PublicRunState::Running);
        failed_bounds.failed_measures = 3;
        assert_eq!(state.apply_event(failed_bounds), Err(RunEventError::ProgressOutOfBounds));

        state.disconnected();
        assert_eq!(state.connection(), RunConnection::Disconnected);
        assert_eq!(state.apply_event(event(9, PublicRunState::Running)), Err(RunEventError::StaleRevision));
        state.reconnect_complete(None);
        assert_eq!(state.connection(), RunConnection::Connected);
        state.request_cancel().unwrap();
        let summary = crate::control_codec::RunSummary {
            revision: Revision(10),
            run_id: RunId("run".into()),
            attempt_id: AttemptId("attempt".into()),
            state: PublicRunState::Failed,
            created_revision: Revision(1),
            plan_sha256: "a".repeat(64),
        };
        state.observe_summary(&summary).unwrap();
        assert_eq!(state.state(), &RunState::Failed);
        assert_eq!(state.statistics().unwrap().bounded_progress_percent(), 100);

        let mut empty = LaunchState::new({
            let mut b = binding();
            b.measure_ids.clear();
            b
        });
        empty.begin_launch().unwrap();
        let summary = crate::control_codec::RunSummary { state: PublicRunState::Running, revision: Revision(1), ..summary };
        empty.observe_summary(&summary).unwrap();
        assert_eq!(empty.statistics().unwrap().bounded_progress_percent(), 0);
    }
}
