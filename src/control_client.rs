// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral state for the authenticated ASB control connection.
//!
//! This module deliberately contains no terminal, widget, or benchmark code.  It
//! models the wire boundary and the state which a future transport adapter may
//! consume.  A runner remains authoritative: disconnecting this state machine
//! never creates, retries, cancels, or otherwise mutates a run.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_RUNNER_ID_BYTES: usize = 128;
pub const MAX_PAGE_ITEMS: u16 = 256;
pub const MAX_TIMEOUT_MS: u64 = 5 * 60 * 1000;
pub const MAX_REQUEST_ID_BYTES: usize = 128;
pub const MAX_CURSOR_BYTES: usize = 20;

/// Versions which this client can actually decode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlVersion {
    pub major: u16,
    pub minor: u16,
}

pub const V1_0: ControlVersion = ControlVersion { major: 1, minor: 0 };
pub const V1_2: ControlVersion = ControlVersion { major: 1, minor: 2 };
pub const V1_3: ControlVersion = ControlVersion { major: 1, minor: 3 };
pub const V1_4: ControlVersion = ControlVersion { major: 1, minor: 4 };
pub const V1_5: ControlVersion = ControlVersion { major: 1, minor: 5 };

#[must_use]
pub fn supported_versions() -> BTreeSet<ControlVersion> {
    [V1_0, V1_2, V1_3, V1_4, V1_5].into_iter().collect()
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
            max_in_flight: 1,
        }
    }
}

impl ControlLimits {
    pub fn validate(self) -> Result<Self, ClientError> {
        if self.max_frame_bytes == 0 || self.max_frame_bytes as usize > MAX_FRAME_BYTES {
            return Err(ClientError::InvalidLimit("max_frame_bytes"));
        }
        if self.max_timeout_ms == 0 || self.max_timeout_ms > MAX_TIMEOUT_MS {
            return Err(ClientError::InvalidLimit("max_timeout_ms"));
        }
        if self.max_page_items == 0 || self.max_page_items > MAX_PAGE_ITEMS {
            return Err(ClientError::InvalidLimit("max_page_items"));
        }
        if self.max_in_flight == 0 {
            return Err(ClientError::InvalidLimit("max_in_flight"));
        }
        Ok(self)
    }

    pub fn intersect(self, peer: Self) -> Result<Self, ClientError> {
        self.validate()?;
        peer.validate()?;
        Ok(Self {
            max_frame_bytes: self.max_frame_bytes.min(peer.max_frame_bytes),
            max_timeout_ms: self.max_timeout_ms.min(peer.max_timeout_ms),
            max_page_items: self.max_page_items.min(peer.max_page_items),
            max_in_flight: self.max_in_flight.min(peer.max_in_flight),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiateRequest {
    pub versions: BTreeSet<ControlVersion>,
    pub limits: ControlLimits,
}

impl NegotiateRequest {
    pub fn new(limits: ControlLimits) -> Result<Self, ClientError> {
        let limits = limits.validate()?;
        Ok(Self {
            versions: supported_versions(),
            limits,
        })
    }

    pub fn validate(&self) -> Result<(), ClientError> {
        self.limits.validate()?;
        if self.versions.is_empty()
            || self
                .versions
                .iter()
                .any(|v| !supported_versions().contains(v))
        {
            return Err(ClientError::UnsupportedVersion);
        }
        Ok(())
    }
}

/// Select only an offered and implemented version.  Minor versions are not
/// silently upgraded or invented.
pub fn select_version(
    offered: &BTreeSet<ControlVersion>,
    peer: &BTreeSet<ControlVersion>,
) -> Result<ControlVersion, ClientError> {
    let supported = supported_versions();
    if offered.is_empty()
        || peer.is_empty()
        || offered.iter().any(|v| !supported.contains(v))
        || peer.iter().any(|v| !supported.contains(v))
    {
        return Err(ClientError::UnsupportedVersion);
    }
    [V1_5, V1_4, V1_3, V1_2, V1_0]
        .into_iter()
        .find(|v| offered.contains(v) && peer.contains(v))
        .ok_or(ClientError::IncompatibleVersion)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerIdentity {
    pub uid: u32,
    pub pid: u32,
    pub service_generation: u64,
    pub runner_instance_id: String,
}

impl RunnerIdentity {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.pid == 0
            || self.runner_instance_id.is_empty()
            || self.runner_instance_id.len() > MAX_RUNNER_ID_BYTES
            || !safe_id(&self.runner_instance_id)
        {
            return Err(ClientError::InvalidIdentity);
        }
        Ok(())
    }
}

/// Kernel-derived descriptor facts supplied by the broker adapter.  The
/// adapter must obtain these from the received descriptor, never from argv or
/// an endpoint string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCredentials {
    pub uid: u32,
    pub pid: u32,
    pub service_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdoptedChannel {
    descriptor: u32,
    credentials: PeerCredentials,
}

impl AdoptedChannel {
    pub fn new(descriptor: u32, credentials: PeerCredentials) -> Result<Self, ClientError> {
        if descriptor < 3 || credentials.pid == 0 {
            return Err(ClientError::InvalidDescriptor);
        }
        Ok(Self {
            descriptor,
            credentials,
        })
    }

    #[must_use]
    pub fn descriptor(&self) -> u32 {
        self.descriptor
    }

    pub fn authenticate(&self, expected: &RunnerIdentity) -> Result<(), ClientError> {
        expected.validate()?;
        if self.credentials.uid != expected.uid
            || self.credentials.pid != expected.pid
            || self.credentials.service_generation != expected.service_generation
        {
            return Err(ClientError::IdentityChanged);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Negotiated {
    pub version: ControlVersion,
    pub limits: ControlLimits,
    pub runner: RunnerIdentity,
    pub oldest_revision: u64,
    pub latest_revision: u64,
}

impl Negotiated {
    pub fn validate(
        &self,
        offered: &BTreeSet<ControlVersion>,
        local: ControlLimits,
    ) -> Result<(), ClientError> {
        if !supported_versions().contains(&self.version)
            || !offered.contains(&self.version)
            || self.oldest_revision > self.latest_revision
        {
            return Err(ClientError::InvalidNegotiation);
        }
        self.runner.validate()?;
        let limits = self.limits.validate()?;
        local.validate()?;
        if limits.max_frame_bytes > local.max_frame_bytes
            || limits.max_timeout_ms > local.max_timeout_ms
            || limits.max_page_items > local.max_page_items
            || limits.max_in_flight > local.max_in_flight
        {
            return Err(ClientError::InvalidNegotiation);
        }
        Ok(())
    }

    #[must_use]
    pub fn measurement_catalog_available(&self) -> bool {
        self.version >= V1_2
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReconnectCursor {
    revision: Option<u64>,
}

impl ReconnectCursor {
    #[must_use]
    pub fn revision(self) -> Option<u64> {
        self.revision
    }

    pub fn resume_after(self, negotiated: &Negotiated) -> Result<Option<u64>, ClientError> {
        if let Some(revision) = self.revision {
            if revision < negotiated.oldest_revision.saturating_sub(1) {
                return Err(ClientError::StaleCursor);
            }
            if revision > negotiated.latest_revision {
                return Err(ClientError::FutureCursor);
            }
        }
        Ok(self.revision)
    }

    pub fn advance(
        &mut self,
        next_revision: u64,
        negotiated: &Negotiated,
    ) -> Result<(), ClientError> {
        if next_revision < negotiated.oldest_revision || next_revision > negotiated.latest_revision
        {
            return Err(ClientError::InvalidCursor);
        }
        if self
            .revision
            .is_some_and(|current| next_revision <= current)
        {
            return Err(ClientError::NonMonotonicCursor);
        }
        self.revision = Some(next_revision);
        Ok(())
    }
}

/// A bounded request identifier. Request identifiers are opaque protocol
/// values, but are restricted to a small public alphabet so malformed input
/// cannot become an unbounded log/UI payload.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequestId(String);

impl RequestId {
    pub fn new(value: impl Into<String>) -> Result<Self, ClientError> {
        let value = value.into();
        if safe_id(&value) && value.len() <= MAX_REQUEST_ID_BYTES {
            Ok(Self(value))
        } else {
            Err(ClientError::InvalidRequestId)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for RequestId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A textual cursor is accepted only in canonical unsigned-decimal form.
/// Numeric revisions remain the internal representation, preventing leading
/// zero aliases and bounding the wire representation to a u64.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CursorToken(u64);

impl CursorToken {
    pub fn parse(value: &str) -> Result<Self, ClientError> {
        if value.is_empty()
            || value.len() > MAX_CURSOR_BYTES
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(ClientError::InvalidCursorToken);
        }
        value
            .parse::<u64>()
            .map(Self)
            .map_err(|_| ClientError::InvalidCursorToken)
    }

    #[must_use]
    pub fn revision(self) -> u64 {
        self.0
    }
}

impl Serialize for CursorToken {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for CursorToken {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestClass {
    ReadOnly,
    Mutation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryPlan {
    pub read_only: Vec<RequestId>,
    pub abandoned_mutations: Vec<RequestId>,
}

/// Tracks only request identity and class, never request payloads. A
/// reconnect plan can retry reads but explicitly abandons mutations; this
/// prevents accidental replay of run/plan/cancel operations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequestLedger {
    pending: BTreeMap<RequestId, RequestClass>,
}

impl RequestLedger {
    pub fn begin(
        &mut self,
        id: RequestId,
        class: RequestClass,
        limits: ControlLimits,
    ) -> Result<(), ClientError> {
        limits.validate()?;
        if self.pending.len() >= limits.max_in_flight as usize || self.pending.contains_key(&id) {
            return Err(ClientError::RequestLimit);
        }
        self.pending.insert(id, class);
        Ok(())
    }

    pub fn complete(&mut self, id: &RequestId) -> Result<RequestClass, ClientError> {
        self.pending.remove(id).ok_or(ClientError::UnknownRequest)
    }

    pub fn reconnect_plan(&mut self) -> RetryPlan {
        let mut plan = RetryPlan {
            read_only: Vec::new(),
            abandoned_mutations: Vec::new(),
        };
        for (id, class) in std::mem::take(&mut self.pending) {
            match class {
                RequestClass::ReadOnly => plan.read_only.push(id),
                RequestClass::Mutation => plan.abandoned_mutations.push(id),
            }
        }
        plan
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Disconnected {
        cursor: ReconnectCursor,
    },
    Negotiated {
        channel: AdoptedChannel,
        session: Negotiated,
        cursor: ReconnectCursor,
    },
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::Disconnected {
            cursor: ReconnectCursor::default(),
        }
    }
}

impl ConnectionState {
    pub fn adopt(channel: AdoptedChannel, expected: &RunnerIdentity) -> Result<Self, ClientError> {
        channel.authenticate(expected)?;
        Ok(Self::Disconnected {
            cursor: ReconnectCursor::default(),
        })
    }

    pub fn establish(
        self,
        channel: AdoptedChannel,
        negotiated: Negotiated,
        offered: &BTreeSet<ControlVersion>,
        local: ControlLimits,
    ) -> Result<Self, ClientError> {
        let cursor = self.cursor();
        channel.authenticate(&negotiated.runner)?;
        negotiated.validate(offered, local)?;
        cursor.resume_after(&negotiated)?;
        Ok(Self::Negotiated {
            channel,
            session: negotiated,
            cursor,
        })
    }

    pub fn disconnect(&self) -> Self {
        Self::Disconnected {
            cursor: self.cursor(),
        }
    }

    #[must_use]
    pub fn cursor(&self) -> ReconnectCursor {
        match self {
            Self::Disconnected { cursor } | Self::Negotiated { cursor, .. } => *cursor,
        }
    }

    pub fn advance_cursor(&mut self, revision: u64) -> Result<(), ClientError> {
        let (session, cursor) = match self {
            Self::Negotiated {
                session, cursor, ..
            } => (session, cursor),
            _ => return Err(ClientError::NotNegotiated),
        };
        cursor.advance(revision, session)
    }
}

/// Length-prefixed JSON framing with no partial-frame acceptance.
pub fn encode_frame<T: Serialize>(value: &T, max_frame: usize) -> Result<Vec<u8>, ClientError> {
    let body = serde_json::to_vec(value).map_err(|_| ClientError::Serialization)?;
    if body.is_empty()
        || body.len() > max_frame
        || body.len() > MAX_FRAME_BYTES
        || body.len() > u32::MAX as usize
    {
        return Err(ClientError::FrameTooLarge);
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode_frame<T: for<'de> Deserialize<'de>>(
    frame: &[u8],
    max_frame: usize,
) -> Result<T, ClientError> {
    if frame.len() < 4 {
        return Err(ClientError::TruncatedFrame);
    }
    let length = u32::from_be_bytes(
        frame[..4]
            .try_into()
            .map_err(|_| ClientError::TruncatedFrame)?,
    ) as usize;
    if length == 0 || length > max_frame || length > MAX_FRAME_BYTES {
        return Err(ClientError::FrameTooLarge);
    }
    if frame.len() != length + 4 {
        return Err(ClientError::TruncatedFrame);
    }
    serde_json::from_slice(&frame[4..]).map_err(|_| ClientError::MalformedFrame)
}

/// Typed validation failure categories shared by v1.0/v1.2 and v1.3.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCategory {
    InvalidFormat,
    UnsupportedCapability,
    UnverifiedComponent,
    InvalidResourceBound,
    MeasurementIssue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementReason {
    UnknownId,
    Unsupported,
    Unavailable,
    InvalidConfiguration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationIssue {
    pub category: ValidationCategory,
    #[serde(default)]
    pub reason: Option<MeasurementReason>,
    #[serde(default)]
    pub measurement_id: Option<String>,
}

pub fn validate_issue(version: ControlVersion, issue: &ValidationIssue) -> Result<(), ClientError> {
    if version == V1_0 || version == V1_2 {
        if issue.reason.is_some()
            || issue.measurement_id.is_some()
            || issue.category == ValidationCategory::MeasurementIssue
        {
            return Err(ClientError::InvalidIssue);
        }
        if !matches!(
            issue.category,
            ValidationCategory::InvalidFormat
                | ValidationCategory::UnsupportedCapability
                | ValidationCategory::UnverifiedComponent
                | ValidationCategory::InvalidResourceBound
        ) {
            return Err(ClientError::InvalidIssue);
        }
        return Ok(());
    }
    if version != V1_3 {
        return Err(ClientError::UnsupportedVersion);
    }
    if issue.category == ValidationCategory::MeasurementIssue {
        let Some(reason) = issue.reason else {
            return Err(ClientError::InvalidIssue);
        };
        if matches!(reason, MeasurementReason::UnknownId) && issue.measurement_id.is_some() {
            return Err(ClientError::InvalidIssue);
        }
        if let Some(id) = &issue.measurement_id
            && !safe_id(id)
        {
            return Err(ClientError::InvalidIssue);
        }
    } else if issue.reason.is_some() || issue.measurement_id.is_some() {
        return Err(ClientError::InvalidIssue);
    }
    Ok(())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value.is_ascii()
        && !value.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientError {
    InvalidLimit(&'static str),
    UnsupportedVersion,
    IncompatibleVersion,
    InvalidIdentity,
    InvalidDescriptor,
    IdentityChanged,
    InvalidNegotiation,
    StaleCursor,
    FutureCursor,
    InvalidCursor,
    NonMonotonicCursor,
    NotNegotiated,
    FrameTooLarge,
    TruncatedFrame,
    MalformedFrame,
    Serialization,
    InvalidIssue,
    InvalidRequestId,
    InvalidCursorToken,
    RequestLimit,
    UnknownRequest,
    ReconnectExhausted,
    VersionChanged,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit(name) => write!(f, "invalid control limit: {name}"),
            other => write!(f, "{other:?}"),
        }
    }
}
impl std::error::Error for ClientError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn identity() -> RunnerIdentity {
        RunnerIdentity {
            uid: 1000,
            pid: 42,
            service_generation: 7,
            runner_instance_id: "runner-7".into(),
        }
    }
    fn negotiated(version: ControlVersion) -> Negotiated {
        Negotiated {
            version,
            limits: ControlLimits::default(),
            runner: identity(),
            oldest_revision: 1,
            latest_revision: 4,
        }
    }

    #[test]
    fn offers_exact_versions_and_prefers_newest_exact_version() {
        let offered = supported_versions();
        assert_eq!(select_version(&offered, &offered).unwrap(), V1_5);
        assert_eq!(
            select_version(&offered, &[V1_2].into_iter().collect()).unwrap(),
            V1_2
        );
        assert_eq!(
            select_version(&offered, &[V1_0].into_iter().collect()).unwrap(),
            V1_0
        );
        assert_eq!(
            select_version(
                &[ControlVersion { major: 1, minor: 4 }]
                    .into_iter()
                    .collect(),
                &offered
            ),
            Ok(V1_4)
        );
    }

    #[test]
    fn descriptor_authentication_fences_runner_generation() {
        let channel = AdoptedChannel::new(
            3,
            PeerCredentials {
                uid: 1000,
                pid: 42,
                service_generation: 7,
            },
        )
        .unwrap();
        assert!(channel.authenticate(&identity()).is_ok());
        let mut changed = identity();
        changed.service_generation = 8;
        assert_eq!(
            channel.authenticate(&changed),
            Err(ClientError::IdentityChanged)
        );
    }

    #[test]
    fn reconnect_preserves_cursor_without_replaying_mutations() {
        let channel = AdoptedChannel::new(
            3,
            PeerCredentials {
                uid: 1000,
                pid: 42,
                service_generation: 7,
            },
        )
        .unwrap();
        let mut state = ConnectionState::default()
            .establish(
                channel.clone(),
                negotiated(V1_3),
                &supported_versions(),
                ControlLimits::default(),
            )
            .unwrap();
        state.advance_cursor(2).unwrap();
        let disconnected = state.disconnect();
        assert_eq!(disconnected.cursor().revision(), Some(2));
        let reconnected = disconnected
            .establish(
                channel,
                negotiated(V1_3),
                &supported_versions(),
                ControlLimits::default(),
            )
            .unwrap();
        assert_eq!(reconnected.cursor().revision(), Some(2));
    }

    #[test]
    fn cursor_rejects_stale_future_and_non_monotonic_revisions() {
        let n = negotiated(V1_3);
        let mut c = ReconnectCursor::default();
        c.advance(1, &n).unwrap();
        assert_eq!(
            c.resume_after(&Negotiated {
                oldest_revision: 3,
                ..n.clone()
            }),
            Err(ClientError::StaleCursor)
        );
        c.advance(2, &n).unwrap();
        assert_eq!(c.advance(2, &n), Err(ClientError::NonMonotonicCursor));
        assert_eq!(
            ReconnectCursor { revision: Some(5) }.resume_after(&n),
            Err(ClientError::FutureCursor)
        );
    }

    #[test]
    fn framing_rejects_truncation_oversize_and_unknown_json_fields() {
        let frame = encode_frame(&serde_json::json!({"ok":true}), 100).unwrap();
        let decoded: Value = decode_frame(&frame, 100).unwrap();
        assert_eq!(decoded["ok"], true);
        assert_eq!(
            decode_frame::<Value>(&frame[..3], 100),
            Err(ClientError::TruncatedFrame)
        );
        assert_eq!(
            decode_frame::<Value>(&frame, 3),
            Err(ClientError::FrameTooLarge)
        );
        let bad = encode_frame(&serde_json::json!({"unknown":true}), 100).unwrap();
        assert_eq!(
            decode_frame::<NegotiateRequest>(&bad, 100),
            Err(ClientError::MalformedFrame)
        );
    }

    #[test]
    fn issue_decoding_is_version_closed_and_unknown_ids_are_not_reflected() {
        let generic = ValidationIssue {
            category: ValidationCategory::InvalidFormat,
            reason: None,
            measurement_id: None,
        };
        assert!(validate_issue(V1_2, &generic).is_ok());
        let precise = ValidationIssue {
            category: ValidationCategory::MeasurementIssue,
            reason: Some(MeasurementReason::Unsupported),
            measurement_id: Some("measure.cpu".into()),
        };
        assert!(validate_issue(V1_3, &precise).is_ok());
        assert_eq!(
            validate_issue(V1_2, &precise),
            Err(ClientError::InvalidIssue)
        );
        let unknown = ValidationIssue {
            category: ValidationCategory::MeasurementIssue,
            reason: Some(MeasurementReason::UnknownId),
            measurement_id: Some("caller-id".into()),
        };
        assert_eq!(
            validate_issue(V1_3, &unknown),
            Err(ClientError::InvalidIssue)
        );
    }
}
