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
    Io(String),
    TooLarge,
    SymlinkRefused,
    NotRegularFile,
}

impl From<io::Error> for ProviderDefaultsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind().to_string())
    }
}

impl fmt::Display for ProviderDefaultsError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(output, "invalid provider defaults: {reason}"),
            Self::Io(kind) => write!(output, "provider defaults I/O failure: {kind}"),
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
            Err(ProviderDefaultsError::Io(kind))
                if kind == io::ErrorKind::NotFound.to_string()
                    && self
                        .path
                        .parent()
                        .and_then(|parent| fs::symlink_metadata(parent).ok())
                        .is_some_and(|metadata| metadata.is_dir()) =>
            {
                Ok(SharedProviderDefaults::default())
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
        let metadata = fs::metadata(parent)?;
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
            .map_err(|_| ProviderDefaultsError::Io("clock".into()))?
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
