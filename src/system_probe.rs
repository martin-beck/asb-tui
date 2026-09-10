// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded local-system probing for the compatibility evaluator.

use crate::{
    compatibility::{
        Architecture, AsbProbe, COORDINATOR_COMMIT, COORDINATOR_VERSION, ColorLevel,
        CompatibilityProbe, DependencyProbe, Distribution, Multiplexer, OperatingSystem,
        PlatformProbe, QUALITY_COMMIT, QUALITY_VERSION, RuntimeProbe, TerminalChannel,
        TerminalProbe,
    },
    is_safe_version, parse_capability_response,
};
#[cfg(all(unix, test))]
use rustix::fd::AsRawFd;
#[cfg(unix)]
use rustix::{
    fd::OwnedFd,
    fs::{self as unix_fs, AtFlags, Mode, OFlags},
    io::dup,
    process::{Pid, Signal, getuid, kill_process_group},
};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    cell::RefCell,
    fs,
    io::{IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const COMMAND_LIMIT: usize = 16 * 1024;
const FILE_LIMIT: usize = 16 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const SAFE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// Injectable boundary used by deterministic tests and the real local probe.
pub trait ProbeSource {
    fn os(&self) -> &str;
    fn architecture(&self) -> &str;
    fn os_release(&self) -> Option<Vec<u8>>;
    fn asb_version(&self) -> Option<Vec<u8>>;
    fn asb_capabilities(&self) -> Option<Vec<u8>>;
    fn terminal_dimensions(&self) -> Option<(u16, u16)>;
    fn stdin_is_terminal(&self) -> bool;
    fn resize_events_verified(&self) -> bool;
    fn environment(&self, name: &str) -> Option<String>;
    fn runtime(&self) -> RuntimeProbe;
}

/// Production probe. It emits only normalized enums, booleans, bounded versions, and fixed reasons.
pub struct LocalSystem;

impl ProbeSource for LocalSystem {
    fn os(&self) -> &str {
        std::env::consts::OS
    }

    fn architecture(&self) -> &str {
        std::env::consts::ARCH
    }

    fn os_release(&self) -> Option<Vec<u8>> {
        bounded_file(Path::new("/etc/os-release"))
    }

    fn asb_version(&self) -> Option<Vec<u8>> {
        successful_stdout("asb", &["--version"])
    }

    fn asb_capabilities(&self) -> Option<Vec<u8>> {
        successful_stdout("asb", &["capabilities", "--format", "json"])
    }

    fn terminal_dimensions(&self) -> Option<(u16, u16)> {
        let tty = fs::File::open("/dev/tty").ok()?;
        let result = bounded_command_with_stdin("stty", &["size"], Stdio::from(tty))?;
        let output = result.success.then_some(result.stdout)?;
        let text = std::str::from_utf8(&output).ok()?;
        let mut values = text.split_whitespace();
        let rows = values.next()?.parse().ok()?;
        let columns = values.next()?.parse().ok()?;
        (values.next().is_none()).then_some((columns, rows))
    }

    fn stdin_is_terminal(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn resize_events_verified(&self) -> bool {
        false
    }

    fn environment(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn runtime(&self) -> RuntimeProbe {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|path| path.join(".config")));
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|path| path.join(".cache")));
        let config_result = config.as_deref().map(probe_directory).unwrap_or_default();
        let cache_result = cache.as_deref().map(probe_directory).unwrap_or_default();
        RuntimeProbe {
            config_writable: config_result.writable,
            cache_writable: cache_result.writable,
            atomic_rename: config_result.atomic_rename && cache_result.atomic_rename,
            executable_files: cache_result.executable,
            git_available: bounded_command("git", &["--version"]).is_some(),
            ssh_keygen_available: bounded_command("ssh-keygen", &["-?"]).is_some(),
        }
    }
}

/// Collect normalized system facts. Missing or hostile observations become unavailable facts.
pub fn detect(source: &impl ProbeSource) -> CompatibilityProbe {
    let os = match source.os() {
        "linux" => OperatingSystem::Linux,
        "macos" => OperatingSystem::Macos,
        "windows" => OperatingSystem::Windows,
        _ => OperatingSystem::Other,
    };
    let distribution = if os == OperatingSystem::Linux {
        source
            .os_release()
            .as_deref()
            .and_then(classify_distribution)
            .unwrap_or(Distribution::Other)
    } else {
        Distribution::NotApplicable
    };
    let architecture = match source.architecture() {
        "x86_64" => Architecture::X86_64,
        "aarch64" => Architecture::Aarch64,
        _ => Architecture::Other,
    };
    let version = source.asb_version().and_then(parse_asb_version);
    let capability = source.asb_capabilities().and_then(|bytes| {
        std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| parse_capability_response(text).ok())
    });
    let protocol_version = capability
        .filter(|response| {
            Some(response.asb_version.as_str()) == version.as_deref()
                && response.capabilities.supports_tui()
        })
        .map(|response| response.protocol_version);
    let terminal = terminal_probe(source);
    CompatibilityProbe {
        schema_version: 1,
        platform: PlatformProbe {
            os,
            distribution,
            architecture,
        },
        asb: AsbProbe {
            version,
            protocol_version,
        },
        dependencies: DependencyProbe {
            coordinator_version: COORDINATOR_VERSION.into(),
            coordinator_commit: COORDINATOR_COMMIT.into(),
            quality_version: QUALITY_VERSION.into(),
            quality_commit: QUALITY_COMMIT.into(),
        },
        terminal,
        runtime: source.runtime(),
    }
}

fn terminal_probe(source: &impl ProbeSource) -> TerminalProbe {
    let interactive = source.stdin_is_terminal();
    let dimensions = interactive.then(|| source.terminal_dimensions()).flatten();
    let term = source.environment("TERM").unwrap_or_default();
    let color_term = source.environment("COLORTERM").unwrap_or_default();
    let locale = source
        .environment("LC_ALL")
        .or_else(|| source.environment("LANG"))
        .unwrap_or_default();
    let color = if !interactive || term == "dumb" || term.is_empty() {
        ColorLevel::None
    } else if matches!(color_term.as_str(), "truecolor" | "24bit") {
        ColorLevel::Truecolor
    } else if term.contains("256color") {
        ColorLevel::Ansi256
    } else {
        ColorLevel::Ansi16
    };
    let multiplexer = if source.environment("TMUX").is_some() {
        Multiplexer::Tmux
    } else if source.environment("STY").is_some() {
        Multiplexer::Screen
    } else {
        Multiplexer::None
    };
    TerminalProbe {
        columns: dimensions.map_or(0, |value| value.0),
        rows: dimensions.map_or(0, |value| value.1),
        color,
        unicode: locale.to_ascii_lowercase().contains("utf-8")
            || locale.to_ascii_lowercase().contains("utf8"),
        // A size snapshot alone does not prove WINCH delivery. Production stays false until a
        // future PTY verifier supplies authenticated evidence through this explicit seam.
        resize_events: interactive && dimensions.is_some() && source.resize_events_verified(),
        channel: if interactive {
            TerminalChannel::Tty
        } else {
            TerminalChannel::Pipe
        },
        ssh: source.environment("SSH_CONNECTION").is_some()
            || source.environment("SSH_TTY").is_some(),
        multiplexer,
    }
}

fn classify_distribution(input: &[u8]) -> Option<Distribution> {
    if input.len() > FILE_LIMIT {
        return None;
    }
    let text = std::str::from_utf8(input).ok()?;
    let mut id = None;
    let mut version = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            if id.is_some() {
                return None;
            }
            id = Some(normalized_os_release_value(value)?);
        } else if let Some(value) = line.strip_prefix("VERSION_ID=") {
            if version.is_some() {
                return None;
            }
            version = Some(normalized_os_release_value(value)?);
        }
    }
    match (id.as_deref(), version.as_deref()) {
        (Some("ubuntu"), Some("24.04")) => Some(Distribution::Ubuntu2404),
        (Some("debian"), Some("12")) => Some(Distribution::Debian12),
        (Some(_), Some(_)) => Some(Distribution::Other),
        _ => None,
    }
}

fn normalized_os_release_value(value: &str) -> Option<String> {
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    (!value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then(|| value.to_ascii_lowercase())
}

fn parse_asb_version(bytes: Vec<u8>) -> Option<String> {
    if bytes.len() > 128 {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?.trim();
    let version = text.strip_prefix("asb ")?;
    is_safe_version(version).then(|| version.to_owned())
}

fn bounded_file(path: &Path) -> Option<Vec<u8>> {
    let mut file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((FILE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= FILE_LIMIT).then_some(bytes)
}

#[derive(Debug)]
struct CommandResult {
    success: bool,
    stdout: Vec<u8>,
}

fn successful_stdout(program: &str, arguments: &[&str]) -> Option<Vec<u8>> {
    let result = bounded_command(program, arguments)?;
    result.success.then_some(result.stdout)
}

fn bounded_command(program: &str, arguments: &[&str]) -> Option<CommandResult> {
    bounded_command_with_stdin(program, arguments, Stdio::null())
}

fn bounded_command_with_stdin(
    program: &str,
    arguments: &[&str],
    stdin: Stdio,
) -> Option<CommandResult> {
    #[cfg(not(unix))]
    return None;
    #[cfg(unix)]
    let mut command = Command::new(program);
    command
        .args(arguments)
        .env_clear()
        .env("PATH", SAFE_PATH)
        .env("LANG", "C")
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.process_group(0);
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .by_ref()
            .take((COMMAND_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if started.elapsed() >= COMMAND_TIMEOUT {
            terminate_group(&mut child);
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let remaining = match COMMAND_TIMEOUT.checked_sub(started.elapsed()) {
        Some(remaining) => remaining,
        None => {
            terminate_group(&mut child);
            return None;
        }
    };
    let stdout = match receiver.recv_timeout(remaining) {
        Ok(Some(stdout)) if stdout.len() <= COMMAND_LIMIT => stdout,
        _ => {
            terminate_group(&mut child);
            return None;
        }
    };
    kill_group(child.id());
    Some(CommandResult {
        success: status.success(),
        stdout,
    })
}

#[cfg(unix)]
fn kill_group(raw_pid: u32) {
    if let Some(pid) = Pid::from_raw(raw_pid as i32) {
        let _ = kill_process_group(pid, Signal::KILL);
    }
}

#[cfg(unix)]
fn terminate_group(child: &mut std::process::Child) {
    kill_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Debug, Default)]
struct DirectoryResult {
    writable: bool,
    atomic_rename: bool,
    executable: bool,
}

fn probe_directory(base: &Path) -> DirectoryResult {
    let directory = match PrivateDirectory::create(base) {
        Some(directory) => directory,
        None => return DirectoryResult::default(),
    };
    let (writable, atomic_rename, identity) = directory.prepare_destination();
    let executable = identity.is_some_and(|identity| directory.executable_probe(identity));
    DirectoryResult {
        writable,
        atomic_rename,
        executable,
    }
}

#[cfg(unix)]
struct PrivateDirectory {
    base: OwnedFd,
    directory: OwnedFd,
    name: String,
    identity: (u64, u64),
    artifact: RefCell<Option<(bool, OwnedFd)>>,
}

#[cfg(not(unix))]
struct PrivateDirectory;

impl PrivateDirectory {
    #[cfg(unix)]
    fn create(base: &Path) -> Option<Self> {
        let base = unix_fs::openat(
            unix_fs::CWD,
            base,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .ok()?;
        let metadata = unix_fs::fstat(&base).ok()?;
        if metadata.st_uid != getuid().as_raw() || metadata.st_mode & 0o022 != 0 {
            return None;
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        for _ in 0..32 {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let name = format!(".asb-tui-probe-{}-{nonce}-{sequence}", std::process::id());
            match unix_fs::mkdirat(&base, name.as_str(), Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                Ok(()) => {
                    let directory = unix_fs::openat(
                        &base,
                        name.as_str(),
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                        Mode::empty(),
                    )
                    .ok()?;
                    let metadata = unix_fs::fstat(&directory).ok()?;
                    return Some(Self {
                        base,
                        directory,
                        name,
                        identity: (metadata.st_dev, metadata.st_ino),
                        artifact: RefCell::new(None),
                    });
                }
                Err(error) if error == rustix::io::Errno::EXIST => continue,
                Err(_) => return None,
            }
        }
        None
    }

    #[cfg(not(unix))]
    fn create(_base: &Path) -> Option<Self> {
        None
    }

    #[cfg(unix)]
    fn prepare_destination(&self) -> (bool, bool, Option<(u64, u64)>) {
        let file = unix_fs::openat(
            &self.directory,
            "source",
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        );
        let Ok(file) = file else {
            return (false, false, None);
        };
        let identity = unix_fs::fstat(&file)
            .ok()
            .map(|metadata| (metadata.st_dev, metadata.st_ino));
        let mut file = std::fs::File::from(file);
        if file.write_all(b"probe").is_err() {
            return (false, false, None);
        }
        let renamed =
            unix_fs::renameat(&self.directory, "source", &self.directory, "destination").is_ok();
        let name = if renamed { "destination" } else { "source" };
        let held = unix_fs::openat(
            &self.directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        );
        let held = held.ok().filter(|held| {
            identity.is_some_and(|identity| {
                unix_fs::fstat(held)
                    .is_ok_and(|metadata| (metadata.st_dev, metadata.st_ino) == identity)
            })
        });
        drop(file);
        if let Some(held) = held {
            self.artifact.replace(Some((renamed, held)));
        }
        let still_same = self.artifact.borrow().is_some();
        (
            true,
            renamed && still_same,
            (renamed && still_same).then_some(identity).flatten(),
        )
    }

    #[cfg(not(unix))]
    fn prepare_destination(&self) -> (bool, bool, Option<(u64, u64)>) {
        (false, false, None)
    }

    #[cfg(unix)]
    fn executable_probe(&self, expected: (u64, u64)) -> bool {
        let executable = match fs::metadata("/bin/true") {
            Ok(metadata) if metadata.len() <= 1024 * 1024 => match fs::read("/bin/true") {
                Ok(bytes) => bytes,
                Err(_) => return false,
            },
            _ => return false,
        };
        let file = unix_fs::openat(
            &self.directory,
            "destination",
            OFlags::WRONLY | OFlags::TRUNC | OFlags::NOFOLLOW,
            Mode::empty(),
        );
        let Ok(file) = file else {
            return false;
        };
        if !unix_fs::fstat(&file)
            .is_ok_and(|metadata| (metadata.st_dev, metadata.st_ino) == expected)
        {
            return false;
        }
        let mut file = std::fs::File::from(file);
        if file.write_all(&executable).is_err()
            || unix_fs::fchmod(&file, Mode::RUSR | Mode::WUSR | Mode::XUSR).is_err()
        {
            return false;
        }
        // Linux rejects executing a file while this process still has it open writable.
        drop(file);
        let inherited = match dup(&self.directory) {
            Ok(fd) => fd,
            Err(_) => return false,
        };
        let observed =
            bounded_command_with_stdin("/proc/self/fd/0/destination", &[], Stdio::from(inherited));
        observed.is_some_and(|value| value.success)
            && unix_fs::statat(&self.directory, "destination", AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|metadata| (metadata.st_dev, metadata.st_ino) == expected)
    }

    #[cfg(not(unix))]
    fn executable_probe(&self, _expected: (u64, u64)) -> bool {
        false
    }
}

#[cfg(unix)]
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        if let Some((destination, held)) = self.artifact.borrow().as_ref() {
            let name = if *destination {
                "destination"
            } else {
                "source"
            };
            let expected = unix_fs::fstat(held)
                .ok()
                .map(|metadata| (metadata.st_dev, metadata.st_ino));
            let still_same = expected.is_some_and(|expected| {
                unix_fs::statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW)
                    .is_ok_and(|metadata| (metadata.st_dev, metadata.st_ino) == expected)
            });
            if still_same {
                let _ = unix_fs::unlinkat(&self.directory, name, AtFlags::empty());
            }
        }
        let still_same = unix_fs::statat(&self.base, self.name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .is_ok_and(|metadata| (metadata.st_dev, metadata.st_ino) == self.identity);
        if still_same {
            let _ = unix_fs::unlinkat(&self.base, self.name.as_str(), AtFlags::REMOVEDIR);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    struct TestBase(PathBuf);

    #[cfg(unix)]
    impl TestBase {
        fn create() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            for sequence in 0..32_u64 {
                let path = std::env::temp_dir().join(format!(
                    "asb-tui-system-probe-test-{}-{}-{sequence}",
                    std::process::id(),
                    NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
                ));
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                match builder.create(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create test base: {error}"),
                }
            }
            panic!("cannot allocate unique test base")
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(unix)]
    impl Drop for TestBase {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn script(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    #[cfg(unix)]
    fn command_probe_bounds_timeout_output_encoding_and_stderr() {
        let root = TestBase::create();
        let timeout = script(root.path(), "timeout", b"#!/bin/sh\nexec sleep 3\n");
        assert!(bounded_command(timeout.to_str().unwrap(), &[]).is_none());

        let oversized = script(
            root.path(),
            "oversized",
            b"#!/bin/sh\nexec head -c 20000 /dev/zero\n",
        );
        assert!(bounded_command(oversized.to_str().unwrap(), &[]).is_none());

        let non_utf8 = script(root.path(), "non-utf8", b"#!/bin/sh\nprintf '\\377'\n");
        let bytes = successful_stdout(non_utf8.to_str().unwrap(), &[]).unwrap();
        assert!(parse_asb_version(bytes).is_none());

        let private_stderr = script(
            root.path(),
            "stderr",
            b"#!/bin/sh\necho private-value >&2\necho 'asb 0.1.0'\n",
        );
        assert_eq!(
            parse_asb_version(successful_stdout(private_stderr.to_str().unwrap(), &[]).unwrap()),
            Some("0.1.0".into())
        );

        let argv_marker = root.path().join("argv-marker");
        let argv = script(
            root.path(),
            "argv",
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$#:$1:$2\" > '{}'\necho ok\n",
                argv_marker.display()
            )
            .as_bytes(),
        );
        assert_eq!(
            successful_stdout(argv.to_str().unwrap(), &["first", "second"]),
            Some(b"ok\n".to_vec())
        );
        assert_eq!(fs::read_to_string(argv_marker).unwrap(), "2:first:second\n");

        let environment = script(
            root.path(),
            "environment",
            b"#!/bin/sh\nprintf '%s:%s:%s' \"${HOME-unset}\" \"$PATH\" \"$LANG\"\n",
        );
        let observed = successful_stdout(environment.to_str().unwrap(), &[]).unwrap();
        assert_eq!(observed, format!("unset:{SAFE_PATH}:C").into_bytes());

        let descendant_pid = root.path().join("descendant-pid");
        let descendant = script(
            root.path(),
            "descendant",
            format!(
                "#!/bin/sh\nsleep 30 &\nprintf '%s' \"$!\" > '{}'\nwait\n",
                descendant_pid.display()
            )
            .as_bytes(),
        );
        assert!(bounded_command(descendant.to_str().unwrap(), &[]).is_none());
        let pid = fs::read_to_string(descendant_pid).unwrap();
        assert!(
            !Command::new("/bin/kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        );
    }

    #[test]
    #[cfg(unix)]
    fn filesystem_probe_is_executable_atomic_private_and_self_cleaning() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = TestBase::create();
        let before = fs::read_dir(root.path()).unwrap().count();
        let result = probe_directory(root.path());
        assert!(
            result.writable && result.atomic_rename && result.executable,
            "{result:?}"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), before);
        assert_eq!(
            fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );

        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = root.path().join("link");
        symlink(&target, &link).unwrap();
        let rejected = probe_directory(&link);
        assert!(!rejected.writable && !rejected.atomic_rename && !rejected.executable);

        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let rejected = probe_directory(root.path());
        assert!(!rejected.writable && !rejected.atomic_rename && !rejected.executable);
    }

    #[test]
    #[cfg(unix)]
    fn filesystem_probe_is_bound_to_open_directory_after_ancestor_swap() {
        use std::os::unix::fs::symlink;
        let root = TestBase::create();
        let root_path = root.path().to_owned();
        std::mem::forget(root);
        let moved = root_path.with_extension("moved");
        let hostile = root_path.with_extension("hostile");
        let directory = PrivateDirectory::create(&root_path).unwrap();
        fs::rename(&root_path, &moved).unwrap();
        fs::create_dir(&hostile).unwrap();
        symlink(&hostile, &root_path).unwrap();

        let (writable, renamed, identity) = directory.prepare_destination();
        assert!(writable && renamed && identity.is_some());
        drop(directory);
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&hostile).unwrap().count(), 0);

        fs::remove_file(&root_path).unwrap();
        fs::remove_dir(&hostile).unwrap();
        fs::remove_dir(&moved).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn replaced_probe_artifact_is_rejected_and_not_deleted() {
        use std::os::unix::fs::symlink;
        let root = TestBase::create();
        let directory = PrivateDirectory::create(root.path()).unwrap();
        let child_path = fs::read_dir(root.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let (writable, renamed, identity) = directory.prepare_destination();
        assert!(writable && renamed);
        let identity = identity.unwrap();
        let directory_path =
            PathBuf::from(format!("/proc/self/fd/{}", directory.directory.as_raw_fd()));
        fs::remove_file(directory_path.join("destination")).unwrap();
        symlink("/bin/true", directory_path.join("destination")).unwrap();

        assert!(!directory.executable_probe(identity));
        drop(directory);
        assert!(
            fs::symlink_metadata(child_path.join("destination"))
                .unwrap()
                .is_symlink()
        );
        fs::remove_dir_all(child_path).unwrap();
    }

    #[test]
    fn os_release_parser_is_bounded_closed_and_exact() {
        assert_eq!(
            classify_distribution(b"ID=debian\nVERSION_ID=\"12\"\n"),
            Some(Distribution::Debian12)
        );
        assert_eq!(
            classify_distribution(b"ID=ubuntu\nVERSION_ID=\"24.04\"\n"),
            Some(Distribution::Ubuntu2404)
        );
        assert_eq!(
            classify_distribution(b"ID=ubuntu\nVERSION_ID=\"26.04\"\n"),
            Some(Distribution::Other)
        );
        assert_eq!(classify_distribution(&[0xff]), None);
        assert_eq!(classify_distribution(&vec![b'x'; FILE_LIMIT + 1]), None);
        assert_eq!(classify_distribution(b"ID=ubuntu\n"), None);
        assert_eq!(
            classify_distribution(b"ID=ubuntu\nID=debian\nVERSION_ID=12\n"),
            None
        );
        assert_eq!(
            classify_distribution(b"ID=debian\nVERSION_ID=12\nVERSION_ID=24.04\n"),
            None
        );
    }

    #[test]
    fn asb_version_parser_shares_the_closed_schema_boundary() {
        for value in ["0.1.0", "99.0.0", "1.2.3-rc.1"] {
            assert_eq!(
                parse_asb_version(format!("asb {value}\n").into_bytes()),
                Some(value.into())
            );
        }
        for value in ["1.2", "1.2.3+private", "1.2.3/private", "1.2.3\nprivate"] {
            assert_eq!(parse_asb_version(format!("asb {value}").into_bytes()), None);
        }
        assert_eq!(
            parse_asb_version(format!("asb {}", "1".repeat(65)).into_bytes()),
            None
        );
        assert!(!LocalSystem.resize_events_verified());
    }

    #[test]
    fn local_probe_exercises_only_the_normalized_privacy_boundary() {
        let probe = detect(&LocalSystem);
        assert_eq!(probe.schema_version, 1);
        assert!(!probe.terminal.resize_events);
        let report = serde_json::to_value(crate::compatibility::evaluate(probe)).unwrap();
        let object = report.as_object().unwrap();
        assert_eq!(
            object.keys().cloned().collect::<Vec<_>>(),
            [
                "asb_version",
                "bundle",
                "classification",
                "platform",
                "protocol_version",
                "reasons",
                "schema_version",
                "terminal",
            ]
        );
    }
}
