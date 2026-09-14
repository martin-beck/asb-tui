// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded framed transport for an already-adopted ASB control channel.
//!
//! This is deliberately a transport seam, not a broker or runner.  The caller
//! supplies a connected `UnixStream` and independently observed peer identity;
//! this module only binds that identity, applies deadlines and validates the
//! typed JSON-RPC boundary.

use crate::control_client::{AdoptedChannel, PeerCredentials, RunnerIdentity};
use crate::control_codec::{
    self, CodecError, ControlLimits, ControlRequest, ControlResponse, ControlSuccess,
    NegotiateParams, RequestId,
};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Debug)]
pub enum TransportError {
    Io,
    Codec(CodecError),
    PeerIdentity,
    NotNegotiated,
}
impl From<CodecError> for TransportError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
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
        if session.runner_instance_id != self.expected_peer.runner_instance_id
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

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TransportError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    fn peer() -> RunnerIdentity {
        RunnerIdentity {
            uid: 1000,
            pid: 42,
            service_generation: 7,
            runner_instance_id: "runner-7".into(),
        }
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
}
