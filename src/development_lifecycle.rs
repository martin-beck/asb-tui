// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Credential-free, explicitly development-only temporary lifecycle.

use crate::development_channel_manifest::{ConsumedManifest, consume, consume_from_env};
#[cfg(not(test))]
use crate::release_channel::compiled_target;
use crate::sha256::digest_hex;
use rustix::fs::{Mode, OFlags, openat};
#[cfg(not(test))]
use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};
use serde::Serialize;
use std::{
    env, fs,
    fs::File,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    os::unix::io::AsRawFd,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(not(test))]
use std::{
    io::ErrorKind,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::{Mutex, OnceLock, atomic::AtomicUsize};

const CHANNEL: &str = "dev";
const DEFAULT_REPOSITORY: &str = "https://github.com/martin-beck/asb-tui.git";
#[cfg(not(test))]
const MAX_WORKSPACE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(not(test))]
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
#[cfg(not(test))]
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[cfg(test)]
static FAIL_RENAME_AT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
static ACTIVATION_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(test)]
fn activation_test_guard() -> std::sync::MutexGuard<'static, ()> {
    ACTIVATION_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("activation test lock")
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Response {
    pub schema_version: u64,
    pub classification: &'static str,
    pub development_only: bool,
    pub channel: &'static str,
    pub ok: bool,
    pub code: &'static str,
    pub installed: bool,
    pub verified: bool,
    pub source_commit: Option<String>,
    pub source_tree: Option<String>,
    pub executable_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asb_source_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asb_source_tree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_manifest_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<&'static str>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema_version: u64,
    channel: String,
    development_only: bool,
    source_commit: String,
    source_tree: String,
    executable_sha256: String,
    source_repository: String,
    #[serde(default)]
    asb_source_commit: Option<String>,
    #[serde(default)]
    asb_source_tree: Option<String>,
    #[serde(default)]
    channel_manifest_sha256: Option<String>,
    #[serde(default)]
    group_writable_rustup_paths: bool,
}

const GROUP_WRITABLE_RUSTUP_PATH_WARNING: &str =
    "development_user_owned_group_writable_rustup_paths_allowed";

fn response(code: &'static str, ok: bool) -> Response {
    Response {
        schema_version: 1,
        classification: "development_only",
        development_only: true,
        channel: CHANNEL,
        ok,
        code,
        installed: false,
        verified: false,
        source_commit: None,
        source_tree: None,
        executable_sha256: None,
        asb_source_commit: None,
        asb_source_tree: None,
        channel_manifest_sha256: None,
        warnings: None,
    }
}
fn state_path(root: &Path) -> PathBuf {
    root.join("provenance.json")
}
fn executable_path(root: &Path) -> PathBuf {
    root.join("asb-tui")
}

fn root() -> Result<PathBuf, &'static str> {
    let root = env::var_os("ASB_TUI_DEV_INSTALL_ROOT").map_or_else(
        || {
            env::temp_dir().join(format!(
                "asb-tui-dev-{}",
                rustix::process::getuid().as_raw()
            ))
        },
        PathBuf::from,
    );
    if !root.is_absolute() || root.as_os_str().len() > 4096 {
        return Err("development_root_invalid");
    }
    Ok(root)
}

fn private_root(root: &Path) -> Result<(), &'static str> {
    if root.exists() {
        let metadata = fs::symlink_metadata(root).map_err(|_| "development_root_invalid")?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err("development_root_invalid");
        }
    } else {
        fs::create_dir(root).map_err(|_| "development_root_create_failed")?;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "development_root_create_failed")?;
    }
    Ok(())
}

fn read_state(root: &Path) -> Result<Option<State>, &'static str> {
    let path = state_path(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("development_state_unavailable"),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16 * 1024 {
        return Err("development_state_invalid");
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| "development_state_unavailable")?)
        .map_err(|_| "development_state_invalid")
}

fn valid_identity(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn rename_entry(from: &Path, to: &Path, ordinal: usize) -> Result<(), &'static str> {
    #[cfg(test)]
    if FAIL_RENAME_AT.load(Ordering::SeqCst) == ordinal {
        return Err("development_activation_failed");
    }
    #[cfg(not(test))]
    let _ = ordinal;
    fs::rename(from, to).map_err(|_| "development_activation_failed")
}

fn status(root: &Path) -> Response {
    let Ok(Some(state)) = read_state(root) else {
        return response("development_not_installed", false);
    };
    let Ok(bytes) = fs::read(executable_path(root)) else {
        return response("development_installation_invalid", false);
    };
    let manifest = root.join("channel-manifest.json");
    let manifest_result = if manifest.exists() {
        Some(consume(&manifest))
    } else {
        None
    };
    let manifest_valid = match manifest_result.as_ref() {
        Some(Ok(value)) => {
            value.manifest.tui_source_commit == state.source_commit
                && value.manifest.tui_source_tree == state.source_tree
                && value.manifest.validate_executable(&bytes).is_ok()
                && state.channel_manifest_sha256.as_deref() == Some(value.sha256.as_str())
        }
        Some(Err(_)) => false,
        None => state.channel_manifest_sha256.is_none(),
    };
    let valid = state.schema_version == 1
        && state.channel == CHANNEL
        && state.development_only
        && state.source_repository == dev_repository()
        && valid_identity(&state.source_commit)
        && valid_identity(&state.source_tree)
        && digest_hex(&bytes) == state.executable_sha256
        && manifest_valid;
    let code = match manifest_result.as_ref() {
        Some(Err(error)) => error.code(),
        Some(Ok(value)) if !manifest_valid => {
            if value.manifest.tui_source_commit != state.source_commit
                || value.manifest.tui_source_tree != state.source_tree
            {
                "dev_channel_manifest_stale"
            } else {
                "dev_channel_manifest_digest_mismatch"
            }
        }
        None if state.channel_manifest_sha256.is_some() => "dev_channel_manifest_unavailable",
        _ if valid => "development_installed",
        _ => "development_installation_invalid",
    };
    let mut result = response(code, valid);
    result.installed = true;
    result.verified = valid;
    result.source_commit = Some(state.source_commit);
    result.source_tree = Some(state.source_tree);
    result.executable_sha256 = Some(state.executable_sha256);
    result.asb_source_commit = state.asb_source_commit;
    result.asb_source_tree = state.asb_source_tree;
    result.channel_manifest_sha256 = state.channel_manifest_sha256;
    if state.group_writable_rustup_paths {
        result.warnings = Some(vec![GROUP_WRITABLE_RUSTUP_PATH_WARNING]);
    }
    result
}

fn dev_repository() -> String {
    env::var("ASB_TUI_DEV_REPOSITORY").unwrap_or_else(|_| DEFAULT_REPOSITORY.to_owned())
}

#[cfg(not(test))]
fn trusted_executable(variable: &str, name: &str) -> Result<PathBuf, &'static str> {
    let candidates = env::var_os(variable)
        .map(|value| vec![PathBuf::from(value)])
        .unwrap_or_else(|| {
            env::var_os("PATH")
                .unwrap_or_default()
                .to_string_lossy()
                .split(':')
                .filter(|part| !part.is_empty())
                .map(|part| Path::new(part).join(name))
                .collect()
        });
    for candidate in candidates {
        if let Some(trusted) = trusted_candidate(&candidate) {
            return Ok(trusted);
        }
    }
    Err("development_tool_unavailable")
}

#[cfg(not(test))]
fn trusted_candidate(candidate: &Path) -> Option<PathBuf> {
    let is_symlink = fs::symlink_metadata(candidate)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    let canonical = fs::canonicalize(candidate).ok()?;
    let metadata = fs::symlink_metadata(&canonical).ok()?;
    if metadata.is_file()
        && (metadata.uid() == rustix::process::getuid().as_raw() || metadata.uid() == 0)
        && metadata.mode() & 0o022 == 0
    {
        // Preserve a trusted shim's argv[0] (notably cargo -> rustup),
        // while validating the resolved target's ownership and mode.
        Some(if is_symlink {
            candidate.to_owned()
        } else {
            canonical
        })
    } else {
        None
    }
}

#[cfg(not(test))]
fn trusted_rustup_home(candidate: &Path) -> Option<PathBuf> {
    if let Some(value) = env::var_os("ASB_TUI_DEV_RUSTUP_HOME") {
        return Some(PathBuf::from(value));
    }

    // A rustup shim in the conventional user cargo directory has an
    // unambiguous adjacent rustup home.  Do not consult ambient RUSTUP_HOME:
    // development builds must not inherit an unvalidated global override.
    let cargo_bin = candidate.parent()?;
    if cargo_bin.file_name()? != "bin" || cargo_bin.parent()?.file_name()? != ".cargo" {
        return None;
    }
    Some(cargo_bin.parent()?.parent()?.join(".rustup"))
}

fn trusted_owner_and_mode(owner: u32, mode: u32, uid: u32, allow_group: bool) -> bool {
    (owner == 0 || owner == uid)
        && mode & 0o002 == 0
        && (mode & 0o020 == 0 || (allow_group && owner == uid))
}

fn rustup_default_toolchain(home: &Path) -> Option<(String, bool)> {
    let uid = rustix::process::geteuid().as_raw();
    let home_link = fs::symlink_metadata(home).ok()?;
    let directory = File::open(home).ok()?;
    let home_metadata = directory.metadata().ok()?;
    if !home_metadata.is_dir()
        || home_link.file_type().is_symlink()
        || home_link.dev() != home_metadata.dev()
        || home_link.ino() != home_metadata.ino()
        || !trusted_owner_and_mode(home_metadata.uid(), home_metadata.mode(), uid, false)
    {
        return None;
    }
    let settings = File::from(
        openat(
            &directory,
            "settings.toml",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .ok()?,
    );
    let metadata = settings.metadata().ok()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o002 != 0
        || metadata.len() > 64 * 1024
    {
        return None;
    }
    let group_writable = metadata.mode() & 0o020 != 0;
    let mut bytes = Vec::new();
    settings.take(64 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let contents = std::str::from_utf8(&bytes).ok()?;
    let value = contents
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("default_toolchain")?.split_once('='))?
        .1
        .trim()
        .strip_prefix('"')?
        .strip_suffix('"')?;
    if value.is_empty()
        || value.len() > 256
        || Path::new(value).components().count() != 1
        || !matches!(
            Path::new(value).components().next(),
            Some(std::path::Component::Normal(_))
        )
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some((value.to_owned(), group_writable))
}

#[derive(Debug)]
struct RustupToolchain {
    cargo: PathBuf,
    rustc: PathBuf,
    cargo_file: File,
    rustc_file: File,
    group_writable: bool,
}

fn open_rustup_tool(directory: &File, name: &str, uid: u32) -> Option<File> {
    let file = File::from(
        openat(
            directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .ok()?,
    );
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return None;
    }
    Some(file)
}

fn resolve_rustup_toolchain(home: &Path) -> Option<RustupToolchain> {
    let uid = rustix::process::geteuid().as_raw();
    let root = fs::canonicalize(home).ok()?;
    if root != home {
        return None;
    }
    let root_link = fs::symlink_metadata(&root).ok()?;
    let mut directory = File::open(&root).ok()?;
    let root_metadata = directory.metadata().ok()?;
    if !root_metadata.is_dir()
        || root_link.file_type().is_symlink()
        || root_link.dev() != root_metadata.dev()
        || root_link.ino() != root_metadata.ino()
        || !trusted_owner_and_mode(root_metadata.uid(), root_metadata.mode(), uid, false)
    {
        return None;
    }
    let (toolchain, settings_group_writable) = rustup_default_toolchain(&root)?;
    let mut group_writable = settings_group_writable;
    for component in ["toolchains", toolchain.as_str(), "bin"] {
        let next = File::from(
            openat(
                &directory,
                component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .ok()?,
        );
        let metadata = next.metadata().ok()?;
        if !metadata.is_dir() || !trusted_owner_and_mode(metadata.uid(), metadata.mode(), uid, true)
        {
            return None;
        }
        group_writable |= metadata.mode() & 0o020 != 0;
        directory = next;
    }
    let cargo_file = open_rustup_tool(&directory, "cargo", uid)?;
    let rustc_file = open_rustup_tool(&directory, "rustc", uid)?;
    let bin = root.join("toolchains").join(toolchain).join("bin");
    Some(RustupToolchain {
        cargo: bin.join("cargo"),
        rustc: bin.join("rustc"),
        cargo_file,
        rustc_file,
        group_writable,
    })
}

#[derive(Debug)]
struct TrustedCargoResolution {
    path: PathBuf,
    rustup: Option<RustupToolchain>,
    shim_group_writable: bool,
}

#[cfg(not(test))]
fn trusted_cargo_resolution(candidate: &Path) -> Option<TrustedCargoResolution> {
    let canonical = fs::canonicalize(candidate).ok()?;
    let metadata = fs::symlink_metadata(&canonical).ok()?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return None;
    }
    if canonical.file_name() != Some(std::ffi::OsStr::new("rustup")) {
        return trusted_candidate(candidate).map(|path| TrustedCargoResolution {
            path,
            rustup: None,
            shim_group_writable: false,
        });
    }
    let home = trusted_rustup_home(candidate)?;
    trusted_rustup_cargo_resolution(candidate, &canonical, &home)
}

fn trusted_rustup_cargo_resolution(
    candidate: &Path,
    canonical: &Path,
    home: &Path,
) -> Option<TrustedCargoResolution> {
    let shim_group_writable = validate_conventional_rustup_shim(candidate, canonical)?;
    let rustup = resolve_rustup_toolchain(home)?;
    Some(TrustedCargoResolution {
        path: rustup.cargo.clone(),
        rustup: Some(rustup),
        shim_group_writable,
    })
}

fn validate_conventional_rustup_shim(candidate: &Path, canonical: &Path) -> Option<bool> {
    if !candidate.is_absolute()
        || !matches!(candidate.file_name()?.to_str(), Some("cargo" | "rustc"))
        || candidate.parent()?.file_name()? != "bin"
        || candidate.parent()?.parent()?.file_name()? != ".cargo"
    {
        return None;
    }
    let uid = rustix::process::geteuid().as_raw();
    let link = fs::symlink_metadata(candidate).ok()?;
    if !link.file_type().is_symlink() || link.uid() != uid {
        return None;
    }
    if canonical != candidate.parent()?.join("rustup") {
        return None;
    }
    let cargo_root = candidate.parent()?.parent()?;
    let shim_bin = candidate.parent()?;
    let mut current = PathBuf::from("/");
    let mut group_writable = false;
    for component in shim_bin.components() {
        if let std::path::Component::Normal(name) = component {
            current.push(name);
            let metadata = fs::symlink_metadata(&current).ok()?;
            let allow_group = current == cargo_root || current == shim_bin;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || !trusted_owner_and_mode(metadata.uid(), metadata.mode(), uid, allow_group)
            {
                return None;
            }
            group_writable |= allow_group && metadata.mode() & 0o020 != 0;
        }
    }
    Some(group_writable)
}

#[cfg(not(test))]
fn trusted_toolchain_sibling(cargo: &Path, name: &str) -> Result<PathBuf, &'static str> {
    let resolved_cargo = fs::canonicalize(cargo).map_err(|_| "development_tool_unavailable")?;
    let bin = resolved_cargo
        .parent()
        .filter(|path| path.file_name().and_then(|value| value.to_str()) == Some("bin"))
        .ok_or("development_tool_unavailable")?;
    trusted_candidate(&bin.join(name)).ok_or("development_tool_unavailable")
}

#[cfg(not(test))]
fn trusted_tools() -> Result<(PathBuf, PathBuf, TrustedCargoResolution, String), &'static str> {
    let setsid = trusted_executable("ASB_TUI_DEV_SETSID", "setsid")?;
    let git = trusted_executable("ASB_TUI_DEV_GIT", "git")?;
    let cargo_candidates = env::var_os("ASB_TUI_DEV_CARGO")
        .map(|value| vec![PathBuf::from(value)])
        .unwrap_or_else(|| {
            env::var_os("PATH")
                .unwrap_or_default()
                .to_string_lossy()
                .split(':')
                .filter(|part| !part.is_empty())
                .map(|part| Path::new(part).join("cargo"))
                .collect()
        });
    let cargo = cargo_candidates
        .iter()
        .find_map(|candidate| trusted_cargo_resolution(candidate))
        .ok_or("development_tool_unavailable")?;
    let mut path = std::collections::BTreeSet::new();
    for tool in [&setsid, &git, &cargo.path] {
        if let Some(parent) = tool.parent() {
            path.insert(parent.to_string_lossy().into_owned());
        }
    }
    Ok((
        setsid,
        git,
        cargo,
        path.into_iter().collect::<Vec<_>>().join(":"),
    ))
}

#[cfg(not(test))]
struct DevelopmentToolchain {
    setsid: PathBuf,
    git: PathBuf,
    cargo: PathBuf,
    path: String,
    rustc: PathBuf,
    cc: PathBuf,
    ar: PathBuf,
    ld: PathBuf,
    cargo_file: Option<File>,
    rustc_file: Option<File>,
    group_writable_rustup_paths: bool,
}

#[cfg(not(test))]
impl DevelopmentToolchain {
    fn cargo_execution_path(&self) -> PathBuf {
        self.cargo_file
            .as_ref()
            .map(descriptor_path)
            .unwrap_or_else(|| self.cargo.clone())
    }

    fn rustc_execution_path(&self) -> PathBuf {
        self.rustc_file
            .as_ref()
            .map(descriptor_path)
            .unwrap_or_else(|| self.rustc.clone())
    }

    fn make_descriptors_inheritable(
        &self,
    ) -> Result<(Option<FdFlags>, Option<FdFlags>), &'static str> {
        let cargo = self
            .cargo_file
            .as_ref()
            .map(make_descriptor_inheritable)
            .transpose()?;
        match self
            .rustc_file
            .as_ref()
            .map(make_descriptor_inheritable)
            .transpose()
        {
            Ok(rustc) => Ok((cargo, rustc)),
            Err(error) => {
                if let (Some(file), Some(flags)) = (self.cargo_file.as_ref(), cargo) {
                    let _ = fcntl_setfd(file, flags);
                }
                Err(error)
            }
        }
    }

    fn restore_descriptor_flags(&self, flags: (Option<FdFlags>, Option<FdFlags>)) {
        if let (Some(file), Some(flags)) = (self.cargo_file.as_ref(), flags.0) {
            let _ = fcntl_setfd(file, flags);
        }
        if let (Some(file), Some(flags)) = (self.rustc_file.as_ref(), flags.1) {
            let _ = fcntl_setfd(file, flags);
        }
    }
}

#[cfg(not(test))]
fn descriptor_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(not(test))]
fn make_descriptor_inheritable(file: &File) -> Result<FdFlags, &'static str> {
    let flags = fcntl_getfd(file).map_err(|_| "development_tool_unavailable")?;
    fcntl_setfd(file, flags - FdFlags::CLOEXEC).map_err(|_| "development_tool_unavailable")?;
    Ok(flags)
}

#[cfg(not(test))]
fn trusted_toolchain() -> Result<DevelopmentToolchain, &'static str> {
    let (setsid, git, cargo_resolution, path) = trusted_tools()?;
    let TrustedCargoResolution {
        path: cargo,
        rustup,
        shim_group_writable,
    } = cargo_resolution;
    let (rustc, cargo_file, rustc_file, group_writable_rustup_paths) = if let Some(rustup) = rustup
    {
        if let Some(override_path) = env::var_os("ASB_TUI_DEV_RUSTC") {
            let configured = trusted_executable("ASB_TUI_DEV_RUSTC", "rustc")?;
            let resolved =
                fs::canonicalize(&configured).map_err(|_| "development_tool_unavailable")?;
            if !rustc_override_matches_toolchain(
                &configured,
                &resolved,
                &PathBuf::from(override_path),
                &rustup.rustc,
            ) {
                return Err("development_tool_unavailable");
            }
        }
        (
            rustup.rustc,
            Some(rustup.cargo_file),
            Some(rustup.rustc_file),
            rustup.group_writable || shim_group_writable,
        )
    } else {
        let rustc = match env::var_os("ASB_TUI_DEV_RUSTC") {
            Some(_) => trusted_rustc_override(&cargo).map_err(|_| "dbg_rustc")?,
            None => trusted_toolchain_sibling(&cargo, "rustc").map_err(|_| "dbg_rustc")?,
        };
        (rustc, None, None, shim_group_writable)
    };
    Ok(DevelopmentToolchain {
        setsid,
        git,
        cargo,
        path,
        rustc,
        cc: trusted_executable("ASB_TUI_DEV_CC", "cc")?,
        ar: trusted_executable("ASB_TUI_DEV_AR", "ar")?,
        ld: trusted_executable("ASB_TUI_DEV_LD", "ld")?,
        cargo_file,
        rustc_file,
        group_writable_rustup_paths,
    })
}

fn rustc_override_matches_toolchain(
    configured: &Path,
    resolved: &Path,
    requested: &Path,
    expected: &Path,
) -> bool {
    (resolved.file_name() == Some(std::ffi::OsStr::new("rustup"))
        && validate_conventional_rustup_shim(configured, resolved).is_some())
        || configured == expected
        || requested == expected
}

#[cfg(not(test))]
fn trusted_rustc_override(cargo: &Path) -> Result<PathBuf, &'static str> {
    let configured = trusted_executable("ASB_TUI_DEV_RUSTC", "rustc")?;
    let resolved = fs::canonicalize(&configured).map_err(|_| "development_tool_unavailable")?;
    if resolved.file_name() == Some(std::ffi::OsStr::new("rustup")) {
        // A rustup rustc shim cannot be used after the build environment is
        // cleared: it would consult an ambient RUSTUP_HOME. Resolve it to the
        // same already validated toolchain as cargo instead.
        trusted_toolchain_sibling(cargo, "rustc")
    } else {
        Ok(configured)
    }
}

#[cfg(not(test))]
fn dev_ref() -> String {
    env::var("ASB_TUI_DEV_REF").unwrap_or_else(|_| "main".to_owned())
}

#[cfg(not(test))]
fn bounded_size(root: &Path, limit: u64) -> Result<u64, &'static str> {
    fn visit(path: &Path, total: &mut u64, limit: u64) -> Result<(), &'static str> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("development_workspace_unavailable"),
        };
        for entry in entries {
            let entry = entry.map_err(|_| "development_workspace_unavailable")?;
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(_) => return Err("development_workspace_unavailable"),
            };
            if metadata.file_type().is_symlink() {
                return Err("development_workspace_invalid");
            }
            *total = total.saturating_add(metadata.len());
            if *total > limit {
                return Ok(());
            }
            if metadata.is_dir() {
                visit(&entry.path(), total, limit)?;
            }
        }
        Ok(())
    }
    let mut total = 0;
    visit(root, &mut total, limit)?;
    Ok(total)
}

#[cfg(not(test))]
fn run_bounded(
    mut command: Command,
    workspace: &Path,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, &'static str> {
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| "development_tool_unavailable")?;
    let pid =
        rustix::process::Pid::from_raw(child.id() as i32).ok_or("development_tool_unavailable")?;
    let mut stdout = child.stdout.take().ok_or("development_tool_unavailable")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        let mut oversized = false;
        loop {
            let count = stdout.read(&mut buffer).map_err(|_| ())?;
            if count == 0 {
                break;
            }
            let room = MAX_OUTPUT_BYTES.saturating_sub(bytes.len());
            bytes.extend_from_slice(&buffer[..count.min(room)]);
            oversized |= count > room;
        }
        Ok::<_, ()>((bytes, oversized))
    });
    let started = std::time::Instant::now();
    loop {
        if cancelled.is_some_and(|value| value.load(Ordering::Relaxed)) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err("development_cancelled");
        }
        match bounded_size(workspace, MAX_WORKSPACE_BYTES) {
            Ok(size) if size <= MAX_WORKSPACE_BYTES => {}
            Ok(_) => {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = fs::remove_dir_all(workspace);
                return Err("development_workspace_quota_exceeded");
            }
            Err(error) => {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = fs::remove_dir_all(workspace);
                return Err(error);
            }
        }
        if let Some(status) = child.try_wait().map_err(|_| "development_command_failed")? {
            // The wrapper may have exited while a descendant still owns the
            // pipe. Kill/reap the whole session before joining the reader.
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            let _ = child.wait();
            let (output, oversized) = reader
                .join()
                .map_err(|_| "development_command_failed")?
                .map_err(|_| "development_command_failed")?;
            if oversized || !status.success() {
                return Err("development_command_failed");
            }
            return Ok(output);
        }
        if started.elapsed() >= COMMAND_TIMEOUT {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err("development_command_timeout");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(not(test))]
fn source_identity(
    source: &Path,
    name: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<String, &'static str> {
    let tools = trusted_toolchain()?;
    let output = run_bounded(
        {
            let mut command = Command::new(&tools.setsid);
            command
                .args(["--wait"])
                .arg(&tools.git)
                .current_dir(source)
                .env_clear()
                .env("PATH", &tools.path)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0")
                .args(["rev-parse", name]);
            command
        },
        source,
        cancelled,
    )?;
    let value = String::from_utf8(output).map_err(|_| "development_provenance_invalid")?;
    let value = value.trim();
    if valid_identity(value) {
        Ok(value.to_owned())
    } else {
        Err("development_provenance_invalid")
    }
}

#[cfg(not(test))]
fn build_candidate(
    workspace: &Path,
    cancelled: Option<&AtomicBool>,
) -> Result<(Vec<u8>, State), &'static str> {
    let source = workspace.join("source");
    let target = workspace.join("target");
    let cargo_home = workspace.join("cargo-home");
    fs::create_dir_all(&target).map_err(|_| "development_workspace_create_failed")?;
    fs::create_dir_all(&cargo_home).map_err(|_| "development_workspace_create_failed")?;
    let repository = dev_repository();
    let reference = dev_ref();
    let tools = trusted_toolchain()?;
    let rustup_home = env::var_os("ASB_TUI_DEV_RUSTUP_HOME");
    // The clean-room paths are intentionally disposable, but rustc embeds
    // source and build paths in debuginfo and panic metadata.  Bind both
    // private roots to stable development-only names so equivalent
    // materializations have the same executable digest without changing the
    // source, toolchain, or runtime verification contract.
    let remap_flags = format!(
        "--remap-path-prefix={}=/asb-tui-build --remap-path-prefix={}=/asb-tui-cargo-home",
        workspace.display(),
        cargo_home.display()
    );
    let mut clone = Command::new(&tools.setsid);
    clone
        .args(["--wait"])
        .arg(&tools.git)
        .args([
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--branch",
            &reference,
            "--",
            &repository,
        ])
        .env_clear()
        .env("PATH", &tools.path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg(&source);
    run_bounded(clone, workspace, cancelled)?;
    let commit = source_identity(&source, "HEAD", cancelled)?;
    let tree = source_identity(&source, "HEAD^{tree}", cancelled)?;
    let mut build = Command::new(&tools.setsid);
    build
        .args(["--wait"])
        .arg(tools.cargo_execution_path())
        .current_dir(&source)
        .env_clear()
        .env("HOME", workspace)
        .env("CARGO_HOME", &cargo_home)
        .env("CARGO_TARGET_DIR", &target)
        .env("PATH", &tools.path)
        .env("RUSTC", tools.rustc_execution_path())
        .env("CC", &tools.cc)
        .env("AR", &tools.ar)
        .env("LD", &tools.ld)
        .env("RUSTC_LINKER", &tools.cc)
        .env(
            format!(
                "CARGO_TARGET_{}_LINKER",
                compiled_target().to_ascii_uppercase().replace('-', "_")
            ),
            &tools.cc,
        )
        .env(
            format!(
                "CARGO_TARGET_{}_RUSTFLAGS",
                compiled_target().to_ascii_uppercase().replace('-', "_")
            ),
            format!(
                "-C link-arg=-B{} {remap_flags}",
                tools
                    .ld
                    .parent()
                    .ok_or("development_tool_unavailable")?
                    .display()
            ),
        )
        .env("RUSTFLAGS", &remap_flags)
        .args(["build", "--locked", "--release", "--bin", "asb-tui"]);
    if let Some(rustup_home) = rustup_home {
        build.env("RUSTUP_HOME", rustup_home);
    }
    let descriptor_flags = tools.make_descriptors_inheritable()?;
    let build_result = run_bounded(build, workspace, cancelled);
    tools.restore_descriptor_flags(descriptor_flags);
    build_result?;
    let executable = target.join("release/asb-tui");
    let metadata = fs::metadata(&executable).map_err(|_| "development_build_missing")?;
    if !metadata.is_file() {
        return Err("development_build_invalid");
    }
    let bytes = fs::read(&executable).map_err(|_| "development_build_unavailable")?;
    let state = State {
        schema_version: 1,
        channel: CHANNEL.into(),
        development_only: true,
        source_commit: commit,
        source_tree: tree,
        executable_sha256: digest_hex(&bytes),
        source_repository: repository,
        asb_source_commit: None,
        asb_source_tree: None,
        channel_manifest_sha256: None,
        group_writable_rustup_paths: tools.group_writable_rustup_paths,
    };
    Ok((bytes, state))
}

#[cfg(test)]
fn build_candidate(
    _workspace: &Path,
    cancelled: Option<&AtomicBool>,
) -> Result<(Vec<u8>, State), &'static str> {
    if cancelled.is_some_and(|value| value.load(Ordering::Relaxed)) {
        return Err("development_cancelled");
    }
    let bytes = b"test-development-build".to_vec();
    Ok((
        bytes.clone(),
        State {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            executable_sha256: digest_hex(&bytes),
            source_repository: dev_repository(),
            asb_source_commit: None,
            asb_source_tree: None,
            channel_manifest_sha256: None,
            group_writable_rustup_paths: false,
        },
    ))
}

fn materialize(
    root: &Path,
    replacing: bool,
    cancelled: Option<&AtomicBool>,
) -> Result<Response, &'static str> {
    private_root(root)?;
    if !replacing && read_state(root)?.is_some() {
        return Err("development_already_installed");
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "development_clock_invalid")?
        .as_nanos();
    let stage = root.join(format!(".stage-{nonce}"));
    let workspace = root.join(format!(".workspace-{nonce}"));
    fs::create_dir(&stage).map_err(|_| "development_stage_failed")?;
    fs::create_dir(&workspace).map_err(|_| "development_workspace_create_failed")?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))
        .map_err(|_| "development_stage_failed")?;
    fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700))
        .map_err(|_| "development_workspace_create_failed")?;
    let result = (|| {
        let (bytes, mut state) = build_candidate(&workspace, cancelled)?;
        let consumed: Option<ConsumedManifest> =
            consume_from_env().map_err(|error| error.code())?;
        if let Some(value) = &consumed {
            value
                .manifest
                .validate_current_main(
                    &value.manifest.asb_source_commit,
                    &value.manifest.asb_source_tree,
                    &state.source_commit,
                    &state.source_tree,
                )
                .map_err(|error| error.code())?;
            value
                .manifest
                .validate_executable(&bytes)
                .map_err(|error| error.code())?;
            state.asb_source_commit = Some(value.manifest.asb_source_commit.clone());
            state.asb_source_tree = Some(value.manifest.asb_source_tree.clone());
            state.channel_manifest_sha256 = Some(value.sha256.clone());
        }
        if cancelled.is_some_and(|value| value.load(Ordering::Relaxed)) {
            return Err("development_cancelled");
        }
        let staged = stage.join("asb-tui");
        fs::write(&staged, &bytes).map_err(|_| "development_stage_failed")?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o700))
            .map_err(|_| "development_stage_failed")?;
        fs::write(
            stage.join("provenance.json"),
            serde_json::to_vec(&state).map_err(|_| "development_stage_failed")?,
        )
        .map_err(|_| "development_stage_failed")?;
        if let Some(value) = consumed {
            fs::write(stage.join("channel-manifest.json"), value.bytes)
                .map_err(|_| "development_stage_failed")?;
        }
        let backup = root.join(format!(".backup-{nonce}"));
        if replacing {
            fs::create_dir(&backup).map_err(|_| "development_activation_failed")?;
            for name in ["asb-tui", "provenance.json", "channel-manifest.json"] {
                let old = root.join(name);
                if old.exists() && fs::rename(&old, backup.join(name)).is_err() {
                    for restore in ["asb-tui", "provenance.json", "channel-manifest.json"] {
                        let saved = backup.join(restore);
                        if saved.exists() {
                            let _ = fs::rename(saved, root.join(restore));
                        }
                    }
                    let _ = fs::remove_dir_all(&backup);
                    return Err("development_activation_failed");
                }
            }
        }
        let activation = (|| {
            rename_entry(&stage.join("asb-tui"), &root.join("asb-tui"), 1)?;
            rename_entry(
                &stage.join("provenance.json"),
                &root.join("provenance.json"),
                2,
            )?;
            if stage.join("channel-manifest.json").exists() {
                rename_entry(
                    &stage.join("channel-manifest.json"),
                    &root.join("channel-manifest.json"),
                    3,
                )?;
            }
            Ok::<(), &'static str>(())
        })();
        if let Err(error) = activation {
            // Restore both old entries before reporting failure; a failed
            // upgrade must never leave a half-new installation active.
            for name in ["asb-tui", "provenance.json", "channel-manifest.json"] {
                let current = root.join(name);
                if current.exists() {
                    let _ = fs::remove_file(&current);
                }
                let old = backup.join(name);
                if old.exists() {
                    let _ = fs::rename(old, current);
                }
            }
            if replacing {
                let _ = fs::remove_dir_all(&backup);
            }
            return Err(error);
        }
        if replacing {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&workspace);
    result?;
    Ok(status(root))
}

pub fn execute_with_cancel(operation: &str, cancelled: &AtomicBool) -> Response {
    let Ok(root) = root() else {
        return response("development_root_invalid", false);
    };
    match operation {
        "install" => {
            if !crate::development_preflight::run().ok {
                return response("development_host_preflight_failed", false);
            }
            materialize(&root, false, Some(cancelled)).unwrap_or_else(|code| response(code, false))
        }
        "upgrade" => {
            if read_state(&root).ok().flatten().is_none() {
                response("development_not_installed", false)
            } else if !crate::development_preflight::run().ok {
                response("development_host_preflight_failed", false)
            } else {
                materialize(&root, true, Some(cancelled))
                    .unwrap_or_else(|code| response(code, false))
            }
        }
        "status" => status(&root),
        "launch" => {
            let current = status(&root);
            if current.verified {
                let mut result = current;
                result.ok = true;
                result.code = "development_launch_ready";
                result
            } else {
                response("development_launch_unavailable", false)
            }
        }
        "remove" => {
            if !status(&root).installed {
                response("development_not_installed", false)
            } else {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|value| value.as_nanos())
                    .unwrap_or_default();
                let trash = root.join(format!(".remove-{nonce}"));
                if fs::create_dir(&trash).is_err() {
                    response("development_remove_failed", false)
                } else {
                    let moved = (|| {
                        for name in ["asb-tui", "provenance.json", "channel-manifest.json"] {
                            let current = root.join(name);
                            if current.exists() {
                                fs::rename(current, trash.join(name))
                                    .map_err(|_| "development_remove_failed")?;
                            }
                        }
                        fs::remove_dir_all(&trash).map_err(|_| "development_remove_failed")
                    })();
                    if moved.is_err() {
                        for name in ["asb-tui", "provenance.json", "channel-manifest.json"] {
                            let old = trash.join(name);
                            if old.exists() {
                                let _ = fs::rename(old, root.join(name));
                            }
                        }
                        let _ = fs::remove_dir_all(&trash);
                        response("development_remove_failed", false)
                    } else {
                        response("development_removed", true)
                    }
                }
            }
        }
        _ => response("development_operation_invalid", false),
    }
}

pub fn execute(operation: &str) -> Response {
    execute_with_cancel(operation, &AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("asb-tui-dev-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    fn rustup_fixture(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = PathBuf::from(env::var_os("HOME").unwrap())
            .join(format!("asb-tui-dev-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let cargo_root = root.join("home/.cargo");
        let shim_bin = cargo_root.join("bin");
        fs::create_dir_all(&shim_bin).unwrap();
        fs::set_permissions(root.join("home"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&cargo_root, fs::Permissions::from_mode(0o775)).unwrap();
        fs::set_permissions(&shim_bin, fs::Permissions::from_mode(0o775)).unwrap();
        fs::write(shim_bin.join("rustup"), b"rustup").unwrap();
        fs::set_permissions(shim_bin.join("rustup"), fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink("rustup", shim_bin.join("cargo")).unwrap();

        let rustup_home = root.join("rustup");
        let bin = rustup_home.join("toolchains/fixture-toolchain/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::set_permissions(&rustup_home, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            rustup_home.join("settings.toml"),
            b"version = \"12\"\ndefault_toolchain = \"fixture-toolchain\"\n",
        )
        .unwrap();
        fs::set_permissions(
            rustup_home.join("settings.toml"),
            fs::Permissions::from_mode(0o664),
        )
        .unwrap();
        for directory in [
            rustup_home.join("toolchains/fixture-toolchain"),
            bin.clone(),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o775)).unwrap();
        }
        for tool in ["cargo", "rustc"] {
            fs::write(bin.join(tool), format!("validated-{tool}")).unwrap();
            fs::set_permissions(bin.join(tool), fs::Permissions::from_mode(0o700)).unwrap();
        }
        (root, shim_bin.join("cargo"), rustup_home)
    }

    #[test]
    fn owner_group_writable_rustup_layout_is_bounded_and_warned() {
        let (root, shim, rustup_home) = rustup_fixture("owner-group-rustup");
        let canonical = fs::canonicalize(&shim).unwrap();
        assert!(validate_conventional_rustup_shim(&shim, &canonical).is_some());
        let resolved = resolve_rustup_toolchain(&rustup_home).unwrap();
        assert!(resolved.group_writable);
        assert_eq!(resolved.rustc.file_name().unwrap(), "rustc");
        assert_eq!(
            fs::read_to_string(format!("/proc/self/fd/{}", resolved.rustc_file.as_raw_fd()))
                .unwrap(),
            "validated-rustc"
        );
        assert_eq!(
            fs::read_to_string(format!("/proc/self/fd/{}", resolved.cargo_file.as_raw_fd()))
                .unwrap(),
            "validated-cargo"
        );

        let state = State {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            executable_sha256: digest_hex(b"candidate"),
            source_repository: DEFAULT_REPOSITORY.into(),
            asb_source_commit: None,
            asb_source_tree: None,
            channel_manifest_sha256: None,
            group_writable_rustup_paths: true,
        };
        fs::write(executable_path(&root), b"candidate").unwrap();
        fs::write(state_path(&root), serde_json::to_vec(&state).unwrap()).unwrap();
        let installed = status(&root);
        assert_eq!(
            installed.warnings,
            Some(vec![GROUP_WRITABLE_RUSTUP_PATH_WARNING])
        );
        let restarted = status(&root);
        assert_eq!(restarted.warnings, installed.warnings);
        let mut launched = status(&root);
        launched.code = "development_launch_ready";
        assert!(
            serde_json::to_string(&launched)
                .unwrap()
                .contains(GROUP_WRITABLE_RUSTUP_PATH_WARNING)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn group_writable_shim_dirs_alone_require_the_persisted_warning() {
        let (root, shim, rustup_home) = rustup_fixture("owner-group-shim-only");
        fs::set_permissions(
            rustup_home.join("settings.toml"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        for directory in [
            rustup_home.join("toolchains"),
            rustup_home.join("toolchains/fixture-toolchain"),
            rustup_home.join("toolchains/fixture-toolchain/bin"),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let canonical = fs::canonicalize(&shim).unwrap();
        assert_eq!(
            validate_conventional_rustup_shim(&shim, &canonical),
            Some(true)
        );
        let resolved = resolve_rustup_toolchain(&rustup_home).unwrap();
        assert!(!resolved.group_writable);
        assert!(
            validate_conventional_rustup_shim(&shim, &canonical).unwrap()
                || resolved.group_writable
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opened_rustup_tools_survive_hostile_path_replacement() {
        let (root, _shim, rustup_home) = rustup_fixture("rustup-object-binding");
        let resolved = resolve_rustup_toolchain(&rustup_home).unwrap();
        fs::rename(&resolved.cargo, resolved.cargo.with_extension("validated")).unwrap();
        fs::write(&resolved.cargo, b"substituted-cargo").unwrap();
        fs::set_permissions(&resolved.cargo, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            fs::read_to_string(format!("/proc/self/fd/{}", resolved.cargo_file.as_raw_fd()))
                .unwrap(),
            "validated-cargo"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rustup_origin_never_falls_back_to_paths_after_second_pass_drift() {
        let (root, shim, rustup_home) = rustup_fixture("rustup-no-path-fallback");
        let canonical = fs::canonicalize(&shim).unwrap();
        let selected = trusted_rustup_cargo_resolution(&shim, &canonical, &rustup_home).unwrap();
        assert_eq!(selected.path.file_name().unwrap(), "cargo");
        let rustup = selected.rustup.as_ref().unwrap();
        assert!(selected.shim_group_writable || rustup.group_writable);

        fs::remove_file(rustup_home.join("settings.toml")).unwrap();
        fs::rename(
            rustup_home.join("toolchains/fixture-toolchain"),
            rustup_home.join("toolchains/retained"),
        )
        .unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        assert_eq!(
            fs::read_to_string(format!("/proc/self/fd/{}", rustup.cargo_file.as_raw_fd())).unwrap(),
            "validated-cargo"
        );
        assert_eq!(
            fs::read_to_string(format!("/proc/self/fd/{}", rustup.rustc_file.as_raw_fd())).unwrap(),
            "validated-rustc"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hostile_rustup_layouts_fail_closed() {
        let (root, shim, rustup_home) = rustup_fixture("hostile-rustup");
        let canonical = fs::canonicalize(&shim).unwrap();
        fs::set_permissions(
            shim.parent().unwrap().parent().unwrap(),
            fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        assert!(validate_conventional_rustup_shim(&shim, &canonical).is_none());
        fs::set_permissions(
            shim.parent().unwrap().parent().unwrap(),
            fs::Permissions::from_mode(0o775),
        )
        .unwrap();

        let settings = rustup_home.join("settings.toml");
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o664)).unwrap();
        fs::write(&settings, b"default_toolchain = \"../escaped\"\n").unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::write(&settings, vec![b'x'; 64 * 1024 + 1]).unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::remove_file(&settings).unwrap();
        std::os::unix::fs::symlink("elsewhere", &settings).unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rustup_toolchain_rejects_symlinked_dirs_and_mutable_or_nonregular_tools() {
        let (root, _shim, rustup_home) = rustup_fixture("hostile-rustup-tool");
        let bin = rustup_home.join("toolchains/fixture-toolchain/bin");
        let cargo = bin.join("cargo");
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o720)).unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::remove_file(&cargo).unwrap();
        fs::create_dir(&cargo).unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::remove_dir(&cargo).unwrap();
        fs::write(&cargo, b"cargo").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let rustc = bin.join("rustc");
        fs::set_permissions(&rustc, fs::Permissions::from_mode(0o720)).unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::set_permissions(&rustc, fs::Permissions::from_mode(0o700)).unwrap();
        let selected = rustup_home.join("toolchains/fixture-toolchain");
        let retained = rustup_home.join("toolchains/retained");
        fs::rename(&selected, &retained).unwrap();
        std::os::unix::fs::symlink(&retained, &selected).unwrap();
        assert!(resolve_rustup_toolchain(&rustup_home).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owner_and_toolchain_pairing_rules_reject_wrong_owner_and_mismatch() {
        let uid = rustix::process::geteuid().as_raw();
        assert!(!trusted_owner_and_mode(
            uid.wrapping_add(1),
            0o755,
            uid,
            true
        ));
        assert!(!rustc_override_matches_toolchain(
            Path::new("/safe/other/rustc"),
            Path::new("/safe/other/rustc"),
            Path::new("/safe/other/rustc"),
            Path::new("/safe/selected/rustc"),
        ));
    }

    #[test]
    fn legacy_state_defaults_to_strict_layout_without_warning() {
        let root = temp_root("legacy-state");
        let bytes = b"candidate";
        fs::write(executable_path(&root), bytes).unwrap();
        fs::write(
            state_path(&root),
            format!(
                "{{\"schema_version\":1,\"channel\":\"dev\",\"development_only\":true,\"source_commit\":\"{}\",\"source_tree\":\"{}\",\"executable_sha256\":\"{}\",\"source_repository\":\"{}\"}}",
                "a".repeat(40),
                "b".repeat(40),
                digest_hex(bytes),
                DEFAULT_REPOSITORY,
            ),
        )
        .unwrap();
        let result = status(&root);
        assert!(result.verified);
        assert_eq!(result.warnings, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_build_provenance_fails_closed() {
        let state = State {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            source_commit: "unknown".into(),
            source_tree: "unknown".into(),
            executable_sha256: digest_hex(b"candidate"),
            source_repository: DEFAULT_REPOSITORY.into(),
            asb_source_commit: None,
            asb_source_tree: None,
            channel_manifest_sha256: None,
            group_writable_rustup_paths: false,
        };
        let root = temp_root("malformed");
        fs::write(state_path(&root), serde_json::to_vec(&state).unwrap()).unwrap();
        fs::write(executable_path(&root), b"candidate").unwrap();
        assert_eq!(status(&root).code, "development_installation_invalid");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn second_activation_rename_restores_previous_pair() {
        let _guard = activation_test_guard();
        let root = temp_root("rollback");
        assert!(materialize(&root, false, None).unwrap().verified);
        let before = status(&root);
        FAIL_RENAME_AT.store(2, Ordering::SeqCst);
        assert_eq!(
            materialize(&root, true, None),
            Err("development_activation_failed")
        );
        FAIL_RENAME_AT.store(0, Ordering::SeqCst);
        let after = status(&root);
        assert_eq!(after.code, "development_installed");
        assert_eq!(after.executable_sha256, before.executable_sha256);
        assert_eq!(after.source_commit, before.source_commit);
        assert_eq!(after.source_tree, before.source_tree);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn development_install_status_launch_upgrade_and_remove_are_reentrant() {
        let _guard = activation_test_guard();
        FAIL_RENAME_AT.store(0, Ordering::SeqCst);
        let root = temp_root("operations");
        assert_eq!(status(&root).code, "development_not_installed");
        assert_eq!(
            materialize(&root, false, None).unwrap().code,
            "development_installed"
        );
        assert_eq!(
            materialize(&root, false, None),
            Err("development_already_installed")
        );
        let installed = status(&root);
        assert!(installed.installed && installed.verified);
        assert_eq!(
            materialize(&root, true, None).unwrap().code,
            "development_installed"
        );
        assert_eq!(status(&root).code, "development_installed");
        fs::rename(root.join("asb-tui"), root.join("asb-tui.staged")).unwrap();
        assert_eq!(status(&root).code, "development_installation_invalid");
        fs::rename(root.join("asb-tui.staged"), root.join("asb-tui")).unwrap();
        assert_eq!(status(&root).code, "development_installed");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pre_cancelled_materialization_never_creates_an_install() {
        let _guard = activation_test_guard();
        let root = temp_root("cancelled");
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            materialize(&root, false, Some(&cancelled)),
            Err("development_cancelled")
        );
        assert!(!root.join("asb-tui").exists());
        assert!(!root.join("provenance.json").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn development_state_validation_rejects_every_mutable_identity() {
        let root = temp_root("state-validation");
        let bytes = b"candidate";
        fs::write(executable_path(&root), bytes).unwrap();
        let valid = State {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            executable_sha256: digest_hex(bytes),
            source_repository: DEFAULT_REPOSITORY.into(),
            asb_source_commit: None,
            asb_source_tree: None,
            channel_manifest_sha256: None,
            group_writable_rustup_paths: false,
        };
        fs::write(state_path(&root), serde_json::to_vec(&valid).unwrap()).unwrap();
        assert!(status(&root).verified);
        type Mutation = (&'static str, fn(&mut State));
        let mutations: [Mutation; 7] = [
            ("schema", |state: &mut State| state.schema_version = 2),
            ("channel", |state: &mut State| {
                state.channel = "stable".into()
            }),
            ("development", |state: &mut State| {
                state.development_only = false
            }),
            ("repository", |state: &mut State| {
                state.source_repository = "other".into()
            }),
            ("commit", |state: &mut State| {
                state.source_commit = "unknown".into()
            }),
            ("tree", |state: &mut State| {
                state.source_tree = "unknown".into()
            }),
            ("digest", |state: &mut State| {
                state.executable_sha256 = "0".repeat(64)
            }),
        ];
        for (label, mutate) in mutations {
            let mut state = valid.clone();
            mutate(&mut state);
            fs::write(state_path(&root), serde_json::to_vec(&state).unwrap()).unwrap();
            assert_eq!(
                status(&root).code,
                "development_installation_invalid",
                "{label}"
            );
        }
        fs::write(state_path(&root), serde_json::to_vec(&valid).unwrap()).unwrap();
        fs::write(executable_path(&root), b"tampered").unwrap();
        assert_eq!(status(&root).code, "development_installation_invalid");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_consumes_channel_manifest_and_binds_actual_executable_digest() {
        let root = temp_root("channel-manifest");
        let bytes = b"candidate";
        fs::write(executable_path(&root), bytes).unwrap();
        let manifest = crate::development_channel_manifest::DevelopmentChannelManifest {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            asb_repository: crate::development_channel_manifest::ASB_REPOSITORY.into(),
            asb_ref: crate::development_channel_manifest::MAIN_REF.into(),
            asb_source_commit: "c".repeat(40),
            asb_source_tree: "d".repeat(40),
            tui_repository: DEFAULT_REPOSITORY.into(),
            tui_ref: crate::development_channel_manifest::MAIN_REF.into(),
            tui_source_commit: "a".repeat(40),
            tui_source_tree: "b".repeat(40),
            executable_sha256: digest_hex(bytes),
            executable_size: bytes.len() as u64,
            built_unix: 1,
            warnings: Vec::new(),
        };
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_digest = digest_hex(&manifest_bytes);
        fs::write(root.join("channel-manifest.json"), &manifest_bytes).unwrap();
        let state = State {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            source_commit: "a".repeat(40),
            source_tree: "b".repeat(40),
            executable_sha256: digest_hex(bytes),
            source_repository: DEFAULT_REPOSITORY.into(),
            asb_source_commit: Some("c".repeat(40)),
            asb_source_tree: Some("d".repeat(40)),
            channel_manifest_sha256: Some(manifest_digest.clone()),
            group_writable_rustup_paths: false,
        };
        fs::write(state_path(&root), serde_json::to_vec(&state).unwrap()).unwrap();
        let result = status(&root);
        assert!(result.verified);
        assert_eq!(result.asb_source_commit, Some("c".repeat(40)));
        assert_eq!(result.channel_manifest_sha256, Some(manifest_digest));
        fs::write(executable_path(&root), b"tampered-executable").unwrap();
        assert_eq!(status(&root).code, "dev_channel_manifest_digest_mismatch");
        fs::write(executable_path(&root), bytes).unwrap();
        fs::write(root.join("channel-manifest.json"), b"tampered").unwrap();
        assert_eq!(status(&root).code, "dev_channel_manifest_invalid");
        fs::write(root.join("channel-manifest.json"), &manifest_bytes).unwrap();
        let mut stale = state;
        stale.source_commit = "e".repeat(40);
        fs::write(state_path(&root), serde_json::to_vec(&stale).unwrap()).unwrap();
        assert_eq!(status(&root).code, "dev_channel_manifest_stale");
        fs::write(state_path(&root), serde_json::to_vec(&stale).unwrap()).unwrap();
        fs::remove_file(root.join("channel-manifest.json")).unwrap();
        assert_eq!(status(&root).code, "dev_channel_manifest_unavailable");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn development_root_and_state_fail_closed_for_unsafe_filesystem_entries() {
        let root = temp_root("unsafe");
        assert!(private_root(&root).is_ok());
        let mode = fs::metadata(&root).unwrap().permissions().mode();
        fs::set_permissions(&root, fs::Permissions::from_mode(mode | 0o077)).unwrap();
        assert_eq!(private_root(&root), Err("development_root_invalid"));
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(state_path(&root), vec![b'x'; 16 * 1024 + 1]).unwrap();
        assert!(matches!(
            read_state(&root),
            Err("development_state_invalid")
        ));
        fs::remove_file(state_path(&root)).unwrap();
        fs::create_dir(state_path(&root)).unwrap();
        assert!(matches!(
            read_state(&root),
            Err("development_state_invalid")
        ));
        fs::remove_dir(state_path(&root)).unwrap();
        assert_eq!(read_state(&root), Ok(None));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rustup_cargo_shim_resolves_only_to_private_default_toolchain_binary() {
        let _guard = activation_test_guard();
        let root = temp_root("rustup-shim");
        let home = root.join("rustup");
        let shim_bin = root.join("cargo-bin");
        let toolchain_bin = home.join("toolchains/default/bin");
        fs::create_dir_all(&shim_bin).unwrap();
        fs::create_dir_all(&toolchain_bin).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            home.join("settings.toml"),
            "default_toolchain = \"default\"\n",
        )
        .unwrap();
        fs::set_permissions(
            home.join("settings.toml"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let rustup = shim_bin.join("rustup");
        fs::write(&rustup, b"rustup proxy").unwrap();
        fs::set_permissions(&rustup, fs::Permissions::from_mode(0o700)).unwrap();
        let cargo = toolchain_bin.join("cargo");
        fs::write(&cargo, b"private cargo").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let rustc = toolchain_bin.join("rustc");
        fs::write(&rustc, b"private rustc").unwrap();
        fs::set_permissions(&rustc, fs::Permissions::from_mode(0o700)).unwrap();
        let shim = shim_bin.join("cargo");
        std::os::unix::fs::symlink("rustup", &shim).unwrap();

        assert_eq!(resolve_rustup_toolchain(&home).unwrap().cargo, cargo);

        let outside = root.join("outside-cargo");
        fs::write(&outside, b"outside cargo").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(&cargo).unwrap();
        std::os::unix::fs::symlink(&outside, &cargo).unwrap();
        assert!(resolve_rustup_toolchain(&home).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
