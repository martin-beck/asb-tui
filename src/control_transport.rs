// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded framed transport for an already-adopted ASB control channel.
//!
//! This is deliberately a transport seam, not a broker or runner.  The caller
//! supplies a connected `UnixStream` and independently observed peer identity;
//! this module only binds that identity, applies deadlines and validates the
//! typed JSON-RPC boundary.

use crate::control_codec::{
    self, CodecError, ControlCall, ControlLimits, ControlRequest, ControlResponse, ControlSuccess,
    NegotiateParams, PageParams, RequestId, Revision,
};
use crate::{
    broker_adoption::{AdoptionError, BrokerGeneration, ReceivedChannel},
    control_client::{AdoptedChannel, PeerCredentials, RunnerIdentity},
    live_projection::ControlProjection,
};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
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
        })
    }

    pub fn negotiate(&mut self, id: RequestId) -> Result<ControlSuccess, TransportError> {
        let request = ControlRequest {
            jsonrpc: control_codec::JSONRPC_VERSION.into(),
            id,
            timeout_ms: self.limits.max_timeout_ms,
            call: control_codec::ControlCall::Negotiate(NegotiateParams {
                versions: [
                    control_codec::V1_3,
                    control_codec::V1_2,
                    control_codec::V1_0,
                ]
                .into_iter()
                .collect(),
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
            || ![
                control_codec::V1_3,
                control_codec::V1_2,
                control_codec::V1_0,
            ]
            .contains(&session.version)
        {
            return Err(TransportError::NotNegotiated);
        }
        self.negotiated = true;
        Ok(ControlSuccess::Negotiated(session))
    }

    pub fn write_request(&mut self, request: &ControlRequest) -> Result<(), TransportError> {
        request.validate(self.limits)?;
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
        let calls = [
            ControlCall::Capabilities,
            ControlCall::MeasurementCatalog,
            ControlCall::History(PageParams {
                after: None::<Revision>,
                limit: limits.max_page_items,
            }),
        ];
        for (offset, call) in calls.into_iter().enumerate() {
            if matches!(&call, ControlCall::MeasurementCatalog)
                && self.negotiated.version < control_codec::CONTROL_MEASUREMENT_CATALOG_V1
            {
                return Err(TransportError::NotNegotiated);
            }
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
        io::{IoSlice, Read, Write},
        mem::MaybeUninit,
        os::fd::AsFd,
        thread,
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
}
