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

#[derive(Clone, Debug)]
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
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
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
