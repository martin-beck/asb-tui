// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Typed, renderer-neutral provider setup drafts.
//!
//! The wizard collects bounded identifiers and digest-only authentication
//! references.  This module is the narrow seam between that draft and the
//! runner-owned configuration operation: catalog membership and compatibility
//! are checked before a draft can be reviewed or applied, and an apply is
//! consumed exactly once.

use crate::{
    agent_catalog::AgentCatalog,
    control_codec::{ConfigurationSelection, ProviderAuthMethod, ProviderCatalog, Revision},
    provider_catalog::{AgentScope, ProviderDefaultDraft},
};

const MAX_ID_BYTES: usize = 128;
const MAX_DIGEST_BYTES: usize = 64;
const MAX_LABEL_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderSetupError {
    Invalid(String),
    StaleCatalog,
    AlreadyApplied,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderSetupDraft {
    selection: ConfigurationSelection,
    catalog_generation: Revision,
    catalog_digest: String,
    configuration_label: String,
}

impl ProviderSetupDraft {
    /// Wrap an already validated selection for a disconnected development
    /// fixture. The production/negotiated path should use
    /// [`Self::from_wizard_values`] so catalog membership is checked.
    pub fn from_selection_for_development(
        selection: ConfigurationSelection,
    ) -> Result<Self, ProviderSetupError> {
        if selection.agent_ids.is_empty()
            || selection.agent_ids.len() > 64
            || selection.agent_ids.iter().any(|id| !is_safe_id(id))
        {
            return Err(ProviderSetupError::Invalid(
                "agent selection is invalid".into(),
            ));
        }
        let provider_id = bounded_id("provider", &selection.provider_id)?;
        let model_id = bounded_id("model", &selection.model_id)?;
        if matches!(
            selection.auth_method,
            ProviderAuthMethod::CredentialReference
        ) != selection.credential_reference_sha256.is_some()
        {
            return Err(ProviderSetupError::Invalid(
                "credential reference does not match authentication method".into(),
            ));
        }
        if let Some(digest) = selection.credential_reference_sha256.as_deref()
            && (digest.len() != MAX_DIGEST_BYTES
                || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(ProviderSetupError::Invalid(
                "credential reference must be a SHA-256 digest".into(),
            ));
        }
        Ok(Self {
            selection: ConfigurationSelection {
                provider_id,
                model_id,
                ..selection
            },
            catalog_generation: Revision(1),
            catalog_digest: "0".repeat(MAX_DIGEST_BYTES),
            configuration_label: "development defaults".into(),
        })
    }

    /// Build a draft from wizard values after validating every value against
    /// the authoritative agent/provider catalog. No network or credentials are
    /// consulted here.
    pub fn from_wizard_values(
        values: &[String; 7],
        agents: &AgentCatalog,
        providers: &ProviderCatalog,
    ) -> Result<Self, ProviderSetupError> {
        let agent_ids = values[0]
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let auth_value = values[4].trim();
        let (auth_method, credential_reference_sha256) = parse_auth(auth_value)?;
        let draft = ProviderDefaultDraft {
            scope: AgentScope::selected(agent_ids).map_err(ProviderSetupError::Invalid)?,
            provider_id: bounded_id("provider", values[1].trim())?,
            model_id: bounded_id("model", values[2].trim())?,
            auth_method,
            credential_reference_sha256,
        };
        let selection = draft
            .into_selection(agents, providers)
            .map_err(ProviderSetupError::Invalid)?;
        let label = bounded_label(values[3].trim())?;
        if providers.generation.0 == 0 {
            return Err(ProviderSetupError::Invalid(
                "provider catalog generation is missing".into(),
            ));
        }
        Ok(Self {
            selection,
            catalog_generation: providers.generation,
            catalog_digest: providers.catalog_sha256.clone(),
            configuration_label: label,
        })
    }

    #[must_use]
    pub const fn selection(&self) -> &ConfigurationSelection {
        &self.selection
    }

    #[must_use]
    pub const fn catalog_generation(&self) -> Revision {
        self.catalog_generation
    }

    #[must_use]
    pub fn catalog_digest(&self) -> &str {
        &self.catalog_digest
    }

    #[must_use]
    pub fn configuration_label(&self) -> &str {
        &self.configuration_label
    }

    /// A review projection contains only identifiers and the authentication
    /// method. Digest references are deliberately represented by presence, not
    /// copied into visible UI text.
    #[must_use]
    pub fn review(&self) -> ProviderSetupReview {
        ProviderSetupReview {
            agent_ids: self.selection.agent_ids.clone(),
            provider_id: self.selection.provider_id.clone(),
            model_id: self.selection.model_id.clone(),
            auth_method: self.selection.auth_method,
            has_credential_reference: self.selection.credential_reference_sha256.is_some(),
            configuration_label: self.configuration_label.clone(),
            catalog_generation: self.catalog_generation,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderSetupReview {
    pub agent_ids: Vec<String>,
    pub provider_id: String,
    pub model_id: String,
    pub auth_method: ProviderAuthMethod,
    pub has_credential_reference: bool,
    pub configuration_label: String,
    pub catalog_generation: Revision,
}

/// Apply gate for a completed setup review. The closure is the only operation
/// allowed to cross into the runner; the draft is consumed before invocation,
/// so cancellation, a stale catalog, or a failed apply can never replay it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AtomicProviderSetup {
    applied_generation: Option<Revision>,
}

impl AtomicProviderSetup {
    pub fn apply<E: std::fmt::Debug>(
        &mut self,
        draft: ProviderSetupDraft,
        current_catalog_generation: Revision,
        apply: impl FnOnce(&ConfigurationSelection) -> Result<(), E>,
    ) -> Result<ProviderSetupReview, ProviderSetupError> {
        if self.applied_generation.is_some() {
            return Err(ProviderSetupError::AlreadyApplied);
        }
        if draft.catalog_generation != current_catalog_generation {
            return Err(ProviderSetupError::StaleCatalog);
        }
        let review = draft.review();
        // `draft` is moved into this operation and cannot be retained by the
        // gate. A failed closure therefore leaves no replayable pending draft.
        apply(&draft.selection)
            .map_err(|error| ProviderSetupError::Invalid(format!("apply failed: {error:?}")))?;
        self.applied_generation = Some(current_catalog_generation);
        Ok(review)
    }

    #[must_use]
    pub const fn applied_generation(&self) -> Option<Revision> {
        self.applied_generation
    }

    pub fn restart(&mut self) {
        self.applied_generation = None;
    }
}

fn bounded_id(field: &str, value: &str) -> Result<String, ProviderSetupError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES || !is_safe_id(value) {
        return Err(ProviderSetupError::Invalid(format!(
            "{field} is not a supported identifier"
        )));
    }
    Ok(value.to_owned())
}

fn bounded_label(value: &str) -> Result<String, ProviderSetupError> {
    if value.len() > MAX_LABEL_BYTES || value.chars().any(char::is_control) {
        return Err(ProviderSetupError::Invalid(
            "configuration label is invalid".into(),
        ));
    }
    Ok(value.to_owned())
}

fn parse_auth(value: &str) -> Result<(ProviderAuthMethod, Option<String>), ProviderSetupError> {
    match value {
        "none" => Ok((ProviderAuthMethod::None, None)),
        "local_daemon" => Ok((ProviderAuthMethod::LocalDaemon, None)),
        value => {
            let Some(digest) = value.strip_prefix("credential_reference:") else {
                return Err(ProviderSetupError::Invalid(
                    "authentication must use a supported non-secret reference".into(),
                ));
            };
            if digest.len() != MAX_DIGEST_BYTES
                || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(ProviderSetupError::Invalid(
                    "credential reference must be a SHA-256 digest".into(),
                ));
            }
            Ok((
                ProviderAuthMethod::CredentialReference,
                Some(digest.to_owned()),
            ))
        }
    }
}

fn is_safe_id(value: &str) -> bool {
    value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent_catalog::{
            AgentAvailability, AgentCatalogEntry, AgentPackage, AgentProvenance, AgentSigner,
            AgentTarget,
        },
        control_codec::{ProviderAvailability, ProviderCatalogEntry, ProviderModel},
    };

    fn agents() -> AgentCatalog {
        AgentCatalog {
            runner_instance_id: "runner".into(),
            generation: 1,
            catalog_sha256: "a".repeat(64),
            target: AgentTarget {
                operating_system: "linux".into(),
                architecture: "x86_64".into(),
                libc: "glibc".into(),
                libc_version: "2.35".into(),
            },
            refreshed: true,
            agents: vec![AgentCatalogEntry {
                agent_id: "agent-a".into(),
                target: AgentTarget {
                    operating_system: "linux".into(),
                    architecture: "x86_64".into(),
                    libc: "glibc".into(),
                    libc_version: "2.35".into(),
                },
                package: Some(AgentPackage {
                    package_id: "pkg".into(),
                    version: "1.0.0".into(),
                    sha256: "b".repeat(64),
                    signature_sha256: "c".repeat(64),
                    signer: AgentSigner {
                        key_id: "key".into(),
                        principal: "dev".into(),
                    },
                }),
                provenance: Some(AgentProvenance {
                    source_revision: "e".repeat(40),
                    manifest_sha256: "d".repeat(64),
                    sbom_sha256: "d".repeat(64),
                    license_ref: "MIT".into(),
                }),
                capabilities: vec!["benchmark".into()],
                availability: AgentAvailability::Available,
            }],
        }
    }

    fn providers() -> ProviderCatalog {
        ProviderCatalog {
            runner_instance_id: "runner".into(),
            generation: Revision(3),
            catalog_sha256: "f".repeat(64),
            refreshed: true,
            providers: vec![ProviderCatalogEntry {
                provider_id: "provider-a".into(),
                display_name: "Provider A".into(),
                auth_methods: vec![ProviderAuthMethod::None],
                availability: ProviderAvailability::Available,
                models: vec![ProviderModel {
                    model_id: "model-a".into(),
                    revision: "1".into(),
                    availability: ProviderAvailability::Available,
                }],
            }],
        }
    }

    fn values() -> [String; 7] {
        [
            "agent-a".into(),
            "provider-a".into(),
            "model-a".into(),
            "defaults".into(),
            "none".into(),
            "record".into(),
            "offline".into(),
        ]
    }

    #[test]
    fn draft_is_catalog_bound_and_review_is_secret_free() {
        let draft =
            ProviderSetupDraft::from_wizard_values(&values(), &agents(), &providers()).unwrap();
        assert_eq!(draft.selection().provider_id, "provider-a");
        assert!(!format!("{:?}", draft.review()).contains(&"a".repeat(64)));
        assert_eq!(draft.review().catalog_generation, Revision(3));
    }

    #[test]
    fn unsupported_model_and_raw_secret_are_rejected() {
        let mut input = values();
        input[2] = "unsupported".into();
        assert!(ProviderSetupDraft::from_wizard_values(&input, &agents(), &providers()).is_err());
        let mut input = values();
        input[4] = "sk-development-secret".into();
        assert!(ProviderSetupDraft::from_wizard_values(&input, &agents(), &providers()).is_err());
    }

    #[test]
    fn apply_is_atomic_single_use_and_failed_apply_is_not_replayed() {
        let draft =
            ProviderSetupDraft::from_wizard_values(&values(), &agents(), &providers()).unwrap();
        let mut gate = AtomicProviderSetup::default();
        let failed = gate.apply(draft, Revision(3), |_selection| {
            Err::<(), _>("fixture failure")
        });
        assert!(failed.is_err());
        assert_eq!(gate.applied_generation(), None);

        let draft =
            ProviderSetupDraft::from_wizard_values(&values(), &agents(), &providers()).unwrap();
        let review = gate
            .apply(draft, Revision(3), |_selection| Ok::<(), &str>(()))
            .unwrap();
        assert_eq!(review.provider_id, "provider-a");
        assert!(matches!(
            gate.apply(
                ProviderSetupDraft::from_wizard_values(&values(), &agents(), &providers()).unwrap(),
                Revision(3),
                |_s| Ok::<(), &str>(())
            ),
            Err(ProviderSetupError::AlreadyApplied)
        ));
        gate.restart();
        assert_eq!(gate.applied_generation(), None);
    }
}
