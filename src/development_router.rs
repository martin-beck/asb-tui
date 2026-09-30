// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Development-only router envelope for the standalone TUI lifecycle client.
//!
//! The envelope deliberately accepts only the local development profile. It provides a
//! deterministic client boundary around the existing closed lifecycle request schema without
//! claiming remote authentication, provider authorization, or production trust.

use crate::delegated::{LifecycleRequest, LifecycleResponse, execute, read_request};
use serde::Deserialize;
use std::io::Read;

const MAX_ROUTER_BYTES: u64 = 128 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouterEnvelope {
    router_version: u64,
    profile: String,
    request: LifecycleRequest,
}

#[derive(Debug, serde::Serialize)]
pub struct RouterResponse {
    pub router_version: u64,
    pub profile: &'static str,
    pub development_only: bool,
    pub lifecycle: LifecycleResponse,
}

fn error(code: &'static str) -> RouterResponse {
    RouterResponse {
        router_version: 1,
        profile: "development",
        development_only: true,
        lifecycle: LifecycleResponse::result(false, code),
    }
}

/// Read one bounded development-router request from a caller-owned stream.
pub fn execute_input(mut input: impl Read) -> RouterResponse {
    let mut bytes = Vec::new();
    if input
        .by_ref()
        .take(MAX_ROUTER_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return error("router_read_failed");
    }
    if bytes.is_empty() || bytes.len() as u64 > MAX_ROUTER_BYTES {
        return error("router_request_size_invalid");
    }
    let envelope: RouterEnvelope = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return error("router_request_invalid"),
    };
    if envelope.router_version != 1 {
        return error("router_version_unsupported");
    }
    if envelope.profile != "development" {
        return error("router_profile_unsupported");
    }
    let lifecycle = execute(envelope.request);
    RouterResponse {
        router_version: 1,
        profile: "development",
        development_only: true,
        lifecycle,
    }
}

/// Validate a development-router request using the same closed lifecycle parser.
pub fn parse_lifecycle_request(input: &[u8]) -> Result<LifecycleRequest, &'static str> {
    read_request(input)
}

#[cfg(test)]
mod tests {
    use super::execute_input;
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{fs, os::unix::fs::PermissionsExt};

    fn private_root() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "asb-tui-development-router-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn development_profile_routes_status_without_remote_authentication() {
        let root = private_root();
        fs::create_dir(root.join("versions")).unwrap();
        fs::set_permissions(root.join("versions"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join(".lifecycle.lock"), b"").unwrap();
        fs::set_permissions(
            root.join(".lifecycle.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let request = format!(
            r#"{{"router_version":1,"profile":"development","request":{{"operation":"status","schema_version":1,"install_root":{root:?}}}}}"#,
            root = root.to_string_lossy()
        );
        let response = execute_input(request.as_bytes());
        assert!(response.development_only);
        assert_eq!(response.profile, "development");
        assert_eq!(response.lifecycle.code, "extension_not_installed");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn production_profile_is_not_silently_downgraded() {
        let response = execute_input(
            &br#"{"router_version":1,"profile":"production","request":{"operation":"status","schema_version":1,"install_root":"/tmp"}}"#[..],
        );
        assert_eq!(response.lifecycle.code, "router_profile_unsupported");
        assert!(response.development_only);
    }

    #[test]
    fn unknown_envelope_fields_are_rejected() {
        let response = execute_input(
            &br#"{"router_version":1,"profile":"development","secret":"no","request":{"operation":"status","schema_version":1,"install_root":"/tmp"}}"#[..],
        );
        assert_eq!(response.lifecycle.code, "router_request_invalid");
    }
}
