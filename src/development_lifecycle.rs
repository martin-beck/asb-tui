// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Credential-free, explicitly development-only temporary lifecycle.

use crate::sha256::digest_hex;
use serde::Serialize;
use std::{
    env, fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(not(test))]
use std::{
    io::{ErrorKind, Read},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

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
}

#[derive(Debug, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema_version: u64,
    channel: String,
    development_only: bool,
    source_commit: String,
    source_tree: String,
    executable_sha256: String,
    source_repository: String,
}

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
    let valid = state.schema_version == 1
        && state.channel == CHANNEL
        && state.development_only
        && state.source_repository == dev_repository()
        && valid_identity(&state.source_commit)
        && valid_identity(&state.source_tree)
        && digest_hex(&bytes) == state.executable_sha256;
    let mut result = response(
        if valid {
            "development_installed"
        } else {
            "development_installation_invalid"
        },
        valid,
    );
    result.installed = true;
    result.verified = valid;
    result.source_commit = Some(state.source_commit);
    result.source_tree = Some(state.source_tree);
    result.executable_sha256 = Some(state.executable_sha256);
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
        let is_symlink = fs::symlink_metadata(&candidate)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        let Ok(canonical) = fs::canonicalize(&candidate) else {
            continue;
        };
        let Ok(metadata) = fs::symlink_metadata(&canonical) else {
            continue;
        };
        if metadata.is_file()
            && (metadata.uid() == rustix::process::getuid().as_raw() || metadata.uid() == 0)
            && metadata.mode() & 0o022 == 0
        {
            // Preserve a trusted shim's argv[0] (notably cargo -> rustup),
            // while validating the resolved target's ownership and mode.
            return Ok(if is_symlink { candidate } else { canonical });
        }
    }
    Err("development_tool_unavailable")
}

#[cfg(not(test))]
fn trusted_tools() -> Result<(PathBuf, PathBuf, PathBuf, String), &'static str> {
    let setsid = trusted_executable("ASB_TUI_DEV_SETSID", "setsid")?;
    let git = trusted_executable("ASB_TUI_DEV_GIT", "git")?;
    let cargo = trusted_executable("ASB_TUI_DEV_CARGO", "cargo")?;
    let mut path = std::collections::BTreeSet::new();
    for tool in [&setsid, &git, &cargo] {
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
fn run_bounded(mut command: Command, workspace: &Path) -> Result<Vec<u8>, &'static str> {
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
fn source_identity(source: &Path, name: &str) -> Result<String, &'static str> {
    let (setsid, git, _cargo, path) = trusted_tools()?;
    let output = run_bounded(
        {
            let mut command = Command::new(&setsid);
            command
                .args(["--wait"])
                .arg(&git)
                .current_dir(source)
                .env_clear()
                .env("PATH", path)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0")
                .args(["rev-parse", name]);
            command
        },
        source,
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
fn build_candidate(workspace: &Path) -> Result<(Vec<u8>, State), &'static str> {
    let source = workspace.join("source");
    let target = workspace.join("target");
    let cargo_home = workspace.join("cargo-home");
    fs::create_dir_all(&target).map_err(|_| "development_workspace_create_failed")?;
    fs::create_dir_all(&cargo_home).map_err(|_| "development_workspace_create_failed")?;
    let repository = dev_repository();
    let reference = dev_ref();
    let (setsid, git, cargo, path) = trusted_tools()?;
    let rustup_home = env::var_os("ASB_TUI_DEV_RUSTUP_HOME");
    let mut clone = Command::new(&setsid);
    clone
        .args(["--wait"])
        .arg(&git)
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
        .env("PATH", &path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg(&source);
    run_bounded(clone, workspace)?;
    let commit = source_identity(&source, "HEAD")?;
    let tree = source_identity(&source, "HEAD^{tree}")?;
    let mut build = Command::new(&setsid);
    build
        .args(["--wait"])
        .arg(&cargo)
        .current_dir(&source)
        .env_clear()
        .env("PATH", &path)
        .env("HOME", workspace)
        .env("CARGO_HOME", &cargo_home)
        .env("CARGO_TARGET_DIR", &target)
        .env("RUSTFLAGS", "")
        .args(["build", "--locked", "--release", "--bin", "asb-tui"]);
    if let Some(rustup_home) = rustup_home {
        build.env("RUSTUP_HOME", rustup_home);
    }
    run_bounded(build, workspace)?;
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
    };
    Ok((bytes, state))
}

#[cfg(test)]
fn build_candidate(_workspace: &Path) -> Result<(Vec<u8>, State), &'static str> {
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
        },
    ))
}

fn materialize(root: &Path, replacing: bool) -> Result<Response, &'static str> {
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
        let (bytes, state) = build_candidate(&workspace)?;
        let staged = stage.join("asb-tui");
        fs::write(&staged, &bytes).map_err(|_| "development_stage_failed")?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o700))
            .map_err(|_| "development_stage_failed")?;
        fs::write(
            stage.join("provenance.json"),
            serde_json::to_vec(&state).map_err(|_| "development_stage_failed")?,
        )
        .map_err(|_| "development_stage_failed")?;
        let backup = root.join(format!(".backup-{nonce}"));
        if replacing {
            fs::create_dir(&backup).map_err(|_| "development_activation_failed")?;
            for name in ["asb-tui", "provenance.json"] {
                let old = root.join(name);
                if old.exists() && fs::rename(&old, backup.join(name)).is_err() {
                    for restore in ["asb-tui", "provenance.json"] {
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
            Ok::<(), &'static str>(())
        })();
        if let Err(error) = activation {
            // Restore both old entries before reporting failure; a failed
            // upgrade must never leave a half-new installation active.
            for name in ["asb-tui", "provenance.json"] {
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

pub fn execute(operation: &str) -> Response {
    let Ok(root) = root() else {
        return response("development_root_invalid", false);
    };
    match operation {
        "install" => materialize(&root, false).unwrap_or_else(|code| response(code, false)),
        "upgrade" => {
            if read_state(&root).ok().flatten().is_none() {
                response("development_not_installed", false)
            } else {
                materialize(&root, true).unwrap_or_else(|code| response(code, false))
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
                        for name in ["asb-tui", "provenance.json"] {
                            fs::rename(root.join(name), trash.join(name))
                                .map_err(|_| "development_remove_failed")?;
                        }
                        fs::remove_dir_all(&trash).map_err(|_| "development_remove_failed")
                    })();
                    if moved.is_err() {
                        for name in ["asb-tui", "provenance.json"] {
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
        };
        let root = temp_root("malformed");
        fs::write(state_path(&root), serde_json::to_vec(&state).unwrap()).unwrap();
        fs::write(executable_path(&root), b"candidate").unwrap();
        assert_eq!(status(&root).code, "development_installation_invalid");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn second_activation_rename_restores_previous_pair() {
        let root = temp_root("rollback");
        assert!(materialize(&root, false).unwrap().verified);
        let before = status(&root);
        FAIL_RENAME_AT.store(2, Ordering::SeqCst);
        assert_eq!(
            materialize(&root, true),
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
}
