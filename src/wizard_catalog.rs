// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded, renderer-neutral choices for the setup wizard.
//!
//! This module deliberately contains no provider discovery, filesystem access,
//! installation, or ASB calls.  Adapters may validate and supply a catalog;
//! the wizard can then filter and select its choices without coupling the
//! state layer to a particular renderer or backend protocol.

use std::collections::BTreeSet;

pub const MAX_OPTIONS: usize = 64;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_LABEL_BYTES: usize = 256;
pub const MAX_QUERY_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OptionKind {
    Agent,
    Provider,
    Model,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WizardOption {
    pub id: String,
    pub label: String,
    pub available: bool,
}

impl WizardOption {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        available: bool,
    ) -> Result<Self, String> {
        let option = Self {
            id: id.into(),
            label: label.into(),
            available,
        };
        validate_option(&option)?;
        Ok(option)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WizardCatalog {
    agents: Vec<WizardOption>,
    providers: Vec<WizardOption>,
    models: Vec<WizardOption>,
}

impl WizardCatalog {
    pub fn new(
        agents: Vec<WizardOption>,
        providers: Vec<WizardOption>,
        models: Vec<WizardOption>,
    ) -> Result<Self, String> {
        validate_options(&agents, "agents")?;
        validate_options(&providers, "providers")?;
        validate_options(&models, "models")?;
        Ok(Self {
            agents,
            providers,
            models,
        })
    }

    #[must_use]
    pub fn options(&self, kind: OptionKind) -> &[WizardOption] {
        match kind {
            OptionKind::Agent => &self.agents,
            OptionKind::Provider => &self.providers,
            OptionKind::Model => &self.models,
        }
    }
}

/// Selection state for one wizard catalog.  It is intentionally independent
/// of Ratatui, terminal events, and backend side effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WizardCatalogState {
    catalog: WizardCatalog,
    kind: OptionKind,
    cursor: usize,
    query: String,
    selected_agent: Option<String>,
    selected_provider: Option<String>,
    selected_model: Option<String>,
}

impl WizardCatalogState {
    pub fn new(catalog: WizardCatalog, kind: OptionKind) -> Self {
        Self {
            catalog,
            kind,
            cursor: 0,
            query: String::new(),
            selected_agent: None,
            selected_provider: None,
            selected_model: None,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> OptionKind {
        self.kind
    }
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }
    #[must_use]
    pub fn selected(&self, kind: OptionKind) -> Option<&str> {
        match kind {
            OptionKind::Agent => self.selected_agent.as_deref(),
            OptionKind::Provider => self.selected_provider.as_deref(),
            OptionKind::Model => self.selected_model.as_deref(),
        }
    }
    #[must_use]
    pub fn visible_options(&self) -> Vec<&WizardOption> {
        let query = self.query.to_ascii_lowercase();
        self.catalog
            .options(self.kind)
            .iter()
            .filter(|option| {
                query.is_empty()
                    || option.id.to_ascii_lowercase().contains(&query)
                    || option.label.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> Result<(), String> {
        let query = query.into();
        if query.len() > MAX_QUERY_BYTES || !query.is_ascii() || query.chars().any(char::is_control)
        {
            return Err("invalid wizard catalog query".into());
        }
        self.query = query;
        self.cursor = 0;
        Ok(())
    }

    pub fn move_cursor(&mut self, offset: isize) {
        let count = self.visible_options().len();
        if count == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = self
            .cursor
            .saturating_add_signed(offset)
            .min(count.saturating_sub(1));
    }

    pub fn select_cursor(&mut self) -> Result<(), String> {
        let option = self
            .visible_options()
            .get(self.cursor)
            .copied()
            .ok_or_else(|| "no wizard catalog option is visible".to_owned())?;
        if !option.available {
            return Err("wizard catalog option is unavailable".into());
        }
        self.set_selected(option.id.clone());
        Ok(())
    }

    fn set_selected(&mut self, id: String) {
        match self.kind {
            OptionKind::Agent => self.selected_agent = Some(id),
            OptionKind::Provider => self.selected_provider = Some(id),
            OptionKind::Model => self.selected_model = Some(id),
        }
    }
}

fn validate_options(options: &[WizardOption], name: &str) -> Result<(), String> {
    if options.len() > MAX_OPTIONS {
        return Err(format!("{name} catalog exceeds {MAX_OPTIONS} options"));
    }
    let mut ids = BTreeSet::new();
    for option in options {
        validate_option(option)?;
        if !ids.insert(&option.id) {
            return Err(format!("duplicate {name} catalog option"));
        }
    }
    Ok(())
}

fn validate_option(option: &WizardOption) -> Result<(), String> {
    if option.id.is_empty()
        || option.id.len() > MAX_ID_BYTES
        || !option.id.is_ascii()
        || option.id.chars().any(char::is_control)
    {
        return Err("invalid wizard catalog option id".into());
    }
    if option.label.trim().is_empty()
        || option.label.len() > MAX_LABEL_BYTES
        || !option.label.is_ascii()
        || option.label.chars().any(char::is_control)
    {
        return Err("invalid wizard catalog option label".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> WizardCatalog {
        WizardCatalog::new(
            vec![WizardOption::new("agent-a", "Agent A", true).unwrap()],
            vec![WizardOption::new("provider-a", "Provider A", true).unwrap()],
            vec![WizardOption::new("model-a", "Model A", true).unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn filters_and_selects_available_option_without_renderer_state() {
        let mut state = WizardCatalogState::new(catalog(), OptionKind::Agent);
        state.set_query("agent").unwrap();
        assert_eq!(state.visible_options()[0].id, "agent-a");
        state.select_cursor().unwrap();
        assert_eq!(state.selected(OptionKind::Agent), Some("agent-a"));
    }

    #[test]
    fn unavailable_and_invalid_inputs_are_rejected_and_bounds_hold() {
        let unavailable = WizardCatalog::new(
            vec![WizardOption::new("agent-a", "Agent A", false).unwrap()],
            vec![],
            vec![],
        )
        .unwrap();
        let mut state = WizardCatalogState::new(unavailable, OptionKind::Agent);
        assert!(state.select_cursor().is_err());
        assert!(state.set_query("x".repeat(MAX_QUERY_BYTES + 1)).is_err());
        assert!(WizardOption::new("agent", "", true).is_err());
    }
}
