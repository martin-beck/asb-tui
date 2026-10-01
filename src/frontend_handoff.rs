// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned, process-bound handoff from the installed lifecycle to the frontend.

use crate::lifecycle::Installation;

pub const HANDOFF_VERSION: &str = "asb-tui-frontend/v1";
pub const DEFAULT_ENDPOINT: &str = "asb://control/v1";
pub const DEFAULT_CHANNEL: &str = "verified";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontendHandoff {
    pub version: &'static str,
    pub endpoint: String,
    pub channel: String,
    pub manifest_sha256: String,
    pub source_commit: String,
    pub source_tree: String,
    pub workspace_state_root: String,
    pub workspace_config_root: String,
    pub workspace_cache_root: String,
}

impl FrontendHandoff {
    pub fn from_installation(installation: &Installation) -> Self {
        Self {
            version: HANDOFF_VERSION,
            endpoint: installation.endpoint.clone(),
            channel: installation.channel.clone(),
            manifest_sha256: installation.manifest_sha256.clone(),
            source_commit: installation.source_commit.clone(),
            source_tree: installation.source_tree.clone(),
            workspace_state_root: installation.workspace_state_root.clone(),
            workspace_config_root: installation.workspace_config_root.clone(),
            workspace_cache_root: installation.workspace_cache_root.clone(),
        }
    }

    pub fn env_pairs(&self) -> [(&'static str, &str); 9] {
        [
            ("ASB_TUI_FRONTEND_HANDOFF", self.version),
            ("ASB_TUI_ENDPOINT", &self.endpoint),
            ("ASB_TUI_CHANNEL", &self.channel),
            ("ASB_TUI_MANIFEST_SHA256", &self.manifest_sha256),
            ("ASB_TUI_SOURCE_COMMIT", &self.source_commit),
            ("ASB_TUI_SOURCE_TREE", &self.source_tree),
            ("ASB_TUI_WORKSPACE_STATE_ROOT", &self.workspace_state_root),
            ("ASB_TUI_WORKSPACE_CONFIG_ROOT", &self.workspace_config_root),
            ("ASB_TUI_WORKSPACE_CACHE_ROOT", &self.workspace_cache_root),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_is_versioned_and_complete() {
        let installation = Installation {
            schema_version: 1,
            release: "v1.0.0".into(),
            executable_sha256: "a".repeat(64),
            source_commit: "b".repeat(40),
            source_tree: "c".repeat(40),
            target: "x86_64-unknown-linux-gnu".into(),
            bundle: "asb-tui-v1-linux-x86_64".into(),
            asb_version: "0.1.0".into(),
            protocol_version: 1,
            coordinator_version: "v0.3.5".into(),
            coordinator_commit: "d".repeat(40),
            quality_version: "v0.23.0".into(),
            quality_commit: "e".repeat(40),
            classification: "verified_extension".into(),
            endpoint: DEFAULT_ENDPOINT.into(),
            channel: DEFAULT_CHANNEL.into(),
            manifest_sha256: "f".repeat(64),
            workspace_state_root: "/state".into(),
            workspace_config_root: "/config".into(),
            workspace_cache_root: "/cache".into(),
        };
        let handoff = FrontendHandoff::from_installation(&installation);
        assert_eq!(handoff.version, HANDOFF_VERSION);
        assert_eq!(handoff.env_pairs().len(), 9);
        assert_eq!(handoff.endpoint, DEFAULT_ENDPOINT);
    }
}
