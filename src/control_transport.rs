// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded framed transport for an already-adopted ASB control channel.
//!
//! This is deliberately a transport seam, not a broker or runner.  The caller
//! supplies a connected `UnixStream` and independently observed peer identity;
//! this module only binds that identity, applies deadlines and validates the
//! typed JSON-RPC boundary.

use crate::control_codec::{
    self, AuthEnrollParams, AuthRevokeParams, AuthRotateParams, AuthStatusParams, CodecError,
    ConfigurationApplyParams, ConfigurationSelection, ControlCall, ControlLimits, ControlRequest,
    ControlResponse, ControlSuccess, NegotiateParams, PageParams, RequestId, Revision,
};
use crate::{
    broker_adoption::{AdoptionError, BrokerGeneration, ReceivedChannel},
    control_client::{AdoptedChannel, PeerCredentials, RunnerIdentity},
    launch_statistics::{LaunchRequest, LaunchState},
    live_projection::ControlProjection,
    protocol_compatibility,
};
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Eq, PartialEq)]
pub enum TransportError {
    Io,
    Codec(CodecError),
    PeerIdentity,
    NotNegotiated,
    BrokerPacket(AdoptionError),
    Continuity,
    Projection,
    RemoteFailure,
    /// A bounded runner error code preserved for recovery decisions without
    /// exposing the remote message or any credential material.
    RemoteFailureCode(i32),
}

impl TransportError {
    #[must_use]
    pub const fn is_reconciliation_required(&self) -> bool {
        matches!(self, Self::RemoteFailureCode(-33008))
    }
}
impl From<CodecError> for TransportError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}
impl From<AdoptionError> for TransportError {
    fn from(value: AdoptionError) -> Self {
        Self::BrokerPacket(value)
    }
}

pub struct FramedControlStream {
    stream: UnixStream,
    limits: ControlLimits,
    expected_peer: RunnerIdentity,
    negotiated: bool,
    negotiated_version: Option<control_codec::ControlVersion>,
}

impl FramedControlStream {
    pub fn adopt(
        stream: UnixStream,
        observed: PeerCredentials,
        expected_peer: RunnerIdentity,
        limits: ControlLimits,
    ) -> Result<Self, TransportError> {
        let limits = limits.validate()?;
        let descriptor = stream.as_raw_fd();
        if descriptor < 3 {
            return Err(TransportError::PeerIdentity);
        }
        AdoptedChannel::new(descriptor as u32, observed)
            .map_err(|_| TransportError::PeerIdentity)?
            .authenticate(&expected_peer)
            .map_err(|_| TransportError::PeerIdentity)?;
        Ok(Self {
            stream,
            limits,
            expected_peer,
            negotiated: false,
            negotiated_version: None,
        })
    }

    /// Adopt a broker-transferred stream after validating only its kernel
    /// peer uid/pid. Unlike the legacy `adopt` API, this cannot accept or
    /// infer a service generation; that value is authenticated by negotiate.
    pub fn adopt_broker(
        stream: UnixStream,
        peer_uid: u32,
        peer_pid: u32,
        limits: ControlLimits,
    ) -> Result<Self, TransportError> {
        let limits = limits.validate()?;
        if stream.as_raw_fd() < 3 || peer_pid == 0 {
            return Err(TransportError::PeerIdentity);
        }
        let credentials = rustix::net::sockopt::socket_peercred(&stream)
            .map_err(|_| TransportError::PeerIdentity)?;
        if credentials.uid.as_raw() != peer_uid || credentials.pid.as_raw_pid() as u32 != peer_pid {
            return Err(TransportError::PeerIdentity);
        }
        Ok(Self {
            stream,
            limits,
            expected_peer: RunnerIdentity {
                uid: peer_uid,
                pid: peer_pid,
                // Deliberately an unusable sentinel: broker sessions do not
                // authenticate generation through this legacy path.
                service_generation: 0,
                runner_instance_id: String::new(),
            },
            negotiated: false,
            negotiated_version: None,
        })
    }

    pub fn negotiate(&mut self, id: RequestId) -> Result<ControlSuccess, TransportError> {
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id,
            timeout_ms: self.limits.max_timeout_ms,
            call: control_codec::ControlCall::Negotiate(NegotiateParams {
                versions: protocol_compatibility::supported_versions(),
                limits: self.limits,
            }),
        };
        self.write_request(&request)?;
        let response = self.read_response(&request)?;
        let ControlResponse::Success(value) = response else {
            return Err(TransportError::NotNegotiated);
        };
        let ControlSuccess::Negotiated(session) = value.result else {
            return Err(TransportError::NotNegotiated);
        };
        if (!self.expected_peer.runner_instance_id.is_empty()
            && session.runner_instance_id != self.expected_peer.runner_instance_id)
            || protocol_compatibility::validate_version(session.version).is_err()
        {
            return Err(TransportError::NotNegotiated);
        }
        if session.version < control_codec::V1_0 {
            return Err(TransportError::NotNegotiated);
        }
        self.negotiated_version = Some(session.version);
        self.negotiated = true;
        Ok(ControlSuccess::Negotiated(session))
    }

    pub fn write_request(&mut self, request: &ControlRequest) -> Result<(), TransportError> {
        request.validate(self.limits)?;
        if let Some(version) = self.negotiated_version {
            protocol_compatibility::validate_operation(version, &request.call)
                .map_err(|_| TransportError::NotNegotiated)?;
        }
        self.set_write_deadline(request.timeout_ms)?;
        let frame = control_codec::encode(request, self.limits.max_frame_bytes as usize)?;
        self.stream
            .write_all(&frame)
            .map_err(|_| TransportError::Io)
    }

    pub fn read_response(
        &mut self,
        request: &ControlRequest,
    ) -> Result<ControlResponse, TransportError> {
        if !self.negotiated && !matches!(request.call, control_codec::ControlCall::Negotiate(_)) {
            return Err(TransportError::NotNegotiated);
        }
        self.set_read_deadline(request.timeout_ms)?;
        let frame = self.read_frame()?;
        let response: ControlResponse =
            control_codec::decode(&frame, self.limits.max_frame_bytes as usize)?;
        response.validate_for(request, self.limits)?;
        if let Some(version) = self.negotiated_version
            && minimum_version_for_call(&request.call).is_some_and(|minimum| version < minimum)
        {
            return Err(TransportError::NotNegotiated);
        }
        Ok(response)
    }

    /// Execute one bounded typed request and validate its response against the
    /// exact request envelope before returning it to a projection layer.
    pub fn round_trip(
        &mut self,
        request: &ControlRequest,
    ) -> Result<ControlResponse, TransportError> {
        self.write_request(request)?;
        self.read_response(request)
    }

    fn set_read_deadline(&self, timeout_ms: u64) -> Result<(), TransportError> {
        self.stream
            .set_read_timeout(Some(Duration::from_millis(
                timeout_ms.min(self.limits.max_timeout_ms),
            )))
            .map_err(|_| TransportError::Io)
    }
    fn set_write_deadline(&self, timeout_ms: u64) -> Result<(), TransportError> {
        self.stream
            .set_write_timeout(Some(Duration::from_millis(
                timeout_ms.min(self.limits.max_timeout_ms),
            )))
            .map_err(|_| TransportError::Io)
    }
    fn read_frame(&mut self) -> Result<Vec<u8>, TransportError> {
        let mut header = [0_u8; 4];
        self.stream
            .read_exact(&mut header)
            .map_err(|_| TransportError::Io)?;
        let size = u32::from_be_bytes(header) as usize;
        if size == 0
            || size > self.limits.max_frame_bytes as usize
            || size > control_codec::MAX_FRAME_BYTES
        {
            return Err(TransportError::Codec(CodecError::FrameTooLarge));
        }
        let mut body = vec![0_u8; size];
        self.stream
            .read_exact(&mut body)
            .map_err(|_| TransportError::Io)?;
        let mut frame = header.to_vec();
        frame.extend(body);
        Ok(frame)
    }
}

fn minimum_version_for_call(
    call: &control_codec::ControlCall,
) -> Option<control_codec::ControlVersion> {
    call.minimum_version()
}

/// Reject secret-shaped fields before an arbitrary provider profile reaches
/// the wire. The profile remains JSON because ASB owns its full schema, but
/// this frontend boundary must never accept raw credential names or helper
/// paths. Credential provenance is allowed when it contains only digests.
fn validate_credential_free_profile(value: &serde_json::Value) -> Result<(), TransportError> {
    const MAX_DEPTH: usize = 16;
    const MAX_NODES: usize = 4096;
    const FORBIDDEN: &[&str] = &[
        "apikey",
        "accesstoken",
        "password",
        "privatekey",
        "secret",
        "helperpath",
        "executablepath",
        "token",
    ];

    fn walk(
        value: &serde_json::Value,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<(), TransportError> {
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(TransportError::Codec(CodecError::InvalidValue(
                "credential-free profile bounds",
            )));
        }
        match value {
            serde_json::Value::Object(fields) => {
                for (key, child) in fields {
                    let normalized: String = key
                        .chars()
                        .filter(|character| *character != '_' && *character != '-')
                        .flat_map(char::to_lowercase)
                        .collect();
                    if FORBIDDEN.iter().any(|name| normalized == *name) {
                        return Err(TransportError::Codec(CodecError::InvalidValue(
                            "credential-free profile",
                        )));
                    }
                    walk(child, depth + 1, nodes)?;
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    walk(child, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    walk(value, 0, &mut 0)
}

#[derive(Clone, Copy)]
enum RecordingMutation {
    Execute,
    Cancel,
    Reconcile,
    OfflineDefault,
}

/// Continuity evidence carried by a broker handoff. Epoch and sequence are
/// retained as opaque broker evidence; they are never converted to a service
/// generation or used as an identity substitute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerContinuity {
    generation: BrokerGeneration,
}

/// Kernel credentials retained from the adopted control stream.  The process
/// id is an observation, not a locator or a value taken from user input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerPeerCredentials {
    uid: u32,
    pid: u32,
}

impl BrokerPeerCredentials {
    #[must_use]
    pub const fn uid(self) -> u32 {
        self.uid
    }

    #[must_use]
    pub const fn pid(self) -> u32 {
        self.pid
    }
}

impl BrokerContinuity {
    fn new(generation: BrokerGeneration) -> Result<Self, TransportError> {
        if generation.epoch == [0; 16] || generation.sequence == 0 {
            return Err(TransportError::Continuity);
        }
        Ok(Self { generation })
    }

    #[must_use]
    pub fn generation(self) -> BrokerGeneration {
        self.generation
    }

    /// Accept only the next packet in the same broker epoch. Wraparound is
    /// rejected rather than treated as continuity.
    pub fn accept_successor(&mut self, next: BrokerGeneration) -> Result<(), TransportError> {
        if next.epoch != self.generation.epoch
            || next.sequence
                != self
                    .generation
                    .sequence
                    .checked_add(1)
                    .ok_or(TransportError::Continuity)?
        {
            return Err(TransportError::Continuity);
        }
        self.generation = next;
        Ok(())
    }
}

/// Authenticated broker control session. Construction consumes the received
/// descriptor and exposes it only after packet, peer, typed negotiation, and
/// runner-identity checks all succeed.
pub struct AuthenticatedBrokerSession {
    transport: FramedControlStream,
    negotiated: crate::control_codec::Negotiated,
    continuity: BrokerContinuity,
    peer: BrokerPeerCredentials,
}

impl AuthenticatedBrokerSession {
    /// Connect to an ASB owner-private control socket and negotiate before
    /// entering the interactive frontend. The socket is a local transport
    /// boundary; no path or peer metadata is accepted as authority.
    pub fn connect(path: &Path, limits: ControlLimits) -> Result<Self, TransportError> {
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err(TransportError::PeerIdentity);
        }
        let metadata = std::fs::symlink_metadata(path).map_err(|_| TransportError::PeerIdentity)?;
        let parent = path.parent().ok_or(TransportError::PeerIdentity)?;
        let parent_metadata =
            std::fs::symlink_metadata(parent).map_err(|_| TransportError::PeerIdentity)?;
        if !parent_metadata.is_dir()
            || parent_metadata.file_type().is_symlink()
            || parent_metadata.uid() != rustix::process::geteuid().as_raw()
            || parent_metadata.mode() & 0o077 != 0
        {
            return Err(TransportError::PeerIdentity);
        }
        if !metadata.file_type().is_socket()
            || metadata.file_type().is_symlink()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(TransportError::PeerIdentity);
        }
        let stream = UnixStream::connect(path).map_err(|_| TransportError::Io)?;
        let credentials = rustix::net::sockopt::socket_peercred(&stream)
            .map_err(|_| TransportError::PeerIdentity)?;
        let peer_uid = credentials.uid.as_raw();
        let peer_pid = credentials.pid.as_raw_pid() as u32;
        if peer_uid != rustix::process::geteuid().as_raw() || peer_pid == 0 {
            return Err(TransportError::PeerIdentity);
        }
        let mut transport = FramedControlStream::adopt_broker(stream, peer_uid, peer_pid, limits)?;
        let ControlSuccess::Negotiated(negotiated) = transport.negotiate(RequestId(1))? else {
            return Err(TransportError::NotNegotiated);
        };
        Ok(Self {
            transport,
            negotiated,
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [1; 16],
                sequence: 1,
            })?,
            peer: BrokerPeerCredentials {
                uid: peer_uid,
                pid: peer_pid,
            },
        })
    }

    pub fn establish(
        received: ReceivedChannel,
        expected_uid: u32,
        expected_pid: u32,
        limits: ControlLimits,
    ) -> Result<Self, TransportError> {
        received.authenticate_peer_ids(expected_uid, expected_pid)?;
        let packet = received.broker_packet()?;
        let continuity = BrokerContinuity::new(packet.generation)?;
        let peer_uid = received.peer_uid();
        let peer_pid = received.peer_pid();
        let stream = received.into_unix_stream();
        let mut transport = FramedControlStream::adopt_broker(stream, peer_uid, peer_pid, limits)?;
        let id = RequestId(1);
        let ControlSuccess::Negotiated(negotiated) = transport.negotiate(id)? else {
            return Err(TransportError::NotNegotiated);
        };
        packet.verify_runner_identity(&negotiated.runner_instance_id)?;
        Ok(Self {
            transport,
            negotiated,
            continuity,
            peer: BrokerPeerCredentials {
                uid: expected_uid,
                pid: expected_pid,
            },
        })
    }

    /// Establish a broker session using only local policy and kernel evidence.
    /// AR-1060 does not transmit a caller-asserted remote uid/pid: the peer
    /// must be the current effective user, and its nonzero pid observed at
    /// descriptor receipt must match a fresh `SO_PEERCRED` read before I/O.
    /// No argv, environment variable, filesystem path, or packet field is
    /// consulted for peer authority.
    pub fn establish_from_broker(
        received: ReceivedChannel,
        limits: ControlLimits,
    ) -> Result<Self, TransportError> {
        let expected_uid = rustix::process::geteuid().as_raw();
        let expected_pid = received.peer_pid();
        if expected_pid == 0 {
            return Err(TransportError::PeerIdentity);
        }
        Self::establish(received, expected_uid, expected_pid, limits)
    }

    #[must_use]
    pub fn negotiated(&self) -> &crate::control_codec::Negotiated {
        &self.negotiated
    }

    #[must_use]
    pub fn continuity(&self) -> BrokerContinuity {
        self.continuity
    }

    #[must_use]
    pub fn peer_credentials(&self) -> BrokerPeerCredentials {
        self.peer
    }

    pub fn transport_mut(&mut self) -> &mut FramedControlStream {
        &mut self.transport
    }

    /// Apply one wizard selection through the authenticated runner. The
    /// expected generation is read from the last authoritative projection,
    /// making stale concurrent edits fail closed at the backend boundary.
    pub fn apply_configuration(
        &mut self,
        projection: &mut ControlProjection,
        selection: ConfigurationSelection,
    ) -> Result<(), TransportError> {
        if self.negotiated.version < control_codec::V1_7 {
            return Err(TransportError::NotNegotiated);
        }
        let expected_generation = projection
            .snapshot()
            .configuration
            .as_ref()
            .map_or(Revision(1), |snapshot| snapshot.generation);
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(9_000_000_001),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call: ControlCall::ConfigurationApply(ConfigurationApplyParams {
                idempotency_key: format!(
                    "asb-tui-wizard-{}-{}",
                    self.negotiated.runner_instance_id, expected_generation.0
                ),
                expected_generation,
                selection,
            }),
        };
        let response = self.transport.round_trip(&request)?;
        if let ControlResponse::Failure(failure) = &response {
            return Err(TransportError::RemoteFailureCode(failure.error.code));
        }
        if !matches!(response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(&request, &response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)
    }

    /// Create and launch the exact reviewed materialization. Binding checks
    /// happen before the first write, so stale catalog/configuration state
    /// cannot reach the runner. The returned projection contains the runner's
    /// authoritative initial run summary.
    pub fn launch_materialized(
        &mut self,
        projection: &mut ControlProjection,
        bundle: &crate::configuration_materialization::MaterializedBundle,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.require_version(control_codec::V1_0)?;
        bundle
            .validate_integrity()
            .map_err(|_| TransportError::Projection)?;
        let snapshot = projection.snapshot();
        let request_binding = LaunchRequest::new(bundle, &snapshot, idempotency_key)
            .map_err(|_| TransportError::Projection)?;
        let definition: serde_json::Value =
            serde_json::from_str(&bundle.canonical_json).map_err(|_| TransportError::Projection)?;
        let plan_request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(9_000_000_201),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call: ControlCall::CreatePlan(control_codec::MutationParams {
                idempotency_key: request_binding.idempotency_key.clone(),
                definition,
            }),
        };
        let plan_response = self.transport.round_trip(&plan_request)?;
        let ControlResponse::Success(success) = &plan_response else {
            return Err(TransportError::RemoteFailure);
        };
        let ControlSuccess::Operation(result) = &success.result else {
            return Err(TransportError::Projection);
        };
        let control_codec::ControlResult::Plan(plan) = &result.result else {
            return Err(TransportError::Projection);
        };
        let launch_request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(9_000_000_202),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call: ControlCall::Launch(control_codec::LaunchParams {
                idempotency_key: request_binding.idempotency_key,
                plan_id: plan.plan_id.clone(),
            }),
        };
        let launch_response = self.transport.round_trip(&launch_request)?;
        if !matches!(launch_response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(&launch_request, &launch_response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)
    }

    /// Cancel the currently projected active run using its exact run and
    /// attempt identifiers. No cancellation is issued when history is empty
    /// or already terminal.
    pub fn cancel_active_run(
        &mut self,
        projection: &mut ControlProjection,
        launch: &LaunchState,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.require_version(control_codec::V1_0)?;
        let (active_run_id, active_attempt_id) = launch
            .active_run_attempt()
            .ok_or(TransportError::Projection)?;
        let run = projection
            .snapshot()
            .runs
            .into_iter()
            .find(|run| &run.run_id == active_run_id && &run.attempt_id == active_attempt_id)
            .ok_or(TransportError::Projection)?;
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(9_000_000_203),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call: ControlCall::Cancel(control_codec::CancelParams {
                run_id: run.run_id,
                attempt_id: run.attempt_id,
                idempotency_key,
            }),
        };
        let response = self.transport.round_trip(&request)?;
        if !matches!(response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(&request, &response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)?;
        self.poll_projection(projection)
    }

    /// Refresh the runner-owned provider/model catalog. The known generation
    /// is sent as a cache fence; projection rejects an older response.
    pub fn refresh_provider_catalog(
        &mut self,
        projection: &mut ControlProjection,
    ) -> Result<(), TransportError> {
        if self.negotiated.version < control_codec::V1_7 {
            return Err(TransportError::NotNegotiated);
        }
        let known_generation = projection
            .snapshot()
            .provider_catalog
            .as_ref()
            .map(|catalog| catalog.generation);
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(9_000_000_002),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call: ControlCall::ProviderCatalog(control_codec::ProviderCatalogRequest {
                action: control_codec::ProviderCatalogAction::Refresh,
                runner_instance_id: self.negotiated.runner_instance_id.clone(),
                known_generation,
            }),
        };
        let response = self.transport.round_trip(&request)?;
        if let ControlResponse::Failure(failure) = &response {
            return Err(TransportError::RemoteFailureCode(failure.error.code));
        }
        if !matches!(response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(&request, &response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)
    }

    /// Enroll a provider through the runner-owned credential resolver. The
    /// frontend accepts only endpoint and locator digests; a raw API key has
    /// no representable type and therefore cannot cross this boundary.
    pub fn enroll_auth(
        &mut self,
        projection: &mut ControlProjection,
        provider: String,
        endpoint_identity_sha256: String,
        credential_locator_sha256: String,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.auth_mutation(
            projection,
            ControlCall::AuthEnroll(AuthEnrollParams {
                provider,
                endpoint_identity_sha256,
                credential_locator_sha256,
                idempotency_key,
            }),
            9_000_000_101,
        )
    }

    /// Submit a digest-only receipt returned by the approved local helper.
    /// The helper output is parsed before this method is called, so the raw
    /// credential never enters TUI state or the control frame.
    pub fn enroll_auth_receipt(
        &mut self,
        projection: &mut ControlProjection,
        receipt: crate::credential_helper::CredentialEnrollmentReceipt,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.enroll_auth(
            projection,
            receipt.provider,
            receipt.endpoint_identity_sha256,
            receipt.credential_locator_sha256,
            idempotency_key,
        )
    }

    /// Rotate the provider's resolver reference without receiving or sending
    /// the underlying credential value.
    pub fn rotate_auth(
        &mut self,
        projection: &mut ControlProjection,
        provider: String,
        credential_locator_sha256: String,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.auth_mutation(
            projection,
            ControlCall::AuthRotate(AuthRotateParams {
                provider,
                credential_locator_sha256,
                idempotency_key,
            }),
            9_000_000_102,
        )
    }

    /// Revoke enrollment. This only changes runner-owned enrollment state and
    /// does not claim that a provider request was authorized.
    pub fn revoke_auth(
        &mut self,
        projection: &mut ControlProjection,
        provider: String,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.auth_mutation(
            projection,
            ControlCall::AuthRevoke(AuthRevokeParams {
                provider,
                idempotency_key,
            }),
            9_000_000_103,
        )
    }

    /// Read public enrollment state for a provider. A returned status is
    /// intentionally not interpreted as endpoint reachability or authorization.
    pub fn auth_status(
        &mut self,
        projection: &mut ControlProjection,
        provider: String,
    ) -> Result<(), TransportError> {
        if self.negotiated.version < control_codec::CONTROL_AUTH_V1 {
            return Err(TransportError::NotNegotiated);
        }
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(9_000_000_104),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call: ControlCall::AuthStatus(AuthStatusParams { provider }),
        };
        let response = self.transport.round_trip(&request)?;
        if let ControlResponse::Failure(failure) = &response {
            return Err(TransportError::RemoteFailureCode(failure.error.code));
        }
        if !matches!(response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(&request, &response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)
    }

    /// Ask the runner to resolve a credential helper for a complete,
    /// credential-free provider profile. No helper path or secret is accepted.
    pub fn invoke_auth_helper(
        &mut self,
        projection: &mut ControlProjection,
        provider: String,
        profile: serde_json::Value,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        if self.negotiated.version < control_codec::V1_10 {
            return Err(TransportError::NotNegotiated);
        }
        validate_credential_free_profile(&profile)?;
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(9_000_000_105),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call: ControlCall::AuthHelperInvoke(control_codec::AuthHelperInvokeParams {
                provider,
                profile,
                idempotency_key,
            }),
        };
        let response = self.transport.round_trip(&request)?;
        if let ControlResponse::Failure(failure) = &response {
            return Err(TransportError::RemoteFailureCode(failure.error.code));
        }
        if !matches!(response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(&request, &response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)
    }

    fn auth_mutation(
        &mut self,
        projection: &mut ControlProjection,
        call: ControlCall,
        id: u64,
    ) -> Result<(), TransportError> {
        if self.negotiated.version < control_codec::CONTROL_AUTH_V1 {
            return Err(TransportError::NotNegotiated);
        }
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id: RequestId(id),
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call,
        };
        let response = self.transport.round_trip(&request)?;
        if let ControlResponse::Failure(failure) = &response {
            return Err(TransportError::RemoteFailureCode(failure.error.code));
        }
        if !matches!(response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(&request, &response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)
    }

    /// Ask the authenticated runner for a bounded estimate of a recording
    /// matrix.  This is read-only: no campaign is created and no provider is
    /// contacted by the frontend.  The runner remains authoritative for
    /// availability and completeness.
    pub fn estimate_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        provider_id: String,
        model_id: String,
        agent_ids: Vec<String>,
        workload_ids: Vec<String>,
    ) -> Result<(), TransportError> {
        self.require_version(control_codec::V1_7)?;
        let request = self.recording_request(
            RequestId(9_000_000_002),
            ControlCall::RecordingCampaignEstimate(
                control_codec::RecordingCampaignEstimateRequest {
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    provider_id,
                    model_id,
                    agent_ids,
                    workload_ids,
                },
            ),
        );
        self.apply_recording_response(projection, &request)
    }

    /// Create a recording campaign plan from the exact selected matrix. The
    /// runner validates provider connectivity and workload support; the TUI
    /// only transports identifiers and never handles credentials.
    pub fn plan_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        provider_id: String,
        model_id: String,
        agent_ids: Vec<String>,
        workload_ids: Vec<String>,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.require_version(control_codec::V1_7)?;
        let expected_generation = projection
            .snapshot()
            .recording_campaign
            .as_ref()
            .map_or(crate::control_codec::Revision(1), |plan| plan.generation);
        let request = self.recording_request(
            RequestId(9_000_000_008),
            ControlCall::RecordingCampaignPlan(control_codec::RecordingCampaignPlanParams {
                idempotency_key,
                expected_generation,
                runner_instance_id: self.negotiated.runner_instance_id.clone(),
                provider_id,
                model_id,
                agent_ids,
                workload_ids,
            }),
        );
        self.apply_recording_response(projection, &request)
    }

    /// Execute a previously projected campaign.  `idempotency_key` must be
    /// stable for retries of this logical operation; it is sent unchanged to
    /// ASB and is never generated from UI state or credentials.
    pub fn execute_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign_id: String,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.recording_mutation(
            projection,
            campaign_id,
            idempotency_key,
            RecordingMutation::Execute,
            RequestId(9_000_000_003),
        )
    }

    /// Read the authoritative progress of a campaign.  A progress response
    /// for another campaign is rejected before it can alter the projection.
    pub fn progress_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign_id: String,
    ) -> Result<(), TransportError> {
        self.require_version(control_codec::V1_8)?;
        let request = self.recording_request(
            RequestId(9_000_000_004),
            ControlCall::RecordingCampaignProgress(
                control_codec::RecordingCampaignProgressRequest {
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    campaign_id,
                },
            ),
        );
        self.apply_recording_response(projection, &request)
    }

    pub fn cancel_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign_id: String,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.recording_mutation(
            projection,
            campaign_id,
            idempotency_key,
            RecordingMutation::Cancel,
            RequestId(9_000_000_005),
        )
    }

    pub fn reconcile_recording_campaign(
        &mut self,
        projection: &mut ControlProjection,
        campaign_id: String,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.recording_mutation(
            projection,
            campaign_id,
            idempotency_key,
            RecordingMutation::Reconcile,
            RequestId(9_000_000_006),
        )
    }

    pub fn set_recording_campaign_offline_default(
        &mut self,
        projection: &mut ControlProjection,
        campaign_id: String,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        self.recording_mutation(
            projection,
            campaign_id,
            idempotency_key,
            RecordingMutation::OfflineDefault,
            RequestId(9_000_000_007),
        )
    }

    fn require_version(
        &self,
        minimum: control_codec::ControlVersion,
    ) -> Result<(), TransportError> {
        if self.negotiated.version < minimum {
            return Err(TransportError::NotNegotiated);
        }
        Ok(())
    }

    fn recording_request(&self, id: RequestId, call: ControlCall) -> ControlRequest {
        ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id,
            timeout_ms: self.negotiated.limits.max_timeout_ms,
            call,
        }
    }

    fn apply_recording_response(
        &mut self,
        projection: &mut ControlProjection,
        request: &ControlRequest,
    ) -> Result<(), TransportError> {
        let response = self.transport.round_trip(request)?;
        if !matches!(response, ControlResponse::Success(_)) {
            return Err(TransportError::RemoteFailure);
        }
        projection
            .apply(request, &response, self.negotiated.limits)
            .map_err(|_| TransportError::Projection)
    }

    fn recording_mutation(
        &mut self,
        projection: &mut ControlProjection,
        campaign_id: String,
        idempotency_key: String,
        mutation: RecordingMutation,
        request_id: RequestId,
    ) -> Result<(), TransportError> {
        let expected_generation = projection
            .snapshot()
            .recording_campaign_lifecycle
            .as_ref()
            .filter(|campaign| campaign.campaign_id == campaign_id)
            .map(|campaign| campaign.generation)
            .or_else(|| {
                projection
                    .snapshot()
                    .recording_campaign
                    .as_ref()
                    .filter(|campaign| campaign.campaign_id == campaign_id)
                    .map(|campaign| campaign.generation)
            })
            .ok_or(TransportError::Projection)?;
        self.require_version(control_codec::V1_8)?;
        let call = match mutation {
            RecordingMutation::Execute => ControlCall::RecordingCampaignExecute(
                control_codec::RecordingCampaignExecuteParams {
                    idempotency_key,
                    expected_generation,
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    campaign_id,
                },
            ),
            RecordingMutation::Cancel => {
                ControlCall::RecordingCampaignCancel(control_codec::RecordingCampaignCancelParams {
                    idempotency_key,
                    expected_generation,
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    campaign_id,
                })
            }
            RecordingMutation::Reconcile => ControlCall::RecordingCampaignReconcile(
                control_codec::RecordingCampaignReconcileParams {
                    idempotency_key,
                    expected_generation,
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    campaign_id,
                },
            ),
            RecordingMutation::OfflineDefault => ControlCall::RecordingCampaignOfflineDefault(
                control_codec::RecordingCampaignOfflineDefaultParams {
                    idempotency_key,
                    expected_generation,
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    campaign_id,
                },
            ),
        };
        let request = self.recording_request(request_id, call);
        self.apply_recording_response(projection, &request)
    }

    /// Poll the bounded read-only bootstrap state used by the workspace.
    /// Projection happens on a clone and is committed only after all three
    /// responses validate, so a malformed, stale, or failed response cannot
    /// leave a partially refreshed UI snapshot.
    pub fn poll_projection(
        &mut self,
        projection: &mut ControlProjection,
    ) -> Result<(), TransportError> {
        let mut next = projection.clone();
        next.accept_negotiated(self.negotiated.clone())
            .map_err(|_| TransportError::Projection)?;
        let limits = self.negotiated.limits;
        let mut calls = vec![ControlCall::Capabilities];
        if self.negotiated.version >= control_codec::CONTROL_BENCHMARK_CATALOG_V1 {
            calls.push(ControlCall::BenchmarkCatalog);
        }
        if self.negotiated.version >= control_codec::CONTROL_MEASUREMENT_CATALOG_V1 {
            calls.push(ControlCall::MeasurementCatalog);
        }
        calls.push(ControlCall::History(PageParams {
            after: None::<Revision>,
            limit: limits.max_page_items,
        }));
        if self.negotiated.version >= control_codec::CONTROL_AGENT_CATALOG_V1 {
            calls.push(ControlCall::AgentCatalog(
                crate::agent_catalog::AgentCatalogRequest {
                    action: crate::agent_catalog::AgentCatalogAction::Status,
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    known_generation: None,
                },
            ));
        }
        let bootstrap_count = calls.len();
        for (offset, call) in calls.into_iter().enumerate() {
            let id = RequestId(u64::try_from(offset + 2).map_err(|_| TransportError::Io)?);
            let request = ControlRequest {
                jsonrpc: control_codec::JSONRPC_VERSION.into(),
                id,
                timeout_ms: limits.max_timeout_ms,
                call,
            };
            let response = self.transport.round_trip(&request)?;
            if !matches!(response, ControlResponse::Success(_)) {
                return Err(TransportError::RemoteFailure);
            }
            next.apply(&request, &response, limits)
                .map_err(|_| TransportError::Projection)?;
        }
        if self.negotiated.version >= control_codec::CONTROL_AGENT_LIFECYCLE_V1
            && let Some(catalog) = next.snapshot().agent_catalog
        {
            let base_id = 2_u64
                .checked_add(u64::try_from(bootstrap_count).map_err(|_| TransportError::Io)?)
                .ok_or(TransportError::Io)?;
            for (offset, entry) in catalog.agents.iter().enumerate() {
                let request = ControlRequest {
                    jsonrpc: control_codec::JSONRPC_VERSION.into(),
                    id: RequestId(
                        base_id
                            .checked_add(u64::try_from(offset).map_err(|_| TransportError::Io)?)
                            .ok_or(TransportError::Io)?,
                    ),
                    timeout_ms: limits.max_timeout_ms,
                    call: ControlCall::AgentStatus(crate::asb_lifecycle::AgentStatusRequest {
                        binding: crate::asb_lifecycle::AgentLifecycleBinding {
                            agent_id: entry.agent_id.clone(),
                            runner_instance_id: catalog.runner_instance_id.clone(),
                            catalog_sha256: catalog.catalog_sha256.clone(),
                        },
                        operation_id: None,
                    }),
                };
                let response = self.transport.round_trip(&request)?;
                if !matches!(response, ControlResponse::Success(_)) {
                    return Err(TransportError::RemoteFailure);
                }
                next.apply(&request, &response, limits)
                    .map_err(|_| TransportError::Projection)?;
            }
        }
        if self.negotiated.version >= control_codec::V1_7 {
            let setup_calls = [
                ControlCall::ProviderCatalog(control_codec::ProviderCatalogRequest {
                    action: control_codec::ProviderCatalogAction::Status,
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    known_generation: None,
                }),
                ControlCall::ConfigurationStatus(control_codec::ConfigurationStatusRequest {
                    runner_instance_id: self.negotiated.runner_instance_id.clone(),
                }),
                ControlCall::RecordingCampaignStatus(
                    control_codec::RecordingCampaignStatusRequest {
                        runner_instance_id: self.negotiated.runner_instance_id.clone(),
                    },
                ),
            ];
            let base_id = 2_u64
                .checked_add(u64::try_from(bootstrap_count).map_err(|_| TransportError::Io)?)
                .and_then(|value| {
                    next.snapshot()
                        .agent_catalog
                        .as_ref()
                        .and_then(|catalog| u64::try_from(catalog.agents.len()).ok())
                        .and_then(|count| value.checked_add(count))
                })
                .ok_or(TransportError::Io)?;
            for (offset, call) in setup_calls.into_iter().enumerate() {
                let request = ControlRequest {
                    jsonrpc: control_codec::JSONRPC_VERSION.into(),
                    id: RequestId(
                        base_id
                            .checked_add(u64::try_from(offset).map_err(|_| TransportError::Io)?)
                            .ok_or(TransportError::Io)?,
                    ),
                    timeout_ms: limits.max_timeout_ms,
                    call,
                };
                let response = self.transport.round_trip(&request)?;
                if !matches!(response, ControlResponse::Success(_)) {
                    return Err(TransportError::RemoteFailure);
                }
                next.apply(&request, &response, limits)
                    .map_err(|_| TransportError::Projection)?;
            }
        }
        // Authentication status is a separate, credential-free read. Only
        // ask for it after configuration has named a provider; an absent
        // status is deliberately represented as unavailable in the UI.
        if self.negotiated.version >= control_codec::CONTROL_AUTH_V1
            && let Some(provider) = next
                .snapshot()
                .configuration
                .and_then(|configuration| configuration.provider_id)
        {
            let id = 9_000_000_104;
            let request = ControlRequest {
                jsonrpc: control_codec::JSONRPC_VERSION.into(),
                id: RequestId(id),
                timeout_ms: limits.max_timeout_ms,
                call: ControlCall::AuthStatus(control_codec::AuthStatusParams { provider }),
            };
            let response = self.transport.round_trip(&request)?;
            next.apply(&request, &response, limits)
                .map_err(|_| TransportError::Projection)?;
        }
        *projection = next;
        Ok(())
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TransportError {}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::net::{
        AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags,
        SocketType, sendmsg, socketpair,
    };
    use std::{
        fs,
        io::{IoSlice, Read, Write},
        mem::MaybeUninit,
        os::fd::AsFd,
        os::unix::fs::PermissionsExt,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };
    fn peer() -> RunnerIdentity {
        RunnerIdentity {
            uid: 1000,
            pid: 42,
            service_generation: 7,
            runner_instance_id: "runner-7".into(),
        }
    }

    fn send_handoff<S: AsFd, F: AsFd>(sender: S, offered: F, runner: &str) {
        let mut packet = [0_u8; crate::broker_adoption::BROKER_PACKET_BYTES];
        packet[..8].copy_from_slice(b"ASBHND01");
        packet[8..10].copy_from_slice(&1_u16.to_be_bytes());
        packet[10] = 1;
        packet[11] = 1;
        packet[16..32].fill(9);
        packet[32..40].copy_from_slice(&4_u64.to_be_bytes());
        packet[40..72].copy_from_slice(
            &crate::broker_adoption::BrokerPacket::runner_identity_digest(runner).unwrap(),
        );
        let iov = [IoSlice::new(&packet)];
        let mut bytes = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut bytes);
        let rights = [offered.as_fd()];
        ancillary.push(SendAncillaryMessage::ScmRights(&rights));
        sendmsg(&sender, &iov, &mut ancillary, SendFlags::empty()).unwrap();
    }
    fn observed() -> PeerCredentials {
        PeerCredentials {
            uid: 1000,
            pid: 42,
            service_generation: 7,
        }
    }

    #[test]
    fn socket_consumer_rejects_non_socket_and_public_paths_before_connecting() {
        let root = std::env::temp_dir().join(format!(
            "asb-tui-socket-consumer-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let regular = root.join("regular");
        fs::write(&regular, b"not a socket").unwrap();
        fs::set_permissions(&regular, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            AuthenticatedBrokerSession::connect(&regular, ControlLimits::default()),
            Err(TransportError::PeerIdentity)
        ));
        let public = root.join("public");
        fs::write(&public, b"not a socket").unwrap();
        fs::set_permissions(&public, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            AuthenticatedBrokerSession::connect(&public, ControlLimits::default()),
            Err(TransportError::PeerIdentity)
        ));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn socketpair_round_trip_rejects_mismatched_response_id() {
        let (mut server, client) = UnixStream::pair().unwrap();
        let join = thread::spawn(move || {
            let mut header = [0; 4];
            server.read_exact(&mut header).unwrap();
            let n = u32::from_be_bytes(header) as usize;
            let mut body = vec![0; n];
            server.read_exact(&mut body).unwrap();
            let bad = ControlResponse::Success(crate::control_codec::SuccessResponse {
                jsonrpc: "2.0".into(),
                id: RequestId(9),
                result: ControlSuccess::Negotiated(crate::control_codec::Negotiated {
                    version: crate::control_codec::V1_3,
                    limits: ControlLimits::default(),
                    runner_instance_id: "runner-7".into(),
                    oldest_revision: crate::control_codec::Revision(1),
                    latest_revision: crate::control_codec::Revision(1),
                }),
            });
            server
                .write_all(&control_codec::encode(&bad, 4096).unwrap())
                .unwrap();
        });
        let mut stream =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        stream.negotiated = true;
        let request = ControlRequest {
            jsonrpc: "2.0".into(),
            id: RequestId(1),
            timeout_ms: 1000,
            call: crate::control_codec::ControlCall::Negotiate(
                crate::control_codec::NegotiateParams {
                    versions: [crate::control_codec::V1_3].into_iter().collect(),
                    limits: ControlLimits::default(),
                },
            ),
        };
        stream.write_request(&request).unwrap();
        assert!(matches!(
            stream.read_response(&request),
            Err(TransportError::Codec(CodecError::ResponseIdMismatch))
        ));
        join.join().unwrap();
    }
    #[test]
    fn peer_generation_is_bound_before_any_io() {
        let (_server, client) = UnixStream::pair().unwrap();
        let mut wrong = peer();
        wrong.service_generation += 1;
        assert!(matches!(
            FramedControlStream::adopt(client, observed(), wrong, ControlLimits::default()),
            Err(TransportError::PeerIdentity)
        ));
    }

    #[test]
    fn broker_session_authenticates_packet_then_typed_negotiation() {
        let (handoff_sender, handoff_receiver) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let (offered, mut server) = UnixStream::pair().unwrap();
        let runner = "runner-7";
        let join = thread::spawn(move || {
            for index in 0..3 {
                let mut header = [0_u8; 4];
                server.read_exact(&mut header).unwrap();
                let size = u32::from_be_bytes(header) as usize;
                let mut request_body = vec![0_u8; size];
                server.read_exact(&mut request_body).unwrap();
                let request: ControlRequest = serde_json::from_slice(&request_body).unwrap();
                let response = if index == 0 {
                    ControlResponse::Success(crate::control_codec::SuccessResponse {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        result: ControlSuccess::Negotiated(crate::control_codec::Negotiated {
                            version: crate::control_codec::V1_3,
                            limits: ControlLimits::default(),
                            runner_instance_id: runner.into(),
                            oldest_revision: crate::control_codec::Revision(1),
                            latest_revision: crate::control_codec::Revision(4),
                        }),
                    })
                } else if index == 1 {
                    ControlResponse::Success(crate::control_codec::SuccessResponse {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                            request_sha256:
                                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                                    .into(),
                            result: crate::control_codec::ControlResult::Capabilities(
                                crate::control_codec::Capabilities {
                                    validate_settings: true,
                                    run_control: true,
                                    repeat: true,
                                    analysis: true,
                                    events: true,
                                },
                            ),
                        }),
                    })
                } else {
                    ControlResponse::Failure(crate::control_codec::FailureResponse {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        error: crate::control_codec::RpcError {
                            code: -32000,
                            message: "catalog unavailable".into(),
                        },
                    })
                };
                server
                    .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                    .unwrap();
            }
        });
        send_handoff(&handoff_sender, &offered, runner);
        let received = crate::broker_adoption::receive_single(&handoff_receiver).unwrap();
        let mut session =
            AuthenticatedBrokerSession::establish_from_broker(received, ControlLimits::default())
                .unwrap();
        assert_eq!(session.negotiated().runner_instance_id, runner);
        assert_eq!(session.continuity().generation().epoch, [9; 16]);
        assert_eq!(session.continuity().generation().sequence, 4);
        assert_eq!(
            session.peer_credentials().uid(),
            rustix::process::geteuid().as_raw()
        );
        assert!(session.peer_credentials().pid() > 0);
        let mut projection = crate::live_projection::ControlProjection::default();
        assert_eq!(
            session.poll_projection(&mut projection),
            Err(TransportError::RemoteFailure)
        );
        assert_eq!(projection.run_count(), 0);
        assert!(projection.snapshot().capabilities.is_none());
        join.join().unwrap();
    }

    #[test]
    fn poll_projection_commits_capabilities_and_empty_history_atomically() {
        let (mut server, client) = UnixStream::pair().unwrap();
        let runner = "runner-poll";
        let join = thread::spawn(move || {
            for index in 0..2 {
                let mut header = [0_u8; 4];
                server.read_exact(&mut header).unwrap();
                let size = u32::from_be_bytes(header) as usize;
                let mut body = vec![0_u8; size];
                server.read_exact(&mut body).unwrap();
                let request: ControlRequest = serde_json::from_slice(&body).unwrap();
                let result = if index == 0 {
                    crate::control_codec::ControlResult::Capabilities(
                        crate::control_codec::Capabilities {
                            validate_settings: true,
                            run_control: true,
                            repeat: false,
                            analysis: true,
                            events: true,
                        },
                    )
                } else {
                    crate::control_codec::ControlResult::History(crate::control_codec::Page {
                        items: Vec::new(),
                        next: None,
                        has_more: false,
                    })
                };
                let response = ControlResponse::Success(crate::control_codec::SuccessResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                        request_sha256: "a".repeat(64),
                        result,
                    }),
                });
                server
                    .write_all(&control_codec::encode(&response, 4096).unwrap())
                    .unwrap();
            }
        });
        let mut transport = FramedControlStream::adopt_broker(
            client,
            rustix::process::geteuid().as_raw(),
            rustix::process::getpid().as_raw_pid() as u32,
            ControlLimits::default(),
        )
        .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(crate::control_codec::V1_0);
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_0,
            limits: ControlLimits::default(),
            runner_instance_id: runner.into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(4),
        };
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated,
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [4; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials {
                uid: rustix::process::geteuid().as_raw(),
                pid: rustix::process::getpid().as_raw_pid() as u32,
            },
        };
        let mut projection = ControlProjection::default();
        session.poll_projection(&mut projection).unwrap();
        assert!(projection.snapshot().capabilities.is_some());
        assert_eq!(projection.run_count(), 0);
        join.join().unwrap();
    }

    #[test]
    fn poll_projection_v17_fixture_projects_every_bootstrap_surface() {
        use crate::control_codec::*;

        let (mut server, client) = UnixStream::pair().unwrap();
        let runner = "runner-poll-v17";
        let join = thread::spawn(move || {
            let agent_catalog: crate::agent_catalog::AgentCatalog =
                crate::agent_catalog::parse_agent_catalog_response(include_str!(
                    "../tests/fixtures/asb-v1.4-agent-catalog-response.json"
                ))
                .unwrap();
            let mut agent_catalog = agent_catalog;
            agent_catalog.runner_instance_id = runner.into();
            agent_catalog.catalog_sha256 = agent_catalog.computed_digest().unwrap();
            let agent_entry = agent_catalog.agents[0].clone();
            let measurement: MeasurementCatalogPublication = serde_json::from_value(
                serde_json::json!({
                    "version": {"major": 1, "minor": 2},
                    "freshness": "content_addressed",
                    "source": "built_in_collectors",
                    "catalog": {
                        "schema_version": 1,
                        "catalog_sha256": "",
                        "groups": [{"id": "latency", "label": "Latency", "description": "Request timing"}],
                        "measurements": [{
                            "id": "latency.first_response",
                            "name": "First response",
                            "description": "Time until the first response",
                            "group": "latency",
                            "quantity": "time",
                            "unit": "ns",
                            "aggregation": "gauge",
                            "scope": "attempt",
                            "provenance": {"source": "asb_runner_journal", "qualification": "implemented"},
                            "source_identity": "procfs_process_stat",
                            "resolution_ns": 1,
                            "overhead": {"class": "low", "minimum_interval_ns": 1, "requires_privilege": false},
                            "live": {"status": "supported"},
                            "replay": {"status": "supported"},
                            "platforms": [{"operating_system": "linux", "architectures": ["x86_64"], "required_features": ["procfs"]}],
                            "evidence_limits": ["collector_overhead_recorded"]
                        }]
                    }
                }))
                .unwrap();
            let mut measurement = measurement;
            measurement.catalog.catalog_sha256 = measurement.catalog.computed_digest().unwrap();
            let mut benchmark = BenchmarkCatalogPublication {
                generation: Revision(7),
                catalog_sha256: String::new(),
                pools: vec![BenchmarkCatalogPool {
                    id: "default".into(),
                    groups: vec![BenchmarkCatalogGroup {
                        id: "latency".into(),
                        benchmarks: vec![BenchmarkCatalogEntry {
                            id: "latency-basic".into(),
                            measure_ids: vec!["latency.first_response".into()],
                        }],
                    }],
                }],
            };
            benchmark.catalog_sha256 = benchmark.computed_digest().unwrap();
            let provider = ProviderCatalog {
                runner_instance_id: runner.into(),
                generation: Revision(3),
                catalog_sha256: "b".repeat(64),
                providers: vec![ProviderCatalogEntry {
                    provider_id: "openrouter".into(),
                    display_name: "OpenRouter".into(),
                    auth_methods: vec![ProviderAuthMethod::CredentialReference],
                    models: vec![ProviderModel {
                        model_id: "free-model".into(),
                        revision: "2026-01".into(),
                        availability: ProviderAvailability::Available,
                    }],
                    availability: ProviderAvailability::Available,
                }],
                refreshed: false,
            };
            let configuration = ConfigurationSnapshot {
                runner_instance_id: runner.into(),
                generation: Revision(4),
                configured: true,
                agent_ids: vec![agent_entry.agent_id.clone()],
                provider_id: Some("openrouter".into()),
                model_id: Some("free-model".into()),
                auth_method: Some(ProviderAuthMethod::CredentialReference),
                credential_reference_sha256: Some("c".repeat(64)),
            };
            let recording = RecordingCampaignStatus {
                runner_instance_id: runner.into(),
                generation: Revision(5),
                campaign: Some(RecordingCampaignPlan {
                    runner_instance_id: runner.into(),
                    generation: Revision(5),
                    campaign_id: "campaign-1".into(),
                    provider_id: "openrouter".into(),
                    model_id: "free-model".into(),
                    agent_ids: vec![agent_entry.agent_id.clone()],
                    workload_ids: vec!["latency-basic".into()],
                    tuple_count: 1,
                    state: "planned".into(),
                    offline_ready: false,
                    unavailable_reason: Some("recording-required".into()),
                }),
            };
            let run = RunSummary {
                run_id: RunId("run-history".into()),
                attempt_id: AttemptId("attempt-1".into()),
                state: PublicRunState::Completed,
                created_revision: Revision(6),
                revision: Revision(6),
                plan_sha256: "d".repeat(64),
            };
            let auth = AuthStatusResponse {
                provider: "openrouter".into(),
                endpoint_identity_sha256: "e".repeat(64),
                credential_locator_sha256: "f".repeat(64),
                generation: Revision(6),
                status: "active".into(),
            };
            loop {
                let mut header = [0; 4];
                if server.read_exact(&mut header).is_err() {
                    break;
                }
                let size = u32::from_be_bytes(header) as usize;
                let mut body = vec![0; size];
                server.read_exact(&mut body).unwrap();
                let request: ControlRequest = serde_json::from_slice(&body).unwrap();
                let result = match request.call {
                    ControlCall::Capabilities => ControlResult::Capabilities(Capabilities {
                        validate_settings: true,
                        run_control: true,
                        repeat: true,
                        analysis: true,
                        events: true,
                    }),
                    ControlCall::BenchmarkCatalog => {
                        ControlResult::BenchmarkCatalog(benchmark.clone())
                    }
                    ControlCall::MeasurementCatalog => {
                        ControlResult::MeasurementCatalog(measurement.clone())
                    }
                    ControlCall::History(_) => ControlResult::History(Page {
                        items: vec![run.clone()],
                        next: None,
                        has_more: false,
                    }),
                    ControlCall::AgentCatalog(_) => {
                        ControlResult::AgentCatalog(agent_catalog.clone())
                    }
                    ControlCall::AgentStatus(params) => ControlResult::AgentLifecycle(
                        crate::asb_lifecycle::AgentLifecycleResponse {
                            binding: params.binding,
                            operation_id: "operation-1".into(),
                            state: crate::asb_lifecycle::AgentLifecycleState::Active,
                            generation: 3,
                            progress_percent: 100,
                            failure: None,
                        },
                    ),
                    ControlCall::ProviderCatalog(_) => {
                        ControlResult::ProviderCatalog(provider.clone())
                    }
                    ControlCall::ConfigurationStatus(_) => {
                        ControlResult::Configuration(configuration.clone())
                    }
                    ControlCall::RecordingCampaignStatus(_) => {
                        ControlResult::RecordingCampaignStatus(recording.clone())
                    }
                    ControlCall::AuthStatus(_) => ControlResult::AuthStatus(auth.clone()),
                    other => panic!("unexpected poll call: {other:?}"),
                };
                let response = ControlResponse::Success(SuccessResponse {
                    jsonrpc: JSONRPC_VERSION.into(),
                    id: request.id,
                    result: ControlSuccess::Operation(BoundResult {
                        request_sha256: "a".repeat(64),
                        result,
                    }),
                });
                server
                    .write_all(&control_codec::encode(&response, 256 * 1024).unwrap())
                    .unwrap();
            }
        });
        let mut transport = FramedControlStream::adopt_broker(
            client,
            rustix::process::geteuid().as_raw(),
            rustix::process::getpid().as_raw_pid() as u32,
            ControlLimits::default(),
        )
        .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(V1_10);
        let negotiated = Negotiated {
            version: V1_10,
            limits: ControlLimits::default(),
            runner_instance_id: runner.into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(10),
        };
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated,
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [5; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials {
                uid: rustix::process::geteuid().as_raw(),
                pid: rustix::process::getpid().as_raw_pid() as u32,
            },
        };
        let mut projection = crate::live_projection::ControlProjection::default();
        session.poll_projection(&mut projection).unwrap();
        let snapshot = projection.snapshot();
        assert!(snapshot.capabilities.is_some());
        assert!(snapshot.benchmark_catalog.is_some());
        assert!(snapshot.measurement_catalog.is_some());
        assert_eq!(projection.run_count(), 1);
        assert!(snapshot.agent_catalog.is_some());
        assert!(snapshot.agent_lifecycle.is_some());
        assert!(snapshot.provider_catalog.is_some());
        assert!(snapshot.configuration.is_some());
        assert!(snapshot.recording_campaign_lifecycle.is_none());
        let provider = snapshot.provider_catalog.as_ref().unwrap();
        assert_eq!(provider.runner_instance_id, runner);
        assert_eq!(provider.generation, Revision(3));
        assert_eq!(provider.providers.len(), 1);
        assert_eq!(provider.providers[0].provider_id, "openrouter");
        assert_eq!(provider.providers[0].models.len(), 1);
        assert_eq!(provider.providers[0].models[0].model_id, "free-model");
        let configuration = snapshot.configuration.as_ref().unwrap();
        assert!(configuration.configured);
        assert_eq!(configuration.runner_instance_id, runner);
        assert_eq!(configuration.generation, Revision(4));
        assert_eq!(configuration.agent_ids, vec!["agent-a"]);
        assert_eq!(configuration.provider_id.as_deref(), Some("openrouter"));
        assert_eq!(configuration.model_id.as_deref(), Some("free-model"));
        assert_eq!(
            configuration
                .credential_reference_sha256
                .as_ref()
                .map(String::len),
            Some(64)
        );
        let auth = snapshot.auth_status.as_ref().unwrap();
        assert_eq!(auth.provider, "openrouter");
        assert_eq!(auth.generation, Revision(6));
        assert_eq!(auth.endpoint_identity_sha256, "e".repeat(64));
        assert_eq!(auth.credential_locator_sha256, "f".repeat(64));
        assert_eq!(auth.status, "active");
        let benchmark = snapshot.benchmark_catalog.as_ref().unwrap();
        assert_eq!(benchmark.generation, Revision(7));
        assert_eq!(benchmark.pools.len(), 1);
        assert_eq!(benchmark.pools[0].groups.len(), 1);
        assert_eq!(benchmark.pools[0].groups[0].benchmarks.len(), 1);
        assert_eq!(
            benchmark.pools[0].groups[0].benchmarks[0].measure_ids.len(),
            1
        );
        let measurement = snapshot.measurement_catalog.as_ref().unwrap();
        assert_eq!(measurement.schema_version, 1);
        assert_eq!(measurement.groups.len(), 1);
        assert_eq!(measurement.measurements.len(), 1);
        assert_eq!(measurement.measurements[0].id, "latency.first_response");
        assert_eq!(measurement.measurements[0].unit, "ns");
        let agents = snapshot.agent_catalog.as_ref().unwrap();
        assert_eq!(agents.runner_instance_id, runner);
        assert_eq!(agents.generation, 3);
        assert_eq!(agents.agents.len(), 1);
        assert_eq!(agents.agents[0].agent_id, "agent-a");
        assert!(agents.agents[0].package.is_some());
        let lifecycle = snapshot.agent_lifecycle.as_ref().unwrap();
        assert_eq!(lifecycle.binding.agent_id, "agent-a");
        assert_eq!(lifecycle.binding.runner_instance_id, runner);
        assert_eq!(lifecycle.operation_id, "operation-1");
        assert_eq!(lifecycle.progress_percent, 100);
        assert_eq!(lifecycle.generation, 3);
        let run = snapshot.runs.first().unwrap();
        assert_eq!(run.run_id.0, "run-history");
        assert_eq!(run.attempt_id.0, "attempt-1");
        assert_eq!(run.revision, Revision(6));
        assert_eq!(run.created_revision, Revision(6));
        assert!(snapshot.capabilities.as_ref().unwrap().run_control);
        assert!(snapshot.capabilities.as_ref().unwrap().analysis);
        assert!(snapshot.capabilities.as_ref().unwrap().validate_settings);
        assert!(snapshot.capabilities.as_ref().unwrap().repeat);
        assert!(snapshot.capabilities.as_ref().unwrap().events);
        assert_eq!(snapshot.runs.len(), 1);
        assert_eq!(snapshot.runs[0].state, PublicRunState::Completed);
        assert_eq!(snapshot.runs[0].plan_sha256.len(), 64);
        assert_eq!(measurement.catalog_sha256.len(), 64);
        assert_eq!(benchmark.catalog_sha256.len(), 64);
        assert_eq!(provider.catalog_sha256.len(), 64);
        assert_eq!(agents.catalog_sha256.len(), 64);
        drop(session);
        join.join().unwrap();
    }

    #[test]
    fn wizard_configuration_apply_round_trips_and_updates_projection() {
        let (server, client) = UnixStream::pair().unwrap();
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_7,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-7".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(1),
        };
        let join = thread::spawn(move || {
            let mut server = server;
            let mut header = [0_u8; 4];
            server.read_exact(&mut header).unwrap();
            let size = u32::from_be_bytes(header) as usize;
            let mut body = vec![0_u8; size];
            server.read_exact(&mut body).unwrap();
            let request: ControlRequest = serde_json::from_slice(&body).unwrap();
            let response = ControlResponse::Success(crate::control_codec::SuccessResponse {
                jsonrpc: "2.0".into(),
                id: request.id,
                result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                    request_sha256:
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    result: crate::control_codec::ControlResult::Configuration(
                        crate::control_codec::ConfigurationSnapshot {
                            runner_instance_id: "runner-7".into(),
                            generation: Revision(2),
                            configured: true,
                            agent_ids: vec!["codex".into()],
                            provider_id: Some("openai".into()),
                            model_id: Some("gpt-5.2".into()),
                            auth_method: Some(crate::control_codec::ProviderAuthMethod::None),
                            credential_reference_sha256: None,
                        },
                    ),
                }),
            });
            server
                .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                .unwrap();
        });
        let mut transport =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(crate::control_codec::V1_7);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated: negotiated.clone(),
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [9; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        let mut projection = ControlProjection::default();
        projection.accept_negotiated(negotiated).unwrap();
        session
            .apply_configuration(
                &mut projection,
                crate::control_codec::ConfigurationSelection {
                    agent_ids: vec!["codex".into()],
                    provider_id: "openai".into(),
                    model_id: "gpt-5.2".into(),
                    auth_method: crate::control_codec::ProviderAuthMethod::None,
                    credential_reference_sha256: None,
                },
            )
            .unwrap();
        assert_eq!(
            projection.snapshot().configuration.unwrap().generation,
            Revision(2)
        );
        join.join().unwrap();
    }

    #[test]
    fn recording_control_session_round_trips_all_operations_with_generation_fencing() {
        use crate::control_codec::{
            ControlResult, RecordingCampaignEstimate, RecordingCampaignLifecycle,
            RecordingCampaignPlan,
        };
        let (server, client) = UnixStream::pair().unwrap();
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_8,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-7".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(2),
        };
        let join = thread::spawn(move || {
            let mut server = server;
            for expected in 0..6_u64 {
                let mut header = [0_u8; 4];
                server.read_exact(&mut header).unwrap();
                let size = u32::from_be_bytes(header) as usize;
                let mut body = vec![0_u8; size];
                server.read_exact(&mut body).unwrap();
                let request: ControlRequest = serde_json::from_slice(&body).unwrap();
                assert_eq!(request.id, RequestId(9_000_000_002 + expected));
                let (result, generation) = match &request.call {
                    ControlCall::RecordingCampaignEstimate(_) => (
                        ControlResult::RecordingCampaignEstimate(RecordingCampaignEstimate {
                            runner_instance_id: "runner-7".into(),
                            generation: Revision(2),
                            tuple_count: 1,
                            complete_coverage: true,
                            offline_ready: true,
                            unavailable_reason: None,
                        }),
                        2,
                    ),
                    ControlCall::RecordingCampaignExecute(params) => {
                        assert_eq!(params.idempotency_key, "execute-key");
                        assert_eq!(params.expected_generation, Revision(2));
                        (lifecycle("recording", 0, Some("recording"), 2), 2)
                    }
                    ControlCall::RecordingCampaignProgress(params) => {
                        assert_eq!(params.campaign_id, "campaign-1");
                        (lifecycle("recording", 0, Some("recording"), 3), 3)
                    }
                    ControlCall::RecordingCampaignCancel(params) => {
                        assert_eq!(params.idempotency_key, "cancel-key");
                        assert_eq!(params.expected_generation, Revision(3));
                        (lifecycle("cancelled", 1, Some("cancelled"), 4), 4)
                    }
                    ControlCall::RecordingCampaignReconcile(params) => {
                        assert_eq!(params.idempotency_key, "reconcile-key");
                        assert_eq!(params.expected_generation, Revision(4));
                        (lifecycle("complete", 1, None, 5), 5)
                    }
                    ControlCall::RecordingCampaignOfflineDefault(params) => {
                        assert_eq!(params.idempotency_key, "offline-key");
                        assert_eq!(params.expected_generation, Revision(5));
                        (lifecycle("complete", 1, None, 5), 5)
                    }
                    call => panic!("unexpected call: {call:?}"),
                };
                assert_eq!(
                    generation,
                    match &result {
                        ControlResult::RecordingCampaignEstimate(value) => value.generation.0,
                        ControlResult::RecordingCampaignLifecycle(value) => value.generation.0,
                        _ => 0,
                    }
                );
                let response = ControlResponse::Success(crate::control_codec::SuccessResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                        request_sha256: "a".repeat(64),
                        result,
                    }),
                });
                server
                    .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                    .unwrap();
            }

            fn lifecycle(
                state: &str,
                covered: u16,
                reason: Option<&str>,
                generation: u64,
            ) -> ControlResult {
                ControlResult::RecordingCampaignLifecycle(RecordingCampaignLifecycle {
                    runner_instance_id: "runner-7".into(),
                    generation: Revision(generation),
                    campaign_id: "campaign-1".into(),
                    provider_id: "provider".into(),
                    model_id: "model".into(),
                    agent_ids: vec!["agent".into()],
                    workload_ids: vec!["workload".into()],
                    tuple_count: 1,
                    covered_tuple_count: covered,
                    state: state.into(),
                    offline_ready: state == "complete" && covered == 1,
                    unavailable_reason: reason.map(str::to_owned),
                })
            }
        });
        let mut transport =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(crate::control_codec::V1_8);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated: negotiated.clone(),
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [9; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        let mut projection = ControlProjection::default();
        projection.accept_negotiated(negotiated).unwrap();
        session
            .estimate_recording_campaign(
                &mut projection,
                "provider".into(),
                "model".into(),
                vec!["agent".into()],
                vec!["workload".into()],
            )
            .unwrap();
        projection
            .apply(
                &ControlRequest {
                    jsonrpc: "2.0".into(),
                    id: RequestId(100),
                    timeout_ms: 1_000,
                    call: ControlCall::RecordingCampaignPlan(
                        crate::control_codec::RecordingCampaignPlanParams {
                            idempotency_key: "plan".into(),
                            expected_generation: Revision(2),
                            runner_instance_id: "runner-7".into(),
                            provider_id: "provider".into(),
                            model_id: "model".into(),
                            agent_ids: vec!["agent".into()],
                            workload_ids: vec!["workload".into()],
                        },
                    ),
                },
                &ControlResponse::Success(crate::control_codec::SuccessResponse {
                    jsonrpc: "2.0".into(),
                    id: RequestId(100),
                    result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                        request_sha256: "a".repeat(64),
                        result: ControlResult::RecordingCampaign(RecordingCampaignPlan {
                            runner_instance_id: "runner-7".into(),
                            generation: Revision(2),
                            campaign_id: "campaign-1".into(),
                            provider_id: "provider".into(),
                            model_id: "model".into(),
                            agent_ids: vec!["agent".into()],
                            workload_ids: vec!["workload".into()],
                            tuple_count: 1,
                            state: "planned".into(),
                            offline_ready: false,
                            unavailable_reason: Some("recording-required".into()),
                        }),
                    }),
                }),
                ControlLimits::default(),
            )
            .unwrap();
        session
            .execute_recording_campaign(&mut projection, "campaign-1".into(), "execute-key".into())
            .unwrap();
        session
            .progress_recording_campaign(&mut projection, "campaign-1".into())
            .unwrap();
        session
            .cancel_recording_campaign(&mut projection, "campaign-1".into(), "cancel-key".into())
            .unwrap();
        session
            .reconcile_recording_campaign(
                &mut projection,
                "campaign-1".into(),
                "reconcile-key".into(),
            )
            .unwrap();
        session
            .set_recording_campaign_offline_default(
                &mut projection,
                "campaign-1".into(),
                "offline-key".into(),
            )
            .unwrap();
        assert_eq!(
            projection
                .snapshot()
                .recording_campaign_lifecycle
                .unwrap()
                .generation,
            Revision(5)
        );
        assert!(
            projection
                .snapshot()
                .recording_estimate
                .unwrap()
                .offline_ready
        );
        join.join().unwrap();
    }

    #[test]
    fn helper_profile_rejects_nested_secret_fields_and_bounds() {
        assert!(
            validate_credential_free_profile(&serde_json::json!({
                "credential": {"source": "helper", "reference_sha256": "a".repeat(64)},
                "model": "fixture-model"
            }))
            .is_ok()
        );
        assert_eq!(
            validate_credential_free_profile(&serde_json::json!({
                "credential": {"api-key": "sk-secret"}
            })),
            Err(TransportError::Codec(CodecError::InvalidValue(
                "credential-free profile"
            )))
        );
        assert_eq!(
            validate_credential_free_profile(&serde_json::json!({
                "nested": {"password": "secret"}
            })),
            Err(TransportError::Codec(CodecError::InvalidValue(
                "credential-free profile"
            )))
        );
        assert_eq!(
            validate_credential_free_profile(&serde_json::json!({
                "credential": {"apiKey": "sk-secret", "token": "raw"}
            })),
            Err(TransportError::Codec(CodecError::InvalidValue(
                "credential-free profile"
            )))
        );
    }

    #[test]
    fn helper_invocation_requires_v110_and_projects_only_typed_status() {
        let (server, client) = UnixStream::pair().unwrap();
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_10,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-7".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(1),
        };
        let join = thread::spawn(move || {
            let mut server = server;
            let mut header = [0_u8; 4];
            server.read_exact(&mut header).unwrap();
            let size = u32::from_be_bytes(header) as usize;
            let mut body = vec![0_u8; size];
            server.read_exact(&mut body).unwrap();
            let request: ControlRequest = serde_json::from_slice(&body).unwrap();
            assert_eq!(request.id, RequestId(9_000_000_105));
            assert_eq!(request.timeout_ms, ControlLimits::default().max_timeout_ms);
            assert_eq!(
                request.call.minimum_version(),
                Some(crate::control_codec::V1_10)
            );
            let ControlCall::AuthHelperInvoke(params) = request.call else {
                panic!("helper invocation request was not typed");
            };
            assert_eq!(params.provider, "openai");
            assert_eq!(params.profile, serde_json::json!({"model": "free"}));
            assert_eq!(params.idempotency_key, "helper-1");
            assert!(
                !body
                    .windows(b"api_key".len())
                    .any(|window| window == b"api_key")
            );
            let response = ControlResponse::Success(crate::control_codec::SuccessResponse {
                jsonrpc: "2.0".into(),
                id: request.id,
                result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                    request_sha256: "a".repeat(64),
                    result: crate::control_codec::ControlResult::AuthStatus(
                        crate::control_codec::AuthStatusResponse {
                            provider: "openai".into(),
                            endpoint_identity_sha256: "b".repeat(64),
                            credential_locator_sha256: "c".repeat(64),
                            generation: Revision(2),
                            status: "active".into(),
                        },
                    ),
                }),
            });
            server
                .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                .unwrap();
        });
        let mut transport =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(crate::control_codec::V1_10);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated: negotiated.clone(),
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [9; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        let mut projection = ControlProjection::default();
        projection.accept_negotiated(negotiated).unwrap();
        session
            .invoke_auth_helper(
                &mut projection,
                "openai".into(),
                serde_json::json!({"model": "free"}),
                "helper-1".into(),
            )
            .unwrap();
        let status = projection.snapshot().auth_status.unwrap();
        assert_eq!(status.provider, "openai");
        assert_eq!(status.status, "active");
        assert_eq!(status.generation, Revision(2));
        join.join().unwrap();

        let (server, client) = UnixStream::pair().unwrap();
        drop(server);
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_7,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-7".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(1),
        };
        let mut transport =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(crate::control_codec::V1_7);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated,
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [9; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        let mut projection = ControlProjection::default();
        assert_eq!(
            session.invoke_auth_helper(
                &mut projection,
                "openai".into(),
                serde_json::json!({"model": "free"}),
                "helper-2".into(),
            ),
            Err(TransportError::NotNegotiated)
        );
    }

    #[test]
    fn helper_reconciliation_failure_is_typed_and_does_not_mutate_projection() {
        let (server, client) = UnixStream::pair().unwrap();
        let join = thread::spawn(move || {
            let mut server = server;
            let mut header = [0_u8; 4];
            server.read_exact(&mut header).unwrap();
            let size = u32::from_be_bytes(header) as usize;
            let mut body = vec![0_u8; size];
            server.read_exact(&mut body).unwrap();
            let request: ControlRequest = serde_json::from_slice(&body).unwrap();
            assert!(matches!(request.call, ControlCall::AuthHelperInvoke(_)));
            let response = ControlResponse::Failure(crate::control_codec::FailureResponse {
                jsonrpc: "2.0".into(),
                id: request.id,
                error: crate::control_codec::RpcError {
                    code: -33008,
                    message: "runner reconciliation is required".into(),
                },
            });
            server
                .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                .unwrap();
        });
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_10,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-reconcile".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(1),
        };
        let mut transport =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(control_codec::V1_10);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated: negotiated.clone(),
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [7; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        let mut projection = ControlProjection::default();
        projection.accept_negotiated(negotiated).unwrap();
        assert_eq!(
            session.invoke_auth_helper(
                &mut projection,
                "openai".into(),
                serde_json::json!({"model": "free"}),
                "reconcile-retry-1".into(),
            ),
            Err(TransportError::RemoteFailureCode(-33008))
        );
        assert!(TransportError::RemoteFailureCode(-33008).is_reconciliation_required());
        assert!(projection.snapshot().auth_status.is_none());
        join.join().unwrap();
    }

    #[test]
    fn helper_reconciliation_fixture_retries_with_digest_only_auth_status() {
        let (server, client) = UnixStream::pair().unwrap();
        let join = thread::spawn(move || {
            let mut server = server;
            for (attempt, expected_key) in [(0, "reconcile-1"), (1, "reconcile-2")] {
                let mut header = [0_u8; 4];
                server.read_exact(&mut header).unwrap();
                let size = u32::from_be_bytes(header) as usize;
                let mut body = vec![0_u8; size];
                server.read_exact(&mut body).unwrap();
                assert!(!body.windows(b"api_key".len()).any(|w| w == b"api_key"));
                assert!(!body.windows(b"secret".len()).any(|w| w == b"secret"));
                let request: ControlRequest = serde_json::from_slice(&body).unwrap();
                let ControlCall::AuthHelperInvoke(params) = request.call else {
                    panic!("reconciliation fixture received a non-helper request");
                };
                assert_eq!(params.idempotency_key, expected_key);
                let response = if attempt == 0 {
                    ControlResponse::Failure(crate::control_codec::FailureResponse {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        error: crate::control_codec::RpcError {
                            code: -33008,
                            message: "runner reconciliation is required".into(),
                        },
                    })
                } else {
                    ControlResponse::Success(crate::control_codec::SuccessResponse {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                            request_sha256: "d".repeat(64),
                            result: crate::control_codec::ControlResult::AuthStatus(
                                crate::control_codec::AuthStatusResponse {
                                    provider: "openai".into(),
                                    endpoint_identity_sha256: "e".repeat(64),
                                    credential_locator_sha256: "f".repeat(64),
                                    generation: Revision(3),
                                    status: "active".into(),
                                },
                            ),
                        }),
                    })
                };
                server
                    .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                    .unwrap();
            }
        });
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_10,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-reconcile-fixture".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(1),
        };
        let mut transport =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(control_codec::V1_10);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated: negotiated.clone(),
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [8; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        let mut projection = ControlProjection::default();
        projection.accept_negotiated(negotiated).unwrap();
        assert_eq!(
            session.invoke_auth_helper(
                &mut projection,
                "openai".into(),
                serde_json::json!({"model": "free"}),
                "reconcile-1".into(),
            ),
            Err(TransportError::RemoteFailureCode(-33008))
        );
        assert!(projection.snapshot().auth_status.is_none());
        session
            .invoke_auth_helper(
                &mut projection,
                "openai".into(),
                serde_json::json!({"model": "free"}),
                "reconcile-2".into(),
            )
            .unwrap();
        let status = projection.snapshot().auth_status.unwrap();
        assert_eq!(status.provider, "openai");
        assert_eq!(status.status, "active");
        assert_eq!(status.generation, Revision(3));
        assert_eq!(status.endpoint_identity_sha256.len(), 64);
        assert_eq!(status.credential_locator_sha256.len(), 64);
        join.join().unwrap();
    }

    #[test]
    fn helper_reconciliation_fixture_survives_transport_reconnect() {
        let (server1, client1) = UnixStream::pair().unwrap();
        let first = thread::spawn(move || {
            let mut server = server1;
            let mut header = [0_u8; 4];
            server.read_exact(&mut header).unwrap();
            let size = u32::from_be_bytes(header) as usize;
            let mut body = vec![0_u8; size];
            server.read_exact(&mut body).unwrap();
            let request: ControlRequest = serde_json::from_slice(&body).unwrap();
            let response = ControlResponse::Failure(crate::control_codec::FailureResponse {
                jsonrpc: "2.0".into(),
                id: request.id,
                error: crate::control_codec::RpcError {
                    code: -33008,
                    message: "runner reconciliation is required".into(),
                },
            });
            server
                .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                .unwrap();
        });
        let (server2, client2) = UnixStream::pair().unwrap();
        let second = thread::spawn(move || {
            let mut server = server2;
            let mut header = [0_u8; 4];
            server.read_exact(&mut header).unwrap();
            let size = u32::from_be_bytes(header) as usize;
            let mut body = vec![0_u8; size];
            server.read_exact(&mut body).unwrap();
            assert!(!body.windows(b"api_key".len()).any(|w| w == b"api_key"));
            let request: ControlRequest = serde_json::from_slice(&body).unwrap();
            let ControlCall::AuthHelperInvoke(params) = request.call else {
                panic!("reconnected fixture received a non-helper request");
            };
            assert_eq!(params.idempotency_key, "reconnect-retry");
            let response = ControlResponse::Success(crate::control_codec::SuccessResponse {
                jsonrpc: "2.0".into(),
                id: request.id,
                result: ControlSuccess::Operation(crate::control_codec::BoundResult {
                    request_sha256: "1".repeat(64),
                    result: crate::control_codec::ControlResult::AuthStatus(
                        crate::control_codec::AuthStatusResponse {
                            provider: "openai".into(),
                            endpoint_identity_sha256: "2".repeat(64),
                            credential_locator_sha256: "3".repeat(64),
                            generation: Revision(4),
                            status: "active".into(),
                        },
                    ),
                }),
            });
            server
                .write_all(&crate::control_codec::encode(&response, 4096).unwrap())
                .unwrap();
        });
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_10,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-reconnect-fixture".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(1),
        };
        let mut projection = ControlProjection::default();
        projection.accept_negotiated(negotiated.clone()).unwrap();
        let mut transport =
            FramedControlStream::adopt(client1, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(control_codec::V1_10);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated: negotiated.clone(),
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [6; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        assert_eq!(
            session.invoke_auth_helper(
                &mut projection,
                "openai".into(),
                serde_json::json!({"model": "free"}),
                "reconnect-first".into(),
            ),
            Err(TransportError::RemoteFailureCode(-33008))
        );
        drop(session);
        first.join().unwrap();

        let mut transport =
            FramedControlStream::adopt(client2, observed(), peer(), ControlLimits::default())
                .unwrap();
        transport.negotiated = true;
        transport.negotiated_version = Some(control_codec::V1_10);
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated,
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [6; 16],
                sequence: 2,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        session
            .invoke_auth_helper(
                &mut projection,
                "openai".into(),
                serde_json::json!({"model": "free"}),
                "reconnect-retry".into(),
            )
            .unwrap();
        assert_eq!(
            projection.snapshot().auth_status.unwrap().generation,
            Revision(4)
        );
        second.join().unwrap();
    }

    #[test]
    fn broker_continuity_rejects_epoch_change_and_sequence_skip() {
        let mut continuity = BrokerContinuity::new(BrokerGeneration {
            epoch: [1; 16],
            sequence: 8,
        })
        .unwrap();
        assert_eq!(
            continuity.accept_successor(BrokerGeneration {
                epoch: [1; 16],
                sequence: 10,
            }),
            Err(TransportError::Continuity)
        );
        assert_eq!(
            continuity.accept_successor(BrokerGeneration {
                epoch: [2; 16],
                sequence: 9,
            }),
            Err(TransportError::Continuity)
        );
        continuity
            .accept_successor(BrokerGeneration {
                epoch: [1; 16],
                sequence: 9,
            })
            .unwrap();
        assert_eq!(continuity.generation().sequence, 9);
    }

    #[test]
    fn setup_calls_require_v17_and_invalid_frames_fail_before_decode() {
        let agent_call = ControlCall::AgentCatalog(crate::agent_catalog::AgentCatalogRequest {
            action: crate::agent_catalog::AgentCatalogAction::Status,
            runner_instance_id: "runner-7".into(),
            known_generation: None,
        });
        assert_eq!(
            minimum_version_for_call(&agent_call),
            Some(crate::control_codec::CONTROL_AGENT_CATALOG_V1)
        );
        let lifecycle_call = ControlCall::RecordingCampaignOfflineDefault(
            crate::control_codec::RecordingCampaignOfflineDefaultParams {
                idempotency_key: "offline".into(),
                expected_generation: Revision(1),
                runner_instance_id: "runner-7".into(),
                campaign_id: "campaign-1".into(),
            },
        );
        assert_eq!(
            minimum_version_for_call(&lifecycle_call),
            Some(crate::control_codec::CONTROL_RECORDING_LIFECYCLE_V1)
        );
        let estimate_call = ControlCall::RecordingCampaignEstimate(
            crate::control_codec::RecordingCampaignEstimateRequest {
                runner_instance_id: "runner-7".into(),
                provider_id: "provider".into(),
                model_id: "model".into(),
                agent_ids: vec!["agent".into()],
                workload_ids: vec!["workload".into()],
            },
        );
        assert_eq!(
            minimum_version_for_call(&estimate_call),
            Some(crate::control_codec::V1_7)
        );
        assert_eq!(minimum_version_for_call(&ControlCall::Capabilities), None);

        let (mut server, client) = UnixStream::pair().unwrap();
        let join = thread::spawn(move || {
            let mut request_header = [0_u8; 4];
            server.read_exact(&mut request_header).unwrap();
            let request_size = u32::from_be_bytes(request_header) as usize;
            let mut request_body = vec![0_u8; request_size];
            server.read_exact(&mut request_body).unwrap();
            server.write_all(&0_u32.to_be_bytes()).unwrap();
        });
        let mut stream =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        stream.negotiated = true;
        let request = ControlRequest {
            jsonrpc: "2.0".into(),
            id: RequestId(20),
            timeout_ms: 1000,
            call: ControlCall::Capabilities,
        };
        stream.write_request(&request).unwrap();
        assert_eq!(
            stream.read_response(&request),
            Err(TransportError::Codec(CodecError::FrameTooLarge))
        );
        join.join().unwrap();
    }

    #[test]
    fn coverage_auth_and_recording_version_gates_fail_before_io() {
        let (_server, client) = UnixStream::pair().unwrap();
        let transport =
            FramedControlStream::adopt(client, observed(), peer(), ControlLimits::default())
                .unwrap();
        let negotiated = crate::control_codec::Negotiated {
            version: crate::control_codec::V1_5,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-7".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(1),
        };
        let mut session = AuthenticatedBrokerSession {
            transport,
            negotiated,
            continuity: BrokerContinuity::new(BrokerGeneration {
                epoch: [9; 16],
                sequence: 1,
            })
            .unwrap(),
            peer: BrokerPeerCredentials { uid: 1000, pid: 42 },
        };
        let mut projection = ControlProjection::default();
        assert_eq!(
            session.auth_status(&mut projection, "provider".into()),
            Err(TransportError::NotNegotiated)
        );
        assert_eq!(
            session.enroll_auth(
                &mut projection,
                "provider".into(),
                "a".repeat(64),
                "b".repeat(64),
                "key".into(),
            ),
            Err(TransportError::NotNegotiated)
        );
        assert_eq!(
            session.estimate_recording_campaign(
                &mut projection,
                "provider".into(),
                "model".into(),
                vec!["agent".into()],
                vec!["workload".into()],
            ),
            Err(TransportError::NotNegotiated)
        );
        assert_eq!(
            session.progress_recording_campaign(&mut projection, "campaign".into()),
            Err(TransportError::NotNegotiated)
        );
        assert_eq!(
            session.execute_recording_campaign(&mut projection, "campaign".into(), "key".into(),),
            Err(TransportError::Projection)
        );
    }
}
