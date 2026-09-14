// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Explicit identity mapping at the ASB/TUI boundary.
//!
//! ASB's durable identity contains an opaque runner digest, an execution
//! epoch, and a journal sequence.  The adopted TUI channel has kernel-derived
//! peer credentials and a service generation.  This module is deliberately a
//! pure compatibility gate: it does not derive one identity from another,
//! open a socket, or perform transport I/O.  Callers must provide the
//! authenticated values from both sides and pass this gate before using them.

use crate::control_client::{MAX_RUNNER_ID_BYTES, PeerCredentials, RunnerIdentity};

/// Maximum encoded length of the ASB runner digest (a SHA-256 hex digest).
pub const RUNNER_DIGEST_BYTES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsbExecutionIdentity {
    /// Opaque digest published by ASB for the runner instance.
    pub runner_digest: String,
    /// ASB's durable execution epoch.
    pub epoch: u64,
    /// Monotonic journal sequence within the epoch.
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiPeerIdentity {
    /// Credentials obtained from the adopted channel, never from user input.
    pub credentials: PeerCredentials,
    /// Explicit ASB runner identifier returned by the negotiated handshake.
    pub runner_instance_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityMapping {
    pub asb: AsbExecutionIdentity,
    pub tui: TuiPeerIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityCompatibilityError {
    InvalidRunnerDigest,
    InvalidRunnerIdentity,
    EpochGenerationMismatch,
    PeerChanged,
    RunnerChanged,
    SequenceRegressed,
}

impl AsbExecutionIdentity {
    pub fn validate(&self) -> Result<(), IdentityCompatibilityError> {
        if self.runner_digest.len() != RUNNER_DIGEST_BYTES
            || !self
                .runner_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(IdentityCompatibilityError::InvalidRunnerDigest);
        }
        Ok(())
    }
}

impl TuiPeerIdentity {
    pub fn validate(&self) -> Result<(), IdentityCompatibilityError> {
        let identity = RunnerIdentity {
            uid: self.credentials.uid,
            pid: self.credentials.pid,
            service_generation: self.credentials.service_generation,
            runner_instance_id: self.runner_instance_id.clone(),
        };
        if self.runner_instance_id.len() > MAX_RUNNER_ID_BYTES || identity.validate().is_err() {
            return Err(IdentityCompatibilityError::InvalidRunnerIdentity);
        }
        Ok(())
    }
}

impl IdentityMapping {
    /// Validate the explicit cross-repository contract.
    ///
    /// Equality between `epoch` and `service_generation` is intentional and
    /// must be documented by the broker handshake.  The runner digest is not
    /// hashed into `runner_instance_id`; the latter is accepted only as an
    /// independently authenticated value, preventing accidental identity
    /// derivation from a truncated or differently-scoped identifier.
    pub fn validate(&self) -> Result<(), IdentityCompatibilityError> {
        self.asb.validate()?;
        self.tui.validate()?;
        if self.asb.epoch != self.tui.credentials.service_generation {
            return Err(IdentityCompatibilityError::EpochGenerationMismatch);
        }
        Ok(())
    }

    /// Accept a later observation only when it is from the same explicit
    /// runner/peer mapping and its journal sequence has not regressed.
    pub fn accept_next(&self, next: &Self) -> Result<(), IdentityCompatibilityError> {
        self.validate()?;
        next.validate()?;
        if self.asb.runner_digest != next.asb.runner_digest {
            return Err(IdentityCompatibilityError::RunnerChanged);
        }
        if self.tui.runner_instance_id != next.tui.runner_instance_id
            || self.tui.credentials.uid != next.tui.credentials.uid
            || self.tui.credentials.pid != next.tui.credentials.pid
            || self.tui.credentials.service_generation != next.tui.credentials.service_generation
        {
            return Err(IdentityCompatibilityError::PeerChanged);
        }
        if next.asb.sequence < self.asb.sequence {
            return Err(IdentityCompatibilityError::SequenceRegressed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(sequence: u64) -> IdentityMapping {
        IdentityMapping {
            asb: AsbExecutionIdentity {
                runner_digest: "a".repeat(RUNNER_DIGEST_BYTES),
                epoch: 7,
                sequence,
            },
            tui: TuiPeerIdentity {
                credentials: PeerCredentials {
                    uid: 1000,
                    pid: 42,
                    service_generation: 7,
                },
                runner_instance_id: "runner-explicit-id".into(),
            },
        }
    }

    #[test]
    fn same_runner_accepts_equal_or_later_sequence() {
        let current = mapping(4);
        assert!(current.accept_next(&mapping(4)).is_ok());
        assert!(current.accept_next(&mapping(5)).is_ok());
    }

    #[test]
    fn rejects_digest_generation_peer_and_sequence_mismatches() {
        let current = mapping(4);
        let mut digest = mapping(5);
        digest.asb.runner_digest = "b".repeat(RUNNER_DIGEST_BYTES);
        assert_eq!(
            current.accept_next(&digest),
            Err(IdentityCompatibilityError::RunnerChanged)
        );

        let mut generation = mapping(5);
        generation.asb.epoch = 8;
        assert_eq!(
            current.accept_next(&generation),
            Err(IdentityCompatibilityError::EpochGenerationMismatch)
        );

        let mut peer = mapping(5);
        peer.tui.credentials.pid = 43;
        assert_eq!(
            current.accept_next(&peer),
            Err(IdentityCompatibilityError::PeerChanged)
        );

        assert_eq!(
            current.accept_next(&mapping(3)),
            Err(IdentityCompatibilityError::SequenceRegressed)
        );
    }

    #[test]
    fn rejects_invalid_digest_and_does_not_infer_runner_id() {
        let mut invalid = mapping(0);
        invalid.asb.runner_digest = "A".repeat(RUNNER_DIGEST_BYTES);
        assert_eq!(
            invalid.validate(),
            Err(IdentityCompatibilityError::InvalidRunnerDigest)
        );
        let mut explicit = mapping(0);
        explicit.tui.runner_instance_id = "runner-different-explicit-id".into();
        assert!(explicit.validate().is_ok());
    }
}
