// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Development-only router envelope for the standalone TUI lifecycle client.
//!
//! The envelope deliberately accepts only the local development profile. It provides a
//! deterministic client boundary around the existing closed lifecycle request schema without
//! claiming remote authentication, provider authorization, or production trust.

use crate::delegated::{LifecycleRequest, LifecycleResponse, execute, read_request};
use serde::{Deserialize, Serialize};
use std::io::Read;

const MAX_ROUTER_BYTES: u64 = 128 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouterEnvelope {
    router_version: u64,
    profile: String,
    channel: String,
    current_main: CurrentMainIdentity,
    request: LifecycleRequest,
}

/// Identity of the two source trees participating in the development channel.
/// These values are public provenance, never credentials or local paths.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CurrentMainIdentity {
    pub asb_source_commit: String,
    pub asb_source_tree: String,
    pub tui_source_commit: String,
    pub tui_source_tree: String,
}

#[derive(Debug, Serialize)]
pub struct RouterResponse {
    pub router_version: u64,
    pub profile: &'static str,
    pub channel: &'static str,
    pub development_only: bool,
    pub current_main: Option<CurrentMainIdentity>,
    pub lifecycle: LifecycleResponse,
}

fn error(code: &'static str, current_main: Option<CurrentMainIdentity>) -> RouterResponse {
    RouterResponse {
        router_version: 1,
        profile: "development",
        channel: "dev",
        development_only: true,
        current_main,
        lifecycle: LifecycleResponse::result(false, code),
    }
}

fn valid_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn identity_is_valid(identity: &CurrentMainIdentity) -> bool {
    valid_commit(&identity.asb_source_commit)
        && valid_commit(&identity.asb_source_tree)
        && valid_commit(&identity.tui_source_commit)
        && valid_commit(&identity.tui_source_tree)
}

fn request_identity(request: &LifecycleRequest) -> Option<(&str, &str)> {
    match request {
        LifecycleRequest::Install {
            expected_source_commit,
            expected_source_tree,
            ..
        }
        | LifecycleRequest::Upgrade {
            expected_source_commit,
            expected_source_tree,
            ..
        } => Some((expected_source_commit, expected_source_tree)),
        LifecycleRequest::Status { .. }
        | LifecycleRequest::Remove { .. }
        | LifecycleRequest::Launch { .. } => None,
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
        return error("router_read_failed", None);
    }
    if bytes.is_empty() || bytes.len() as u64 > MAX_ROUTER_BYTES {
        return error("router_request_size_invalid", None);
    }
    let envelope: RouterEnvelope = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return error("router_request_invalid", None),
    };
    if envelope.router_version != 1 {
        return error("router_version_unsupported", None);
    }
    if envelope.profile != "development" {
        return error("router_profile_unsupported", None);
    }
    if envelope.channel != "dev" {
        return error("router_channel_unsupported", Some(envelope.current_main));
    }
    if !identity_is_valid(&envelope.current_main) {
        return error("router_identity_invalid", Some(envelope.current_main));
    }
    if let Some((expected_commit, expected_tree)) = request_identity(&envelope.request)
        && (expected_commit != envelope.current_main.tui_source_commit
            || expected_tree != envelope.current_main.tui_source_tree)
    {
        return error("router_identity_stale", Some(envelope.current_main));
    }
    let lifecycle = execute(envelope.request);
    RouterResponse {
        router_version: 1,
        profile: "development",
        channel: "dev",
        development_only: true,
        current_main: Some(envelope.current_main),
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
            r#"{{"router_version":1,"profile":"development","channel":"dev","current_main":{{"asb_source_commit":"{commit}","asb_source_tree":"{tree}","tui_source_commit":"{tui_commit}","tui_source_tree":"{tui_tree}"}},"request":{{"operation":"status","schema_version":1,"install_root":{root:?}}}}}"#,
            commit = "a".repeat(40),
            tree = "b".repeat(40),
            tui_commit = "c".repeat(40),
            tui_tree = "d".repeat(40),
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
            &br#"{"router_version":1,"profile":"production","channel":"dev","current_main":{"asb_source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","asb_source_tree":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","tui_source_commit":"cccccccccccccccccccccccccccccccccccccccc","tui_source_tree":"dddddddddddddddddddddddddddddddddddddddd"},"request":{"operation":"status","schema_version":1,"install_root":"/tmp"}}"#[..],
        );
        assert_eq!(response.lifecycle.code, "router_profile_unsupported");
        assert!(response.development_only);
    }

    #[test]
    fn unknown_envelope_fields_are_rejected() {
        let response = execute_input(
            &br#"{"router_version":1,"profile":"development","channel":"dev","current_main":{"asb_source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","asb_source_tree":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","tui_source_commit":"cccccccccccccccccccccccccccccccccccccccc","tui_source_tree":"dddddddddddddddddddddddddddddddddddddddd"},"secret":"no","request":{"operation":"status","schema_version":1,"install_root":"/tmp"}}"#[..],
        );
        assert_eq!(response.lifecycle.code, "router_request_invalid");
    }

    #[test]
    fn unsupported_channels_and_stale_asb_identity_fail_closed() {
        let base = r#"{"router_version":1,"profile":"development","channel":"CHANNEL","current_main":{"asb_source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","asb_source_tree":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","tui_source_commit":"cccccccccccccccccccccccccccccccccccccccc","tui_source_tree":"dddddddddddddddddddddddddddddddddddddddd"},"request":{"operation":"status","schema_version":1,"install_root":"/tmp"}}"#;
        assert_eq!(
            execute_input(base.replace("CHANNEL", "stable").as_bytes())
                .lifecycle
                .code,
            "router_channel_unsupported"
        );
        let malformed = base
            .replace("CHANNEL", "dev")
            .replace("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "stale");
        assert_eq!(
            execute_input(malformed.as_bytes()).lifecycle.code,
            "router_identity_invalid"
        );

        let stale_install = base
            .replace("CHANNEL", "dev")
            .replace(
                r#"{"operation":"status","schema_version":1,"install_root":"/tmp"}"#,
                r#"{"operation":"install","schema_version":1,"install_root":"/tmp","manifest":"/tmp/m","signature":"/tmp/s","artifacts":"/tmp/a","target":"x86_64-unknown-linux-gnu","asb_version":"0.1.0","protocol_version":1,"expected_release":"v0.1.0","expected_source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","expected_source_tree":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","expected_executable_sha256":"1111111111111111111111111111111111111111111111111111111111111111"}"#,
            );
        assert_eq!(
            execute_input(stale_install.as_bytes()).lifecycle.code,
            "router_identity_stale"
        );
    }
}
