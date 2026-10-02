// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Coding-agent adapter compatibility and transactional wizard selection.

use std::collections::{BTreeMap, BTreeSet};

const MAX_ITEMS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AuthMethod {
    None,
    LocalDaemon,
    CredentialReference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterRecord {
    pub id: String,
    pub label: String,
    pub providers: BTreeMap<String, BTreeSet<String>>,
    pub auth_methods: BTreeSet<AuthMethod>,
}

/// A renderer-neutral choice.  Unavailable choices remain visible so the
/// wizard can explain why a tuple cannot be selected without asking users to
/// type opaque provider or model identifiers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterOption {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityOptions {
    pub adapter_id: String,
    pub provider_id: Option<String>,
    pub providers: Vec<AdapterOption>,
    pub models: Vec<AdapterOption>,
    pub auth_methods: Vec<AdapterOption>,
}

/// One deterministic result in the adapter/provider/model acceptance matrix.
/// The report intentionally contains identifiers and typed status only; it
/// never carries credentials or provider response bodies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixCase {
    pub adapter_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub supported: bool,
    pub reason: Option<String>,
}

impl AdapterRecord {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            providers: BTreeMap::new(),
            auth_methods: BTreeSet::new(),
        }
    }
    pub fn provider(
        mut self,
        id: impl Into<String>,
        models: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.providers
            .insert(id.into(), models.into_iter().map(Into::into).collect());
        self
    }
    pub fn auth(mut self, method: AuthMethod) -> Self {
        self.auth_methods.insert(method);
        self
    }
    fn valid(&self) -> bool {
        valid_id(&self.id)
            && !self.label.trim().is_empty()
            && self.label.len() <= 128
            && !self.providers.is_empty()
            && self.providers.len() <= MAX_ITEMS
            && self.providers.keys().all(|id| valid_id(id))
            && self.providers.values().all(|models| {
                !models.is_empty()
                    && models.len() <= MAX_ITEMS
                    && models.iter().all(|id| valid_id(id))
            })
            && !self.auth_methods.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterCatalog {
    records: BTreeMap<String, AdapterRecord>,
}

/// Explicit compatibility view used by setup and offline replay.  It keeps
/// unavailable provider reasons visible while filtering model choices to the
/// selected adapter/provider tuple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityMatrix {
    catalog: AdapterCatalog,
    unavailable: BTreeMap<String, String>,
    defaults: BTreeMap<String, AdapterSelection>,
    overrides: BTreeMap<String, AdapterSelection>,
}

impl CompatibilityMatrix {
    pub fn development() -> Self {
        Self {
            catalog: AdapterCatalog::development(),
            unavailable: BTreeMap::new(),
            defaults: BTreeMap::new(),
            overrides: BTreeMap::new(),
        }
    }
    pub fn mark_unavailable(&mut self, provider: impl Into<String>, reason: impl Into<String>) {
        self.unavailable.insert(provider.into(), reason.into());
    }
    pub fn options(
        &self,
        adapter: &str,
        provider: Option<&str>,
    ) -> Result<CompatibilityOptions, SelectionError> {
        let mut options = SelectionSession::new(self.catalog.clone())
            .compatibility_options_for(adapter, provider)?;
        for option in &mut options.providers {
            if let Some(reason) = self.unavailable.get(&option.id) {
                option.available = false;
                option.reason = Some(reason.clone());
            }
        }
        if let Some(provider) = provider
            && self.unavailable.contains_key(provider)
        {
            for option in &mut options.models {
                option.available = false;
                option.reason = self.unavailable.get(provider).cloned();
            }
        }
        Ok(options)
    }
    pub fn set_default(
        &mut self,
        agent: impl Into<String>,
        selection: AdapterSelection,
    ) -> Result<(), SelectionError> {
        SelectionSession::new(self.catalog.clone()).validate(selection.clone())?;
        self.defaults.insert(agent.into(), selection);
        Ok(())
    }
    pub fn set_override(
        &mut self,
        agent: impl Into<String>,
        selection: AdapterSelection,
    ) -> Result<(), SelectionError> {
        SelectionSession::new(self.catalog.clone()).validate(selection.clone())?;
        self.overrides.insert(agent.into(), selection);
        Ok(())
    }
    pub fn resolve(&self, agent: &str) -> Option<&AdapterSelection> {
        self.overrides
            .get(agent)
            .or_else(|| self.defaults.get(agent))
    }
    pub fn restart(&mut self) {
        self.overrides.clear();
    }
    pub fn validate_offline(&self, selection: AdapterSelection) -> Result<(), SelectionError> {
        SelectionSession::new(self.catalog.clone()).validate(selection)
    }

    /// Evaluate every provider/model tuple for every adapter in the catalog.
    /// Ordering is stable (adapter, provider, model) because all catalog
    /// collections are ordered. Unavailable providers remain in the report so
    /// the TUI can explain a negative tuple without inventing an identifier.
    pub fn evaluate(&self) -> Vec<MatrixCase> {
        self.catalog
            .records()
            .flat_map(|adapter| {
                adapter
                    .providers
                    .iter()
                    .flat_map(move |(provider, models)| {
                        models.iter().map(move |model| {
                            let reason = self.unavailable.get(provider).cloned();
                            MatrixCase {
                                adapter_id: adapter.id.clone(),
                                provider_id: provider.clone(),
                                model_id: model.clone(),
                                supported: reason.is_none(),
                                reason,
                            }
                        })
                    })
            })
            .collect()
    }
}

impl AdapterCatalog {
    pub fn new(records: impl IntoIterator<Item = AdapterRecord>) -> Result<Self, String> {
        let records: BTreeMap<String, AdapterRecord> =
            records.into_iter().map(|r| (r.id.clone(), r)).collect();
        if records.is_empty() || records.len() > MAX_ITEMS || records.values().any(|r| !r.valid()) {
            return Err("invalid adapter catalog".into());
        }
        Ok(Self { records })
    }
    pub fn get(&self, id: &str) -> Option<&AdapterRecord> {
        self.records.get(id)
    }
    pub fn records(&self) -> impl Iterator<Item = &AdapterRecord> {
        self.records.values()
    }

    /// Project all known adapters for a selection widget.  Stable catalogs may
    /// mark an adapter unavailable while retaining its explanation.
    pub fn options(&self) -> Vec<AdapterOption> {
        self.records
            .values()
            .map(|record| AdapterOption {
                id: record.id.clone(),
                label: record.label.clone(),
                available: true,
                reason: None,
            })
            .collect()
    }
    pub fn development() -> Self {
        Self::new([
            AdapterRecord::new("opencode", "OpenCode")
                .provider("openai", ["gpt-4o", "fixture-model"])
                .provider("openrouter", ["openai/gpt-4o"])
                .auth(AuthMethod::CredentialReference)
                .auth(AuthMethod::None),
            AdapterRecord::new("opendesk", "OpenDesk")
                .provider("openai", ["gpt-4o", "fixture-model"])
                .provider("local", ["fixture-model"])
                .auth(AuthMethod::LocalDaemon)
                .auth(AuthMethod::None),
        ])
        .expect("development adapter catalog is valid")
    }

    /// Build the adapter view from an authenticated provider catalog. The
    /// runner remains authoritative; this adapter view only narrows the
    /// provider/model/auth tuples exposed to the wizard.
    pub fn from_provider_catalog(
        catalog: &crate::control_codec::ProviderCatalog,
    ) -> Result<Self, String> {
        let mut record = AdapterRecord::new("opencode", "OpenCode");
        for provider in &catalog.providers {
            let models = provider.models.iter().map(|model| model.model_id.clone());
            record = record.provider(provider.provider_id.clone(), models);
            for auth in &provider.auth_methods {
                record = record.auth(match auth {
                    crate::control_codec::ProviderAuthMethod::CredentialReference => {
                        AuthMethod::CredentialReference
                    }
                    crate::control_codec::ProviderAuthMethod::LocalDaemon => {
                        AuthMethod::LocalDaemon
                    }
                    crate::control_codec::ProviderAuthMethod::None => AuthMethod::None,
                });
            }
        }
        // OpenCode and OpenDesk consume the same authoritative provider/model
        // catalog. Adapter-specific capability filtering happens at selection
        // time; dropping OpenDesk here made connected catalogs impossible to
        // qualify for that route.
        let opendesk = AdapterRecord {
            id: "opendesk".into(),
            label: "OpenDesk".into(),
            providers: record.providers.clone(),
            auth_methods: record.auth_methods.clone(),
        };
        Self::new([record, opendesk])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterSelection {
    pub adapter_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub auth: AuthMethod,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionError {
    UnknownAdapter,
    UnknownProvider,
    UnknownModel,
    UnsupportedAuth,
    InvalidSelection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSession {
    catalog: AdapterCatalog,
    committed: Option<AdapterSelection>,
    draft: Option<AdapterSelection>,
}

impl SelectionSession {
    pub fn new(catalog: AdapterCatalog) -> Self {
        Self {
            catalog,
            committed: None,
            draft: None,
        }
    }
    pub fn begin(&mut self) {
        self.draft = self.committed.clone();
    }
    pub fn committed(&self) -> Option<&AdapterSelection> {
        self.committed.as_ref()
    }
    pub fn draft(&self) -> Option<&AdapterSelection> {
        self.draft.as_ref()
    }

    pub fn options(&self) -> Vec<AdapterOption> {
        self.catalog.options()
    }

    /// Return the provider/model/auth choices accepted by the selected
    /// adapter.  The optional current tuple is used only for a human-readable
    /// disabled reason; it never broadens the authoritative catalog.
    pub fn compatibility_options(
        &self,
        adapter_id: &str,
    ) -> Result<CompatibilityOptions, SelectionError> {
        self.compatibility_options_for(adapter_id, None)
    }

    pub fn compatibility_options_for(
        &self,
        adapter_id: &str,
        provider_id: Option<&str>,
    ) -> Result<CompatibilityOptions, SelectionError> {
        let adapter = self
            .catalog
            .get(adapter_id)
            .ok_or(SelectionError::UnknownAdapter)?;
        let providers = adapter
            .providers
            .keys()
            .map(|id| AdapterOption {
                id: id.clone(),
                label: id.clone(),
                available: true,
                reason: None,
            })
            .collect();
        let models = adapter
            .providers
            .iter()
            .filter(|(id, _)| provider_id.is_none_or(|selected| selected == id.as_str()))
            .flat_map(|(_, models)| models.iter())
            .map(|id| AdapterOption {
                id: id.clone(),
                label: id.clone(),
                available: true,
                reason: None,
            })
            .collect();
        let auth_methods = adapter
            .auth_methods
            .iter()
            .map(|auth| AdapterOption {
                id: auth_label(*auth).into(),
                label: auth_label(*auth).into(),
                available: true,
                reason: None,
            })
            .collect();
        Ok(CompatibilityOptions {
            adapter_id: adapter_id.into(),
            provider_id: provider_id.map(str::to_owned),
            providers,
            models,
            auth_methods,
        })
    }

    pub fn diagnostics(&self, selection: &AdapterSelection) -> String {
        match self.validate(selection.clone()) {
            Ok(()) => "compatible provider, model, and authentication".into(),
            Err(SelectionError::UnknownAdapter) => {
                format!("adapter '{}' is unavailable", selection.adapter_id)
            }
            Err(SelectionError::UnknownProvider) => format!(
                "provider '{}' is not supported by {}",
                selection.provider_id, selection.adapter_id
            ),
            Err(SelectionError::UnknownModel) => format!(
                "model '{}' is not supported for provider '{}'",
                selection.model_id, selection.provider_id
            ),
            Err(SelectionError::UnsupportedAuth) => format!(
                "authentication '{}' is not supported by {}",
                auth_label(selection.auth),
                selection.adapter_id
            ),
            Err(SelectionError::InvalidSelection) => "selection is incomplete".into(),
        }
    }
    pub fn select(&mut self, selection: AdapterSelection) -> Result<(), SelectionError> {
        let adapter = self
            .catalog
            .get(&selection.adapter_id)
            .ok_or(SelectionError::UnknownAdapter)?;
        let models = adapter
            .providers
            .get(&selection.provider_id)
            .ok_or(SelectionError::UnknownProvider)?;
        if !models.contains(&selection.model_id) {
            return Err(SelectionError::UnknownModel);
        }
        if !adapter.auth_methods.contains(&selection.auth) {
            return Err(SelectionError::UnsupportedAuth);
        }
        self.draft = Some(selection);
        Ok(())
    }
    pub fn commit(&mut self) -> Result<&AdapterSelection, SelectionError> {
        let draft = self.draft.clone().ok_or(SelectionError::InvalidSelection)?;
        self.committed = Some(draft);
        self.draft.as_ref().ok_or(SelectionError::InvalidSelection)
    }
    pub fn cancel(&mut self) {
        self.draft = self.committed.clone();
    }
    /// Retry starts from the last valid state, so failed validation cannot erase it.
    pub fn retry(&mut self) {
        self.begin();
    }

    pub fn validate(&self, selection: AdapterSelection) -> Result<(), SelectionError> {
        let adapter = self
            .catalog
            .get(&selection.adapter_id)
            .ok_or(SelectionError::UnknownAdapter)?;
        let models = adapter
            .providers
            .get(&selection.provider_id)
            .ok_or(SelectionError::UnknownProvider)?;
        if !models.contains(&selection.model_id) {
            return Err(SelectionError::UnknownModel);
        }
        if !adapter.auth_methods.contains(&selection.auth) {
            return Err(SelectionError::UnsupportedAuth);
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
}

fn auth_label(auth: AuthMethod) -> &'static str {
    match auth {
        AuthMethod::None => "none",
        AuthMethod::LocalDaemon => "local_daemon",
        AuthMethod::CredentialReference => "credential_reference",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn choice() -> AdapterSelection {
        AdapterSelection {
            adapter_id: "opencode".into(),
            provider_id: "openai".into(),
            model_id: "gpt-4o".into(),
            auth: AuthMethod::None,
        }
    }
    #[test]
    fn fixtures_expose_both_adapters() {
        let c = AdapterCatalog::development();
        assert!(c.get("opencode").is_some());
        assert!(c.get("opendesk").is_some());
        assert_eq!(
            c.options()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["opencode", "opendesk"]
        );
    }

    #[test]
    fn compatibility_projection_and_diagnostics_are_selection_driven() {
        let session = SelectionSession::new(AdapterCatalog::development());
        let options = session.compatibility_options("opendesk").unwrap();
        assert_eq!(options.adapter_id, "opendesk");
        assert!(options.providers.iter().any(|item| item.id == "local"));
        assert!(
            options
                .auth_methods
                .iter()
                .any(|item| item.id == "local_daemon")
        );
        let incompatible = AdapterSelection {
            adapter_id: "opendesk".into(),
            provider_id: "openrouter".into(),
            model_id: "openai/gpt-4o".into(),
            auth: AuthMethod::CredentialReference,
        };
        assert!(
            session
                .diagnostics(&incompatible)
                .contains("provider 'openrouter'")
        );
    }
    #[test]
    fn incompatible_tuples_are_rejected_without_erasing_valid_state() {
        let mut s = SelectionSession::new(AdapterCatalog::development());
        s.begin();
        s.select(choice()).unwrap();
        s.commit().unwrap();
        let bad = AdapterSelection {
            provider_id: "local".into(),
            ..choice()
        };
        assert_eq!(s.select(bad), Err(SelectionError::UnknownProvider));
        assert_eq!(s.committed(), Some(&choice()));
        s.cancel();
        assert_eq!(s.draft(), Some(&choice()));
    }
    #[test]
    fn retry_preserves_valid_state_and_auth_is_filtered() {
        let mut s = SelectionSession::new(AdapterCatalog::development());
        s.begin();
        let mut c = choice();
        c.auth = AuthMethod::LocalDaemon;
        assert_eq!(s.select(c), Err(SelectionError::UnsupportedAuth));
        s.select(choice()).unwrap();
        s.commit().unwrap();
        s.retry();
        assert_eq!(s.draft(), Some(&choice()));
    }
}
