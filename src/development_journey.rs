// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded qualification gate for the credential-free development journey.
//!
//! This module evaluates only a caller-provided local fixture.  It performs no
//! network, credential, provider, or ASB access and therefore cannot be used as
//! evidence of live-provider or production readiness.

use serde::{Deserialize, Serialize};
use std::io::Read;

const MAX_REQUEST_BYTES: u64 = 16 * 1024;
const MAX_TEXT_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FixtureRequest {
    pub schema_version: u64,
    pub profile: String,
    pub asb_version: String,
    pub expected_asb_version: String,
    pub catalog_version: u64,
    pub expected_catalog_version: u64,
    pub bundle_available: bool,
    pub broker_available: bool,
    pub protocol_version: Option<u64>,
    pub expected_protocol_version: u64,
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub struct FixtureResponse {
    pub schema_version: u64,
    pub profile: &'static str,
    pub development_only: bool,
    pub ready: bool,
    pub code: &'static str,
    pub recovery: &'static str,
}

fn response(ready: bool, code: &'static str, recovery: &'static str) -> FixtureResponse {
    FixtureResponse {
        schema_version: 1,
        profile: "development",
        development_only: true,
        ready,
        code,
        recovery,
    }
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value.chars().all(|character| !character.is_control())
}

/// Evaluate one local fixture and return at most one actionable recovery.
pub fn evaluate_fixture(request: FixtureRequest) -> FixtureResponse {
    if request.schema_version != 1 {
        return response(false, "journey_schema_unsupported", "exit");
    }
    if request.profile != "development" {
        return response(false, "journey_profile_unsupported", "exit");
    }
    if !valid_text(&request.asb_version) || !valid_text(&request.expected_asb_version) {
        return response(false, "journey_version_invalid", "refresh_version");
    }
    if request.asb_version != request.expected_asb_version {
        return response(false, "asb_version_mismatch", "refresh_version");
    }
    if request.catalog_version != request.expected_catalog_version {
        return response(false, "catalog_mismatch", "refresh_catalog");
    }
    if !request.bundle_available {
        return response(false, "development_bundle_unavailable", "repair_bundle");
    }
    if !request.broker_available {
        return response(false, "development_broker_unavailable", "retry_broker");
    }
    if request.protocol_version != Some(request.expected_protocol_version) {
        return response(false, "development_protocol_mismatch", "retry_broker");
    }
    response(true, "development_journey_ready", "continue")
}

/// Read and evaluate one bounded JSON fixture from a caller-owned stream.
pub fn execute_input(mut input: impl Read) -> FixtureResponse {
    let mut bytes = Vec::new();
    if input
        .by_ref()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return response(false, "journey_read_failed", "exit");
    }
    if bytes.is_empty() || bytes.len() as u64 > MAX_REQUEST_BYTES {
        return response(false, "journey_request_size_invalid", "exit");
    }
    let request: FixtureRequest = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return response(false, "journey_request_invalid", "exit"),
    };
    evaluate_fixture(request)
}

#[cfg(test)]
mod tests {
    use super::{FixtureResponse, evaluate_fixture, execute_input};

    struct MismatchCase {
        code: &'static str,
        recovery: &'static str,
        mutate: fn(&mut super::FixtureRequest),
    }

    fn request() -> super::FixtureRequest {
        super::FixtureRequest {
            schema_version: 1,
            profile: "development".into(),
            asb_version: "fixture-1".into(),
            expected_asb_version: "fixture-1".into(),
            catalog_version: 7,
            expected_catalog_version: 7,
            bundle_available: true,
            broker_available: true,
            protocol_version: Some(1),
            expected_protocol_version: 1,
        }
    }

    #[test]
    fn ready_fixture_is_development_only_and_credential_free() {
        assert_eq!(
            evaluate_fixture(request()),
            FixtureResponse {
                schema_version: 1,
                profile: "development",
                development_only: true,
                ready: true,
                code: "development_journey_ready",
                recovery: "continue",
            }
        );
    }

    #[test]
    fn each_mismatch_stops_before_execution_with_one_recovery() {
        let cases = [
            MismatchCase {
                code: "asb_version_mismatch",
                recovery: "refresh_version",
                mutate: |value| value.asb_version = "fixture-0".into(),
            },
            MismatchCase {
                code: "catalog_mismatch",
                recovery: "refresh_catalog",
                mutate: |value| value.catalog_version = 6,
            },
            MismatchCase {
                code: "development_bundle_unavailable",
                recovery: "repair_bundle",
                mutate: |value| value.bundle_available = false,
            },
            MismatchCase {
                code: "development_broker_unavailable",
                recovery: "retry_broker",
                mutate: |value| value.broker_available = false,
            },
            MismatchCase {
                code: "development_protocol_mismatch",
                recovery: "retry_broker",
                mutate: |value| value.protocol_version = Some(2),
            },
        ];
        for MismatchCase {
            code,
            recovery,
            mutate,
        } in cases
        {
            let mut value = request();
            mutate(&mut value);
            let result = evaluate_fixture(value);
            assert!(!result.ready);
            assert_eq!((result.code, result.recovery), (code, recovery));
        }
    }

    #[test]
    fn production_and_unknown_fields_never_downgrade() {
        let mut production = request();
        production.profile = "production".into();
        assert_eq!(
            evaluate_fixture(production).code,
            "journey_profile_unsupported"
        );
        assert_eq!(
            execute_input(
                &br#"{"schema_version":1,"profile":"development","asb_version":"fixture-1","expected_asb_version":"fixture-1","catalog_version":7,"expected_catalog_version":7,"bundle_available":true,"broker_available":true,"protocol_version":1,"expected_protocol_version":1,"secret":"no"}"#[..],
            )
            .code,
            "journey_request_invalid"
        );
    }
}
