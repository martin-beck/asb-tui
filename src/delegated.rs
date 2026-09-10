// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed JSON lifecycle request boundary delegated by a future ASB CLI router.

use crate::{
    bundle::{ExpectedCompatibility, verify_bundle_manifest, verify_local_bundle_artifacts},
    compatibility::Architecture,
    lifecycle::{
        ExecutableSelfTest, FilesystemLifecycle, ProcessLauncher, install, launch, remove, status,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
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
        allowed_signers: PathBuf,
        artifacts: PathBuf,
        now_unix: u64,
        expected_bundle: String,
        architecture: Architecture,
        asb_version: String,
    },
    Upgrade {
        schema_version: u64,
        install_root: PathBuf,
        manifest: PathBuf,
        signature: PathBuf,
        allowed_signers: PathBuf,
        artifacts: PathBuf,
        now_unix: u64,
        expected_bundle: String,
        architecture: Architecture,
        asb_version: String,
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
}

impl LifecycleResponse {
    fn result(ok: bool, code: &'static str) -> Self {
        Self {
            schema_version: 1,
            classification: "unverified_extension",
            ok,
            code,
            installed: None,
            verified: None,
            release: None,
            executable_sha256: None,
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
            let Ok(store) = FilesystemLifecycle::open(&install_root) else {
                return LifecycleResponse::result(false, "install_root_unavailable");
            };
            let observed = status(&store);
            return LifecycleResponse {
                schema_version: 1,
                classification: "unverified_extension",
                ok: observed.verified,
                code: observed.reason,
                installed: Some(observed.installed),
                verified: Some(observed.verified),
                release: observed.release,
                executable_sha256: observed.executable_sha256,
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
            allowed_signers,
            artifacts,
            now_unix,
            expected_bundle,
            architecture,
            asb_version,
        } => install_or_upgrade(InstallInput {
            schema_version,
            install_root,
            manifest,
            signature,
            allowed_signers,
            artifacts,
            now_unix,
            expected_bundle,
            architecture,
            asb_version,
        })
        .map(|()| "extension_installed"),
        LifecycleRequest::Upgrade {
            schema_version,
            install_root,
            manifest,
            signature,
            allowed_signers,
            artifacts,
            now_unix,
            expected_bundle,
            architecture,
            asb_version,
        } => install_or_upgrade(InstallInput {
            schema_version,
            install_root,
            manifest,
            signature,
            allowed_signers,
            artifacts,
            now_unix,
            expected_bundle,
            architecture,
            asb_version,
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
    allowed_signers: PathBuf,
    artifacts: PathBuf,
    now_unix: u64,
    expected_bundle: String,
    architecture: Architecture,
    asb_version: String,
}

fn install_or_upgrade(input: InstallInput) -> Result<(), &'static str> {
    if input.schema_version != 1 {
        return Err("request_version_unsupported");
    }
    if ![
        &input.install_root,
        &input.manifest,
        &input.signature,
        &input.allowed_signers,
        &input.artifacts,
    ]
    .into_iter()
    .all(|path| safe_path(path))
    {
        return Err("request_path_invalid");
    }
    let expected_for_architecture = match input.architecture {
        Architecture::X86_64 => "asb-tui-v1-linux-x86_64",
        Architecture::Aarch64 => "asb-tui-v1-linux-aarch64",
        Architecture::Other => return Err("request_compatibility_invalid"),
    };
    if input.asb_version != "0.1.0" || input.expected_bundle != expected_for_architecture {
        return Err("request_compatibility_invalid");
    }
    ensure_private_root(&input.install_root)?;
    let manifest_bytes = read_bounded(&input.manifest, MAX_MANIFEST_BYTES)?;
    let manifest = verify_bundle_manifest(
        &manifest_bytes,
        &input.signature,
        &input.allowed_signers,
        input.now_unix,
        ExpectedCompatibility {
            bundle: &input.expected_bundle,
            architecture: input.architecture,
            asb_version: &input.asb_version,
            protocol_version: 1,
        },
    )?;
    let verified = verify_local_bundle_artifacts(&manifest, &input.artifacts)?;
    let mut store =
        FilesystemLifecycle::open(&input.install_root).map_err(|_| "install_root_unavailable")?;
    install(&manifest, &verified, &mut store, &mut ExecutableSelfTest).map(|_| ())
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
