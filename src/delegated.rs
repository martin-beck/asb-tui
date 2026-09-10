// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed JSON lifecycle request boundary delegated by a future ASB CLI router.

use crate::{
    bundle::{
        ExpectedCompatibility, verify_bundle_manifest_with_trusted_signer,
        verify_local_bundle_artifacts,
    },
    compatibility::Architecture,
    lifecycle::{
        ExecutableSelfTest, FilesystemLifecycle, LifecycleStore, ProcessLauncher, install, launch,
        remove, status,
    },
    release_channel::{ReleaseClassification, channel_permits_install, compiled_classification},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 128 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleRequest {
    Status {
        schema_version: u64,
        install_root: PathBuf,
    },
    Remove {
        schema_version: u64,
        install_root: PathBuf,
    },
    Launch {
        schema_version: u64,
        install_root: PathBuf,
    },
    Install {
        schema_version: u64,
        install_root: PathBuf,
        manifest: PathBuf,
        signature: PathBuf,
        artifacts: PathBuf,
        target: String,
        asb_version: String,
        protocol_version: u64,
        expected_release: String,
        expected_source_commit: String,
        expected_source_tree: String,
        expected_executable_sha256: String,
    },
    Upgrade {
        schema_version: u64,
        install_root: PathBuf,
        manifest: PathBuf,
        signature: PathBuf,
        artifacts: PathBuf,
        target: String,
        asb_version: String,
        protocol_version: u64,
        expected_release: String,
        expected_source_commit: String,
        expected_source_tree: String,
        expected_executable_sha256: String,
    },
}

#[derive(Debug, Serialize)]
pub struct LifecycleResponse {
    pub schema_version: u64,
    pub classification: &'static str,
    pub ok: bool,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asb_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_tree: Option<String>,
}

impl LifecycleResponse {
    fn result(ok: bool, code: &'static str) -> Self {
        Self {
            schema_version: 1,
            classification: compiled_classification().as_str(),
            ok,
            code,
            installed: None,
            verified: None,
            release: None,
            executable_sha256: None,
            target: None,
            bundle: None,
            asb_version: None,
            protocol_version: None,
            source_commit: None,
            source_tree: None,
        }
    }
}

pub fn read_request(mut input: impl Read) -> Result<LifecycleRequest, &'static str> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "request_read_failed")?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("request_size_invalid");
    }
    serde_json::from_slice(&bytes).map_err(|_| "request_invalid")
}

pub fn execute_input(input: impl Read) -> LifecycleResponse {
    match read_request(input) {
        Ok(request) => execute(request),
        Err(code) => LifecycleResponse::result(false, code),
    }
}

pub fn execute(request: LifecycleRequest) -> LifecycleResponse {
    let result = match request {
        LifecycleRequest::Status {
            schema_version,
            install_root,
        } => {
            if schema_version != 1 {
                return LifecycleResponse::result(false, "request_version_unsupported");
            }
            if !safe_path(&install_root) {
                return LifecycleResponse::result(false, "request_path_invalid");
            }
            let Ok(store) = FilesystemLifecycle::open_read_only(&install_root) else {
                return LifecycleResponse::result(false, "install_root_unavailable");
            };
            let observed = status(&store);
            return LifecycleResponse {
                schema_version: 1,
                classification: if observed.verified {
                    ReleaseClassification::VerifiedExtension.as_str()
                } else {
                    compiled_classification().as_str()
                },
                ok: observed.verified,
                code: observed.reason,
                installed: Some(observed.installed),
                verified: Some(observed.verified),
                release: observed.release,
                executable_sha256: observed.executable_sha256,
                target: observed.target,
                bundle: observed.bundle,
                asb_version: observed.asb_version,
                protocol_version: observed.protocol_version,
                source_commit: observed.source_commit,
                source_tree: observed.source_tree,
            };
        }
        LifecycleRequest::Remove {
            schema_version,
            install_root,
        } => lifecycle_store(schema_version, &install_root)
            .and_then(|mut store| remove(&mut store))
            .map(|()| "extension_removed"),
        LifecycleRequest::Launch {
            schema_version,
            install_root,
        } => lifecycle_store(schema_version, &install_root)
            .and_then(|store| launch(&store, &mut ExecutableSelfTest, &mut ProcessLauncher))
            .map(|()| "frontend_exited"),
        LifecycleRequest::Install {
            schema_version,
            install_root,
            manifest,
            signature,
            artifacts,
            target,
            asb_version,
            protocol_version,
            expected_release,
            expected_source_commit,
            expected_source_tree,
            expected_executable_sha256,
        } => install_or_upgrade(InstallInput {
            schema_version,
            install_root,
            manifest,
            signature,
            artifacts,
            target,
            asb_version,
            protocol_version,
            expected_release,
            expected_source_commit,
            expected_source_tree,
            expected_executable_sha256,
            operation: InstallOperation::Install,
        })
        .map(|()| "extension_installed"),
        LifecycleRequest::Upgrade {
            schema_version,
            install_root,
            manifest,
            signature,
            artifacts,
            target,
            asb_version,
            protocol_version,
            expected_release,
            expected_source_commit,
            expected_source_tree,
            expected_executable_sha256,
        } => install_or_upgrade(InstallInput {
            schema_version,
            install_root,
            manifest,
            signature,
            artifacts,
            target,
            asb_version,
            protocol_version,
            expected_release,
            expected_source_commit,
            expected_source_tree,
            expected_executable_sha256,
            operation: InstallOperation::Upgrade,
        })
        .map(|()| "extension_upgraded"),
    };
    match result {
        Ok(code) => LifecycleResponse::result(true, code),
        Err(code) => LifecycleResponse::result(false, code),
    }
}

fn lifecycle_store(schema_version: u64, root: &Path) -> Result<FilesystemLifecycle, &'static str> {
    if schema_version != 1 {
        return Err("request_version_unsupported");
    }
    if !safe_path(root) {
        return Err("request_path_invalid");
    }
    FilesystemLifecycle::open(root).map_err(|_| "install_root_unavailable")
}

struct InstallInput {
    schema_version: u64,
    install_root: PathBuf,
    manifest: PathBuf,
    signature: PathBuf,
    artifacts: PathBuf,
    target: String,
    asb_version: String,
    protocol_version: u64,
    expected_release: String,
    expected_source_commit: String,
    expected_source_tree: String,
    expected_executable_sha256: String,
    operation: InstallOperation,
}

#[derive(Clone, Copy)]
enum InstallOperation {
    Install,
    Upgrade,
}

fn install_or_upgrade(input: InstallInput) -> Result<(), &'static str> {
    if input.schema_version != 1 {
        return Err("request_version_unsupported");
    }
    if ![
        &input.install_root,
        &input.manifest,
        &input.signature,
        &input.artifacts,
    ]
    .into_iter()
    .all(|path| safe_path(path))
    {
        return Err("request_path_invalid");
    }
    let (architecture, expected_bundle) = match input.target.as_str() {
        "x86_64-unknown-linux-gnu" => (Architecture::X86_64, "asb-tui-v1-linux-x86_64"),
        "aarch64-unknown-linux-gnu" => (Architecture::Aarch64, "asb-tui-v1-linux-aarch64"),
        _ => return Err("request_compatibility_invalid"),
    };
    if input.asb_version != "0.1.0"
        || input.protocol_version != 1
        || !valid_version(&input.expected_release)
        || !valid_hex(&input.expected_source_commit, 40)
        || !valid_hex(&input.expected_source_tree, 40)
        || !valid_hex(&input.expected_executable_sha256, 64)
    {
        return Err("request_compatibility_invalid");
    }
    let manifest_bytes = read_bounded(&input.manifest, MAX_MANIFEST_BYTES)?;
    let now_unix = system_now_unix()?;
    let manifest = verify_bundle_manifest_with_trusted_signer(
        &manifest_bytes,
        &input.signature,
        now_unix,
        ExpectedCompatibility {
            bundle: expected_bundle,
            architecture,
            asb_version: &input.asb_version,
            protocol_version: input.protocol_version,
        },
    )?;
    let (source_commit, source_tree) = manifest.source_identity();
    if manifest.release() != input.expected_release
        || source_commit != input.expected_source_commit
        || source_tree != input.expected_source_tree
        || manifest.executable_sha256() != Some(input.expected_executable_sha256.as_str())
    {
        return Err("request_release_identity_mismatch");
    }
    if !channel_permits_install() {
        return Err("release_channel_unverified");
    }
    let verified = verify_local_bundle_artifacts(&manifest, &input.artifacts)?;
    ensure_private_root(&input.install_root)?;
    let mut store =
        FilesystemLifecycle::open(&input.install_root).map_err(|_| "install_root_unavailable")?;
    enforce_operation(&store, input.operation, manifest.release())?;
    install(&manifest, &verified, &mut store, &mut ExecutableSelfTest).map(|_| ())
}

fn system_now_unix() -> Result<u64, &'static str> {
    bounded_unix_time(SystemTime::now())
}

fn bounded_unix_time(time: SystemTime) -> Result<u64, &'static str> {
    let now = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system_clock_invalid")?
        .as_secs();
    (1_704_067_200..=4_102_444_800)
        .contains(&now)
        .then_some(now)
        .ok_or("system_clock_invalid")
}

fn enforce_operation(
    store: &FilesystemLifecycle,
    operation: InstallOperation,
    candidate_release: &str,
) -> Result<(), &'static str> {
    let active = store
        .active()
        .map_err(|_| "installation_verification_failed")?;
    match (operation, active) {
        (InstallOperation::Install, None) => Ok(()),
        (InstallOperation::Install, Some(_)) => Err("extension_already_installed"),
        (InstallOperation::Upgrade, None) => Err("extension_not_installed"),
        (InstallOperation::Upgrade, Some(active)) => {
            if !status(store).verified {
                return Err("installation_verification_failed");
            }
            (version_parts(candidate_release)? > version_parts(&active.release)?)
                .then_some(())
                .ok_or("upgrade_not_newer")
        }
    }
}

fn version_parts(value: &str) -> Result<(u64, u64, u64), &'static str> {
    let mut parts = value
        .strip_prefix('v')
        .ok_or("request_compatibility_invalid")?
        .split('.');
    let version = (
        parts.next().and_then(|part| part.parse().ok()),
        parts.next().and_then(|part| part.parse().ok()),
        parts.next().and_then(|part| part.parse().ok()),
    );
    match version {
        (Some(major), Some(minor), Some(patch)) if parts.next().is_none() => {
            Ok((major, minor, patch))
        }
        _ => Err("request_compatibility_invalid"),
    }
}

fn valid_version(value: &str) -> bool {
    value.len() <= 32 && version_parts(value).is_ok()
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn ensure_private_root(root: &Path) -> Result<(), &'static str> {
    if root.exists() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(root)
        .map_err(|_| "install_root_create_failed")
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, &'static str> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path).map_err(|_| "manifest_unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err("manifest_unavailable");
    }
    let mut file = File::open(path).map_err(|_| "manifest_unavailable")?;
    let opened = file.metadata().map_err(|_| "manifest_unavailable")?;
    if (metadata.dev(), metadata.ino()) != (opened.dev(), opened.ino()) {
        return Err("manifest_unavailable");
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "manifest_unavailable")?;
    let after = file.metadata().map_err(|_| "manifest_unavailable")?;
    if bytes.is_empty()
        || bytes.len() as u64 > maximum
        || (after.dev(), after.ino(), after.len())
            != (opened.dev(), opened.ino(), bytes.len() as u64)
    {
        Err("manifest_unavailable")
    } else {
        Ok(bytes)
    }
}

fn safe_path(path: &Path) -> bool {
    let length = path.as_os_str().as_encoded_bytes().len();
    path.is_absolute() && length != 0 && length <= 4096
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::Installation;
    use std::time::Duration;

    #[test]
    fn injected_clock_seam_rejects_pre_epoch_stale_and_implausible_time() {
        assert_eq!(
            bounded_unix_time(UNIX_EPOCH - Duration::from_secs(1)),
            Err("system_clock_invalid")
        );
        assert_eq!(
            bounded_unix_time(UNIX_EPOCH + Duration::from_secs(1_704_067_199)),
            Err("system_clock_invalid")
        );
        assert_eq!(
            bounded_unix_time(UNIX_EPOCH + Duration::from_secs(4_102_444_801)),
            Err("system_clock_invalid")
        );
        assert_eq!(
            bounded_unix_time(UNIX_EPOCH + Duration::from_secs(1_800_000_000)),
            Ok(1_800_000_000)
        );
    }

    #[test]
    fn release_versions_are_numeric_and_order_without_lexical_downgrades() {
        assert!(version_parts("v1.10.0").unwrap() > version_parts("v1.9.9").unwrap());
        assert!(version_parts("v2.0.0").unwrap() > version_parts("v1.99.99").unwrap());
        assert!(version_parts("1.0.0").is_err());
        assert!(version_parts("v1.0.0-extra").is_err());
        assert!(version_parts("v1.0").is_err());
    }

    #[test]
    fn delegated_operation_policy_rejects_reinstall_and_unverified_upgrade() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "asb-tui-delegated-policy-{}-{nonce}",
            std::process::id()
        ));
        ensure_private_root(&root).unwrap();
        ensure_private_root(&root).unwrap();
        let mut store = FilesystemLifecycle::open(&root).unwrap();
        assert!(enforce_operation(&store, InstallOperation::Install, "v1.0.0").is_ok());
        assert_eq!(
            enforce_operation(&store, InstallOperation::Upgrade, "v1.0.0"),
            Err("extension_not_installed")
        );
        let installation = Installation {
            schema_version: 1,
            release: "v1.0.0".into(),
            executable_sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                .into(),
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            target: "x86_64-unknown-linux-gnu".into(),
            bundle: "asb-tui-v1-linux-x86_64".into(),
            asb_version: "0.1.0".into(),
            protocol_version: 1,
            coordinator_version: crate::compatibility::COORDINATOR_VERSION.into(),
            coordinator_commit: crate::compatibility::COORDINATOR_COMMIT.into(),
            quality_version: crate::compatibility::QUALITY_VERSION.into(),
            quality_commit: crate::compatibility::QUALITY_COMMIT.into(),
            classification: "verified_extension".into(),
        };
        store.stage(&installation, b"hello").unwrap();
        store.activate(&installation).unwrap();
        assert_eq!(
            enforce_operation(&store, InstallOperation::Install, "v1.1.0"),
            Err("extension_already_installed")
        );
        assert_eq!(
            enforce_operation(&store, InstallOperation::Upgrade, "v1.1.0"),
            Err("installation_verification_failed")
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
