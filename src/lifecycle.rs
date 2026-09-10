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
use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, TargetArch};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    convert::TryInto,
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Read, Write},
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STATE_BYTES: u64 = 64 * 1024;
const SELF_TEST_OUTPUT_BYTES: u64 = 64 * 1024;
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_CLEANUP_ITEMS: usize = 256;
const PROBE_CLEANUP_DEPTH: usize = 8;
const PROBE_CLEANUP_TIME: Duration = Duration::from_millis(250);

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
        if update {
            cleanup_stale_probe_directories(&directory);
        }
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

pub fn local_self_test_response<'a>(
    release: &'a str,
    expected_asb_version: &str,
    expected_protocol_version: u64,
) -> Option<LocalSelfTestResponse<'a>> {
    if !valid_release(release) || expected_asb_version != "0.1.0" || expected_protocol_version != 1
    {
        return None;
    }
    let identity = promoted_self_identity(release);
    let classification = if identity.is_some() {
        ReleaseClassification::VerifiedExtension
    } else {
        ReleaseClassification::SourceOnlyUnverified
    };
    let mut probe = detect(&LocalSystem);
    probe.asb.version = Some(expected_asb_version.to_owned());
    probe.asb.protocol_version = Some(expected_protocol_version);
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
            && evaluate(probe).bundle.is_some(),
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
pub struct ExecutableSelfTest {
    lifecycle_root: File,
    supervisor: File,
}

impl ExecutableSelfTest {
    pub fn for_store(store: &FilesystemLifecycle) -> Result<Self, LifecycleIoError> {
        Self::for_store_with_supervisor(store, Path::new("/proc/self/exe"))
    }

    /// Bind an explicit trusted supervisor executable for an embedding application or test.
    pub fn for_store_with_supervisor(
        store: &FilesystemLifecycle,
        supervisor: &Path,
    ) -> Result<Self, LifecycleIoError> {
        store.validate_root()?;
        let supervisor = File::open(supervisor).map_err(|_| LifecycleIoError)?;
        let metadata = supervisor.metadata().map_err(|_| LifecycleIoError)?;
        if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
            return Err(LifecycleIoError);
        }
        Ok(Self {
            lifecycle_root: store.directory.try_clone().map_err(|_| LifecycleIoError)?,
            supervisor,
        })
    }
}

impl SelfTest for ExecutableSelfTest {
    fn verify_protocol_and_terminal(&mut self, installation: &Installation, bytes: &[u8]) -> bool {
        executable_self_test(installation, bytes, &self.lifecycle_root, &self.supervisor).is_ok()
    }
}

struct ProbeDirectories {
    root: File,
    name: String,
    runtime: File,
    config: Option<File>,
    cache: Option<File>,
}

impl ProbeDirectories {
    fn inherited_paths(&self) -> Result<(String, String), LifecycleIoError> {
        Ok((
            format!(
                "/proc/self/fd/{}",
                self.config.as_ref().ok_or(LifecycleIoError)?.as_raw_fd()
            ),
            format!(
                "/proc/self/fd/{}",
                self.cache.as_ref().ok_or(LifecycleIoError)?.as_raw_fd()
            ),
        ))
    }
}

struct CleanupBudget {
    remaining: usize,
    deadline: Instant,
}

fn cleanup_directory_contents(
    directory: &File,
    depth: usize,
    budget: &mut CleanupBudget,
) -> Result<(), LifecycleIoError> {
    if depth > PROBE_CLEANUP_DEPTH || budget.remaining == 0 || Instant::now() >= budget.deadline {
        return Err(LifecycleIoError);
    }
    let path = format!("/proc/self/fd/{}", directory.as_raw_fd());
    for entry in fs::read_dir(path).map_err(|_| LifecycleIoError)? {
        if budget.remaining == 0 || Instant::now() >= budget.deadline {
            return Err(LifecycleIoError);
        }
        budget.remaining -= 1;
        let name = entry.map_err(|_| LifecycleIoError)?.file_name();
        let metadata = rustix::fs::statat(directory, &name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| LifecycleIoError)?;
        if rustix::fs::FileType::from_raw_mode(metadata.st_mode) == rustix::fs::FileType::Directory
        {
            let child = rustix::fs::openat(
                directory,
                &name,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map(File::from)
            .map_err(|_| LifecycleIoError)?;
            cleanup_directory_contents(&child, depth + 1, budget)?;
            rustix::fs::unlinkat(directory, &name, rustix::fs::AtFlags::REMOVEDIR)
                .map_err(|_| LifecycleIoError)?;
        } else {
            rustix::fs::unlinkat(directory, &name, rustix::fs::AtFlags::empty())
                .map_err(|_| LifecycleIoError)?;
        }
    }
    Ok(())
}

fn cleanup_stale_probe_directories(root: &File) {
    let path = format!("/proc/self/fd/{}", root.as_raw_fd());
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    let mut budget = CleanupBudget {
        remaining: PROBE_CLEANUP_ITEMS,
        deadline: Instant::now() + PROBE_CLEANUP_TIME,
    };
    for entry in entries.filter_map(Result::ok) {
        if budget.remaining == 0 || Instant::now() >= budget.deadline {
            break;
        }
        let name = entry.file_name();
        if !valid_probe_name(&name.to_string_lossy()) {
            continue;
        }
        let Ok(directory) = rustix::fs::openat(
            root,
            &name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) else {
            continue;
        };
        let directory = File::from(directory);
        let Ok(metadata) = rustix::fs::fstat(&directory) else {
            continue;
        };
        if metadata.st_uid != rustix::process::getuid().as_raw() || metadata.st_mode & 0o077 != 0 {
            continue;
        }
        let identity = (metadata.st_dev, metadata.st_ino);
        let _ = cleanup_directory_contents(&directory, 0, &mut budget);
        let still_same = rustix::fs::statat(root, &name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|value| (value.st_dev, value.st_ino) == identity);
        if still_same {
            let _ = rustix::fs::unlinkat(root, &name, rustix::fs::AtFlags::REMOVEDIR);
        }
    }
}

fn valid_probe_name(name: &str) -> bool {
    name.strip_prefix(".self-test-runtime-")
        .is_some_and(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

impl Drop for ProbeDirectories {
    fn drop(&mut self) {
        let mut budget = CleanupBudget {
            remaining: PROBE_CLEANUP_ITEMS,
            deadline: Instant::now() + PROBE_CLEANUP_TIME,
        };
        let runtime_identity = rustix::fs::fstat(&self.runtime)
            .ok()
            .map(|value| (value.st_dev, value.st_ino));
        if let Some(config) = self.config.as_ref() {
            let _ = cleanup_directory_contents(config, 0, &mut budget);
        }
        if let Some(cache) = self.cache.as_ref() {
            let _ = cleanup_directory_contents(cache, 0, &mut budget);
        }
        let _ = rustix::fs::unlinkat(&self.runtime, "config", rustix::fs::AtFlags::REMOVEDIR);
        let _ = rustix::fs::unlinkat(&self.runtime, "cache", rustix::fs::AtFlags::REMOVEDIR);
        let _ = cleanup_directory_contents(&self.runtime, 0, &mut budget);
        let still_same = runtime_identity.is_some_and(|identity| {
            rustix::fs::statat(
                &self.root,
                self.name.as_str(),
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .is_ok_and(|value| (value.st_dev, value.st_ino) == identity)
        });
        if still_same {
            let _ = rustix::fs::unlinkat(
                &self.root,
                self.name.as_str(),
                rustix::fs::AtFlags::REMOVEDIR,
            );
        }
    }
}

fn private_probe_directories(root: &File) -> Result<ProbeDirectories, LifecycleIoError> {
    let open_private_child = |parent: &File, name: &str| {
        rustix::fs::mkdirat(
            parent,
            name,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR | rustix::fs::Mode::XUSR,
        )
        .map_err(|_| LifecycleIoError)?;
        let descriptor = rustix::fs::openat(
            parent,
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| LifecycleIoError)?;
        let metadata = rustix::fs::fstat(&descriptor).map_err(|_| LifecycleIoError)?;
        if metadata.st_uid != rustix::process::getuid().as_raw() || metadata.st_mode & 0o077 != 0 {
            return Err(LifecycleIoError);
        }
        Ok(File::from(descriptor))
    };
    let mut created = None;
    for _ in 0..4 {
        let mut entropy = [0_u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut entropy))
            .map_err(|_| LifecycleIoError)?;
        let name = format!(
            ".self-test-runtime-{}",
            entropy
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        match rustix::fs::mkdirat(
            root,
            name.as_str(),
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR | rustix::fs::Mode::XUSR,
        ) {
            Err(rustix::io::Errno::EXIST) => continue,
            Err(_) => return Err(LifecycleIoError),
            Ok(()) => {}
        }
        let runtime = match rustix::fs::openat(
            root,
            name.as_str(),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) {
            Ok(runtime) => File::from(runtime),
            Err(_) => {
                let _ = rustix::fs::unlinkat(root, name.as_str(), rustix::fs::AtFlags::REMOVEDIR);
                return Err(LifecycleIoError);
            }
        };
        created = Some((name, runtime));
        break;
    }
    let (name, runtime) = created.ok_or(LifecycleIoError)?;
    let mut probes = ProbeDirectories {
        root: root.try_clone().map_err(|_| LifecycleIoError)?,
        name,
        runtime,
        config: None,
        cache: None,
    };
    probes.config = Some(open_private_child(&probes.runtime, "config")?);
    probes.cache = Some(open_private_child(&probes.runtime, "cache")?);
    let config = probes.config.as_ref().ok_or(LifecycleIoError)?;
    let cache = probes.cache.as_ref().ok_or(LifecycleIoError)?;
    rustix::io::fcntl_setfd(config, rustix::io::FdFlags::empty()).map_err(|_| LifecycleIoError)?;
    rustix::io::fcntl_setfd(cache, rustix::io::FdFlags::empty()).map_err(|_| LifecycleIoError)?;
    Ok(probes)
}

static SELF_TEST_PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const PROCESS_CLEANUP_TIME: Duration = Duration::from_millis(750);

struct CandidateProcess {
    child: std::process::Child,
    leader: rustix::process::Pid,
    leader_fd: OwnedFd,
    cleaned: bool,
}

impl CandidateProcess {
    fn spawn(command: &mut Command) -> Result<Self, LifecycleIoError> {
        command.process_group(0);
        let mut child = command.spawn().map_err(|_| LifecycleIoError)?;
        let leader = rustix::process::Pid::from_raw(child.id() as i32).ok_or(LifecycleIoError)?;
        let leader_fd =
            match rustix::process::pidfd_open(leader, rustix::process::PidfdFlags::NONBLOCK) {
                Ok(fd) => fd,
                Err(_) => {
                    let _ =
                        rustix::process::kill_process_group(leader, rustix::process::Signal::KILL);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(LifecycleIoError);
                }
            };
        Ok(Self {
            child,
            leader,
            leader_fd,
            cleaned: false,
        })
    }

    fn exited_without_reaping(&self) -> Result<bool, LifecycleIoError> {
        rustix::process::waitid(
            rustix::process::WaitId::PidFd(self.leader_fd.as_fd()),
            rustix::process::WaitIdOptions::EXITED
                | rustix::process::WaitIdOptions::NOHANG
                | rustix::process::WaitIdOptions::NOWAIT,
        )
        .map(|status| status.is_some())
        .map_err(|_| LifecycleIoError)
    }

    fn terminate_tree(&mut self) -> Result<std::process::ExitStatus, LifecycleIoError> {
        if self.cleaned {
            return self.child.try_wait().ok().flatten().ok_or(LifecycleIoError);
        }
        let _ = rustix::process::kill_process_group(self.leader, rustix::process::Signal::KILL);
        let _ = rustix::process::pidfd_send_signal(&self.leader_fd, rustix::process::Signal::KILL);
        let deadline = Instant::now() + PROCESS_CLEANUP_TIME;
        let status = loop {
            if let Some(status) = self.child.try_wait().map_err(|_| LifecycleIoError)? {
                break status;
            }
            if Instant::now() >= deadline {
                return Err(LifecycleIoError);
            }
            thread::sleep(Duration::from_millis(5));
        };
        self.cleaned = true;
        Ok(status)
    }
}

impl Drop for CandidateProcess {
    fn drop(&mut self) {
        let _ = self.terminate_tree();
    }
}

fn candidate_confinement_filter() -> Result<BpfProgram, LifecycleIoError> {
    let rules = [libc::SYS_setpgid, libc::SYS_setsid]
        .into_iter()
        .map(|syscall| (syscall, Vec::new()))
        .collect();
    SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        TargetArch::try_from(std::env::consts::ARCH).map_err(|_| LifecycleIoError)?,
    )
    .map_err(|_| LifecycleIoError)?
    .try_into()
    .map_err(|_| LifecycleIoError)
}

/// Execute a sealed self-test candidate after denying all process-group/session escape syscalls.
///
/// This is an internal CLI boundary. The caller must already place this supervisor in a fresh
/// process group and pass the candidate's inherited sealed memfd path as the first argument.
#[doc(hidden)]
pub fn run_self_test_supervisor(arguments: &[String]) -> Result<(), LifecycleIoError> {
    let (candidate_path, candidate_arguments) = arguments.split_first().ok_or(LifecycleIoError)?;
    let candidate = File::open(candidate_path).map_err(|_| LifecycleIoError)?;
    let required = rustix::fs::SealFlags::WRITE
        | rustix::fs::SealFlags::GROW
        | rustix::fs::SealFlags::SHRINK
        | rustix::fs::SealFlags::SEAL;
    if !rustix::fs::fcntl_get_seals(&candidate)
        .map_err(|_| LifecycleIoError)?
        .contains(required)
    {
        return Err(LifecycleIoError);
    }
    rustix::io::fcntl_setfd(&candidate, rustix::io::FdFlags::empty())
        .map_err(|_| LifecycleIoError)?;
    let program = format!("/proc/self/fd/{}", candidate.as_raw_fd());
    let filter = candidate_confinement_filter()?;
    seccompiler::apply_filter(&filter).map_err(|_| LifecycleIoError)?;
    Err(Command::new(program).args(candidate_arguments).exec()).map_err(|_| LifecycleIoError)
}

fn executable_self_test(
    installation: &Installation,
    bytes: &[u8],
    lifecycle_root: &File,
    supervisor: &File,
) -> Result<(), LifecycleIoError> {
    let executable = executable_memfd("asb-tui-self-test", bytes)?;
    let candidate = format!("/proc/self/fd/{}", executable.as_raw_fd());
    let supervisor = format!("/proc/self/fd/{}", supervisor.as_raw_fd());
    let probes = private_probe_directories(lifecycle_root)?;
    let (config_path, cache_path) = probes.inherited_paths()?;
    let (input, _, _) = controlling_terminal()?;
    let mut command = Command::new(supervisor);
    command
        .args([
            "__self-test-supervisor",
            candidate.as_str(),
            "lifecycle-self-test",
            "--release",
            installation.release.as_str(),
            "--asb-version",
            installation.asb_version.as_str(),
            "--protocol-version",
            &installation.protocol_version.to_string(),
            "--format",
            "json",
        ])
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("LLVM_PROFILE_FILE", "/dev/null")
        .env("XDG_CONFIG_HOME", config_path)
        .env("XDG_CACHE_HOME", cache_path)
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for name in ["TERM", "COLORTERM"] {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
            && value.len() <= 128
            && !value.chars().any(char::is_control)
        {
            command.env(name, value);
        }
    }
    for (source, normalized) in [
        ("SSH_CONNECTION", "SSH_CONNECTION"),
        ("SSH_TTY", "SSH_TTY"),
        ("TMUX", "TMUX"),
        ("STY", "STY"),
    ] {
        if std::env::var_os(source).is_some() {
            command.env(normalized, "present");
        }
    }
    let _process_lock = SELF_TEST_PROCESS_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| LifecycleIoError)?;
    let mut child = CandidateProcess::spawn(&mut command)?;
    let mut stdout = child.child.stdout.take().ok_or(LifecycleIoError)?;
    let flags = rustix::fs::fcntl_getfl(&stdout).map_err(|_| LifecycleIoError)?;
    rustix::fs::fcntl_setfl(&stdout, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|_| LifecycleIoError)?;
    let started = Instant::now();
    let mut output = Vec::new();
    let mut eof = false;
    loop {
        let mut chunk = [0_u8; 4096];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(count) => {
                    output.extend_from_slice(&chunk[..count]);
                    if output.len() > SELF_TEST_OUTPUT_BYTES as usize {
                        let _ = child.terminate_tree();
                        return Err(LifecycleIoError);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    let _ = child.terminate_tree();
                    return Err(LifecycleIoError);
                }
            }
        }
        if child.exited_without_reaping()? {
            break;
        }
        if started.elapsed() >= SELF_TEST_TIMEOUT {
            let _ = child.terminate_tree();
            return Err(LifecycleIoError);
        }
        thread::sleep(Duration::from_millis(5));
    }
    let status = child.terminate_tree()?;
    let drain_deadline = Instant::now() + PROCESS_CLEANUP_TIME;
    while !eof && Instant::now() < drain_deadline {
        let mut chunk = [0_u8; 4096];
        match stdout.read(&mut chunk) {
            Ok(0) => eof = true,
            Ok(count) => {
                output.extend_from_slice(&chunk[..count]);
                if output.len() > SELF_TEST_OUTPUT_BYTES as usize {
                    return Err(LifecycleIoError);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return Err(LifecycleIoError),
        }
    }
    if !eof {
        return Err(LifecycleIoError);
    }
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
    let descriptor = rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::ALLOW_SEALING)
        .map_err(|_| LifecycleIoError)?;
    let mut executable: File = descriptor.into();
    executable.write_all(bytes).map_err(|_| LifecycleIoError)?;
    executable.sync_all().map_err(|_| LifecycleIoError)?;
    executable
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|_| LifecycleIoError)?;
    let required = rustix::fs::SealFlags::WRITE
        | rustix::fs::SealFlags::GROW
        | rustix::fs::SealFlags::SHRINK
        | rustix::fs::SealFlags::SEAL;
    rustix::fs::fcntl_add_seals(&executable, required).map_err(|_| LifecycleIoError)?;
    if !rustix::fs::fcntl_get_seals(&executable)
        .map_err(|_| LifecycleIoError)?
        .contains(required)
    {
        return Err(LifecycleIoError);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_memfd_is_immutable_after_complete_write() {
        let mut executable = executable_memfd("asb-tui-seal-test", b"immutable").unwrap();
        let required = rustix::fs::SealFlags::WRITE
            | rustix::fs::SealFlags::GROW
            | rustix::fs::SealFlags::SHRINK
            | rustix::fs::SealFlags::SEAL;
        assert_eq!(
            rustix::fs::fcntl_get_seals(&executable).unwrap() & required,
            required
        );
        assert!(executable.write_all(b"tamper").is_err());
        assert!(executable.set_len(0).is_err());
    }

    #[test]
    fn stale_probe_cleanup_is_bounded_and_never_reuses_residue() {
        let root =
            std::env::temp_dir().join(format!("asb-tui-probe-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(&root).unwrap();
        let runtime_name = ".self-test-runtime-cccccccccccccccccccccccccccccccc";
        let runtime = root.join(runtime_name);
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        for index in 0..(PROBE_CLEANUP_ITEMS + 32) {
            File::create(runtime.join(format!("residue-{index}"))).unwrap();
        }
        let root_fd = File::open(&root).unwrap();
        cleanup_stale_probe_directories(&root_fd);
        assert!(
            runtime.exists(),
            "bounded cleanup unexpectedly widened its budget"
        );
        let remaining = fs::read_dir(&runtime).unwrap().count();
        assert!(
            remaining > 0 && remaining < PROBE_CLEANUP_ITEMS + 32,
            "unexpected remaining residue count {remaining}"
        );
        cleanup_stale_probe_directories(&root_fd);
        assert!(
            !runtime.exists(),
            "a later exclusive pass did not prune residue"
        );
        drop(root_fd);
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn candidate_confinement_filter_compiles_and_cleanup_has_no_proc_dependency() {
        assert!(!candidate_confinement_filter().unwrap().is_empty());
        let source = include_str!("lifecycle.rs");
        assert!(!source.contains(&["/proc/self", "/task"].concat()));
        assert!(!source.contains(&["set_child_", "subreaper"].concat()));
    }
}
