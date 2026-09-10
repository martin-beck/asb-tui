// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bootstrap-safe compile-time release policy for the delegated lifecycle.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const CHANNEL_STATUS: &str = include_str!("../release/channel-status.json");
const SOURCE_COMMIT: Option<&str> = option_env!("ASB_TUI_SOURCE_COMMIT");
const SOURCE_TREE: Option<&str> = option_env!("ASB_TUI_SOURCE_TREE");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseClassification {
    SourceOnlyUnverified,
    VerifiedExtension,
}

impl ReleaseClassification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceOnlyUnverified => "source_only_unverified",
            Self::VerifiedExtension => "verified_extension",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelStatus {
    schema_version: u64,
    trust_policy_version: u64,
    classification: String,
    installable: bool,
    release_artifacts_published: bool,
    promotion_requirements: BTreeMap<String, bool>,
}

const REQUIRED_PROMOTION_REQUIREMENTS: [&str; 8] = [
    "aarch64_release_qualification",
    "asb_external_protocol",
    "asb_lifecycle_router",
    "hosted_exact_release_ci",
    "interactive_ui",
    "signed_release_bundle",
    "trusted_exact_release_ci",
    "x86_64_release_qualification",
];

fn status() -> Option<ChannelStatus> {
    serde_json::from_str(CHANNEL_STATUS).ok()
}

pub fn compiled_classification() -> ReleaseClassification {
    status().map_or(ReleaseClassification::SourceOnlyUnverified, |status| {
        classification_for(&status)
    })
}

fn classification_for(status: &ChannelStatus) -> ReleaseClassification {
    let requirements: BTreeSet<_> = status
        .promotion_requirements
        .keys()
        .map(String::as_str)
        .collect();
    let expected: BTreeSet<_> = REQUIRED_PROMOTION_REQUIREMENTS.into_iter().collect();
    if status.schema_version == 1
        && status.trust_policy_version == 1
        && status.classification == "verified_extension"
        && status.installable
        && status.release_artifacts_published
        && requirements == expected
        && status.promotion_requirements.values().all(|value| *value)
    {
        ReleaseClassification::VerifiedExtension
    } else {
        ReleaseClassification::SourceOnlyUnverified
    }
}

/// The signed external bundle manifest supplies dynamic source and artifact identities.
pub fn channel_permits_install() -> bool {
    compiled_classification() == ReleaseClassification::VerifiedExtension
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildIdentity {
    pub source_commit: String,
    pub source_tree: String,
}

pub fn compiled_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-gnu";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-gnu";
    #[allow(unreachable_code)]
    "unsupported"
}

/// Return source identity injected by the release builder, never an embedded self hash.
pub fn promoted_self_identity(release: &str) -> Option<BuildIdentity> {
    build_identity_from(
        compiled_classification(),
        release,
        env!("CARGO_PKG_VERSION"),
        SOURCE_COMMIT,
        SOURCE_TREE,
    )
}

fn build_identity_from(
    classification: ReleaseClassification,
    release: &str,
    package_version: &str,
    source_commit: Option<&str>,
    source_tree: Option<&str>,
) -> Option<BuildIdentity> {
    if classification != ReleaseClassification::VerifiedExtension
        || release != format!("v{package_version}")
    {
        return None;
    }
    let source_commit = source_commit?;
    let source_tree = source_tree?;
    if !valid_hex(source_commit, 40) || !valid_hex(source_tree, 40) {
        return None;
    }
    Some(BuildIdentity {
        source_commit: source_commit.into(),
        source_tree: source_tree.into(),
    })
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified_status() -> ChannelStatus {
        serde_json::from_str(
            r#"{"schema_version":1,"trust_policy_version":1,"classification":"verified_extension","installable":true,"release_artifacts_published":true,"promotion_requirements":{"asb_external_protocol":true,"asb_lifecycle_router":true,"interactive_ui":true,"x86_64_release_qualification":true,"aarch64_release_qualification":true,"signed_release_bundle":true,"hosted_exact_release_ci":true,"trusted_exact_release_ci":true}}"#,
        )
        .unwrap()
    }

    #[test]
    fn source_channel_is_fail_closed_without_dynamic_embedded_identity() {
        assert_eq!(
            compiled_classification(),
            ReleaseClassification::SourceOnlyUnverified
        );
        assert!(!channel_permits_install());
        assert!(promoted_self_identity("v0.1.0").is_none());
        assert!(!CHANNEL_STATUS.contains("source_commit"));
        assert!(!CHANNEL_STATUS.contains("source_tree"));
        assert!(!CHANNEL_STATUS.contains("executable_sha256"));
        assert!(!CHANNEL_STATUS.contains("manifest_sha256"));
    }

    #[test]
    fn verified_policy_requires_every_static_gate_but_no_self_hash() {
        let mut status = verified_status();
        assert_eq!(
            classification_for(&status),
            ReleaseClassification::VerifiedExtension
        );
        status.trust_policy_version = 2;
        assert_eq!(
            classification_for(&status),
            ReleaseClassification::SourceOnlyUnverified
        );
        let mut status = verified_status();
        status
            .promotion_requirements
            .insert("unexpected".into(), true);
        assert_eq!(
            classification_for(&status),
            ReleaseClassification::SourceOnlyUnverified
        );
        let mut status = verified_status();
        status
            .promotion_requirements
            .insert("interactive_ui".into(), false);
        assert_eq!(
            classification_for(&status),
            ReleaseClassification::SourceOnlyUnverified
        );
    }

    #[test]
    fn build_identity_contract_is_realizable_without_self_reference() {
        assert!(valid_hex(&"a".repeat(40), 40));
        assert!(!valid_hex(&"A".repeat(40), 40));
        assert!(!valid_hex(&"a".repeat(64), 40));
        assert_eq!(compiled_target(), "x86_64-unknown-linux-gnu");
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.0");
        let identity = build_identity_from(
            ReleaseClassification::VerifiedExtension,
            "v0.1.0",
            "0.1.0",
            Some("a7ca8e07f177fc6a647b3297df624137cfb85e86"),
            Some("8556090e21336df0766263e6244084865f92d1a3"),
        )
        .unwrap();
        assert_eq!(identity.source_commit.len(), 40);
        assert!(
            build_identity_from(
                ReleaseClassification::VerifiedExtension,
                "v0.1.1",
                "0.1.0",
                Some("a7ca8e07f177fc6a647b3297df624137cfb85e86"),
                Some("8556090e21336df0766263e6244084865f92d1a3"),
            )
            .is_none()
        );
    }
}
