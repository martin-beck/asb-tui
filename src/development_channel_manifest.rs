// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Consumer for the ASB development-channel handoff manifest.
//!
//! ASB owns repository-head resolution and materialization.  The standalone
//! TUI only consumes the bounded, content-addressed diagnostic manifest and
//! never resolves a second source of truth.  This remains development-only:
//! missing authentication, signatures, and key management are not consulted.

use crate::sha256::digest_hex;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const ASB_REPOSITORY: &str = "https://github.com/martin-beck/agent-systems-benchmark.git";
pub const TUI_REPOSITORY: &str = "https://github.com/martin-beck/asb-tui.git";
pub const MAIN_REF: &str = "refs/heads/main";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentChannelManifest {
    pub schema_version: u64,
    pub channel: String,
    pub development_only: bool,
    pub asb_repository: String,
    pub asb_ref: String,
    pub asb_source_commit: String,
    pub asb_source_tree: String,
    pub tui_repository: String,
    pub tui_ref: String,
    pub tui_source_commit: String,
    pub tui_source_tree: String,
    pub executable_sha256: String,
    pub executable_size: u64,
    pub built_unix: u64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    Unavailable,
    Invalid,
    DigestMismatch,
    Stale,
}

impl ManifestError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "dev_channel_manifest_unavailable",
            Self::Invalid => "dev_channel_manifest_invalid",
            Self::DigestMismatch => "dev_channel_manifest_digest_mismatch",
            Self::Stale => "dev_channel_manifest_stale",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumedManifest {
    pub manifest: DevelopmentChannelManifest,
    pub sha256: String,
}

impl DevelopmentChannelManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(ManifestError::Invalid);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ManifestError::Invalid)?;
        value.validate_shape()?;
        Ok(value)
    }

    fn validate_shape(&self) -> Result<(), ManifestError> {
        if self.schema_version != 1
            || self.channel != "dev"
            || !self.development_only
            || self.asb_repository != ASB_REPOSITORY
            || self.asb_ref != MAIN_REF
            || self.tui_repository != TUI_REPOSITORY
            || self.tui_ref != MAIN_REF
            || self.executable_size == 0
            || self.built_unix == 0
            || !valid_identity(&self.asb_source_commit)
            || !valid_identity(&self.asb_source_tree)
            || !valid_identity(&self.tui_source_commit)
            || !valid_identity(&self.tui_source_tree)
            || !valid_digest(&self.executable_sha256)
        {
            return Err(ManifestError::Invalid);
        }
        Ok(())
    }

    pub fn validate_current_main(
        &self,
        asb_source_commit: &str,
        asb_source_tree: &str,
        tui_source_commit: &str,
        tui_source_tree: &str,
    ) -> Result<(), ManifestError> {
        if self.asb_source_commit != asb_source_commit
            || self.asb_source_tree != asb_source_tree
            || self.tui_source_commit != tui_source_commit
            || self.tui_source_tree != tui_source_tree
        {
            return Err(ManifestError::Stale);
        }
        Ok(())
    }

    pub fn validate_executable(&self, bytes: &[u8]) -> Result<(), ManifestError> {
        if self.executable_sha256 != digest_hex(bytes) || self.executable_size != bytes.len() as u64
        {
            return Err(ManifestError::DigestMismatch);
        }
        Ok(())
    }
}

pub fn consume(path: &Path) -> Result<ConsumedManifest, ManifestError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ManifestError::Unavailable)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_MANIFEST_BYTES
    {
        return Err(ManifestError::Invalid);
    }
    let bytes = fs::read(path).map_err(|_| ManifestError::Unavailable)?;
    let manifest = DevelopmentChannelManifest::parse(&bytes)?;
    Ok(ConsumedManifest {
        manifest,
        sha256: digest_hex(&bytes),
    })
}

pub fn consume_from_env() -> Result<Option<ConsumedManifest>, ManifestError> {
    let Some(value) = env::var_os("ASB_TUI_CHANNEL_MANIFEST") else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(ManifestError::Invalid);
    }
    consume(&path).map(Some)
}

fn valid_identity(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> DevelopmentChannelManifest {
        DevelopmentChannelManifest {
            schema_version: 1,
            channel: "dev".into(),
            development_only: true,
            asb_repository: ASB_REPOSITORY.into(),
            asb_ref: MAIN_REF.into(),
            asb_source_commit: "a".repeat(40),
            asb_source_tree: "b".repeat(40),
            tui_repository: TUI_REPOSITORY.into(),
            tui_ref: MAIN_REF.into(),
            tui_source_commit: "c".repeat(40),
            tui_source_tree: "d".repeat(40),
            executable_sha256: "e".repeat(64),
            executable_size: 1,
            built_unix: 1,
            warnings: vec!["development_missing_authentication_allowed".into()],
        }
    }

    #[test]
    fn consumes_manifest_and_reports_content_digest() {
        let bytes = serde_json::to_vec(&fixture()).unwrap();
        let path =
            std::env::temp_dir().join(format!("asb-tui-channel-manifest-{}", std::process::id()));
        fs::write(&path, bytes.clone()).unwrap();
        let consumed = consume(&path).unwrap();
        assert_eq!(consumed.sha256, digest_hex(&bytes));
        assert_eq!(consumed.manifest.channel, "dev");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_stale_current_main_without_falling_back() {
        let manifest = fixture();
        assert_eq!(
            manifest.validate_current_main(
                &"f".repeat(40),
                &"b".repeat(40),
                &"c".repeat(40),
                &"d".repeat(40)
            ),
            Err(ManifestError::Stale)
        );
    }

    #[test]
    fn rejects_unknown_fields_and_production_channels() {
        let mut value = serde_json::to_value(fixture()).unwrap();
        value["unexpected"] = true.into();
        assert_eq!(
            DevelopmentChannelManifest::parse(&serde_json::to_vec(&value).unwrap()),
            Err(ManifestError::Invalid)
        );
        value = serde_json::to_value(fixture()).unwrap();
        value["channel"] = "stable".into();
        assert_eq!(
            DevelopmentChannelManifest::parse(&serde_json::to_vec(&value).unwrap()),
            Err(ManifestError::Invalid)
        );
    }
}
