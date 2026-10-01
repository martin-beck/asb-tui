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

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

const CHANNEL: &str = "dev";

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

fn identity(tree: bool) -> Result<String, &'static str> {
    let value = if tree {
        option_env!("ASB_TUI_SOURCE_TREE")
    } else {
        option_env!("ASB_TUI_SOURCE_COMMIT")
    };
    value
        .filter(|value| {
            value.len() == 40
                && value
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        .map(str::to_owned)
        .ok_or("development_provenance_unavailable")
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

fn materialize(root: &Path, replacing: bool) -> Result<Response, &'static str> {
    private_root(root)?;
    if !replacing && read_state(root)?.is_some() {
        return Err("development_already_installed");
    }
    let source = env::current_exe().map_err(|_| "development_source_unavailable")?;
    let metadata = fs::symlink_metadata(&source).map_err(|_| "development_source_unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("development_source_unavailable");
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "development_clock_invalid")?
        .as_nanos();
    let stage = root.join(format!(".stage-{nonce}"));
    fs::create_dir(&stage).map_err(|_| "development_stage_failed")?;
    let result = (|| {
        let staged = stage.join("asb-tui");
        fs::copy(&source, &staged).map_err(|_| "development_stage_failed")?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o700))
            .map_err(|_| "development_stage_failed")?;
        let bytes = fs::read(&staged).map_err(|_| "development_stage_failed")?;
        let state = State {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            source_commit: identity(false)?,
            source_tree: identity(true)?,
            executable_sha256: digest_hex(&bytes),
        };
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
        assert!(
            identity(false).is_ok(),
            "test checkout build derives git identity"
        );
        let state = State {
            schema_version: 1,
            channel: CHANNEL.into(),
            development_only: true,
            source_commit: "unknown".into(),
            source_tree: "unknown".into(),
            executable_sha256: digest_hex(b"candidate"),
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
