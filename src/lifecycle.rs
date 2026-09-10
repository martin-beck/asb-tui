// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Transactional lifecycle boundary for the optional standalone frontend.

use crate::{
    bundle::{BundleManifest, digest_bytes},
    compatibility::evaluate,
    compatibility::{COORDINATOR_COMMIT, COORDINATOR_VERSION, QUALITY_COMMIT, QUALITY_VERSION},
    release_channel::{
        ReleaseClassification, channel_permits_install, compiled_target, promoted_self_identity,
    },
    system_probe::{LocalSystem, detect},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{IsTerminal, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STATE_BYTES: u64 = 64 * 1024;
const SELF_TEST_OUTPUT_BYTES: u64 = 64 * 1024;
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub schema_version: u64,
    pub release: String,
    pub executable_sha256: String,
    pub source_commit: String,
    pub source_tree: String,
    pub target: String,
    pub bundle: String,
    pub asb_version: String,
    pub protocol_version: u64,
    pub coordinator_version: String,
    pub coordinator_commit: String,
    pub quality_version: String,
    pub quality_commit: String,
    pub classification: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LifecycleStatus {
    pub installed: bool,
    pub verified: bool,
    pub release: Option<String>,
    pub executable_sha256: Option<String>,
    pub target: Option<String>,
    pub bundle: Option<String>,
    pub asb_version: Option<String>,
    pub protocol_version: Option<u64>,
    pub source_commit: Option<String>,
    pub source_tree: Option<String>,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleIoError;

/// Owner-private transactional storage rooted at a retained directory descriptor.
/// The advisory lock is held for this object's lifetime and is released by the kernel.
pub struct FilesystemLifecycle {
    directory: File,
    lock: File,
    retained_path: PathBuf,
}

impl FilesystemLifecycle {
    pub fn open(root: &Path) -> Result<Self, LifecycleIoError> {
        Self::open_with_mode(root, true)
    }

    pub fn open_read_only(root: &Path) -> Result<Self, LifecycleIoError> {
        Self::open_with_mode(root, false)
    }

    fn open_with_mode(root: &Path, update: bool) -> Result<Self, LifecycleIoError> {
        let before = fs::symlink_metadata(root).map_err(|_| LifecycleIoError)?;
        let directory = File::open(root).map_err(|_| LifecycleIoError)?;
        let metadata = directory.metadata().map_err(|_| LifecycleIoError)?;
        if !metadata.is_dir()
            || before.file_type().is_symlink()
            || (before.dev(), before.ino()) != (metadata.dev(), metadata.ino())
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(LifecycleIoError);
        }
        let retained_path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let lock_path = retained_path.join(".lifecycle.lock");
        let lock_before = match fs::symlink_metadata(&lock_path) {
            Ok(value) if value.is_file() && !value.file_type().is_symlink() => Some(value),
            Ok(_) => return Err(LifecycleIoError),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && update => None,
            Err(_) => return Err(LifecycleIoError),
        };
        let mut lock_options = OpenOptions::new();
        lock_options.read(true).write(update).truncate(false);
        if update {
            lock_options.create(true).mode(0o600);
        }
        let lock = lock_options
            .open(&lock_path)
            .map_err(|_| LifecycleIoError)?;
        let lock_after_path = fs::symlink_metadata(&lock_path).map_err(|_| LifecycleIoError)?;
        let lock_after_open = lock.metadata().map_err(|_| LifecycleIoError)?;
        if !lock_after_path.is_file()
            || lock_after_path.file_type().is_symlink()
            || (lock_after_path.dev(), lock_after_path.ino())
                != (lock_after_open.dev(), lock_after_open.ino())
            || lock_after_open.uid() != rustix::process::getuid().as_raw()
            || lock_after_open.mode() & 0o077 != 0
            || lock_before.is_some_and(|before| {
                (before.dev(), before.ino()) != (lock_after_open.dev(), lock_after_open.ino())
            })
        {
            return Err(LifecycleIoError);
        }
        rustix::fs::flock(
            &lock,
            if update {
                rustix::fs::FlockOperation::NonBlockingLockExclusive
            } else {
                rustix::fs::FlockOperation::NonBlockingLockShared
            },
        )
        .map_err(|_| LifecycleIoError)?;
        let versions = retained_path.join("versions");
        if !versions.exists() {
            if !update {
                return Err(LifecycleIoError);
            }
            let mut builder = fs::DirBuilder::new();
            builder
                .mode(0o700)
                .create(&versions)
                .map_err(|_| LifecycleIoError)?;
        }
        let versions_metadata = fs::symlink_metadata(&versions).map_err(|_| LifecycleIoError)?;
        if !versions_metadata.is_dir()
            || versions_metadata.file_type().is_symlink()
            || versions_metadata.uid() != rustix::process::getuid().as_raw()
            || versions_metadata.mode() & 0o077 != 0
        {
            return Err(LifecycleIoError);
        }
        if update {
            for entry in fs::read_dir(&versions).map_err(|_| LifecycleIoError)? {
                let entry = entry.map_err(|_| LifecycleIoError)?;
                if entry.file_name().to_string_lossy().starts_with(".stage-") {
                    fs::remove_dir_all(entry.path()).map_err(|_| LifecycleIoError)?;
                }
            }
        }
        Ok(Self {
            directory,
            lock,
            retained_path,
        })
    }

    fn validate_root(&self) -> Result<(), LifecycleIoError> {
        let metadata = self.directory.metadata().map_err(|_| LifecycleIoError)?;
        let lock_metadata = self.lock.metadata().map_err(|_| LifecycleIoError)?;
        if metadata.is_dir()
            && metadata.uid() == rustix::process::getuid().as_raw()
            && metadata.mode() & 0o077 == 0
            && lock_metadata.is_file()
            && lock_metadata.uid() == rustix::process::getuid().as_raw()
            && lock_metadata.mode() & 0o077 == 0
        {
            Ok(())
        } else {
            Err(LifecycleIoError)
        }
    }

    fn valid_digest(digest: &str) -> bool {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }

    fn version_path(&self, installation: &Installation) -> Result<PathBuf, LifecycleIoError> {
        Self::valid_digest(&installation.executable_sha256)
            .then(|| {
                self.retained_path
                    .join("versions")
                    .join(&installation.executable_sha256)
            })
            .ok_or(LifecycleIoError)
    }

    fn stage_path(&self, installation: &Installation) -> Result<PathBuf, LifecycleIoError> {
        Self::valid_digest(&installation.executable_sha256)
            .then(|| {
                self.retained_path
                    .join("versions")
                    .join(format!(".stage-{}", installation.executable_sha256))
            })
            .ok_or(LifecycleIoError)
    }

    fn read_bounded(
        path: &Path,
        maximum: u64,
        executable: bool,
    ) -> Result<Vec<u8>, LifecycleIoError> {
        let before = fs::symlink_metadata(path).map_err(|_| LifecycleIoError)?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || before.uid() != rustix::process::getuid().as_raw()
            || before.mode() & 0o077 != 0
            || (executable && before.mode() & 0o100 == 0)
            || before.len() > maximum
        {
            return Err(LifecycleIoError);
        }
        let mut file = File::open(path).map_err(|_| LifecycleIoError)?;
        let opened = file.metadata().map_err(|_| LifecycleIoError)?;
        if (opened.dev(), opened.ino()) != (before.dev(), before.ino()) {
            return Err(LifecycleIoError);
        }
        let mut bytes =
            Vec::with_capacity(usize::try_from(opened.len()).map_err(|_| LifecycleIoError)?);
        Read::by_ref(&mut file)
            .take(maximum + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| LifecycleIoError)?;
        let after = file.metadata().map_err(|_| LifecycleIoError)?;
        if (after.dev(), after.ino(), after.len())
            == (opened.dev(), opened.ino(), bytes.len() as u64)
            && bytes.len() as u64 <= maximum
        {
            Ok(bytes)
        } else {
            Err(LifecycleIoError)
        }
    }

    fn read_state(path: &Path) -> Result<Installation, LifecycleIoError> {
        serde_json::from_slice(&Self::read_bounded(path, MAX_STATE_BYTES, false)?)
            .map_err(|_| LifecycleIoError)
    }
}

/// Storage is restricted to one dedicated extension root by its implementation.
pub trait LifecycleStore {
    fn active(&self) -> Result<Option<Installation>, LifecycleIoError>;
    fn active_executable(&self, installation: &Installation) -> Result<Vec<u8>, LifecycleIoError>;
    fn stage(
        &mut self,
        installation: &Installation,
        executable: &[u8],
    ) -> Result<(), LifecycleIoError>;
    fn staged_executable(&self, installation: &Installation) -> Result<Vec<u8>, LifecycleIoError>;
    fn activate(&mut self, installation: &Installation) -> Result<(), LifecycleIoError>;
    fn discard_stage(&mut self, installation: &Installation) -> Result<(), LifecycleIoError>;
    fn remove(&mut self) -> Result<(), LifecycleIoError>;
}

pub trait SelfTest {
    fn verify_protocol_and_terminal(
        &mut self,
        installation: &Installation,
        executable: &[u8],
    ) -> bool;
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutableSelfTestResponse {
    schema_version: u64,
    classification: String,
    release: String,
    target: String,
    source_commit: String,
    source_tree: String,
    asb_version: String,
    protocol_version: u64,
    coordinator_version: String,
    coordinator_commit: String,
    quality_version: String,
    quality_commit: String,
    ready: bool,
}

#[derive(Debug, Serialize)]
pub struct LocalSelfTestResponse<'a> {
    pub schema_version: u64,
    pub classification: &'static str,
    pub release: &'a str,
    pub target: String,
    pub source_commit: String,
    pub source_tree: String,
    pub asb_version: &'static str,
    pub protocol_version: u64,
    pub coordinator_version: &'static str,
    pub coordinator_commit: &'static str,
    pub quality_version: &'static str,
    pub quality_commit: &'static str,
    pub ready: bool,
}

pub fn local_self_test_response(release: &str) -> Option<LocalSelfTestResponse<'_>> {
    if !valid_release(release) {
        return None;
    }
    let identity = promoted_self_identity(release);
    let classification = if identity.is_some() {
        ReleaseClassification::VerifiedExtension
    } else {
        ReleaseClassification::SourceOnlyUnverified
    };
    Some(LocalSelfTestResponse {
        schema_version: 1,
        classification: classification.as_str(),
        release,
        target: compiled_target().into(),
        source_commit: identity
            .as_ref()
            .map_or_else(String::new, |value| value.source_commit.clone()),
        source_tree: identity
            .as_ref()
            .map_or_else(String::new, |value| value.source_tree.clone()),
        asb_version: "0.1.0",
        protocol_version: 1,
        coordinator_version: COORDINATOR_VERSION,
        coordinator_commit: COORDINATOR_COMMIT,
        quality_version: QUALITY_VERSION,
        quality_commit: QUALITY_COMMIT,
        ready: identity.is_some()
            && compiled_target() != "unsupported"
            && evaluate(detect(&LocalSystem)).bundle.is_some(),
    })
}

fn valid_release(value: &str) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    value.len() <= 32
        && version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Executes the exact candidate bytes from an anonymous file and validates its closed response.
#[derive(Default)]
pub struct ExecutableSelfTest;

impl SelfTest for ExecutableSelfTest {
    fn verify_protocol_and_terminal(&mut self, installation: &Installation, bytes: &[u8]) -> bool {
        executable_self_test(installation, bytes).is_ok()
    }
}

fn executable_self_test(installation: &Installation, bytes: &[u8]) -> Result<(), LifecycleIoError> {
    let executable = executable_memfd("asb-tui-self-test", bytes)?;
    let program = format!("/proc/self/fd/{}", executable.as_raw_fd());
    let mut child = Command::new(program)
        .args([
            "lifecycle-self-test",
            "--release",
            installation.release.as_str(),
            "--format",
            "json",
        ])
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("LLVM_PROFILE_FILE", "/dev/null")
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| LifecycleIoError)?;
    let mut stdout = child.stdout.take().ok_or(LifecycleIoError)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = Read::by_ref(&mut stdout)
            .take(SELF_TEST_OUTPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| LifecycleIoError)? {
            break status;
        }
        if started.elapsed() >= SELF_TEST_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LifecycleIoError);
        }
        thread::sleep(Duration::from_millis(10));
    };
    let remaining = SELF_TEST_TIMEOUT
        .checked_sub(started.elapsed())
        .ok_or(LifecycleIoError)?;
    let output = receiver
        .recv_timeout(remaining)
        .map_err(|_| LifecycleIoError)?
        .map_err(|_| LifecycleIoError)?;
    let response: ExecutableSelfTestResponse =
        serde_json::from_slice(&output).map_err(|_| LifecycleIoError)?;
    if status.success()
        && output.len() <= SELF_TEST_OUTPUT_BYTES as usize
        && response.schema_version == 1
        && response.classification == "verified_extension"
        && response.release == installation.release
        && response.target == installation.target
        && response.source_commit == installation.source_commit
        && response.source_tree == installation.source_tree
        && response.asb_version == installation.asb_version
        && response.protocol_version == 1
        && response.coordinator_version == installation.coordinator_version
        && response.coordinator_commit == installation.coordinator_commit
        && response.quality_version == installation.quality_version
        && response.quality_commit == installation.quality_commit
        && response.ready
    {
        Ok(())
    } else {
        Err(LifecycleIoError)
    }
}

fn executable_memfd(name: &str, bytes: &[u8]) -> Result<File, LifecycleIoError> {
    let descriptor = rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::empty())
        .map_err(|_| LifecycleIoError)?;
    let mut executable: File = descriptor.into();
    executable.write_all(bytes).map_err(|_| LifecycleIoError)?;
    executable.sync_all().map_err(|_| LifecycleIoError)?;
    executable
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|_| LifecycleIoError)?;
    Ok(executable)
}

pub trait FrontendLauncher {
    fn launch_frontend(
        &mut self,
        installation: &Installation,
        executable: &[u8],
    ) -> Result<(), LifecycleIoError>;
}

/// Runs only the already verified bytes; it does not own or signal benchmark processes.
#[derive(Default)]
pub struct ProcessLauncher;

fn controlling_terminal() -> Result<(Stdio, Stdio, Stdio), LifecycleIoError> {
    let terminal = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| LifecycleIoError)?;
    let metadata = terminal.metadata().map_err(|_| LifecycleIoError)?;
    if !terminal.is_terminal() || !metadata.file_type().is_char_device() {
        return Err(LifecycleIoError);
    }
    let input = terminal.try_clone().map_err(|_| LifecycleIoError)?;
    let output = terminal.try_clone().map_err(|_| LifecycleIoError)?;
    Ok((
        Stdio::from(input),
        Stdio::from(output),
        Stdio::from(terminal),
    ))
}

impl FrontendLauncher for ProcessLauncher {
    fn launch_frontend(
        &mut self,
        _installation: &Installation,
        executable: &[u8],
    ) -> Result<(), LifecycleIoError> {
        let executable = executable_memfd("asb-tui-frontend", executable)?;
        let program = format!("/proc/self/fd/{}", executable.as_raw_fd());
        let (input, output, error) = controlling_terminal()?;
        let status = Command::new(program)
            .stdin(input)
            .stdout(output)
            .stderr(error)
            .status()
            .map_err(|_| LifecycleIoError)?;
        status.success().then_some(()).ok_or(LifecycleIoError)
    }
}

impl LifecycleStore for FilesystemLifecycle {
    fn active(&self) -> Result<Option<Installation>, LifecycleIoError> {
        self.validate_root()?;
        let path = self.retained_path.join("active.json");
        match Self::read_state(&path) {
            Ok(state) => Ok(Some(state)),
            Err(_) if !path.exists() => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn active_executable(&self, installation: &Installation) -> Result<Vec<u8>, LifecycleIoError> {
        self.validate_root()?;
        let active = self.active()?.ok_or(LifecycleIoError)?;
        if &active != installation {
            return Err(LifecycleIoError);
        }
        Self::read_bounded(
            &self.version_path(installation)?.join("asb-tui"),
            MAX_EXECUTABLE_BYTES,
            true,
        )
    }

    fn stage(
        &mut self,
        installation: &Installation,
        executable: &[u8],
    ) -> Result<(), LifecycleIoError> {
        self.validate_root()?;
        if executable.is_empty() || executable.len() as u64 > MAX_EXECUTABLE_BYTES {
            return Err(LifecycleIoError);
        }
        let staging = self.stage_path(installation)?;
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|_| LifecycleIoError)?;
        }
        let mut builder = fs::DirBuilder::new();
        builder
            .mode(0o700)
            .create(&staging)
            .map_err(|_| LifecycleIoError)?;
        let result = (|| {
            let mut binary = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(staging.join("asb-tui"))
                .map_err(|_| LifecycleIoError)?;
            binary.write_all(executable).map_err(|_| LifecycleIoError)?;
            binary.sync_all().map_err(|_| LifecycleIoError)?;
            let state_bytes = serde_json::to_vec(installation).map_err(|_| LifecycleIoError)?;
            let mut state = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(staging.join("installation.json"))
                .map_err(|_| LifecycleIoError)?;
            state
                .write_all(&state_bytes)
                .map_err(|_| LifecycleIoError)?;
            state.sync_all().map_err(|_| LifecycleIoError)?;
            File::open(&staging)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| LifecycleIoError)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn staged_executable(&self, installation: &Installation) -> Result<Vec<u8>, LifecycleIoError> {
        self.validate_root()?;
        let staging = self.stage_path(installation)?;
        if Self::read_state(&staging.join("installation.json"))? != *installation {
            return Err(LifecycleIoError);
        }
        Self::read_bounded(&staging.join("asb-tui"), MAX_EXECUTABLE_BYTES, true)
    }

    fn activate(&mut self, installation: &Installation) -> Result<(), LifecycleIoError> {
        self.validate_root()?;
        let staging = self.stage_path(installation)?;
        if Self::read_state(&staging.join("installation.json"))? != *installation {
            return Err(LifecycleIoError);
        }
        let version = self.version_path(installation)?;
        if version.exists() {
            if Self::read_state(&version.join("installation.json"))? != *installation {
                return Err(LifecycleIoError);
            }
            let existing =
                Self::read_bounded(&version.join("asb-tui"), MAX_EXECUTABLE_BYTES, true)?;
            if digest_bytes(&existing).map_err(|_| LifecycleIoError)?
                != installation.executable_sha256
            {
                return Err(LifecycleIoError);
            }
            fs::remove_dir_all(&staging).map_err(|_| LifecycleIoError)?;
        } else {
            fs::rename(&staging, &version).map_err(|_| LifecycleIoError)?;
            File::open(self.retained_path.join("versions"))
                .and_then(|directory| directory.sync_all())
                .map_err(|_| LifecycleIoError)?;
        }
        let state_bytes = serde_json::to_vec(installation).map_err(|_| LifecycleIoError)?;
        let temporary = self
            .retained_path
            .join(format!(".active-{}.tmp", std::process::id()));
        let mut state = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| LifecycleIoError)?;
        let result = (|| {
            state
                .write_all(&state_bytes)
                .map_err(|_| LifecycleIoError)?;
            state.sync_all().map_err(|_| LifecycleIoError)?;
            fs::rename(&temporary, self.retained_path.join("active.json"))
                .map_err(|_| LifecycleIoError)?;
            self.directory.sync_all().map_err(|_| LifecycleIoError)
        })();
        drop(state);
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn discard_stage(&mut self, installation: &Installation) -> Result<(), LifecycleIoError> {
        self.validate_root()?;
        let staging = self.stage_path(installation)?;
        match fs::remove_dir_all(staging) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(LifecycleIoError),
        }
    }

    fn remove(&mut self) -> Result<(), LifecycleIoError> {
        self.validate_root()?;
        match fs::remove_file(self.retained_path.join("active.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(LifecycleIoError),
        }
        let versions = self.retained_path.join("versions");
        for entry in fs::read_dir(&versions).map_err(|_| LifecycleIoError)? {
            let entry = entry.map_err(|_| LifecycleIoError)?;
            if entry.file_type().map_err(|_| LifecycleIoError)?.is_dir() {
                fs::remove_dir_all(entry.path()).map_err(|_| LifecycleIoError)?;
            } else {
                return Err(LifecycleIoError);
            }
        }
        self.directory.sync_all().map_err(|_| LifecycleIoError)
    }
}

pub fn install(
    manifest: &BundleManifest,
    verified_artifacts: &BTreeMap<String, Vec<u8>>,
    store: &mut impl LifecycleStore,
    self_test: &mut impl SelfTest,
) -> Result<Installation, &'static str> {
    let executable = verified_artifacts
        .get("asb-tui")
        .ok_or("verified_executable_missing")?;
    let declared = manifest
        .artifacts()
        .iter()
        .find(|artifact| artifact.name == "asb-tui")
        .ok_or("verified_executable_missing")?;
    if executable.len() as u64 != declared.size || digest_bytes(executable)? != declared.sha256 {
        return Err("verified_executable_mismatch");
    }
    let (source_commit, source_tree) = manifest.source_identity();
    let compatibility = manifest.compatibility();
    let target = match compatibility.architecture {
        crate::compatibility::Architecture::X86_64 => "x86_64-unknown-linux-gnu",
        crate::compatibility::Architecture::Aarch64 => "aarch64-unknown-linux-gnu",
        crate::compatibility::Architecture::Other => return Err("verified_target_invalid"),
    };
    let installation = Installation {
        schema_version: 1,
        release: manifest.release().to_owned(),
        executable_sha256: declared.sha256.clone(),
        source_commit: source_commit.to_owned(),
        source_tree: source_tree.to_owned(),
        target: target.into(),
        bundle: compatibility.bundle.clone(),
        asb_version: compatibility.asb_version.clone(),
        protocol_version: compatibility.protocol_version,
        coordinator_version: COORDINATOR_VERSION.into(),
        coordinator_commit: COORDINATOR_COMMIT.into(),
        quality_version: QUALITY_VERSION.into(),
        quality_commit: QUALITY_COMMIT.into(),
        classification: ReleaseClassification::VerifiedExtension.as_str().into(),
    };
    store
        .stage(&installation, executable)
        .map_err(|_| "install_stage_failed")?;
    let staged = match store.staged_executable(&installation) {
        Ok(bytes) => bytes,
        Err(_) => {
            let _ = store.discard_stage(&installation);
            return Err("install_stage_verification_failed");
        }
    };
    if staged.len() != executable.len()
        || digest_bytes(&staged)? != installation.executable_sha256
        || !self_test.verify_protocol_and_terminal(&installation, &staged)
    {
        let _ = store.discard_stage(&installation);
        return Err("install_self_test_failed");
    }
    store
        .activate(&installation)
        .map_err(|_| "install_activation_failed")?;
    Ok(installation)
}

pub fn status(store: &impl LifecycleStore) -> LifecycleStatus {
    let Ok(Some(installation)) = store.active() else {
        return LifecycleStatus {
            installed: false,
            verified: false,
            release: None,
            executable_sha256: None,
            target: None,
            bundle: None,
            asb_version: None,
            protocol_version: None,
            source_commit: None,
            source_tree: None,
            reason: "extension_not_installed",
        };
    };
    let valid = installation.schema_version == 1
        && installation.classification == "verified_extension"
        && installation.target == compiled_target()
        && installation.asb_version == "0.1.0"
        && installation.protocol_version == 1
        && ((installation.target == "x86_64-unknown-linux-gnu"
            && installation.bundle == "asb-tui-v1-linux-x86_64")
            || (installation.target == "aarch64-unknown-linux-gnu"
                && installation.bundle == "asb-tui-v1-linux-aarch64"))
        && installation.coordinator_version == COORDINATOR_VERSION
        && installation.coordinator_commit == COORDINATOR_COMMIT
        && installation.quality_version == QUALITY_VERSION
        && installation.quality_commit == QUALITY_COMMIT
        && store
            .active_executable(&installation)
            .ok()
            .and_then(|bytes| digest_bytes(&bytes).ok())
            .as_deref()
            == Some(&installation.executable_sha256)
        && channel_permits_install();
    LifecycleStatus {
        installed: true,
        verified: valid,
        release: Some(installation.release),
        executable_sha256: Some(installation.executable_sha256),
        target: Some(installation.target),
        bundle: Some(installation.bundle),
        asb_version: Some(installation.asb_version),
        protocol_version: Some(installation.protocol_version),
        source_commit: Some(installation.source_commit),
        source_tree: Some(installation.source_tree),
        reason: if valid {
            "verified_installation"
        } else {
            "installation_verification_failed"
        },
    }
}

pub fn remove(store: &mut impl LifecycleStore) -> Result<(), &'static str> {
    store.remove().map_err(|_| "extension_remove_failed")
}

pub fn launch(
    store: &impl LifecycleStore,
    self_test: &mut impl SelfTest,
    launcher: &mut impl FrontendLauncher,
) -> Result<(), &'static str> {
    let installation = store
        .active()
        .map_err(|_| "installation_verification_failed")?
        .ok_or("extension_not_installed")?;
    let executable = store
        .active_executable(&installation)
        .map_err(|_| "installation_verification_failed")?;
    if !channel_permits_install() {
        return Err("installation_verification_failed");
    }
    if digest_bytes(&executable)? != installation.executable_sha256
        || !self_test.verify_protocol_and_terminal(&installation, &executable)
    {
        return Err("launch_self_test_failed");
    }
    launcher
        .launch_frontend(&installation, &executable)
        .map_err(|_| "frontend_launch_failed")
}
