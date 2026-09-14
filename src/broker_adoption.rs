//! Safe, bounded adoption of the anonymous control channel sent by ASB.
//!
//! The broker sends one connected Unix stream descriptor as `SCM_RIGHTS`.
//! This module owns the receive boundary only: it never discovers an endpoint,
//! renders a screen, or retries a benchmark mutation.  All descriptors yielded
//! by `recvmsg` stay owned until the packet has passed every validation step;
//! rejected descriptors are therefore closed by normal `OwnedFd` drop.

use std::{
    io::IoSliceMut,
    mem::MaybeUninit,
    os::fd::{AsFd, OwnedFd},
};

use rustix::{
    cmsg_space,
    net::{
        RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SocketAddrUnix,
        SocketType, getpeername, getsockname, recvmsg, sockopt,
    },
};

use crate::control_client::{MAX_FRAME_BYTES, RunnerIdentity};

/// Payload on the provisioning socket is deliberately much smaller than a
/// regular control frame.  The broker's response is a fixed, bounded header.
pub const MAX_ADOPTION_PAYLOAD: usize = 4096;
/// Frozen ASB broker packet size.
pub const BROKER_PACKET_BYTES: usize = 72;
/// Receive enough ancillary space to detect two descriptors and reject them;
/// one descriptor is the only accepted rights payload.
const MAX_RIGHTS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdoptionError {
    Io,
    WrongSocketShape,
    Truncated,
    MissingDescriptor,
    MultipleDescriptors,
    UnexpectedAncillary,
    EmptyPayload,
    PayloadTooLarge,
    PeerChanged,
    InvalidPeer,
    InvalidPacket,
    IdentityMismatch,
}

impl std::fmt::Display for AdoptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "broker channel adoption failed: {self:?}")
    }
}

impl std::error::Error for AdoptionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerOperation {
    Initial,
    Replacement,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffStatus {
    Request,
    Success,
    Unavailable,
    Rejected,
    Exhausted,
}
/// Broker continuity evidence. It is intentionally not a service generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerGeneration {
    pub epoch: [u8; 16],
    pub sequence: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerPacket {
    pub operation: BrokerOperation,
    pub status: HandoffStatus,
    pub generation: BrokerGeneration,
    pub expected_runner_identity: [u8; 32],
}

impl BrokerPacket {
    pub fn decode(bytes: &[u8]) -> Result<Self, AdoptionError> {
        if bytes.len() != BROKER_PACKET_BYTES
            || bytes[..8] != *b"ASBHND01"
            || bytes[8..10] != 1_u16.to_be_bytes()
            || bytes[12..16] != [0; 4]
        {
            return Err(AdoptionError::InvalidPacket);
        }
        let operation = match bytes[10] {
            1 => BrokerOperation::Initial,
            2 => BrokerOperation::Replacement,
            _ => return Err(AdoptionError::InvalidPacket),
        };
        let status = match bytes[11] {
            0 => HandoffStatus::Request,
            1 => HandoffStatus::Success,
            2 => HandoffStatus::Unavailable,
            3 => HandoffStatus::Rejected,
            4 => HandoffStatus::Exhausted,
            _ => return Err(AdoptionError::InvalidPacket),
        };
        let mut epoch = [0; 16];
        epoch.copy_from_slice(&bytes[16..32]);
        let mut identity = [0; 32];
        identity.copy_from_slice(&bytes[40..72]);
        let packet = Self {
            operation,
            status,
            generation: BrokerGeneration {
                epoch,
                sequence: u64::from_be_bytes(bytes[32..40].try_into().expect("eight bytes")),
            },
            expected_runner_identity: identity,
        };
        let generation = epoch != [0; 16] && packet.generation.sequence != 0;
        let valid = match status {
            HandoffStatus::Success => generation && identity != [0; 32],
            HandoffStatus::Request | HandoffStatus::Unavailable | HandoffStatus::Rejected => {
                identity == [0; 32] && (operation == BrokerOperation::Initial || generation)
            }
            HandoffStatus::Exhausted => {
                operation == BrokerOperation::Replacement && generation && identity == [0; 32]
            }
        };
        valid.then_some(packet).ok_or(AdoptionError::InvalidPacket)
    }
}

/// An accepted descriptor and its bounded broker payload.  The descriptor is
/// intentionally owned, so callers cannot accidentally leak or reuse it after
/// a failed authentication step.
#[derive(Debug)]
pub struct ReceivedChannel {
    channel: OwnedFd,
    payload: Vec<u8>,
    peer_uid: u32,
    peer_pid: u32,
}

impl ReceivedChannel {
    #[must_use]
    pub fn channel(&self) -> &OwnedFd {
        &self.channel
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub fn peer_uid(&self) -> u32 {
        self.peer_uid
    }

    #[must_use]
    pub fn peer_pid(&self) -> u32 {
        self.peer_pid
    }

    /// Authenticate the kernel-derived peer tuple before protocol use.  The
    /// service generation is authenticated by the control handshake and is
    /// deliberately not inferred from descriptor metadata.
    pub fn authenticate_peer(&self, expected: &RunnerIdentity) -> Result<(), AdoptionError> {
        expected
            .validate()
            .map_err(|_| AdoptionError::InvalidPeer)?;
        if self.peer_uid != expected.uid || self.peer_pid != expected.pid {
            return Err(AdoptionError::PeerChanged);
        }
        Ok(())
    }

    /// Validate the packet's explicit runner digest against the identity
    /// returned by typed control negotiation. Epoch/sequence are retained as
    /// continuity evidence and are never converted into service generation.
    pub fn authenticate_negotiated_identity(
        &self,
        runner_instance_id: &str,
    ) -> Result<BrokerGeneration, AdoptionError> {
        let packet = BrokerPacket::decode(&self.payload)?;
        if packet.status != HandoffStatus::Success {
            return Err(AdoptionError::InvalidPacket);
        }
        if packet.expected_runner_identity != runner_identity_digest(runner_instance_id)? {
            return Err(AdoptionError::IdentityMismatch);
        }
        Ok(packet.generation)
    }
}

fn runner_identity_digest(value: &str) -> Result<[u8; 32], AdoptionError> {
    if value.is_empty()
        || value.len() > 128
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(AdoptionError::InvalidPeer);
    }
    let length = u16::try_from(value.len()).map_err(|_| AdoptionError::InvalidPeer)?;
    let mut input = b"asb-control-runner-instance-v1".to_vec();
    input.push(0);
    input.extend_from_slice(&length.to_be_bytes());
    input.extend_from_slice(value.as_bytes());
    let hex = crate::sha256::digest_hex(&input);
    let mut digest = [0; 32];
    for (i, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        digest[i] = u8::from_str_radix(
            std::str::from_utf8(chunk).map_err(|_| AdoptionError::InvalidPeer)?,
            16,
        )
        .map_err(|_| AdoptionError::InvalidPeer)?;
    }
    Ok(digest)
}

/// Validate the shape of an already received channel before reading protocol
/// bytes.  Paths and abstract names are rejected: only unnamed connected
/// `AF_UNIX`/`SOCK_STREAM` sockets are admissible.
pub fn validate_channel_shape<Fd: AsFd>(fd: Fd) -> Result<(u32, u32), AdoptionError> {
    let socket_type = sockopt::socket_type(&fd).map_err(|_| AdoptionError::Io)?;
    if socket_type != SocketType::STREAM {
        return Err(AdoptionError::WrongSocketShape);
    }
    let socket_error = sockopt::socket_error(&fd).map_err(|_| AdoptionError::Io)?;
    if socket_error.is_err() {
        return Err(AdoptionError::WrongSocketShape);
    }
    let local = getsockname(&fd).map_err(|_| AdoptionError::WrongSocketShape)?;
    let peer = getpeername(&fd).map_err(|_| AdoptionError::WrongSocketShape)?;
    let peer = peer.ok_or(AdoptionError::WrongSocketShape)?;
    let local = SocketAddrUnix::try_from(local).map_err(|_| AdoptionError::WrongSocketShape)?;
    let peer = SocketAddrUnix::try_from(peer).map_err(|_| AdoptionError::WrongSocketShape)?;
    // Linux reports socketpair endpoints as an empty abstract name; this is
    // the unnamed form. Any non-empty path or abstract name is a locator and
    // is rejected before credentials/protocol bytes are consumed.
    let unnamed = |address: &SocketAddrUnix| {
        address.path_bytes().is_none()
            && address.abstract_name().is_some_and(|name| name.is_empty())
    };
    if !unnamed(&local) || !unnamed(&peer) {
        return Err(AdoptionError::WrongSocketShape);
    }
    let credentials = sockopt::socket_peercred(&fd).map_err(|_| AdoptionError::InvalidPeer)?;
    Ok((
        credentials.uid.as_raw(),
        credentials.pid.as_raw_pid() as u32,
    ))
}

/// Receive exactly one bounded payload and exactly one `SCM_RIGHTS` descriptor.
/// `MSG_CMSG_CLOEXEC` is mandatory; ancillary truncation and every cardinality
/// violation fail closed.  `recvmsg` owns each received descriptor in an
/// `OwnedFd`, and all rejected values are dropped before returning.
pub fn receive_single<Fd: AsFd>(socket: Fd) -> Result<ReceivedChannel, AdoptionError> {
    let mut data = vec![0u8; MAX_ADOPTION_PAYLOAD];
    let mut iov = [IoSliceMut::new(&mut data)];
    let mut control = [MaybeUninit::uninit(); cmsg_space!(ScmRights(MAX_RIGHTS))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut control);
    let message = recvmsg(&socket, &mut iov, &mut ancillary, RecvFlags::CMSG_CLOEXEC)
        .map_err(|_| AdoptionError::Io)?;
    if message.bytes == 0 {
        return Err(AdoptionError::EmptyPayload);
    }
    if message.bytes > MAX_ADOPTION_PAYLOAD || message.bytes > MAX_FRAME_BYTES {
        return Err(AdoptionError::PayloadTooLarge);
    }
    if message
        .flags
        .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
    {
        return Err(AdoptionError::Truncated);
    }

    let mut accepted: Option<OwnedFd> = None;
    let mut rights_messages = 0usize;
    for item in ancillary.drain() {
        match item {
            RecvAncillaryMessage::ScmRights(mut fds) => {
                rights_messages += 1;
                if rights_messages > 1 {
                    return Err(AdoptionError::MultipleDescriptors);
                }
                let descriptor = fds.next().ok_or(AdoptionError::MissingDescriptor)?;
                if fds.next().is_some() {
                    return Err(AdoptionError::MultipleDescriptors);
                }
                accepted = Some(descriptor);
            }
            _ => return Err(AdoptionError::UnexpectedAncillary),
        }
    }
    let channel = accepted.ok_or(AdoptionError::MissingDescriptor)?;
    let (peer_uid, peer_pid) = validate_channel_shape(&channel)?;
    data.truncate(message.bytes);
    Ok(ReceivedChannel {
        channel,
        payload: data,
        peer_uid,
        peer_pid,
    })
}

/// Adopt the conventional inherited broker socket at stdin without accepting
/// an endpoint path or a caller-selected descriptor number.  This is a thin
/// fd-0 entry point; callers still receive the same shape and ancillary checks.
pub fn receive_from_stdin() -> Result<ReceivedChannel, AdoptionError> {
    let stdin = std::io::stdin();
    receive_single(stdin.as_fd())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::net::{
        AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketType, sendmsg,
        socketpair,
    };
    use std::{io::IoSlice, os::fd::AsRawFd};

    fn pair() -> (OwnedFd, OwnedFd) {
        socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .unwrap()
    }

    fn send_with_fd(sender: &OwnedFd, fd: &OwnedFd, body: &[u8]) {
        let iov = [IoSlice::new(body)];
        let mut bytes = [MaybeUninit::uninit(); cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut bytes);
        let rights = [fd.as_fd()];
        ancillary.push(SendAncillaryMessage::ScmRights(&rights));
        sendmsg(sender, &iov, &mut ancillary, SendFlags::empty()).unwrap();
    }

    #[test]
    fn receives_one_cloexec_stream_descriptor() {
        let (sender, receiver) = pair();
        let (offered, _peer) = pair();
        send_with_fd(&sender, &offered, b"hello");
        let received = receive_single(&receiver).unwrap();
        assert_eq!(received.payload(), b"hello");
        assert!(received.channel().as_raw_fd() >= 3);
        assert!(
            rustix::io::fcntl_getfd(received.channel())
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
    }

    #[test]
    fn rejects_without_rights() {
        let (sender, receiver) = pair();
        let iov = [IoSlice::new(b"hello")];
        let mut ancillary = SendAncillaryBuffer::default();
        sendmsg(&sender, &iov, &mut ancillary, SendFlags::empty()).unwrap();
        assert!(matches!(
            receive_single(&receiver),
            Err(AdoptionError::MissingDescriptor)
        ));
    }

    #[test]
    fn rejects_two_rights_and_closes_them() {
        let (sender, receiver) = pair();
        let (first, _) = pair();
        let (second, _) = pair();
        let iov = [IoSlice::new(b"hello")];
        let mut bytes = [MaybeUninit::uninit(); cmsg_space!(ScmRights(2))];
        let mut ancillary = SendAncillaryBuffer::new(&mut bytes);
        let rights = [first.as_fd(), second.as_fd()];
        ancillary.push(SendAncillaryMessage::ScmRights(&rights));
        sendmsg(&sender, &iov, &mut ancillary, SendFlags::empty()).unwrap();
        assert!(matches!(
            receive_single(&receiver),
            Err(AdoptionError::MultipleDescriptors)
        ));
    }

    fn success_packet(identity: [u8; 32]) -> [u8; BROKER_PACKET_BYTES] {
        let mut bytes = [0; BROKER_PACKET_BYTES];
        bytes[..8].copy_from_slice(b"ASBHND01");
        bytes[8..10].copy_from_slice(&1_u16.to_be_bytes());
        bytes[10] = 1;
        bytes[11] = 1;
        bytes[16..32].fill(7);
        bytes[32..40].copy_from_slice(&1_u64.to_be_bytes());
        bytes[40..72].copy_from_slice(&identity);
        bytes
    }

    #[test]
    fn broker_generation_and_identity_digest_bind_over_socketpair() {
        let (channel, _peer) = pair();
        let identity = runner_identity_digest("runner-handoff-test").unwrap();
        let received = ReceivedChannel {
            channel,
            payload: success_packet(identity).to_vec(),
            peer_uid: 0,
            peer_pid: 1,
        };
        let generation = received
            .authenticate_negotiated_identity("runner-handoff-test")
            .unwrap();
        assert_eq!(generation.sequence, 1);
        assert_eq!(generation.epoch, [7; 16]);
    }

    #[test]
    fn broker_identity_mismatch_and_epoch_reuse_fail_closed() {
        let (channel, _peer) = pair();
        let received = ReceivedChannel {
            channel,
            payload: success_packet(runner_identity_digest("runner-handoff-test").unwrap())
                .to_vec(),
            peer_uid: 0,
            peer_pid: 1,
        };
        assert_eq!(
            received.authenticate_negotiated_identity("different-runner"),
            Err(AdoptionError::IdentityMismatch)
        );
        let mut malformed = received.payload.clone();
        malformed[16..32].fill(0);
        assert_eq!(
            BrokerPacket::decode(&malformed),
            Err(AdoptionError::InvalidPacket)
        );
    }
}
