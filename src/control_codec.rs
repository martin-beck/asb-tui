// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed, renderer-neutral JSON-RPC codec for the ASB frontend control API.
//!
//! This module intentionally contains no transport or terminal code.  It is a
//! wire boundary: unknown fields, unsupported versions, oversized values and
//! response/request mismatches are rejected before callers can act on them.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, fmt};

pub const JSONRPC_VERSION: &str = "2.0";
pub const V1_0: ControlVersion = ControlVersion { major: 1, minor: 0 };
pub const V1_2: ControlVersion = ControlVersion { major: 1, minor: 2 };
pub const V1_3: ControlVersion = ControlVersion { major: 1, minor: 3 };
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_PUBLIC_STRING_BYTES: usize = 4096;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_PAGE_ITEMS: u16 = 256;
pub const MAX_ANALYSIS_RUNS: usize = 256;
pub const MAX_JSON_NODES: usize = 4096;
pub const MAX_JSON_DEPTH: usize = 16;
pub const MAX_MEASUREMENT_GROUPS: usize = 7;
pub const MAX_MEASUREMENTS: usize = 128;
pub const MAX_MEASUREMENT_CATALOG_WIRE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlVersion {
    pub major: u16,
    pub minor: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlLimits {
    pub max_frame_bytes: u32,
    pub max_timeout_ms: u64,
    pub max_page_items: u16,
    pub max_in_flight: u16,
}

impl Default for ControlLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 256 * 1024,
            max_timeout_ms: 30_000,
            max_page_items: 100,
            max_in_flight: 16,
        }
    }
}
impl ControlLimits {
    pub fn validate(self) -> Result<Self, CodecError> {
        if self.max_frame_bytes == 0 || self.max_frame_bytes as usize > MAX_FRAME_BYTES {
            return Err(CodecError::InvalidLimit("max_frame_bytes"));
        }
        if self.max_timeout_ms == 0 || self.max_timeout_ms > 300_000 {
            return Err(CodecError::InvalidLimit("max_timeout_ms"));
        }
        if self.max_page_items == 0 || self.max_page_items > MAX_PAGE_ITEMS {
            return Err(CodecError::InvalidLimit("max_page_items"));
        }
        if self.max_in_flight == 0 || self.max_in_flight > 64 {
            return Err(CodecError::InvalidLimit("max_in_flight"));
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Revision(pub u64);

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl $name {
            pub fn validate(&self) -> Result<(), CodecError> {
                validate_id(&self.0)
            }
        }
    };
}
id_type!(RunId);
id_type!(AttemptId);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub timeout_ms: u64,
    #[serde(flatten)]
    pub call: ControlCall,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ControlCall {
    Negotiate(NegotiateParams),
    Capabilities,
    MeasurementCatalog,
    ValidateSettings { settings: Value },
    CreatePlan(MutationParams),
    Launch(LaunchParams),
    Status { run_id: RunId },
    Cancel(CancelParams),
    History(PageParams),
    Repeat(RepeatParams),
    Analyze { run_ids: Vec<RunId> },
    Events(PageParams),
    ArtifactMetadata { run_id: RunId, digest: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiateParams {
    pub versions: BTreeSet<ControlVersion>,
    pub limits: ControlLimits,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationParams {
    pub idempotency_key: String,
    pub definition: Value,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchParams {
    pub idempotency_key: String,
    pub plan_id: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancelParams {
    pub run_id: RunId,
    pub attempt_id: AttemptId,
    pub idempotency_key: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatParams {
    pub run_id: RunId,
    pub idempotency_key: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PageParams {
    pub after: Option<Revision>,
    pub limit: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ControlResponse {
    Success(SuccessResponse),
    Failure(FailureResponse),
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    pub result: ControlSuccess,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FailureResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    pub error: RpcError,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ControlSuccess {
    Negotiated(Negotiated),
    Operation(BoundResult),
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Negotiated {
    pub version: ControlVersion,
    pub limits: ControlLimits,
    pub runner_instance_id: String,
    pub oldest_revision: Revision,
    pub latest_revision: Revision,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundResult {
    pub request_sha256: String,
    pub result: ControlResult,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicRunState {
    Planned,
    Prepared,
    Running,
    Collecting,
    Completed,
    Failed,
    Cancelled,
    NeedsReconciliation,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummary {
    pub run_id: RunId,
    pub attempt_id: AttemptId,
    pub state: PublicRunState,
    pub created_revision: Revision,
    pub revision: Revision,
    pub plan_sha256: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceAvailability {
    Available,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultIntegrity {
    Verified,
    Incomplete,
    Invalid,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableOutcome {
    Completed,
    Failed,
    Cancelled,
    Unavailable,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsValidation {
    pub valid: bool,
    pub issues: Vec<SettingsIssue>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsIssue {
    InvalidFormat,
    UnsupportedCapability,
    UnverifiedComponent,
    InvalidResourceBound,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub validate_settings: bool,
    pub run_control: bool,
    pub repeat: bool,
    pub analysis: bool,
    pub events: bool,
}

/// The additive control extension that publishes the immutable measurement
/// catalog.  ASB defines this at control version 1.2.
pub const CONTROL_MEASUREMENT_CATALOG_V1: ControlVersion = ControlVersion { major: 1, minor: 2 };

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementCatalogFreshness {
    ContentAddressed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementCatalogPublicationSource {
    BuiltInCollectors,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementGroupId {
    SystemResources,
    SchedulingContention,
    Latency,
    QualityReliability,
    Fairness,
    Cost,
    Provenance,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementGroup {
    pub id: MeasurementGroupId,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementQuantity {
    Time,
    Bytes,
    Count,
    Ratio,
    Score,
    Currency,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementScope {
    Process,
    Cgroup,
    Attempt,
    Run,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSource {
    AsbMetricsProcfs,
    AsbMetricsCgroupV2,
    AsbRunnerJournal,
    IndependentGrader,
    ProviderUsage,
    OptionalCsb,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementSourceIdentity {
    ProcfsProcessStat,
    ProcfsProcessIo,
    CgroupV2CpuStat,
    CgroupV2MemoryCurrent,
    CgroupV2MemoryPeak,
    CgroupV2MemoryStat,
    CgroupV2IoStat,
    CgroupV2CpuPressure,
    CgroupV2MemoryPressure,
    CgroupV2IoPressure,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementQualification {
    Implemented,
    Unqualified,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementUnavailableReason {
    NotQualified,
    PlatformUnsupported,
    PermissionRequired,
    NotApplicable,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MeasurementModeSupport {
    Supported,
    Unsupported {
        reason: MeasurementUnavailableReason,
    },
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementOperatingSystem {
    Linux,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementArchitecture {
    X86_64,
    Aarch64,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementPlatformFeature {
    Procfs,
    CgroupV2,
}
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementPlatform {
    pub operating_system: MeasurementOperatingSystem,
    pub architectures: Vec<MeasurementArchitecture>,
    pub required_features: Vec<MeasurementPlatformFeature>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementOverheadClass {
    Low,
    Material,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementOverhead {
    pub class: MeasurementOverheadClass,
    pub minimum_interval_ns: u64,
    pub requires_privilege: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementEvidenceLimit {
    MissingIsUnavailable,
    NotCausal,
    RequiresComparableExperiment,
    CollectorOverheadRecorded,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementAggregation {
    Gauge,
    Counter,
    Distribution,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementReplayMode {
    Live,
    Replay,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementProvenance {
    pub source: MeasurementSource,
    pub qualification: MeasurementQualification,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub group: MeasurementGroupId,
    pub quantity: MeasurementQuantity,
    pub unit: String,
    pub aggregation: MeasurementAggregation,
    pub scope: MeasurementScope,
    pub provenance: MeasurementProvenance,
    pub source_identity: MeasurementSourceIdentity,
    pub resolution_ns: u64,
    pub overhead: MeasurementOverhead,
    pub live: MeasurementModeSupport,
    pub replay: MeasurementModeSupport,
    pub platforms: Vec<MeasurementPlatform>,
    pub evidence_limits: Vec<MeasurementEvidenceLimit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementCatalog {
    pub schema_version: u16,
    pub catalog_sha256: String,
    pub groups: Vec<MeasurementGroup>,
    pub measurements: Vec<MeasurementDefinition>,
}

#[derive(Serialize)]
struct MeasurementCatalogAddress<'a> {
    schema_version: u16,
    groups: &'a [MeasurementGroup],
    measurements: &'a [MeasurementDefinition],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementCatalogPublication {
    pub version: ControlVersion,
    pub freshness: MeasurementCatalogFreshness,
    pub source: MeasurementCatalogPublicationSource,
    pub catalog: MeasurementCatalog,
}

impl MeasurementCatalogPublication {
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.version != CONTROL_MEASUREMENT_CATALOG_V1
            || self.catalog.schema_version != 1
            || self.catalog.groups.len() > MAX_MEASUREMENT_GROUPS
            || self.catalog.measurements.len() > MAX_MEASUREMENTS
        {
            return Err(CodecError::InvalidValue("measurement_catalog"));
        }
        validate_digest(&self.catalog.catalog_sha256)?;
        let mut groups = BTreeSet::new();
        let mut previous_group = None;
        for group in &self.catalog.groups {
            validate_catalog_text(&group.label, MAX_PUBLIC_STRING_BYTES)?;
            validate_catalog_text(&group.description, MAX_PUBLIC_STRING_BYTES)?;
            if !groups.insert(group.id) {
                return Err(CodecError::InvalidValue("measurement_catalog.groups"));
            }
            if previous_group.is_some_and(|previous| previous >= group.id) {
                return Err(CodecError::InvalidValue("measurement_catalog.groups"));
            }
            previous_group = Some(group.id);
        }
        let mut ids = BTreeSet::new();
        let mut previous_id = None;
        for definition in &self.catalog.measurements {
            validate_catalog_id(&definition.id)?;
            validate_catalog_text(&definition.name, MAX_PUBLIC_STRING_BYTES)?;
            validate_catalog_text(&definition.description, MAX_PUBLIC_STRING_BYTES)?;
            validate_catalog_text(&definition.unit, 32)?;
            if !groups.contains(&definition.group) || !ids.insert(definition.id.as_str()) {
                return Err(CodecError::InvalidValue("measurement_catalog.measurements"));
            }
            if previous_id.is_some_and(|previous: &str| previous >= definition.id.as_str()) {
                return Err(CodecError::InvalidValue("measurement_catalog.measurements"));
            }
            previous_id = Some(definition.id.as_str());
            if definition.platforms.is_empty()
                || definition.platforms.len() > 8
                || definition.evidence_limits.len() > 4
                || definition.overhead.minimum_interval_ns == 0
            {
                return Err(CodecError::InvalidValue("measurement_catalog.measurements"));
            }
        }
        let computed = self.catalog.computed_digest()?;
        if computed != self.catalog.catalog_sha256 {
            return Err(CodecError::InvalidValue("measurement_catalog.digest"));
        }
        let encoded = serde_json::to_vec(self).map_err(|_| CodecError::Serialization)?;
        if encoded.len() > MAX_MEASUREMENT_CATALOG_WIRE_BYTES {
            return Err(CodecError::FrameTooLarge);
        }
        Ok(())
    }
}

impl MeasurementCatalog {
    /// Compute ASB's canonical catalog digest (domain plus exact serde field
    /// order, excluding `catalog_sha256`).
    pub fn computed_digest(&self) -> Result<String, CodecError> {
        let address = serde_json::to_vec(&MeasurementCatalogAddress {
            schema_version: self.schema_version,
            groups: &self.groups,
            measurements: &self.measurements,
        })
        .map_err(|_| CodecError::Serialization)?;
        let mut input = b"asb-measurement-catalog-v1\0".to_vec();
        input.extend_from_slice(&address);
        Ok(crate::sha256::digest_hex(&input))
    }
}

fn validate_catalog_text(value: &str, max: usize) -> Result<(), CodecError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(CodecError::InvalidValue("measurement_catalog.text"));
    }
    Ok(())
}

fn validate_catalog_id(value: &str) -> Result<(), CodecError> {
    if value.len() < 3
        || value.len() > MAX_ID_BYTES
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        || !value.as_bytes()[0].is_ascii_lowercase()
    {
        return Err(CodecError::InvalidValue("measurement_catalog.id"));
    }
    Ok(())
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReference {
    pub plan_id: String,
    pub plan_sha256: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationAcknowledgement {
    pub accepted: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSummary {
    pub run_count: u16,
    pub analysis_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadata {
    pub sha256: String,
    pub size_bytes: u64,
    pub sensitivity: ArtifactSensitivity,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSensitivity {
    Public,
    Sensitive,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEvent {
    pub revision: Revision,
    pub kind: ControlEventKind,
    pub run_id: Option<RunId>,
    pub attempt_id: Option<AttemptId>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlEventKind {
    RunnerReady,
    PlanCreated,
    RunStarted,
    RunUpdated,
    RunCompleted,
    RunFailed,
    RunCancelled,
    ReconciliationRequired,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<Revision>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ControlResult {
    Capabilities(Capabilities),
    MeasurementCatalog(MeasurementCatalogPublication),
    SettingsValidation(SettingsValidation),
    Plan(PlanReference),
    Launch(RunSummary),
    Status(RunSummary),
    Acknowledged(MutationAcknowledgement),
    History(Page<RunSummary>),
    Events(Page<ControlEvent>),
    Analysis(AnalysisSummary),
    ArtifactMetadata(ArtifactMetadata),
}

impl ControlRequest {
    pub fn validate(&self, limits: ControlLimits) -> Result<(), CodecError> {
        if self.jsonrpc != JSONRPC_VERSION {
            return Err(CodecError::InvalidVersion);
        }
        let limits = limits.validate()?;
        if self.timeout_ms == 0 || self.timeout_ms > limits.max_timeout_ms {
            return Err(CodecError::InvalidLimit("timeout_ms"));
        }
        validate_call(&self.call)
    }
}
impl ControlResponse {
    pub fn id(&self) -> RequestId {
        match self {
            Self::Success(v) => v.id,
            Self::Failure(v) => v.id,
        }
    }
    pub fn validate(&self, limits: ControlLimits) -> Result<(), CodecError> {
        match self {
            Self::Success(v) => {
                if v.jsonrpc != JSONRPC_VERSION {
                    return Err(CodecError::InvalidVersion);
                }
                validate_success(&v.result, limits)
            }
            Self::Failure(v) => {
                if v.jsonrpc != JSONRPC_VERSION {
                    return Err(CodecError::InvalidVersion);
                }
                if v.message_len_bad() {
                    return Err(CodecError::InvalidValue("error.message"));
                }
                Ok(())
            }
        }
    }
    pub fn validate_for(
        &self,
        request: &ControlRequest,
        limits: ControlLimits,
    ) -> Result<(), CodecError> {
        self.validate(limits)?;
        if self.id() != request.id {
            return Err(CodecError::ResponseIdMismatch);
        }
        if let Self::Success(value) = self
            && let ControlSuccess::Operation(bound) = &value.result
            && !bound.result.matches(&request.call)
        {
            return Err(CodecError::ResultMismatch);
        }
        Ok(())
    }
}
impl FailureResponse {
    fn message_len_bad(&self) -> bool {
        self.error.message.len() > MAX_PUBLIC_STRING_BYTES
            || self.error.message.chars().any(char::is_control)
    }
}

pub fn encode<T: Serialize>(value: &T, max_frame: usize) -> Result<Vec<u8>, CodecError> {
    let body = serde_json::to_vec(value).map_err(|_| CodecError::Serialization)?;
    if body.is_empty() || body.len() > max_frame || body.len() > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend(body);
    Ok(out)
}
pub fn decode<T: for<'de> Deserialize<'de>>(
    frame: &[u8],
    max_frame: usize,
) -> Result<T, CodecError> {
    if frame.len() < 4 {
        return Err(CodecError::TruncatedFrame);
    }
    let n = u32::from_be_bytes(
        frame[..4]
            .try_into()
            .map_err(|_| CodecError::TruncatedFrame)?,
    ) as usize;
    if n == 0 || n > max_frame || n > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge);
    }
    if frame.len() != n + 4 {
        return Err(CodecError::TruncatedFrame);
    }
    serde_json::from_slice(&frame[4..]).map_err(|_| CodecError::MalformedFrame)
}

fn validate_call(call: &ControlCall) -> Result<(), CodecError> {
    match call {
        ControlCall::Negotiate(v) => {
            if v.versions.is_empty() || v.versions.iter().any(|v| !matches!(*v, V1_0 | V1_2 | V1_3))
            {
                return Err(CodecError::UnsupportedVersion);
            }
            v.limits.validate()?;
        }
        ControlCall::ValidateSettings { settings } => validate_json(settings)?,
        ControlCall::CreatePlan(v) => {
            validate_id(&v.idempotency_key)?;
            validate_json(&v.definition)?;
        }
        ControlCall::Launch(v) => {
            validate_id(&v.idempotency_key)?;
            validate_id(&v.plan_id)?;
        }
        ControlCall::Status { run_id } => run_id.validate()?,
        ControlCall::Cancel(v) => {
            v.run_id.validate()?;
            v.attempt_id.validate()?;
            validate_id(&v.idempotency_key)?;
        }
        ControlCall::History(v) | ControlCall::Events(v) => {
            if v.limit == 0 || v.limit > MAX_PAGE_ITEMS {
                return Err(CodecError::InvalidLimit("limit"));
            }
        }
        ControlCall::Repeat(v) => {
            v.run_id.validate()?;
            validate_id(&v.idempotency_key)?;
        }
        ControlCall::Analyze { run_ids } => {
            if run_ids.is_empty() || run_ids.len() > MAX_ANALYSIS_RUNS {
                return Err(CodecError::InvalidValue("run_ids"));
            }
            for id in run_ids {
                id.validate()?;
            }
            if run_ids.iter().collect::<BTreeSet<_>>().len() != run_ids.len() {
                return Err(CodecError::InvalidValue("run_ids"));
            }
        }
        ControlCall::ArtifactMetadata { run_id, digest } => {
            run_id.validate()?;
            validate_digest(digest)?;
        }
        ControlCall::Capabilities => {}
        ControlCall::MeasurementCatalog => {}
    }
    Ok(())
}
fn validate_success(success: &ControlSuccess, limits: ControlLimits) -> Result<(), CodecError> {
    match success {
        ControlSuccess::Negotiated(v) => {
            if !matches!(v.version, V1_0 | V1_2 | V1_3) || v.oldest_revision > v.latest_revision {
                return Err(CodecError::InvalidVersion);
            }
            v.limits.validate()?;
            validate_id(&v.runner_instance_id)?;
        }
        ControlSuccess::Operation(v) => {
            validate_digest(&v.request_sha256)?;
            validate_result(&v.result, limits)?;
        }
    }
    Ok(())
}
fn validate_result(result: &ControlResult, limits: ControlLimits) -> Result<(), CodecError> {
    match result {
        ControlResult::Capabilities(_) => {}
        ControlResult::MeasurementCatalog(v) => v.validate()?,
        ControlResult::SettingsValidation(v) => {
            if v.issues.len() > limits.max_page_items as usize
                || v.valid != v.issues.is_empty()
                || v.issues.iter().collect::<BTreeSet<_>>().len() != v.issues.len()
            {
                return Err(CodecError::InvalidValue("settings validation"));
            }
        }
        ControlResult::Plan(v) => {
            validate_id(&v.plan_id)?;
            validate_digest(&v.plan_sha256)?;
        }
        ControlResult::Launch(v) | ControlResult::Status(v) => validate_summary(v)?,
        ControlResult::Acknowledged(v) => {
            if !v.accepted {
                return Err(CodecError::InvalidValue("accepted"));
            }
        }
        ControlResult::History(v) => {
            if v.items.len() > limits.max_page_items as usize {
                return Err(CodecError::InvalidValue("history.items"));
            }
            for x in &v.items {
                validate_summary(x)?;
            }
        }
        ControlResult::Events(v) => {
            if v.items.len() > limits.max_page_items as usize {
                return Err(CodecError::InvalidValue("events.items"));
            }
            for x in &v.items {
                validate_event(x)?;
            }
        }
        ControlResult::Analysis(v) => {
            if v.run_count == 0 || v.run_count as usize > MAX_ANALYSIS_RUNS {
                return Err(CodecError::InvalidValue("run_count"));
            }
            validate_digest(&v.analysis_sha256)?;
        }
        ControlResult::ArtifactMetadata(v) => validate_digest(&v.sha256)?,
    }
    Ok(())
}
fn validate_summary(v: &RunSummary) -> Result<(), CodecError> {
    v.run_id.validate()?;
    v.attempt_id.validate()?;
    validate_digest(&v.plan_sha256)?;
    if v.created_revision > v.revision {
        return Err(CodecError::InvalidValue("revision"));
    }
    Ok(())
}
fn validate_event(v: &ControlEvent) -> Result<(), CodecError> {
    let associated = v.run_id.is_some() && v.attempt_id.is_some();
    match v.kind {
        ControlEventKind::RunnerReady | ControlEventKind::PlanCreated if associated => {
            Err(CodecError::InvalidValue("event association"))
        }
        ControlEventKind::RunStarted
        | ControlEventKind::RunUpdated
        | ControlEventKind::RunCompleted
        | ControlEventKind::RunFailed
        | ControlEventKind::RunCancelled
        | ControlEventKind::ReconciliationRequired
            if !associated =>
        {
            Err(CodecError::InvalidValue("event association"))
        }
        _ => {
            if let Some(v) = &v.run_id {
                v.validate()?;
            }
            if let Some(v) = &v.attempt_id {
                v.validate()?;
            }
            Ok(())
        }
    }
}
fn validate_id(value: &str) -> Result<(), CodecError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value.is_ascii()
        || value.chars().any(char::is_control)
    {
        Err(CodecError::InvalidValue("identity"))
    } else {
        Ok(())
    }
}
fn validate_digest(value: &str) -> Result<(), CodecError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Err(CodecError::InvalidValue("sha256"))
    } else {
        Ok(())
    }
}
fn validate_json(value: &Value) -> Result<(), CodecError> {
    fn walk(v: &Value, depth: usize, nodes: &mut usize) -> Result<(), CodecError> {
        *nodes += 1;
        if *nodes > MAX_JSON_NODES || depth > MAX_JSON_DEPTH {
            return Err(CodecError::JsonTooLarge);
        }
        match v {
            Value::Array(a) => a.iter().try_for_each(|v| walk(v, depth + 1, nodes)),
            Value::Object(o) => o.values().try_for_each(|v| walk(v, depth + 1, nodes)),
            _ => Ok(()),
        }
    }
    walk(value, 0, &mut 0)
}
impl ControlResult {
    fn matches(&self, call: &ControlCall) -> bool {
        matches!(
            (call, self),
            (ControlCall::Capabilities, Self::Capabilities(_))
                | (ControlCall::MeasurementCatalog, Self::MeasurementCatalog(_))
                | (
                    ControlCall::ValidateSettings { .. },
                    Self::SettingsValidation(_)
                )
                | (ControlCall::CreatePlan(_), Self::Plan(_))
                | (ControlCall::Launch(_), Self::Launch(_))
                | (ControlCall::Status { .. }, Self::Status(_))
                | (ControlCall::Cancel(_), Self::Acknowledged(_))
                | (ControlCall::History(_), Self::History(_))
                | (ControlCall::Repeat(_), Self::Plan(_))
                | (ControlCall::Analyze { .. }, Self::Analysis(_))
                | (ControlCall::Events(_), Self::Events(_))
                | (
                    ControlCall::ArtifactMetadata { .. },
                    Self::ArtifactMetadata(_)
                )
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    InvalidLimit(&'static str),
    InvalidVersion,
    UnsupportedVersion,
    InvalidValue(&'static str),
    JsonTooLarge,
    FrameTooLarge,
    TruncatedFrame,
    MalformedFrame,
    Serialization,
    ResponseIdMismatch,
    ResultMismatch,
}
impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CodecError {}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(id: u64) -> ControlRequest {
        ControlRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId(id),
            timeout_ms: 1000,
            call: ControlCall::Capabilities,
        }
    }
    #[test]
    fn numeric_ids_round_trip() {
        let bytes = encode(&req(u64::MAX), 10000).unwrap();
        let got: ControlRequest = decode(&bytes, 10000).unwrap();
        assert_eq!(got.id, RequestId(u64::MAX));
    }
    #[test]
    fn unknown_fields_are_rejected() {
        let x = br#"{"jsonrpc":"2.0","id":1,"timeout_ms":1,"method":"capabilities","extra":true}"#;
        assert!(serde_json::from_slice::<ControlRequest>(x).is_err());
    }
    #[test]
    fn oversized_frame_is_rejected() {
        assert_eq!(encode(&req(1), 1), Err(CodecError::FrameTooLarge));
    }
    #[test]
    fn oversized_nested_json_is_rejected() {
        let settings = Value::Array((0..(MAX_JSON_NODES + 1)).map(|_| Value::Null).collect());
        let r = ControlRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId(1),
            timeout_ms: 1,
            call: ControlCall::ValidateSettings { settings },
        };
        assert_eq!(
            r.validate(ControlLimits::default()),
            Err(CodecError::JsonTooLarge)
        );
    }
    #[test]
    fn response_id_must_match_request() {
        let r = ControlResponse::Failure(FailureResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId(2),
            error: RpcError {
                code: -1,
                message: "x".into(),
            },
        });
        assert_eq!(
            r.validate_for(&req(1), ControlLimits::default()),
            Err(CodecError::ResponseIdMismatch)
        );
    }
    #[test]
    fn unsupported_version_is_rejected() {
        let r = ControlRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId(1),
            timeout_ms: 1,
            call: ControlCall::Negotiate(NegotiateParams {
                versions: [ControlVersion { major: 9, minor: 0 }]
                    .into_iter()
                    .collect(),
                limits: ControlLimits::default(),
            }),
        };
        assert_eq!(
            r.validate(ControlLimits::default()),
            Err(CodecError::UnsupportedVersion)
        );
    }

    fn catalog() -> MeasurementCatalogPublication {
        let mut publication = MeasurementCatalogPublication {
            version: CONTROL_MEASUREMENT_CATALOG_V1,
            freshness: MeasurementCatalogFreshness::ContentAddressed,
            source: MeasurementCatalogPublicationSource::BuiltInCollectors,
            catalog: MeasurementCatalog {
                schema_version: 1,
                catalog_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
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
    fn measurement_catalog_round_trips_and_matches_call() {
        let request = ControlRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId(4),
            timeout_ms: 1_000,
            call: ControlCall::MeasurementCatalog,
        };
        request.validate(ControlLimits::default()).unwrap();
        let response = ControlResponse::Success(SuccessResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId(4),
            result: ControlSuccess::Operation(BoundResult {
                request_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
                result: ControlResult::MeasurementCatalog(catalog()),
            }),
        });
        response
            .validate_for(&request, ControlLimits::default())
            .unwrap();
        let bytes = encode(&response, MAX_MEASUREMENT_CATALOG_WIRE_BYTES).unwrap();
        let decoded: ControlResponse = decode(&bytes, MAX_MEASUREMENT_CATALOG_WIRE_BYTES).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn measurement_catalog_rejects_unknown_group_and_duplicate_id() {
        let mut publication = catalog();
        publication.catalog.measurements[0].group = MeasurementGroupId::Cost;
        assert_eq!(
            publication.validate(),
            Err(CodecError::InvalidValue("measurement_catalog.measurements"))
        );
        let mut publication = catalog();
        publication
            .catalog
            .measurements
            .push(publication.catalog.measurements[0].clone());
        assert_eq!(
            publication.validate(),
            Err(CodecError::InvalidValue("measurement_catalog.measurements"))
        );
    }

    #[test]
    fn asb_catalog_vector_matches_canonical_digest_domain() {
        // This is the first group and first definition from ASB's checked-in
        // v1 catalog vector, reduced only to a bounded fixture.
        let raw = r#"{"schema_version":1,"catalog_sha256":"dd196af95a66cc083a3d3d136adfe8522f7ac0abc653ccb0eceeda263bdeb2f0","groups":[{"id":"system_resources","label":"System resources","description":"CPU, memory, faults, and physical IO attributed to a process or cgroup."}],"measurements":[{"id":"cgroup.cpu.periods","name":"Cgroup CPU periods","description":"A direct bounded Linux kernel counter collected by ASB.","group":"system_resources","quantity":"count","unit":"{event}","aggregation":"counter","scope":"cgroup","provenance":{"source":"asb_metrics_cgroup_v2","qualification":"implemented"},"source_identity":"cgroup_v2_cpu_stat","resolution_ns":0,"overhead":{"class":"low","minimum_interval_ns":1000000,"requires_privilege":false},"live":{"status":"supported"},"replay":{"status":"supported"},"platforms":[{"operating_system":"linux","architectures":["x86_64","aarch64"],"required_features":["cgroup_v2"]}],"evidence_limits":["missing_is_unavailable","requires_comparable_experiment","collector_overhead_recorded"]}]}"#;
        let catalog: MeasurementCatalog = serde_json::from_str(raw).unwrap();
        assert_eq!(catalog.computed_digest().unwrap(), catalog.catalog_sha256);
        let publication = MeasurementCatalogPublication {
            version: CONTROL_MEASUREMENT_CATALOG_V1,
            freshness: MeasurementCatalogFreshness::ContentAddressed,
            source: MeasurementCatalogPublicationSource::BuiltInCollectors,
            catalog,
        };
        publication.validate().unwrap();
    }
}
