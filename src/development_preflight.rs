// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded, secret-free host checks for the development broker.

use serde::Serialize;
use std::{
    env, fs,
    io::IsTerminal,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
};

const MAX_PATH: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: CheckStatus,
    pub remediation: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Report {
    pub schema_version: u64,
    pub classification: &'static str,
    pub development_only: bool,
    pub ok: bool,
    pub code: &'static str,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn human_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("development host preflight: {}", self.code)];
        for check in &self.checks {
            lines.push(format!(
                "{}: {:?}; {}",
                check.id, check.status, check.remediation
            ));
        }
        lines
    }
}

fn trusted(path: &Path) -> bool {
    let metadata = fs::symlink_metadata(path).ok();
    let canonical = fs::canonicalize(path).ok();
    let Some(metadata) = metadata else {
        return false;
    };
    let Some(canonical) = canonical else {
        return false;
    };
    let Ok(target) = fs::metadata(canonical) else {
        return false;
    };
    (metadata.is_file() || metadata.file_type().is_symlink())
        && target.is_file()
        && (target.uid() == rustix::process::getuid().as_raw() || target.uid() == 0)
        && target.mode() & 0o022 == 0
}

fn executable(name: &'static str, variable: &'static str, remediation: &'static str) -> Check {
    let found = env::var_os(variable).is_some_and(|path| trusted(Path::new(&path)))
        || env::var_os(variable).is_none()
            && env::var_os("PATH").is_some_and(|path| {
                path.to_string_lossy()
                    .split(':')
                    .filter(|p| !p.is_empty())
                    .any(|p| {
                        let candidate = Path::new(p).join(name);
                        trusted(&candidate)
                    })
            });
    Check {
        id: name,
        status: if found {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        remediation,
    }
}

fn private_root() -> Check {
    let status = match env::var_os("ASB_TUI_DEV_INSTALL_ROOT") {
        None => CheckStatus::Passed,
        Some(value) if value.len() > MAX_PATH => CheckStatus::Failed,
        Some(value) => match fs::symlink_metadata(value) {
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.uid() == rustix::process::getuid().as_raw()
                    && metadata.mode() & 0o077 == 0 =>
            {
                CheckStatus::Passed
            }
            Ok(_) => CheckStatus::Failed,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CheckStatus::Passed,
            Err(_) => CheckStatus::Failed,
        },
    };
    Check {
        id: "private_state_root",
        status,
        remediation: "use a bounded private development install path",
    }
}

fn terminal(required: bool) -> Check {
    let configured = env::var_os("ASB_TUI_DEVELOPMENT_TERMINAL_PATH");
    let configured_none = configured.is_none();
    let valid = configured_none
        && (std::io::stdin().is_terminal() || std::io::stdout().is_terminal())
        || configured.is_some_and(|path| {
            let path = Path::new(&path);
            path.to_string_lossy().starts_with("/dev/pts/")
                && fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_char_device())
        });
    Check {
        id: "pty",
        status: if valid || !required {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        remediation: "run launch from a terminal-capable session and retry",
    }
}

fn run_with_terminal_requirement(require_terminal: bool) -> Report {
    let checks = vec![
        executable(
            "git",
            "ASB_TUI_DEV_GIT",
            "install git and retry development setup",
        ),
        executable(
            "cargo",
            "ASB_TUI_DEV_CARGO",
            "install a supported Rust toolchain and retry",
        ),
        executable(
            "setsid",
            "ASB_TUI_DEV_SETSID",
            "install util-linux setsid and retry launch",
        ),
        executable(
            "cc",
            "ASB_TUI_DEV_CC",
            "install a C linker and retry development build",
        ),
        private_root(),
        terminal(require_terminal),
    ];
    let ok = checks
        .iter()
        .all(|check| check.status == CheckStatus::Passed);
    Report {
        schema_version: 1,
        classification: "development_only",
        development_only: true,
        ok,
        code: if ok {
            "development_host_ready"
        } else {
            "development_host_preflight_failed"
        },
        checks,
    }
}

pub fn run() -> Report {
    run_with_terminal_requirement(false)
}

pub fn run_for_launch() -> Report {
    run_with_terminal_requirement(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_bounded_and_secret_free() {
        let report = run();
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(encoded.len() < 16 * 1024);
        assert!(!encoded.to_ascii_lowercase().contains("token"));
        let json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert!(
            json.get("checks")
                .and_then(serde_json::Value::as_array)
                .is_some()
        );
        assert!(
            report
                .human_lines()
                .iter()
                .any(|line| line.contains("preflight"))
        );
        assert!(
            report.code == "development_host_ready"
                || report.code == "development_host_preflight_failed"
        );
    }

    #[test]
    fn failed_check_has_actionable_recovery() {
        let check = executable(
            "__missing_asb_tui_tool__",
            "ASB_TUI_DEV_MISSING_TOOL",
            "install the missing tool and retry",
        );
        assert_eq!(check.status, CheckStatus::Failed);
        assert!(check.remediation.contains("retry"));
    }
}
