// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral projection for the development-channel materializer.
//!
//! The materializer owns cloning, building and activation.  This module owns
//! only the operator-facing handoff: bounded progress, public provenance and
//! recoverable outcomes.  In particular, an error never implies that an
//! existing installation was replaced.

use crate::development_lifecycle::Response;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Ready,
    Cloning,
    Building,
    Installing,
    Installed,
    Cancelled,
    RolledBack,
    Failed,
    Incompatible,
}

impl Phase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Cloning => "cloning source",
            Self::Building => "building temporary candidate",
            Self::Installing => "activating candidate",
            Self::Installed => "installed",
            Self::Cancelled => "cancelled; no active change",
            Self::RolledBack => "rolled back to previous install",
            Self::Failed => "failed; previous install preserved",
            Self::Incompatible => "incompatible manifest; install refused",
        }
    }

    const fn active(self) -> bool {
        matches!(self, Self::Cloning | Self::Building | Self::Installing)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandoffProjection {
    pub schema_version: u64,
    pub classification: &'static str,
    pub development_only: bool,
    pub channel: &'static str,
    pub phase: Phase,
    pub code: String,
    pub previous_install_preserved: bool,
    pub source_commit: Option<String>,
    pub source_tree: Option<String>,
    pub executable_sha256: Option<String>,
    pub warning: &'static str,
}

impl Default for HandoffProjection {
    fn default() -> Self {
        Self {
            schema_version: 1,
            classification: "development_only",
            development_only: true,
            channel: "dev",
            phase: Phase::Ready,
            code: "development_not_installed".into(),
            previous_install_preserved: true,
            source_commit: None,
            source_tree: None,
            executable_sha256: None,
            warning: "development-only; authentication and signatures are non-blocking",
        }
    }
}

impl HandoffProjection {
    pub fn operation(&self) -> &'static str {
        if self.code == "development_retry"
            || matches!(self.phase, Phase::Installed | Phase::RolledBack)
        {
            "upgrade"
        } else {
            "install"
        }
    }

    pub fn begin(&mut self) {
        self.phase = Phase::Cloning;
        self.code = "development_materializing".into();
        self.previous_install_preserved = true;
    }

    pub fn begin_retry(&mut self) {
        self.phase = Phase::Cloning;
        self.code = "development_retry".into();
        self.previous_install_preserved = true;
    }

    pub fn advance(&mut self) {
        self.phase = match self.phase {
            Phase::Cloning => Phase::Building,
            Phase::Building => Phase::Installing,
            other => other,
        };
    }

    pub fn cancel(&mut self) {
        if self.phase.active() {
            self.phase = Phase::Cancelled;
            self.code = "development_cancelled".into();
            self.previous_install_preserved = true;
        }
    }

    pub fn apply_response(&mut self, response: &Response) {
        self.code = response.code.to_owned();
        self.source_commit = response.source_commit.clone();
        self.source_tree = response.source_tree.clone();
        self.executable_sha256 = response.executable_sha256.clone();
        self.previous_install_preserved = !response.ok;
        self.phase = match response.code {
            "development_installed" | "development_launch_ready" => Phase::Installed,
            "development_activation_failed" => Phase::RolledBack,
            "development_manifest_incompatible" | "development_installation_invalid" => {
                Phase::Incompatible
            }
            "development_cancelled" => Phase::Cancelled,
            _ if response.ok => Phase::Installed,
            _ => Phase::Failed,
        };
    }

    pub fn human_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("development channel: {}", self.channel),
            format!("state: {}", self.phase.label()),
            format!("result: {}", self.code),
            format!(
                "previous install preserved: {}",
                self.previous_install_preserved
            ),
            format!("warning: {}", self.warning),
        ];
        if let Some(commit) = &self.source_commit {
            lines.push(format!("source commit: {commit}"));
        }
        if let Some(tree) = &self.source_tree {
            lines.push(format!("source tree: {tree}"));
        }
        if let Some(digest) = &self.executable_sha256 {
            lines.push(format!("executable sha256: {digest}"));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_and_cancel_are_recoverable() {
        let mut handoff = HandoffProjection::default();
        handoff.begin();
        handoff.advance();
        assert_eq!(handoff.phase, Phase::Building);
        handoff.cancel();
        assert_eq!(handoff.phase, Phase::Cancelled);
        assert!(handoff.previous_install_preserved);
    }

    #[test]
    fn activation_failure_reports_rollback_and_provenance() {
        let mut handoff = HandoffProjection::default();
        let response = Response {
            schema_version: 1,
            classification: "development_only",
            development_only: true,
            channel: "dev",
            ok: false,
            code: "development_activation_failed",
            installed: true,
            verified: false,
            source_commit: Some("a".repeat(40)),
            source_tree: Some("b".repeat(40)),
            executable_sha256: None,
        };
        handoff.apply_response(&response);
        assert_eq!(handoff.phase, Phase::RolledBack);
        let expected_commit = "a".repeat(40);
        assert_eq!(
            handoff.source_commit.as_deref(),
            Some(expected_commit.as_str())
        );
        assert!(
            handoff
                .human_lines()
                .iter()
                .any(|line| line.contains("rolled back"))
        );
    }

    #[test]
    fn rollback_retry_uses_upgrade_without_losing_previous_install() {
        let mut handoff = HandoffProjection {
            phase: Phase::RolledBack,
            ..HandoffProjection::default()
        };
        assert_eq!(handoff.operation(), "upgrade");
        handoff.begin_retry();
        assert_eq!(handoff.operation(), "upgrade");
        assert!(handoff.previous_install_preserved);
    }
}
