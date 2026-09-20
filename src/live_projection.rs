// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Authenticated-control response projection for the interactive frontend.
//!
//! This is deliberately a transport seam, rather than a transport
//! implementation.  A broker adapter supplies a typed request/response pair;
//! this module validates the pair at the JSON-RPC boundary and publishes only
//! bounded, renderer-safe state.  It cannot launch, retry, or cancel work.

use crate::control_codec::{
    AuthStatusResponse, ConfigurationSnapshot, ControlCall, ControlLimits, ControlRequest,
    ControlResponse, ControlResult, ControlSuccess, MeasurementCatalog, Negotiated,
    ProviderCatalog, RecordingCampaignPlan, Revision, RunSummary,
};
use std::{collections::BTreeMap, fmt};

pub const MAX_PROJECTED_RUNS: usize = 256;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Connection {
    #[default]
    Disconnected,
    Negotiated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSnapshot {
    pub connection: Connection,
    pub runner_instance_id: Option<String>,
    pub latest_revision: Option<Revision>,
    pub capabilities: Option<crate::control_codec::Capabilities>,
    pub measurement_catalog: Option<MeasurementCatalog>,
    pub agent_catalog: Option<crate::agent_catalog::AgentCatalog>,
    pub agent_lifecycle: Option<crate::asb_lifecycle::AgentLifecycleResponse>,
    pub provider_catalog: Option<ProviderCatalog>,
    pub configuration: Option<ConfigurationSnapshot>,
    /// Last public provider enrollment status. This contains digests only;
    /// absence means unavailable, not unauthorized or connected.
    pub auth_status: Option<AuthStatusResponse>,
    pub recording_campaign: Option<RecordingCampaignPlan>,
    pub runs: Vec<RunSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    InvalidResponse,
    UnexpectedResult,
    StaleRun,
    TooManyRuns,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidResponse => "invalid control response",
            Self::UnexpectedResult => "response does not match projection request",
            Self::StaleRun => "run revision moved backwards",
            Self::TooManyRuns => "projected run limit exceeded",
        })
    }
}
impl std::error::Error for ProjectionError {}

/// State owned by the frontend projection.  The runner remains authoritative.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ControlProjection {
    connection: Connection,
    negotiated: Option<Negotiated>,
    capabilities: Option<crate::control_codec::Capabilities>,
    measurement_catalog: Option<MeasurementCatalog>,
    agent_catalog: Option<crate::agent_catalog::AgentCatalog>,
    agent_lifecycle: Option<crate::asb_lifecycle::AgentLifecycleResponse>,
    provider_catalog: Option<ProviderCatalog>,
    configuration: Option<ConfigurationSnapshot>,
    auth_status: Option<AuthStatusResponse>,
    recording_campaign: Option<RecordingCampaignPlan>,
    runs: BTreeMap<String, RunSummary>,
}

impl ControlProjection {
    /// Seed the projection from the already authenticated negotiation result.
    /// This avoids fabricating a wire response while keeping all subsequent
    /// operations behind the same negotiated connection gate.
    pub fn accept_negotiated(&mut self, negotiated: Negotiated) -> Result<(), ProjectionError> {
        self.connection = Connection::Negotiated;
        self.negotiated = Some(negotiated);
        Ok(())
    }

    pub fn apply(
        &mut self,
        request: &ControlRequest,
        response: &ControlResponse,
        limits: ControlLimits,
    ) -> Result<(), ProjectionError> {
        response
            .validate_for(request, limits)
            .map_err(|_| ProjectionError::InvalidResponse)?;
        let ControlResponse::Success(success) = response else {
            return Ok(());
        };
        if !matches!(&request.call, ControlCall::Negotiate(_))
            && !matches!(self.connection, Connection::Negotiated)
        {
            return Err(ProjectionError::UnexpectedResult);
        }
        match (&request.call, &success.result) {
            (ControlCall::Negotiate(_), ControlSuccess::Negotiated(negotiated)) => {
                self.connection = Connection::Negotiated;
                self.negotiated = Some(negotiated.clone());
            }
            (_, ControlSuccess::Operation(bound)) => {
                self.apply_result(&request.call, &bound.result)?
            }
            _ => return Err(ProjectionError::UnexpectedResult),
        }
        Ok(())
    }

    fn apply_result(
        &mut self,
        call: &ControlCall,
        result: &ControlResult,
    ) -> Result<(), ProjectionError> {
        match (call, result) {
            (ControlCall::Capabilities, ControlResult::Capabilities(value)) => {
                self.capabilities = Some(value.clone());
            }
            (ControlCall::MeasurementCatalog, ControlResult::MeasurementCatalog(value)) => {
                if self.negotiated.as_ref().is_none_or(|session| {
                    session.version < crate::control_codec::CONTROL_MEASUREMENT_CATALOG_V1
                }) {
                    return Err(ProjectionError::UnexpectedResult);
                }
                self.measurement_catalog = Some(value.catalog.clone());
            }
            (ControlCall::AgentCatalog(_), ControlResult::AgentCatalog(value)) => {
                if self.negotiated.as_ref().is_none_or(|session| {
                    session.version < crate::control_codec::CONTROL_AGENT_CATALOG_V1
                }) {
                    return Err(ProjectionError::UnexpectedResult);
                }
                self.agent_catalog = Some(value.clone());
            }
            (
                ControlCall::AgentInstall(_)
                | ControlCall::AgentStatus(_)
                | ControlCall::AgentCancel(_)
                | ControlCall::AgentRetry(_)
                | ControlCall::AgentRemove(_),
                ControlResult::AgentLifecycle(value),
            ) => {
                if self.negotiated.as_ref().is_none_or(|session| {
                    session.version < crate::control_codec::CONTROL_AGENT_LIFECYCLE_V1
                }) {
                    return Err(ProjectionError::UnexpectedResult);
                }
                self.agent_lifecycle = Some(value.clone());
            }
            (ControlCall::ProviderCatalog(_), ControlResult::ProviderCatalog(value)) => {
                if self
                    .negotiated
                    .as_ref()
                    .is_none_or(|session| session.version < crate::control_codec::V1_7)
                {
                    return Err(ProjectionError::UnexpectedResult);
                }
                self.provider_catalog = Some(value.clone());
            }
            (ControlCall::ConfigurationStatus(_), ControlResult::Configuration(value))
            | (ControlCall::ConfigurationApply(_), ControlResult::Configuration(value)) => {
                if self
                    .negotiated
                    .as_ref()
                    .is_none_or(|session| session.version < crate::control_codec::V1_7)
                {
                    return Err(ProjectionError::UnexpectedResult);
                }
                self.configuration = Some(value.clone());
            }
            (ControlCall::AuthStatus(_), ControlResult::AuthStatus(value)) => {
                if self
                    .negotiated
                    .as_ref()
                    .is_none_or(|session| session.version < crate::control_codec::CONTROL_AUTH_V1)
                {
                    return Err(ProjectionError::UnexpectedResult);
                }
                self.auth_status = Some(value.clone());
            }
            (ControlCall::RecordingCampaignPlan(_), ControlResult::RecordingCampaign(value)) => {
                if self
                    .negotiated
                    .as_ref()
                    .is_none_or(|session| session.version < crate::control_codec::V1_7)
                {
                    return Err(ProjectionError::UnexpectedResult);
                }
                self.recording_campaign = Some(value.clone());
            }
            (ControlCall::History(_), ControlResult::History(page)) => {
                if self.runs.len() + page.items.len() > MAX_PROJECTED_RUNS {
                    return Err(ProjectionError::TooManyRuns);
                }
                for run in &page.items {
                    self.insert_run(run.clone())?;
                }
            }
            (ControlCall::Status { .. }, ControlResult::Status(run))
            | (ControlCall::Launch(_), ControlResult::Launch(run)) => {
                self.insert_run(run.clone())?;
            }
            _ => {}
        }
        Ok(())
    }

    fn insert_run(&mut self, run: RunSummary) -> Result<(), ProjectionError> {
        let key = run.run_id.0.clone();
        if let Some(previous) = self.runs.get(&key)
            && run.revision < previous.revision
        {
            return Err(ProjectionError::StaleRun);
        }
        if !self.runs.contains_key(&key) && self.runs.len() >= MAX_PROJECTED_RUNS {
            return Err(ProjectionError::TooManyRuns);
        }
        self.runs.insert(key, run);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.connection = Connection::Disconnected;
    }

    #[must_use]
    pub fn snapshot(&self) -> LiveSnapshot {
        let mut runs: Vec<_> = self.runs.values().cloned().collect();
        runs.sort_by(|left, right| {
            right
                .revision
                .cmp(&left.revision)
                .then_with(|| left.run_id.0.cmp(&right.run_id.0))
        });
        LiveSnapshot {
            connection: self.connection,
            runner_instance_id: self
                .negotiated
                .as_ref()
                .map(|value| value.runner_instance_id.clone()),
            latest_revision: self.negotiated.as_ref().map(|value| value.latest_revision),
            capabilities: self.capabilities.clone(),
            measurement_catalog: self.measurement_catalog.clone(),
            agent_catalog: self.agent_catalog.clone(),
            agent_lifecycle: self.agent_lifecycle.clone(),
            provider_catalog: self.provider_catalog.clone(),
            configuration: self.configuration.clone(),
            auth_status: self.auth_status.clone(),
            recording_campaign: self.recording_campaign.clone(),
            runs,
        }
    }

    #[must_use]
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_codec::{
        BoundResult, ControlVersion, Negotiated, Page, PublicRunState, RequestId, SuccessResponse,
    };

    fn request(call: ControlCall, id: u64) -> ControlRequest {
        ControlRequest {
            jsonrpc: "2.0".into(),
            id: RequestId(id),
            timeout_ms: 1_000,
            call,
        }
    }
    fn run(id: &str, revision: u64) -> RunSummary {
        RunSummary {
            run_id: crate::control_codec::RunId(id.into()),
            attempt_id: crate::control_codec::AttemptId(format!("{id}-a")),
            state: PublicRunState::Completed,
            created_revision: Revision(revision),
            revision: Revision(revision),
            plan_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        }
    }
    fn response(id: u64, result: ControlResult) -> ControlResponse {
        ControlResponse::Success(SuccessResponse {
            jsonrpc: "2.0".into(),
            id: RequestId(id),
            result: ControlSuccess::Operation(BoundResult {
                request_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
                result,
            }),
        })
    }

    fn negotiate_response_at(id: u64, version: ControlVersion) -> ControlResponse {
        ControlResponse::Success(SuccessResponse {
            jsonrpc: "2.0".into(),
            id: RequestId(id),
            result: ControlSuccess::Negotiated(Negotiated {
                version,
                limits: ControlLimits::default(),
                runner_instance_id: "runner".into(),
                oldest_revision: Revision(1),
                latest_revision: Revision(5),
            }),
        })
    }

    fn negotiate_response(id: u64) -> ControlResponse {
        negotiate_response_at(id, ControlVersion { major: 1, minor: 3 })
    }

    fn connected_projection() -> ControlProjection {
        let call = ControlCall::Negotiate(crate::control_codec::NegotiateParams {
            versions: [ControlVersion { major: 1, minor: 3 }]
                .into_iter()
                .collect(),
            limits: ControlLimits::default(),
        });
        let mut projection = ControlProjection::default();
        projection
            .apply(
                &request(call, 99),
                &negotiate_response(99),
                ControlLimits::default(),
            )
            .unwrap();
        projection
    }

    fn connected_projection_at(version: ControlVersion) -> ControlProjection {
        let call = ControlCall::Negotiate(crate::control_codec::NegotiateParams {
            versions: [version].into_iter().collect(),
            limits: ControlLimits::default(),
        });
        let mut projection = ControlProjection::default();
        projection
            .apply(
                &request(call, 99),
                &negotiate_response_at(99, version),
                ControlLimits::default(),
            )
            .unwrap();
        projection
    }

    #[test]
    fn v17_setup_catalog_and_campaign_are_projected_only_after_negotiation() {
        use crate::control_codec::*;
        let mut projection = ControlProjection::default();
        let negotiation = ControlResponse::Success(SuccessResponse {
            jsonrpc: "2.0".into(),
            id: RequestId(1),
            result: ControlSuccess::Negotiated(Negotiated {
                version: V1_7,
                limits: ControlLimits::default(),
                runner_instance_id: "runner-1".into(),
                oldest_revision: Revision(1),
                latest_revision: Revision(2),
            }),
        });
        let negotiate = request(
            ControlCall::Negotiate(NegotiateParams {
                versions: [V1_7].into_iter().collect(),
                limits: ControlLimits::default(),
            }),
            1,
        );
        projection
            .apply(&negotiate, &negotiation, ControlLimits::default())
            .unwrap();
        let catalog_call = ControlCall::ProviderCatalog(ProviderCatalogRequest {
            action: ProviderCatalogAction::Status,
            runner_instance_id: "runner-1".into(),
            known_generation: None,
        });
        let catalog = ProviderCatalog {
            runner_instance_id: "runner-1".into(),
            generation: Revision(1),
            catalog_sha256: "a".repeat(64),
            providers: vec![ProviderCatalogEntry {
                provider_id: "openai".into(),
                display_name: "OpenAI".into(),
                auth_methods: vec![ProviderAuthMethod::CredentialReference],
                models: vec![ProviderModel {
                    model_id: "gpt-5.2-2025-12-11".into(),
                    revision: "pinned".into(),
                    availability: ProviderAvailability::Available,
                }],
                availability: ProviderAvailability::Available,
            }],
            refreshed: false,
        };
        projection
            .apply(
                &request(catalog_call, 2),
                &response(2, ControlResult::ProviderCatalog(catalog)),
                ControlLimits::default(),
            )
            .unwrap();
        assert_eq!(
            projection
                .snapshot()
                .provider_catalog
                .unwrap()
                .providers
                .len(),
            1
        );
    }

    #[test]
    fn auth_status_is_projected_only_after_auth_version_and_contains_digests() {
        use crate::control_codec::*;
        let call = ControlCall::AuthStatus(AuthStatusParams {
            provider: "openai".into(),
        });
        let response = response(
            2,
            ControlResult::AuthStatus(AuthStatusResponse {
                provider: "openai".into(),
                endpoint_identity_sha256: "a".repeat(64),
                credential_locator_sha256: "b".repeat(64),
                generation: Revision(1),
                status: "active".into(),
            }),
        );
        let mut old = connected_projection_at(V1_5);
        assert_eq!(
            old.apply(
                &request(call.clone(), 2),
                &response,
                ControlLimits::default()
            ),
            Err(ProjectionError::UnexpectedResult)
        );
        let mut current = connected_projection_at(CONTROL_AUTH_V1);
        current
            .apply(&request(call, 2), &response, ControlLimits::default())
            .unwrap();
        assert_eq!(current.snapshot().auth_status.unwrap().status, "active");
    }

    fn catalog() -> crate::control_codec::MeasurementCatalogPublication {
        use crate::control_codec::*;
        let mut publication = MeasurementCatalogPublication {
            version: CONTROL_MEASUREMENT_CATALOG_V1,
            freshness: MeasurementCatalogFreshness::ContentAddressed,
            source: MeasurementCatalogPublicationSource::BuiltInCollectors,
            catalog: MeasurementCatalog {
                schema_version: 1,
                catalog_sha256: String::new(),
                groups: vec![MeasurementGroup {
                    id: MeasurementGroupId::Latency,
                    label: "Latency".into(),
                    description: "timing".into(),
                }],
                measurements: vec![MeasurementDefinition {
                    id: "latency.first_response".into(),
                    name: "First response".into(),
                    description: "time until first response".into(),
                    group: MeasurementGroupId::Latency,
                    quantity: MeasurementQuantity::Time,
                    unit: "ns".into(),
                    aggregation: MeasurementAggregation::Gauge,
                    scope: MeasurementScope::Attempt,
                    provenance: MeasurementProvenance {
                        source: MeasurementSource::AsbRunnerJournal,
                        qualification: MeasurementQualification::Implemented,
                    },
                    source_identity: MeasurementSourceIdentity::ProcfsProcessStat,
                    resolution_ns: 1,
                    overhead: MeasurementOverhead {
                        class: MeasurementOverheadClass::Low,
                        minimum_interval_ns: 1,
                        requires_privilege: false,
                    },
                    live: MeasurementModeSupport::Supported,
                    replay: MeasurementModeSupport::Supported,
                    platforms: vec![MeasurementPlatform {
                        operating_system: MeasurementOperatingSystem::Linux,
                        architectures: vec![MeasurementArchitecture::X86_64],
                        required_features: vec![MeasurementPlatformFeature::Procfs],
                    }],
                    evidence_limits: vec![MeasurementEvidenceLimit::CollectorOverheadRecorded],
                }],
            },
        };
        publication.catalog.catalog_sha256 = publication.catalog.computed_digest().unwrap();
        publication
    }

    #[test]
    fn catalog_response_is_projected_only_after_v12_negotiation() {
        let call = ControlCall::MeasurementCatalog;
        let req = request(call, 1);
        let response = response(1, ControlResult::MeasurementCatalog(catalog()));
        let mut projection = connected_projection();
        projection
            .apply(&req, &response, ControlLimits::default())
            .unwrap();
        assert_eq!(
            projection
                .snapshot()
                .measurement_catalog
                .unwrap()
                .measurements
                .len(),
            1
        );
    }

    #[test]
    fn history_response_becomes_newest_first_snapshot() {
        let call = ControlCall::History(crate::control_codec::PageParams {
            after: None,
            limit: 2,
        });
        let req = request(call.clone(), 1);
        let page = Page {
            items: vec![run("old", 2), run("new", 5)],
            next: None,
            has_more: false,
        };
        let mut projection = connected_projection();
        projection
            .apply(
                &req,
                &response(1, ControlResult::History(page)),
                ControlLimits::default(),
            )
            .unwrap();
        let snapshot = projection.snapshot();
        assert_eq!(
            snapshot
                .runs
                .iter()
                .map(|v| v.run_id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "old"]
        );
    }

    #[test]
    fn stale_status_cannot_replace_authoritative_run() {
        let call = ControlCall::Status {
            run_id: crate::control_codec::RunId("run".into()),
        };
        let mut projection = connected_projection();
        let req = request(call.clone(), 1);
        projection
            .apply(
                &req,
                &response(1, ControlResult::Status(run("run", 3))),
                ControlLimits::default(),
            )
            .unwrap();
        let req = request(call, 2);
        assert_eq!(
            projection.apply(
                &req,
                &response(2, ControlResult::Status(run("run", 2))),
                ControlLimits::default()
            ),
            Err(ProjectionError::StaleRun)
        );
    }

    #[test]
    fn agent_catalog_and_lifecycle_are_projected_only_at_their_protocol_versions() {
        let catalog: crate::agent_catalog::AgentCatalog =
            crate::agent_catalog::parse_agent_catalog_response(include_str!(
                "../tests/fixtures/asb-v1.4-agent-catalog-response.json"
            ))
            .unwrap();
        let catalog_call = ControlCall::AgentCatalog(crate::agent_catalog::AgentCatalogRequest {
            action: crate::agent_catalog::AgentCatalogAction::Status,
            runner_instance_id: "runner-1".into(),
            known_generation: None,
        });
        let mut old = connected_projection();
        assert_eq!(
            old.apply(
                &request(catalog_call.clone(), 1),
                &response(1, ControlResult::AgentCatalog(catalog.clone())),
                ControlLimits::default(),
            ),
            Err(ProjectionError::UnexpectedResult)
        );

        let mut current = connected_projection_at(crate::control_codec::V1_5);
        current
            .apply(
                &request(catalog_call, 2),
                &response(2, ControlResult::AgentCatalog(catalog)),
                ControlLimits::default(),
            )
            .unwrap();
        let lifecycle: crate::asb_lifecycle::AgentLifecycleResponse =
            crate::asb_lifecycle::parse_lifecycle_response(include_str!(
                "../tests/fixtures/asb-v1.5-agent-install-response.json"
            ))
            .unwrap();
        let lifecycle_call = ControlCall::AgentStatus(crate::asb_lifecycle::AgentStatusRequest {
            binding: lifecycle.binding.clone(),
            operation_id: None,
        });
        current
            .apply(
                &request(lifecycle_call, 3),
                &response(3, ControlResult::AgentLifecycle(lifecycle.clone())),
                ControlLimits::default(),
            )
            .unwrap();
        assert_eq!(current.snapshot().agent_lifecycle, Some(lifecycle));
        assert!(current.snapshot().agent_catalog.is_some());
    }

    #[test]
    fn mismatched_response_id_is_rejected_before_projection() {
        let call = ControlCall::History(crate::control_codec::PageParams {
            after: None,
            limit: 1,
        });
        let req = request(call, 8);
        let page = Page {
            items: vec![],
            next: None,
            has_more: false,
        };
        assert_eq!(
            ControlProjection::default().apply(
                &req,
                &response(9, ControlResult::History(page)),
                ControlLimits::default()
            ),
            Err(ProjectionError::InvalidResponse)
        );
    }
}
