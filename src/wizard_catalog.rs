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
    compatible_ids: Vec<String>,
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
            compatible_ids: Vec::new(),
        };
        validate_option(&option)?;
        Ok(option)
    }

    /// Bind a provider to agent IDs or a model to provider IDs.  The meaning
    /// is determined by the catalog section containing this option.
    pub fn compatible_with(mut self, ids: Vec<String>) -> Result<Self, String> {
        self.compatible_ids = ids;
        validate_option(&self)?;
        Ok(self)
    }

    #[must_use]
    pub fn compatible_ids(&self) -> &[String] {
        &self.compatible_ids
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
        let agent_ids: BTreeSet<&str> = agents.iter().map(|option| option.id.as_str()).collect();
        for provider in &providers {
            if provider.compatible_ids.is_empty()
                || provider
                    .compatible_ids
                    .iter()
                    .any(|id| !agent_ids.contains(id.as_str()))
            {
                return Err("provider compatibility references are incomplete".into());
            }
        }
        let provider_ids: BTreeSet<&str> =
            providers.iter().map(|option| option.id.as_str()).collect();
        for model in &models {
            if model.compatible_ids.is_empty()
                || model
                    .compatible_ids
                    .iter()
                    .any(|id| !provider_ids.contains(id.as_str()))
            {
                return Err("model compatibility references are incomplete".into());
            }
        }
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
    selected_agents: Vec<String>,
    all_agents: bool,
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
            selected_agents: Vec::new(),
            all_agents: false,
            selected_agent: None,
            selected_provider: None,
            selected_model: None,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> OptionKind {
        self.kind
    }
    /// Switch the active picker while retaining selections for all three
    /// option kinds.  Query and cursor are reset so a filter from one picker
    /// cannot silently hide choices in the next one.
    pub fn set_kind(&mut self, kind: OptionKind) {
        self.kind = kind;
        self.query.clear();
        self.cursor = 0;
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
    pub fn catalog(&self) -> &WizardCatalog {
        &self.catalog
    }
    #[must_use]
    pub fn selected(&self, kind: OptionKind) -> Option<&str> {
        match kind {
            OptionKind::Agent => self.selected_agent.as_deref(),
            OptionKind::Provider => self.selected_provider.as_deref(),
            OptionKind::Model => self.selected_model.as_deref(),
        }
    }
    /// IDs selected for a catalog section. Agent selection may contain more
    /// than one entry; provider and model selections remain single-valued.
    #[must_use]
    pub fn selected_ids(&self, kind: OptionKind) -> Vec<&str> {
        match kind {
            OptionKind::Agent => self.selected_agents.iter().map(String::as_str).collect(),
            OptionKind::Provider => self.selected_provider.as_deref().into_iter().collect(),
            OptionKind::Model => self.selected_model.as_deref().into_iter().collect(),
        }
    }
    #[must_use]
    pub const fn all_agents_selected(&self) -> bool {
        self.all_agents
    }
    #[must_use]
    pub fn visible_options(&self) -> Vec<&WizardOption> {
        let query = self.query.to_ascii_lowercase();
        self.catalog
            .options(self.kind)
            .iter()
            .filter(|option| {
                let compatible = match self.kind {
                    OptionKind::Agent => true,
                    OptionKind::Provider => {
                        !self.selected_agents.is_empty()
                            && self.selected_agents.iter().all(|id| {
                                option
                                    .compatible_ids
                                    .iter()
                                    .any(|candidate| candidate == id)
                            })
                    }
                    OptionKind::Model => self.selected_provider.as_deref().is_some_and(|id| {
                        option
                            .compatible_ids
                            .iter()
                            .any(|candidate| candidate == id)
                    }),
                };
                compatible
                    && (query.is_empty()
                        || option.id.to_ascii_lowercase().contains(&query)
                        || option.label.to_ascii_lowercase().contains(&query))
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

    /// Toggle the focused agent without changing provider/model choices. A
    /// provider is shown only when it supports every selected agent.
    pub fn toggle_agent_cursor(&mut self) -> Result<(), String> {
        if self.kind != OptionKind::Agent {
            return Err("agent multi-selection is only available on the agent step".into());
        }
        let option_id = self
            .visible_options()
            .get(self.cursor)
            .copied()
            .ok_or_else(|| "no wizard catalog option is visible".to_owned())?;
        if !option_id.available {
            return Err("wizard catalog option is unavailable".into());
        }
        let option_id = option_id.id.clone();
        self.all_agents = false;
        if let Some(index) = self.selected_agents.iter().position(|id| id == &option_id) {
            self.selected_agents.remove(index);
        } else {
            self.selected_agents.push(option_id);
            self.selected_agents.sort();
        }
        self.selected_agent = self.selected_agents.first().cloned();
        self.selected_provider = None;
        self.selected_model = None;
        Ok(())
    }

    /// Select every available agent in the authenticated catalog. This keeps
    /// the meaning of “all agents” explicit and bounded to the catalog.
    pub fn select_all_agents(&mut self) -> Result<(), String> {
        if self.kind != OptionKind::Agent {
            return Err("all-agent selection is only available on the agent step".into());
        }
        self.selected_agents = self
            .catalog
            .options(OptionKind::Agent)
            .iter()
            .filter(|option| option.available)
            .map(|option| option.id.clone())
            .collect();
        if self.selected_agents.is_empty() {
            return Err("no available agents are present in the catalog".into());
        }
        self.all_agents = true;
        self.selected_agent = self.selected_agents.first().cloned();
        self.selected_provider = None;
        self.selected_model = None;
        Ok(())
    }

    #[must_use]
    pub fn selected_for_active_kind(&self) -> Option<&str> {
        self.selected(self.kind)
    }

    fn set_selected(&mut self, id: String) {
        match self.kind {
            OptionKind::Agent => {
                self.selected_agents = vec![id.clone()];
                self.all_agents = false;
                self.selected_agent = Some(id);
                self.selected_provider = None;
                self.selected_model = None;
            }
            OptionKind::Provider => {
                self.selected_provider = Some(id);
                self.selected_model = None;
            }
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
    let mut ids = BTreeSet::new();
    for id in &option.compatible_ids {
        if id.is_empty()
            || id.len() > MAX_ID_BYTES
            || !id.is_ascii()
            || id.chars().any(char::is_control)
            || !ids.insert(id)
        {
            return Err("invalid wizard catalog compatibility id".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> WizardCatalog {
        WizardCatalog::new(
            vec![WizardOption::new("agent-a", "Agent A", true).unwrap()],
            vec![
                WizardOption::new("provider-a", "Provider A", true)
                    .unwrap()
                    .compatible_with(vec!["agent-a".into()])
                    .unwrap(),
            ],
            vec![
                WizardOption::new("model-a", "Model A", true)
                    .unwrap()
                    .compatible_with(vec!["provider-a".into()])
                    .unwrap(),
            ],
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

    #[test]
    fn compatibility_is_required_and_filters_dependents_fail_closed() {
        let agents = vec![
            WizardOption::new("agent-a", "Agent A", true).unwrap(),
            WizardOption::new("agent-b", "Agent B", true).unwrap(),
        ];
        let providers = vec![
            WizardOption::new("provider-a", "Provider A", true)
                .unwrap()
                .compatible_with(vec!["agent-a".into()])
                .unwrap(),
            WizardOption::new("provider-b", "Provider B", true)
                .unwrap()
                .compatible_with(vec!["agent-b".into()])
                .unwrap(),
        ];
        let models = vec![
            WizardOption::new("model-a", "Model A", true)
                .unwrap()
                .compatible_with(vec!["provider-a".into()])
                .unwrap(),
        ];
        assert!(
            WizardCatalog::new(
                agents.clone(),
                vec![
                    WizardOption::new("provider-x", "Provider X", true)
                        .unwrap()
                        .compatible_with(vec!["missing".into()])
                        .unwrap()
                ],
                models.clone(),
            )
            .is_err()
        );
        let mut state = WizardCatalogState::new(
            WizardCatalog::new(agents, providers, models).unwrap(),
            OptionKind::Provider,
        );
        assert!(state.visible_options().is_empty());
        state.set_kind(OptionKind::Agent);
        state.select_cursor().unwrap();
        state.set_kind(OptionKind::Provider);
        assert_eq!(
            state
                .visible_options()
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            vec!["provider-a"]
        );
        state.select_cursor().unwrap();
        state.set_kind(OptionKind::Model);
        assert_eq!(state.visible_options()[0].id, "model-a");
    }

    #[test]
    fn agent_picker_supports_explicit_multi_and_all_selection() {
        let agents = vec![
            WizardOption::new("agent-a", "Agent A", true).unwrap(),
            WizardOption::new("agent-b", "Agent B", true).unwrap(),
        ];
        let providers = vec![
            WizardOption::new("shared", "Shared", true)
                .unwrap()
                .compatible_with(vec!["agent-a".into(), "agent-b".into()])
                .unwrap(),
            WizardOption::new("agent-a-only", "Agent A only", true)
                .unwrap()
                .compatible_with(vec!["agent-a".into()])
                .unwrap(),
        ];
        let mut state = WizardCatalogState::new(
            WizardCatalog::new(agents, providers, vec![]).unwrap(),
            OptionKind::Agent,
        );
        state.toggle_agent_cursor().unwrap();
        state.move_cursor(1);
        state.toggle_agent_cursor().unwrap();
        assert_eq!(
            state.selected_ids(OptionKind::Agent),
            ["agent-a", "agent-b"]
        );
        state.set_kind(OptionKind::Provider);
        assert_eq!(state.visible_options()[0].id, "shared");
        state.set_kind(OptionKind::Agent);
        state.select_all_agents().unwrap();
        assert!(state.all_agents_selected());
        assert_eq!(
            state.selected_ids(OptionKind::Agent),
            ["agent-a", "agent-b"]
        );
    }
}
