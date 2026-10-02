// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral provider catalog and scoped-default helpers.
//!
//! The runner remains authoritative for provider availability and generations.
//! This module only turns an authenticated catalog into bounded wizard choices
//! and validates the scope of a configuration draft.  It never accepts or
//! transports a raw credential.

use crate::{
    agent_catalog::AgentCatalog,
    control_codec::{
        ConfigurationSelection, ProviderAuthMethod, ProviderAvailability, ProviderCatalog,
        ProviderCatalogEntry, ProviderModel, Revision,
    },
    wizard_catalog::WizardOption,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_SCOPED_AGENTS: usize = 64;

/// Credential-free OpenRouter fixture used by the development wizard. It is
/// deliberately local and warning-only: production catalogs still come from
/// the authenticated runner and API keys are represented only by references.
pub fn development_openrouter_catalog() -> ProviderCatalog {
    ProviderCatalog {
        runner_instance_id: "development-fixture".into(),
        generation: Revision(1),
        catalog_sha256: "0".repeat(64),
        refreshed: false,
        providers: vec![ProviderCatalogEntry {
            provider_id: "openrouter".into(),
            display_name: "OpenRouter (development-only)".into(),
            auth_methods: vec![
                ProviderAuthMethod::CredentialReference,
                ProviderAuthMethod::None,
            ],
            availability: ProviderAvailability::Available,
            models: vec![
                ProviderModel {
                    model_id: "openai/gpt-4o".into(),
                    revision: "fixture".into(),
                    availability: ProviderAvailability::Available,
                },
                ProviderModel {
                    model_id: "anthropic/claude-3.5-sonnet".into(),
                    revision: "fixture".into(),
                    availability: ProviderAvailability::Available,
                },
            ],
        }],
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub enum AgentScope {
    All,
    Selected(Vec<String>),
}

/// The non-secret portion of a provider default. Credential references are
/// resolver-owned SHA-256 locators; raw credentials have no representable
/// field and therefore cannot be written by this module.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDefaultRecord {
    pub scope: AgentScope,
    pub provider_id: String,
    pub model_id: String,
    pub auth_method: ProviderAuthMethod,
    pub credential_reference_sha256: Option<String>,
}

impl ProviderDefaultRecord {
    pub fn from_draft(draft: &ProviderDefaultDraft) -> Self {
        Self {
            scope: draft.scope.clone(),
            provider_id: draft.provider_id.clone(),
            model_id: draft.model_id.clone(),
            auth_method: draft.auth_method,
            credential_reference_sha256: draft.credential_reference_sha256.clone(),
        }
    }

    fn validate(&self) -> Result<(), ProviderDefaultsError> {
        if !is_safe_id(&self.provider_id) || !is_safe_id(&self.model_id) {
            return Err(ProviderDefaultsError::Invalid(
                "provider/model id is invalid".into(),
            ));
        }
        if let AgentScope::Selected(ids) = &self.scope {
            AgentScope::selected(ids.clone()).map_err(ProviderDefaultsError::Invalid)?;
        }
        if matches!(self.auth_method, ProviderAuthMethod::CredentialReference)
            != self.credential_reference_sha256.is_some()
        {
            return Err(ProviderDefaultsError::Invalid(
                "credential reference does not match authentication".into(),
            ));
        }
        if let Some(digest) = &self.credential_reference_sha256
            && (digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(ProviderDefaultsError::Invalid(
                "credential reference must be a SHA-256 digest".into(),
            ));
        }
        Ok(())
    }
}

const DEFAULTS_SCHEMA_VERSION: u32 = 1;
const MAX_DEFAULTS_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedProviderDefaults {
    pub schema_version: u32,
    pub entries: Vec<ProviderDefaultRecord>,
}

impl Default for SharedProviderDefaults {
    fn default() -> Self {
        Self {
            schema_version: DEFAULTS_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

impl SharedProviderDefaults {
    pub fn validate(&self) -> Result<(), ProviderDefaultsError> {
        if self.schema_version != DEFAULTS_SCHEMA_VERSION || self.entries.len() > 64 {
            return Err(ProviderDefaultsError::Invalid(
                "unsupported or oversized defaults".into(),
            ));
        }
        for entry in &self.entries {
            entry.validate()?;
        }
        Ok(())
    }

    /// Replace the default for a scope only after catalog validation. The
    /// previous state remains untouched if provider/model/auth validation fails.
    pub fn apply(
        &mut self,
        draft: ProviderDefaultDraft,
        agents: &AgentCatalog,
        providers: &ProviderCatalog,
    ) -> Result<ConfigurationSelection, ProviderDefaultsError> {
        let selection = draft
            .clone()
            .into_selection(agents, providers)
            .map_err(ProviderDefaultsError::Invalid)?;
        let record = ProviderDefaultRecord::from_draft(&draft);
        record.validate()?;
        self.entries.retain(|old| old.scope != record.scope);
        self.entries.push(record);
        Ok(selection)
    }

    pub fn rollback(&mut self, previous: Self) -> Result<(), ProviderDefaultsError> {
        previous.validate()?;
        *self = previous;
        Ok(())
    }

    pub fn for_agent(&self, agent_id: &str) -> Option<&ProviderDefaultRecord> {
        self.entries.iter().rev().find(|entry| match &entry.scope {
            AgentScope::All => true,
            AgentScope::Selected(ids) => ids.iter().any(|id| id == agent_id),
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "{} provider default{}",
            self.entries.len(),
            if self.entries.len() == 1 { "" } else { "s" }
        )
    }

    /// A renderer-neutral diagnostic that is safe to show in human or JSON
    /// status views. It contains only identifiers and credential digests.
    pub fn diagnostic(&self) -> ProviderDefaultsDiagnostic {
        ProviderDefaultsDiagnostic {
            schema_version: self.schema_version,
            entries: self.entries.len(),
            scopes: self
                .entries
                .iter()
                .map(|entry| match &entry.scope {
                    AgentScope::All => "all".to_owned(),
                    AgentScope::Selected(ids) => format!("selected:{}", ids.join(",")),
                })
                .collect(),
            credential_references: self
                .entries
                .iter()
                .filter_map(|entry| entry.credential_reference_sha256.as_deref())
                .map(|digest| format!("sha256:{digest}"))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDefaultsDiagnostic {
    pub schema_version: u32,
    pub entries: usize,
    pub scopes: Vec<String>,
    pub credential_references: Vec<String>,
}

impl ProviderDefaultsDiagnostic {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl fmt::Display for ProviderDefaultsDiagnostic {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            output,
            "schema {}: {} provider default(s)",
            self.schema_version, self.entries
        )?;
        if !self.scopes.is_empty() {
            write!(output, " [{}]", self.scopes.join(", "))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderDefaultsError {
    Invalid(String),
    Io(io::ErrorKind),
    TooLarge,
    SymlinkRefused,
    NotRegularFile,
}

impl From<io::Error> for ProviderDefaultsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl fmt::Display for ProviderDefaultsError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(output, "invalid provider defaults: {reason}"),
            Self::Io(kind) => write!(output, "provider defaults I/O failure: {kind:?}"),
            Self::TooLarge => output.write_str("provider defaults file is too large"),
            Self::SymlinkRefused => output.write_str("provider defaults symlink refused"),
            Self::NotRegularFile => {
                output.write_str("provider defaults path is not a regular file")
            }
        }
    }
}

/// Private, atomic persistence for shared defaults. It deliberately mirrors
/// ConfigurationStore's symlink and directory checks, but has an independent
/// file so changing frontend preferences cannot corrupt provider defaults.
pub struct ProviderDefaultsStore {
    path: PathBuf,
}

impl ProviderDefaultsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn load(&self) -> Result<SharedProviderDefaults, ProviderDefaultsError> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if metadata.file_type().is_symlink() {
            return Err(ProviderDefaultsError::SymlinkRefused);
        }
        if !metadata.is_file() {
            return Err(ProviderDefaultsError::NotRegularFile);
        }
        if metadata.len() > MAX_DEFAULTS_BYTES {
            return Err(ProviderDefaultsError::TooLarge);
        }
        let defaults: SharedProviderDefaults =
            serde_json::from_str(&fs::read_to_string(&self.path)?)
                .map_err(|e| ProviderDefaultsError::Invalid(e.to_string()))?;
        defaults.validate()?;
        Ok(defaults)
    }
    pub fn load_or_default(&self) -> Result<SharedProviderDefaults, ProviderDefaultsError> {
        match self.load() {
            Ok(value) => Ok(value),
            Err(ProviderDefaultsError::Io(io::ErrorKind::NotFound)) => {
                let parent = self.path.parent().ok_or_else(|| {
                    ProviderDefaultsError::Invalid("defaults path has no parent".into())
                })?;
                let metadata = fs::symlink_metadata(parent)?;
                #[cfg(unix)]
                let private = {
                    use std::os::unix::fs::{MetadataExt, PermissionsExt};
                    metadata.is_dir()
                        && metadata.uid() == rustix::process::geteuid().as_raw()
                        && metadata.permissions().mode() & 0o077 == 0
                };
                #[cfg(not(unix))]
                let private = metadata.is_dir();
                if private {
                    Ok(SharedProviderDefaults::default())
                } else {
                    Err(ProviderDefaultsError::Invalid(
                        "defaults directory is not a private directory owned by the current user"
                            .into(),
                    ))
                }
            }
            Err(error) => Err(error),
        }
    }
    pub fn save(&self, defaults: &SharedProviderDefaults) -> Result<(), ProviderDefaultsError> {
        defaults.validate()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ProviderDefaultsError::Invalid("defaults path has no parent".into()))?;
        let metadata = fs::symlink_metadata(parent)?;
        if metadata.file_type().is_symlink() {
            return Err(ProviderDefaultsError::SymlinkRefused);
        }
        if !metadata.is_dir() {
            return Err(ProviderDefaultsError::NotRegularFile);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(ProviderDefaultsError::Invalid(
                    "defaults directory is not private".into(),
                ));
            }
        }
        if let Ok(existing) = fs::symlink_metadata(&self.path) {
            if existing.file_type().is_symlink() {
                return Err(ProviderDefaultsError::SymlinkRefused);
            }
            if !existing.is_file() {
                return Err(ProviderDefaultsError::NotRegularFile);
            }
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ProviderDefaultsError::Io(io::ErrorKind::Other))?
            .as_nanos();
        let temp = parent.join(format!(".asb-tui-provider-defaults-{nonce}.tmp"));
        let text = serde_json::to_string_pretty(defaults)
            .map_err(|e| ProviderDefaultsError::Invalid(e.to_string()))?;
        let result = (|| {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)?;
            #[cfg(unix)]
            fs::File::open(parent)?.sync_all()?;
            Ok::<(), io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(ProviderDefaultsError::from)
    }
}

impl AgentScope {
    pub fn selected<I, S>(agents: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut agents: Vec<String> = agents.into_iter().map(Into::into).collect();
        agents.sort();
        agents.dedup();
        if agents.is_empty() || agents.len() > MAX_SCOPED_AGENTS {
            return Err("agent scope must contain between one and 64 agents".into());
        }
        if agents.iter().any(|id| !is_safe_id(id)) {
            return Err("agent scope contains an invalid agent id".into());
        }
        Ok(Self::Selected(agents))
    }

    pub fn resolve(&self, catalog: &AgentCatalog) -> Result<Vec<String>, String> {
        let available: Vec<&str> = catalog
            .agents
            .iter()
            .filter(|entry| {
                matches!(
                    entry.availability,
                    crate::agent_catalog::AgentAvailability::Available
                )
            })
            .map(|entry| entry.agent_id.as_str())
            .collect();
        let ids = match self {
            Self::All => available.into_iter().map(str::to_owned).collect(),
            Self::Selected(ids) => {
                if ids.iter().any(|id| !available.contains(&id.as_str())) {
                    return Err("agent scope contains an unavailable agent".into());
                }
                ids.clone()
            }
        };
        if ids.is_empty() {
            return Err("agent scope has no available agents".into());
        }
        Ok(ids)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDefaultDraft {
    pub scope: AgentScope,
    pub provider_id: String,
    pub model_id: String,
    pub auth_method: ProviderAuthMethod,
    /// Digest of a resolver-owned credential locator; the secret itself is
    /// intentionally not representable here.
    pub credential_reference_sha256: Option<String>,
}

impl ProviderDefaultDraft {
    pub fn into_selection(
        self,
        agents: &AgentCatalog,
        providers: &ProviderCatalog,
    ) -> Result<ConfigurationSelection, String> {
        if self.provider_id.is_empty() || self.model_id.is_empty() {
            return Err("provider and model are required".into());
        }
        let provider = providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == self.provider_id)
            .ok_or_else(|| "provider is not present in the current catalog".to_owned())?;
        if !matches!(provider.availability, ProviderAvailability::Available) {
            return Err("provider is unavailable".into());
        }
        let model = provider
            .models
            .iter()
            .find(|model| model.model_id == self.model_id)
            .ok_or_else(|| "model is not present for the selected provider".to_owned())?;
        if !matches!(model.availability, ProviderAvailability::Available) {
            return Err("model is unavailable".into());
        }
        if !provider.auth_methods.contains(&self.auth_method) {
            return Err("authentication method is not supported by the provider".into());
        }
        if matches!(self.auth_method, ProviderAuthMethod::CredentialReference)
            != self.credential_reference_sha256.is_some()
        {
            return Err(
                "credential reference is required only for credential authentication".into(),
            );
        }
        let agent_ids = self.scope.resolve(agents)?;
        Ok(ConfigurationSelection {
            agent_ids,
            provider_id: self.provider_id,
            model_id: model.model_id.clone(),
            auth_method: self.auth_method,
            credential_reference_sha256: self.credential_reference_sha256,
        })
    }
}

/// Return the newest generation observed by the frontend. Equal generations
/// are allowed (a refresh may simply report unchanged content); older ones
/// must be rejected before they can replace wizard choices.
pub fn accept_generation(current: Option<Revision>, next: Revision) -> Result<(), String> {
    if next.0 == 0 || current.is_some_and(|value| next.0 < value.0) {
        return Err("provider catalog generation is stale".into());
    }
    Ok(())
}

/// Build the wizard's provider/model sections while preserving unavailable
/// entries as disabled choices with an explanatory label.
pub fn wizard_options(
    catalog: &ProviderCatalog,
) -> Result<(Vec<WizardOption>, Vec<WizardOption>), String> {
    let providers = catalog
        .providers
        .iter()
        .map(|provider| {
            let available = matches!(provider.availability, ProviderAvailability::Available);
            WizardOption::new(
                provider.provider_id.clone(),
                if available {
                    provider.display_name.clone()
                } else {
                    format!("{} (unavailable)", provider.display_name)
                },
                available,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let models = catalog
        .providers
        .iter()
        .flat_map(|provider| {
            provider.models.iter().map(move |model| {
                model_option(
                    provider.provider_id.as_str(),
                    model,
                    matches!(provider.availability, ProviderAvailability::Available),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((providers, models))
}

/// A renderer-neutral provider add/edit session.  The committed profile and
/// catalog remain intact while a draft is edited or refreshed; cancellation
/// and failed refreshes therefore cannot erase the last valid configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderProfile {
    pub provider_id: String,
    pub display_name: String,
    pub auth_method: ProviderAuthMethod,
    pub credential_reference_sha256: Option<String>,
}

impl ProviderProfile {
    pub fn validate(&self) -> Result<(), String> {
        if !is_safe_id(&self.provider_id) || self.display_name.trim().is_empty() {
            return Err("provider profile identity is invalid".into());
        }
        if matches!(self.auth_method, ProviderAuthMethod::CredentialReference)
            != self.credential_reference_sha256.is_some()
        {
            return Err("credential reference does not match authentication".into());
        }
        if let Some(digest) = &self.credential_reference_sha256
            && (digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("credential reference must be a SHA-256 digest".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderRefreshError {
    Invalid(String),
    Unavailable(String),
    Cancelled,
}

impl fmt::Display for ProviderRefreshError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(output, "invalid provider profile: {reason}"),
            Self::Unavailable(reason) => write!(output, "provider catalog unavailable: {reason}"),
            Self::Cancelled => output.write_str("provider refresh cancelled"),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectedProviderRegistry {
    committed_profile: Option<ProviderProfile>,
    draft_profile: Option<ProviderProfile>,
    catalog: Option<ProviderCatalog>,
}

impl ConnectedProviderRegistry {
    pub fn new(
        catalog: Option<ProviderCatalog>,
        profile: Option<ProviderProfile>,
    ) -> Result<Self, String> {
        if let Some(profile) = &profile {
            profile.validate()?;
        }
        Ok(Self {
            committed_profile: profile,
            draft_profile: None,
            catalog,
        })
    }
    pub fn profile(&self) -> Option<&ProviderProfile> {
        self.committed_profile.as_ref()
    }
    pub fn draft(&self) -> Option<&ProviderProfile> {
        self.draft_profile.as_ref()
    }
    pub fn catalog(&self) -> Option<&ProviderCatalog> {
        self.catalog.as_ref()
    }
    pub fn begin_add(&mut self, profile: ProviderProfile) -> Result<(), ProviderRefreshError> {
        profile.validate().map_err(ProviderRefreshError::Invalid)?;
        self.draft_profile = Some(profile);
        Ok(())
    }
    pub fn begin_edit(&mut self) -> Result<(), ProviderRefreshError> {
        self.draft_profile = self.committed_profile.clone();
        self.draft_profile
            .as_ref()
            .ok_or(ProviderRefreshError::Cancelled)
            .map(|_| ())
    }
    pub fn cancel(&mut self) {
        self.draft_profile = None;
    }
    pub fn refresh<F>(&mut self, fetch: F) -> Result<&ProviderCatalog, ProviderRefreshError>
    where
        F: FnOnce(&ProviderProfile) -> Result<ProviderCatalog, String>,
    {
        let profile = self
            .draft_profile
            .as_ref()
            .ok_or(ProviderRefreshError::Cancelled)?;
        let next = fetch(profile).map_err(ProviderRefreshError::Unavailable)?;
        if next.generation.0 == 0
            || next
                .providers
                .iter()
                .all(|p| p.provider_id != profile.provider_id)
        {
            return Err(ProviderRefreshError::Unavailable(
                "provider is absent from catalog".into(),
            ));
        }
        self.catalog = Some(next);
        self.committed_profile = self.draft_profile.take();
        Ok(self.catalog.as_ref().expect("catalog set above"))
    }
    pub fn available_models(&self) -> Vec<&ProviderModel> {
        let Some(profile) = self.committed_profile.as_ref() else {
            return Vec::new();
        };
        self.catalog
            .as_ref()
            .into_iter()
            .flat_map(|catalog| catalog.providers.iter())
            .filter(|provider| {
                provider.provider_id == profile.provider_id
                    && matches!(provider.availability, ProviderAvailability::Available)
            })
            .flat_map(|provider| {
                provider
                    .models
                    .iter()
                    .filter(|model| matches!(model.availability, ProviderAvailability::Available))
            })
            .collect()
    }
    pub fn diagnostic(&self) -> ProviderRegistryDiagnostic {
        let (provider_id, status) = match (&self.committed_profile, &self.catalog) {
            (Some(profile), Some(catalog)) => (
                Some(profile.provider_id.clone()),
                catalog
                    .providers
                    .iter()
                    .find(|p| p.provider_id == profile.provider_id)
                    .map_or_else(
                        || "unavailable".into(),
                        |p| match &p.availability {
                            ProviderAvailability::Available => "connected".into(),
                            ProviderAvailability::Unavailable(_) => "unavailable".into(),
                        },
                    ),
            ),
            (Some(profile), None) => (Some(profile.provider_id.clone()), "unavailable".into()),
            _ => (None, "not_configured".into()),
        };
        ProviderRegistryDiagnostic {
            provider_id,
            status,
            model_count: self.available_models().len(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderRegistryDiagnostic {
    pub provider_id: Option<String>,
    pub status: String,
    pub model_count: usize,
}

impl ProviderRegistryDiagnostic {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

fn model_option(
    provider_id: &str,
    model: &ProviderModel,
    provider_available: bool,
) -> Result<WizardOption, String> {
    let available =
        provider_available && matches!(model.availability, ProviderAvailability::Available);
    WizardOption::new(
        model.model_id.clone(),
        if available {
            model.model_id.clone()
        } else {
            format!("{} (unavailable)", model.model_id)
        },
        available,
    )?
    .compatible_with(vec![provider_id.to_owned()])
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_catalog::{AgentAvailability, AgentCatalogEntry, AgentTarget};

    fn agents() -> AgentCatalog {
        AgentCatalog {
            runner_instance_id: "runner".into(),
            generation: 1,
            catalog_sha256: "a".repeat(64),
            refreshed: true,
            target: AgentTarget {
                operating_system: "linux".into(),
                architecture: "x86_64".into(),
                libc: "glibc".into(),
                libc_version: "2".into(),
            },
            agents: vec![AgentCatalogEntry {
                agent_id: "agent-1".into(),
                target: AgentTarget {
                    operating_system: "linux".into(),
                    architecture: "x86_64".into(),
                    libc: "glibc".into(),
                    libc_version: "2".into(),
                },
                package: None,
                provenance: None,
                capabilities: vec![],
                availability: AgentAvailability::Available,
            }],
        }
    }

    fn catalog() -> ProviderCatalog {
        let mut catalog = development_openrouter_catalog();
        catalog.providers[0].models[0].model_id = "gpt-4o".into();
        catalog
    }

    fn draft(scope: AgentScope) -> ProviderDefaultDraft {
        ProviderDefaultDraft {
            scope,
            provider_id: "openrouter".into(),
            model_id: "gpt-4o".into(),
            auth_method: ProviderAuthMethod::None,
            credential_reference_sha256: None,
        }
    }

    #[test]
    fn scoped_defaults_apply_replace_rollback_and_diagnose() {
        let mut defaults = SharedProviderDefaults::default();
        let selection = defaults
            .apply(draft(AgentScope::All), &agents(), &catalog())
            .unwrap();
        assert_eq!(selection.agent_ids, vec!["agent-1"]);
        assert_eq!(defaults.summary(), "1 provider default");
        assert_eq!(
            defaults.for_agent("agent-1").unwrap().provider_id,
            "openrouter"
        );
        let before = defaults.clone();
        defaults
            .apply(
                draft(AgentScope::Selected(vec!["agent-1".into()])),
                &agents(),
                &catalog(),
            )
            .unwrap();
        assert_eq!(defaults.entries.len(), 2);
        defaults.rollback(before.clone()).unwrap();
        assert_eq!(defaults, before);
        let diagnostic = defaults.diagnostic();
        assert_eq!(diagnostic.scopes, vec!["all"]);
        assert!(!diagnostic.to_json().unwrap().contains("provider default"));
        assert!(diagnostic.to_string().contains("schema 1"));
    }

    #[test]
    fn provider_validation_rejects_bad_scope_catalog_and_credentials() {
        assert!(AgentScope::selected(Vec::<String>::new()).is_err());
        assert!(AgentScope::selected(["bad/id"]).is_err());
        assert!(
            AgentScope::Selected(vec!["missing".into()])
                .resolve(&agents())
                .is_err()
        );
        assert!(
            AgentScope::All
                .resolve(&AgentCatalog {
                    agents: vec![],
                    ..agents()
                })
                .is_err()
        );
        let mut invalid = draft(AgentScope::All);
        invalid.provider_id = "bad/id".into();
        assert!(invalid.into_selection(&agents(), &catalog()).is_err());
        let mut credentials = draft(AgentScope::All);
        credentials.auth_method = ProviderAuthMethod::CredentialReference;
        assert!(
            credentials
                .clone()
                .into_selection(&agents(), &catalog())
                .is_err()
        );
        credentials.credential_reference_sha256 = Some("z".repeat(64));
        assert!(
            ProviderDefaultRecord::from_draft(&credentials)
                .validate()
                .is_err()
        );
        assert!(accept_generation(None, Revision(0)).is_err());
        assert!(accept_generation(Some(Revision(4)), Revision(3)).is_err());
        assert!(accept_generation(Some(Revision(4)), Revision(4)).is_ok());
    }

    #[test]
    fn options_profiles_registry_refresh_and_diagnostics_are_bounded() {
        let (providers, models) = wizard_options(&catalog()).unwrap();
        assert!(providers[0].available && models.iter().all(|model| model.available));
        let profile = ProviderProfile {
            provider_id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            auth_method: ProviderAuthMethod::None,
            credential_reference_sha256: None,
        };
        let mut registry = ConnectedProviderRegistry::new(None, None).unwrap();
        assert!(matches!(
            registry.begin_edit(),
            Err(ProviderRefreshError::Cancelled)
        ));
        registry.begin_add(profile).unwrap();
        registry.refresh(|_| Ok(catalog())).unwrap();
        assert_eq!(registry.available_models().len(), 2);
        assert_eq!(registry.diagnostic().status, "connected");
        assert!(
            registry
                .diagnostic()
                .to_json()
                .unwrap()
                .contains("openrouter")
        );
        registry.cancel();
        assert!(registry.draft().is_none());
        let invalid = ProviderProfile {
            provider_id: "bad/id".into(),
            display_name: "".into(),
            auth_method: ProviderAuthMethod::None,
            credential_reference_sha256: None,
        };
        assert!(registry.begin_add(invalid).is_err());
    }

    #[test]
    fn defaults_store_round_trip_and_fail_closed_filesystem_checks() {
        let root =
            std::env::temp_dir().join(format!("asb-provider-defaults-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let path = root.join("defaults.json");
        std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let store = ProviderDefaultsStore::new(&path);
        let defaults = SharedProviderDefaults {
            schema_version: 1,
            entries: vec![ProviderDefaultRecord {
                scope: AgentScope::All,
                provider_id: "openrouter".into(),
                model_id: "gpt-4o".into(),
                auth_method: ProviderAuthMethod::None,
                credential_reference_sha256: None,
            }],
        };
        assert!(store.load_or_default().unwrap().entries.is_empty());
        store.save(&defaults).unwrap();
        assert_eq!(store.load().unwrap(), defaults);
        assert_eq!(store.path(), path.as_path());
        assert!(ProviderDefaultsStore::new(&root).load().is_err());
        let link = root.join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(matches!(
            ProviderDefaultsStore::new(&link).load(),
            Err(ProviderDefaultsError::SymlinkRefused)
        ));
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_options_and_registry_refresh_failures_are_explicit() {
        let mut unavailable_catalog = catalog();
        unavailable_catalog.providers[0].availability =
            ProviderAvailability::Unavailable("offline".into());
        unavailable_catalog.providers[0].models[0].availability =
            ProviderAvailability::Unavailable("offline".into());
        let (providers, models) = wizard_options(&unavailable_catalog).unwrap();
        assert!(!providers[0].available && !models[0].available);
        let mut registry = ConnectedProviderRegistry::new(None, None).unwrap();
        let profile = ProviderProfile {
            provider_id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            auth_method: ProviderAuthMethod::None,
            credential_reference_sha256: None,
        };
        registry.begin_add(profile).unwrap();
        assert!(matches!(
            registry.refresh(|_| Err("offline".into())),
            Err(ProviderRefreshError::Unavailable(_))
        ));
        assert!(matches!(
            registry.refresh(|_| Ok(ProviderCatalog {
                generation: Revision(0),
                ..catalog()
            })),
            Err(ProviderRefreshError::Unavailable(_))
        ));
        assert!(
            ProviderProfile {
                provider_id: "openrouter".into(),
                display_name: " ".into(),
                auth_method: ProviderAuthMethod::None,
                credential_reference_sha256: None,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn defaults_validation_rejects_schema_and_malformed_records() {
        let mut defaults = SharedProviderDefaults {
            schema_version: 2,
            entries: Vec::new(),
        };
        assert!(defaults.validate().is_err());
        defaults.schema_version = 1;
        defaults.entries.push(ProviderDefaultRecord {
            scope: AgentScope::All,
            provider_id: "bad/id".into(),
            model_id: "model".into(),
            auth_method: ProviderAuthMethod::None,
            credential_reference_sha256: None,
        });
        assert!(defaults.validate().is_err());
        defaults.entries[0].provider_id = "provider".into();
        defaults.entries[0].auth_method = ProviderAuthMethod::CredentialReference;
        assert!(defaults.validate().is_err());
        defaults.entries[0].credential_reference_sha256 = Some("f".repeat(64));
        assert!(defaults.validate().is_ok());
    }

    #[test]
    fn provider_diagnostics_and_error_projections_cover_empty_states() {
        let empty = SharedProviderDefaults::default();
        assert_eq!(empty.summary(), "0 provider defaults");
        assert!(
            empty
                .diagnostic()
                .to_string()
                .contains("0 provider default(s)")
        );
        let mut record = ProviderDefaultRecord {
            scope: AgentScope::All,
            provider_id: "provider".into(),
            model_id: "model".into(),
            auth_method: ProviderAuthMethod::None,
            credential_reference_sha256: Some("f".repeat(64)),
        };
        assert!(record.validate().is_err());
        record.auth_method = ProviderAuthMethod::CredentialReference;
        record.credential_reference_sha256 = Some("bad".into());
        assert!(record.validate().is_err());
        for error in [
            ProviderRefreshError::Invalid("bad".into()),
            ProviderRefreshError::Unavailable("offline".into()),
            ProviderRefreshError::Cancelled,
        ] {
            assert!(!error.to_string().is_empty());
        }
        let empty_registry = ConnectedProviderRegistry::new(None, None).unwrap();
        assert!(empty_registry.available_models().is_empty());
        assert_eq!(empty_registry.diagnostic().status, "not_configured");
        let registry = ConnectedProviderRegistry::new(
            None,
            Some(ProviderProfile {
                provider_id: "openrouter".into(),
                display_name: "OpenRouter".into(),
                auth_method: ProviderAuthMethod::None,
                credential_reference_sha256: None,
            }),
        )
        .unwrap();
        assert!(registry.available_models().is_empty());
        assert_eq!(registry.diagnostic().status, "unavailable");
        assert!(
            ProviderDefaultsStore::new("defaults.json")
                .load_or_default()
                .is_err()
        );
        assert!(
            ProviderDefaultsStore::new("/no/such/parent/defaults.json")
                .save(&SharedProviderDefaults::default())
                .is_err()
        );
    }
}
