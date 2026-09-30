// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Development-only install and broker onboarding projection.

use serde::{Deserialize, Serialize};
use std::io::Read;

const MAX_REQUEST_BYTES: u64 = 16 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: u64,
    profile: String,
    bundle_available: bool,
    broker_available: bool,
    protocol_version: Option<u64>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Response {
    pub schema_version: u64,
    pub profile: &'static str,
    pub development_only: bool,
    pub ready: bool,
    pub code: &'static str,
    pub recovery: &'static str,
}

fn response(ready: bool, code: &'static str, recovery: &'static str) -> Response {
    Response {
        schema_version: 1,
        profile: "development",
        development_only: true,
        ready,
        code,
        recovery,
    }
}

/// Evaluate a bounded local onboarding fixture and return one actionable recovery choice.
pub fn execute_input(mut input: impl Read) -> Response {
    let mut bytes = Vec::new();
    if input
        .by_ref()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return response(false, "onboarding_read_failed", "exit");
    }
    if bytes.is_empty() || bytes.len() as u64 > MAX_REQUEST_BYTES {
        return response(false, "onboarding_request_size_invalid", "exit");
    }
    let request: Request = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return response(false, "onboarding_request_invalid", "exit"),
    };
    if request.schema_version != 1 {
        return response(false, "onboarding_version_unsupported", "exit");
    }
    if request.profile != "development" {
        return response(false, "onboarding_profile_unsupported", "exit");
    }
    if !request.bundle_available {
        return response(false, "development_bundle_unavailable", "repair_bundle");
    }
    if !request.broker_available {
        return response(false, "development_broker_unavailable", "retry_broker");
    }
    if request.protocol_version != Some(1) {
        return response(false, "development_protocol_unavailable", "exit");
    }
    response(true, "development_onboarding_ready", "continue")
}

#[cfg(test)]
mod tests {
    use super::{Response, execute_input};

    fn request(bundle: bool, broker: bool, protocol: Option<u64>) -> String {
        format!(
            r#"{{"schema_version":1,"profile":"development","bundle_available":{bundle},"broker_available":{broker},"protocol_version":{protocol}}}"#,
            bundle = bundle,
            broker = broker,
            protocol = protocol.map_or_else(|| "null".into(), |v| v.to_string())
        )
    }

    #[test]
    fn ready_fixture_is_explicitly_development_only() {
        assert_eq!(
            execute_input(request(true, true, Some(1)).as_bytes()),
            Response {
                schema_version: 1,
                profile: "development",
                development_only: true,
                ready: true,
                code: "development_onboarding_ready",
                recovery: "continue",
            }
        );
    }

    #[test]
    fn missing_components_have_one_recovery_choice() {
        assert_eq!(
            execute_input(request(false, true, Some(1)).as_bytes()).recovery,
            "repair_bundle"
        );
        assert_eq!(
            execute_input(request(true, false, Some(1)).as_bytes()).recovery,
            "retry_broker"
        );
        assert_eq!(
            execute_input(request(true, true, None).as_bytes()).recovery,
            "exit"
        );
    }

    #[test]
    fn production_profile_and_unknown_fields_never_downgrade() {
        let production = request(true, true, Some(1)).replace("development", "production");
        assert_eq!(
            execute_input(production.as_bytes()).code,
            "onboarding_profile_unsupported"
        );
        assert_eq!(
            execute_input(
                &br#"{"schema_version":1,"profile":"development","bundle_available":true,"broker_available":true,"protocol_version":1,"secret":"no"}"#[..],
            )
            .code,
            "onboarding_request_invalid"
        );
    }
}
