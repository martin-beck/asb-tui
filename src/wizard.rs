// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Standalone setup/reconfiguration wizard.
//!
//! The draft is renderer-owned and has no provider, credential, filesystem,
//! or runner side effects.  Its transitions are checked against the authored
//! UI state model before state is committed.

use crate::{
    adapter_catalog::{
        AdapterCatalog, AdapterOption, AdapterSelection, AuthMethod, CompatibilityOptions,
        SelectionSession,
    },
    control_codec::{ProviderAuthMethod, ProviderCatalog},
    development_auth::{
        DevelopmentAuthError, DevelopmentAuthFlow, DevelopmentAuthMethod, DevelopmentAuthSnapshot,
    },
    provider_catalog::{ProviderProfile, wizard_options},
    terminal::RenderPolicy,
    wizard_catalog::{OptionKind, WizardCatalog, WizardCatalogState},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde::Deserialize;

const MODEL: &str = include_str!("../docs/ui-state-model.json");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupRoute {
    Wizard,
    Landing,
}

#[must_use]
pub fn startup_route(asb_setup_ready: bool) -> StartupRoute {
    if asb_setup_ready {
        StartupRoute::Landing
    } else {
        StartupRoute::Wizard
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    Agent,
    Provider,
    Model,
    Configuration,
    Authentication,
    Recording,
    Replay,
    Review,
}

#[must_use]
pub const fn element_id(step: Step) -> &'static str {
    match step {
        Step::Agent => "wizard.agent",
        Step::Provider => "wizard.provider",
        Step::Model => "wizard.model",
        Step::Configuration => "wizard.configuration",
        Step::Authentication => "wizard.authentication",
        Step::Recording => "wizard.recording",
        Step::Replay => "wizard.replay",
        Step::Review => "wizard.review",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WizardError {
    Missing,
    AtStart,
    AtEnd,
    InvalidValue,
    TooLong,
    InvalidModel,
    Catalog(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WizardMode {
    Development,
    Stable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Wizard {
    step: Step,
    values: [String; 7],
    cancelled: bool,
    catalog: Option<WizardCatalogState>,
    development_auth: DevelopmentAuthFlow,
    adapter_selection: Option<SelectionSession>,
    mode: WizardMode,
    provider_catalog: Option<ProviderCatalog>,
}

impl Default for Wizard {
    fn default() -> Self {
        Self {
            step: Step::Agent,
            values: Default::default(),
            cancelled: false,
            catalog: None,
            development_auth: DevelopmentAuthFlow::new("development")
                .expect("static development provider is valid"),
            adapter_selection: Some(SelectionSession::new(AdapterCatalog::development())),
            mode: WizardMode::Development,
            provider_catalog: None,
        }
    }
}

impl Wizard {
    #[must_use]
    pub fn with_catalog(catalog: WizardCatalog) -> Self {
        Self {
            catalog: Some(WizardCatalogState::new(catalog, OptionKind::Agent)),
            ..Self::default()
        }
    }

    pub fn stable() -> Self {
        Self {
            adapter_selection: None,
            mode: WizardMode::Stable,
            ..Self::default()
        }
    }

    pub fn stable_with_adapter_catalog(catalog: AdapterCatalog) -> Self {
        Self {
            adapter_selection: Some(SelectionSession::new(catalog)),
            mode: WizardMode::Stable,
            ..Self::default()
        }
    }

    pub fn stable_with_catalog_and_adapter(
        catalog: WizardCatalog,
        adapter_catalog: AdapterCatalog,
    ) -> Self {
        Self {
            catalog: Some(WizardCatalogState::new(catalog, OptionKind::Agent)),
            adapter_selection: Some(SelectionSession::new(adapter_catalog)),
            mode: WizardMode::Stable,
            ..Self::default()
        }
    }
    #[must_use]
    pub const fn step(&self) -> Step {
        self.step
    }
    #[must_use]
    pub const fn cancelled(&self) -> bool {
        self.cancelled
    }

    #[must_use]
    pub const fn mode(&self) -> WizardMode {
        self.mode
    }

    pub fn set_provider_catalog(&mut self, catalog: ProviderCatalog) {
        self.provider_catalog = Some(catalog);
    }

    pub fn provider_catalog(&self) -> Option<&ProviderCatalog> {
        self.provider_catalog.as_ref()
    }

    pub fn connected_provider_options(
        &self,
    ) -> Result<
        (
            Vec<crate::wizard_catalog::WizardOption>,
            Vec<crate::wizard_catalog::WizardOption>,
        ),
        WizardError,
    > {
        self.provider_catalog
            .as_ref()
            .ok_or_else(|| WizardError::Catalog("provider catalog unavailable".into()))
            .and_then(|catalog| wizard_options(catalog).map_err(WizardError::Catalog))
    }

    pub fn begin_provider_edit(&mut self, profile: &ProviderProfile) -> Result<(), WizardError> {
        profile.validate().map_err(WizardError::Catalog)?;
        self.values[Step::Provider as usize] = profile.provider_id.clone();
        self.values[Step::Authentication as usize] =
            match (&profile.auth_method, &profile.credential_reference_sha256) {
                (ProviderAuthMethod::CredentialReference, Some(digest)) => {
                    format!("credential_reference:{digest}")
                }
                (ProviderAuthMethod::LocalDaemon, _) => "local_daemon".into(),
                (ProviderAuthMethod::None, _) => "none".into(),
                _ => return Err(WizardError::InvalidValue),
            };
        Ok(())
    }

    pub fn adapter_options(&self) -> Vec<AdapterOption> {
        self.adapter_selection
            .as_ref()
            .map_or_else(Vec::new, SelectionSession::options)
    }

    pub fn selected_adapter_id(&self) -> Option<&str> {
        self.adapter_selection
            .as_ref()
            .and_then(SelectionSession::committed)
            .map(|selection| selection.adapter_id.as_str())
    }

    pub fn adapter_compatibility(
        &self,
        adapter_id: &str,
    ) -> Result<CompatibilityOptions, WizardError> {
        self.adapter_compatibility_for(adapter_id, None)
    }

    pub fn adapter_compatibility_for(
        &self,
        adapter_id: &str,
        provider_id: Option<&str>,
    ) -> Result<CompatibilityOptions, WizardError> {
        self.adapter_selection
            .as_ref()
            .ok_or_else(|| {
                WizardError::Catalog("authoritative adapter catalog is unavailable".into())
            })?
            .compatibility_options_for(adapter_id, provider_id)
            .map_err(|error| WizardError::Catalog(format!("adapter compatibility: {error:?}")))
    }

    pub fn select_adapter(&mut self, selection: AdapterSelection) -> Result<(), WizardError> {
        let session = self.adapter_selection.as_mut().ok_or_else(|| {
            WizardError::Catalog("authoritative adapter catalog is unavailable".into())
        })?;
        session.begin();
        session
            .select(selection)
            .map_err(|error| WizardError::Catalog(format!("adapter compatibility: {error:?}")))?;
        session
            .commit()
            .map_err(|error| WizardError::Catalog(format!("adapter compatibility: {error:?}")))?;
        Ok(())
    }

    pub fn adapter_diagnostics(&self, selection: &AdapterSelection) -> String {
        self.adapter_selection.as_ref().map_or_else(
            || "authoritative adapter catalog is unavailable".into(),
            |session| session.diagnostics(selection),
        )
    }

    /// Return the bounded draft value for the active editable step.
    #[must_use]
    pub fn current_value(&self) -> &str {
        self.values
            .get(self.step as usize)
            .map_or("", String::as_str)
    }

    /// Return a bounded copy of the completed setup draft for the authenticated
    /// control seam. The wizard never stores secrets; authentication values are
    /// validated downstream as a closed method/reference form.
    #[must_use]
    pub fn values(&self) -> [String; 7] {
        self.values.clone()
    }

    #[must_use]
    pub fn catalog(&self) -> Option<&WizardCatalogState> {
        self.catalog.as_ref()
    }

    #[must_use]
    pub fn development_auth(&self) -> DevelopmentAuthSnapshot {
        self.development_auth.snapshot()
    }

    pub fn enroll_development_credential(&mut self) -> Result<(), DevelopmentAuthError> {
        self.development_auth.enroll()?;
        self.sync_development_auth_value();
        Ok(())
    }

    pub fn test_development_credential(&mut self) -> Result<(), DevelopmentAuthError> {
        self.development_auth.test()
    }

    pub fn rotate_development_credential(&mut self) -> Result<(), DevelopmentAuthError> {
        self.development_auth.rotate()?;
        self.sync_development_auth_value();
        Ok(())
    }

    pub fn reset_development_credential(&mut self) {
        self.development_auth.reset();
        self.values[Step::Authentication as usize].clear();
    }

    pub fn select_development_fixture(&mut self) -> Result<(), DevelopmentAuthError> {
        self.development_auth
            .select_method(DevelopmentAuthMethod::LocalFixture)?;
        self.values[Step::Authentication as usize].clear();
        Ok(())
    }

    pub fn select_development_none(&mut self) -> Result<(), DevelopmentAuthError> {
        self.development_auth
            .select_method(DevelopmentAuthMethod::None)?;
        self.values[Step::Authentication as usize] = "none".into();
        Ok(())
    }

    pub fn restart_development_credential(&mut self) {
        self.development_auth.restart();
        self.values[Step::Authentication as usize].clear();
    }

    fn sync_development_auth_value(&mut self) {
        if let Some(locator) = self.development_auth.snapshot().credential_locator_sha256 {
            self.values[Step::Authentication as usize] = format!("credential_reference:{locator}");
        }
    }

    pub fn set_catalog_query(&mut self, query: impl Into<String>) -> Result<(), WizardError> {
        self.catalog
            .as_mut()
            .ok_or_else(|| WizardError::Catalog("wizard catalog is unavailable".into()))?
            .set_query(query)
            .map_err(WizardError::Catalog)
    }

    pub fn move_catalog_cursor(&mut self, offset: isize) -> Result<(), WizardError> {
        self.catalog
            .as_mut()
            .ok_or_else(|| WizardError::Catalog("wizard catalog is unavailable".into()))?
            .move_cursor(offset);
        Ok(())
    }

    pub fn select_catalog_cursor(&mut self) -> Result<(), WizardError> {
        let catalog = self
            .catalog
            .as_mut()
            .ok_or_else(|| WizardError::Catalog("wizard catalog is unavailable".into()))?;
        if catalog.kind() == OptionKind::Agent {
            catalog
                .toggle_agent_cursor()
                .map_err(WizardError::Catalog)?;
            let value = catalog
                .selected_ids(OptionKind::Agent)
                .into_iter()
                .collect::<Vec<_>>()
                .join(",");
            return self.set_value(value);
        }
        catalog.select_cursor().map_err(WizardError::Catalog)?;
        let value = catalog
            .selected_for_active_kind()
            .unwrap_or_default()
            .to_owned();
        if catalog.kind() == OptionKind::Provider
            && !value.is_empty()
            && let Ok(flow) = DevelopmentAuthFlow::new(value.clone())
        {
            self.development_auth = flow;
        }
        self.set_value(value)
    }

    /// Select every available agent in the authenticated catalog and mirror
    /// the canonical IDs into the backend-facing draft field.
    pub fn select_all_agents(&mut self) -> Result<(), WizardError> {
        let catalog = self
            .catalog
            .as_mut()
            .ok_or_else(|| WizardError::Catalog("wizard catalog is unavailable".into()))?;
        catalog.select_all_agents().map_err(WizardError::Catalog)?;
        let value = catalog
            .selected_ids(OptionKind::Agent)
            .into_iter()
            .collect::<Vec<_>>()
            .join(",");
        self.set_value(value)
    }

    fn select_kind_for_step(&mut self) {
        let Some(catalog) = self.catalog.as_mut() else {
            return;
        };
        let kind = match self.step {
            Step::Agent => Some(OptionKind::Agent),
            Step::Provider => Some(OptionKind::Provider),
            Step::Model => Some(OptionKind::Model),
            _ => None,
        };
        if let Some(kind) = kind {
            catalog.set_kind(kind);
        }
    }

    pub fn set_value(&mut self, value: impl Into<String>) -> Result<(), WizardError> {
        if self.step == Step::Review {
            return Err(WizardError::InvalidValue);
        }
        let value = value.into();
        if value.chars().count() > 256 {
            return Err(WizardError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(WizardError::InvalidValue);
        }
        if let Some(slot) = self.values.get_mut(self.step as usize) {
            *slot = value;
        }
        Ok(())
    }

    /// Restore a completed, secret-free draft from the durable materialized
    /// configuration. This is intentionally independent of the current step
    /// and catalog cursor so a frontend restart cannot silently lose the
    /// selected provider, model, execution mode, or replay policy.
    pub fn restore_values(&mut self, values: [String; 7]) -> Result<(), WizardError> {
        if values
            .iter()
            .any(|value| value.chars().count() > 256 || value.chars().any(char::is_control))
        {
            return Err(WizardError::InvalidValue);
        }
        self.values = values;
        self.cancelled = false;
        Ok(())
    }

    pub fn advance(&mut self) -> Result<(), WizardError> {
        if self.step != Step::Review && self.values[self.step as usize].trim().is_empty() {
            return Err(WizardError::Missing);
        }
        self.step = match self.step {
            Step::Agent => Step::Provider,
            Step::Provider => Step::Model,
            Step::Model => Step::Configuration,
            Step::Configuration => Step::Authentication,
            Step::Authentication => Step::Recording,
            Step::Recording => Step::Replay,
            Step::Replay => Step::Review,
            Step::Review => return Err(WizardError::AtEnd),
        };
        self.select_kind_for_step();
        Ok(())
    }

    pub fn back(&mut self) -> Result<(), WizardError> {
        self.step = match self.step {
            Step::Agent => return Err(WizardError::AtStart),
            Step::Provider => Step::Agent,
            Step::Model => Step::Provider,
            Step::Configuration => Step::Model,
            Step::Authentication => Step::Configuration,
            Step::Recording => Step::Authentication,
            Step::Replay => Step::Recording,
            Step::Review => Step::Replay,
        };
        self.select_kind_for_step();
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.step = Step::Agent;
        self.values = Default::default();
        if let Some(catalog) = self.catalog.as_mut() {
            catalog.reset_draft();
        }
        self.restart_development_credential();
    }

    /// Validate OpenRouter's provider/model/auth tuple through the adapter
    /// compatibility catalog before the setup draft can complete.
    pub fn select_openrouter_adapter(&self) -> Result<(), WizardError> {
        let auth = match self.values[Step::Authentication as usize].as_str() {
            "none" => AuthMethod::None,
            "local_daemon" => AuthMethod::LocalDaemon,
            value if value.starts_with("credential_reference:") => AuthMethod::CredentialReference,
            _ => return Err(WizardError::InvalidValue),
        };
        if self.mode == WizardMode::Stable && auth == AuthMethod::None {
            return Err(WizardError::InvalidValue);
        }
        self.adapter_selection
            .as_ref()
            .ok_or_else(|| {
                WizardError::Catalog("authoritative adapter catalog is unavailable".into())
            })?
            .validate(AdapterSelection {
                adapter_id: "opencode".into(),
                provider_id: "openrouter".into(),
                model_id: self.values[Step::Model as usize].trim().into(),
                auth,
            })
            .map_err(|error| WizardError::Catalog(format!("adapter compatibility: {error:?}")))
    }

    pub fn complete(&self) -> Result<(), WizardError> {
        if let Some(selection) = self
            .adapter_selection
            .as_ref()
            .and_then(SelectionSession::committed)
        {
            self.adapter_selection
                .as_ref()
                .expect("selection checked above")
                .validate(selection.clone())
                .map_err(|error| {
                    WizardError::Catalog(format!("adapter compatibility: {error:?}"))
                })?;
        } else if self.values[Step::Provider as usize].trim() == "openrouter" {
            self.select_openrouter_adapter()?;
        }
        (self.step == Step::Review)
            .then_some(())
            .ok_or(WizardError::AtEnd)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormalEvent {
    OpenWizard,
    /// Open the wizard because authoritative startup classified the setup as
    /// absent or incomplete.  This remains distinct from manual reconfigure
    /// so the formal model can check the first-run route explicitly.
    AutoOpenWizard,
    Next,
    Back,
    Complete,
    Cancel,
    SetValue(String),
    CatalogQuery(String),
    CatalogMove(isize),
    CatalogSelect,
    CatalogSelectAllAgents,
    DevelopmentEnroll,
    DevelopmentTest,
    DevelopmentRotate,
    DevelopmentReset,
    DevelopmentSelectFixture,
    DevelopmentSelectNone,
    DevelopmentRestart,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WizardFormalState {
    route: StartupRoute,
    wizard: Wizard,
}

impl WizardFormalState {
    pub fn new() -> Result<Self, WizardError> {
        Self::new_with_wizard(Wizard::default())
    }

    pub fn new_with_catalog(catalog: WizardCatalog) -> Result<Self, WizardError> {
        Self::new_with_wizard(Wizard::with_catalog(catalog))
    }

    pub fn new_with_catalog_and_adapter(
        catalog: WizardCatalog,
        adapter_catalog: AdapterCatalog,
    ) -> Result<Self, WizardError> {
        Self::new_with_wizard(Wizard::stable_with_catalog_and_adapter(
            catalog,
            adapter_catalog,
        ))
    }

    fn new_with_wizard(wizard: Wizard) -> Result<Self, WizardError> {
        validate_model()?;
        Ok(Self {
            route: StartupRoute::Landing,
            wizard,
        })
    }
    pub fn apply(&mut self, event: FormalEvent) -> Result<(), WizardError> {
        let mut next = self.clone();
        let model: Model = serde_json::from_str(MODEL).map_err(|_| WizardError::InvalidModel)?;
        let (event_id, expected_to) = match &event {
            FormalEvent::OpenWizard => ("open_wizard", "wizard"),
            FormalEvent::AutoOpenWizard => ("startup_auto_open_wizard", "wizard"),
            FormalEvent::Next => ("wizard_next", "wizard"),
            FormalEvent::Back => ("wizard_back", "wizard"),
            FormalEvent::Complete => ("complete_wizard", "landing"),
            FormalEvent::Cancel => ("cancel_wizard", "landing"),
            FormalEvent::SetValue(_) => ("wizard_set_value", "wizard"),
            FormalEvent::CatalogQuery(_) => ("wizard_catalog_query", "wizard"),
            FormalEvent::CatalogMove(_) => ("wizard_catalog_move", "wizard"),
            FormalEvent::CatalogSelect => ("wizard_catalog_select", "wizard"),
            FormalEvent::CatalogSelectAllAgents => ("wizard_catalog_select_all_agents", "wizard"),
            FormalEvent::DevelopmentEnroll => ("wizard_development_enroll", "wizard"),
            FormalEvent::DevelopmentTest => ("wizard_development_test", "wizard"),
            FormalEvent::DevelopmentRotate => ("wizard_development_rotate", "wizard"),
            FormalEvent::DevelopmentReset => ("wizard_development_reset", "wizard"),
            FormalEvent::DevelopmentSelectFixture => {
                ("wizard_development_select_fixture", "wizard")
            }
            FormalEvent::DevelopmentSelectNone => ("wizard_development_select_none", "wizard"),
            FormalEvent::DevelopmentRestart => ("wizard_development_restart", "wizard"),
        };
        let from = match next.route {
            StartupRoute::Wizard => "wizard",
            StartupRoute::Landing => "landing",
        };
        let Some(transition) = model
            .transitions
            .iter()
            .find(|item| item.event == event_id && item.from == from && item.to == expected_to)
        else {
            return Err(WizardError::InvalidModel);
        };
        let expected_effects = match event_id {
            "wizard_next" | "wizard_back" => ["wizard_step_changed", "focus_reset"].as_slice(),
            "wizard_set_value" => ["wizard_draft_changed", "focus_reset"].as_slice(),
            "wizard_catalog_query" | "wizard_catalog_move" => {
                ["wizard_catalog_changed", "focus_reset"].as_slice()
            }
            "wizard_catalog_select" => [
                "wizard_catalog_changed",
                "wizard_draft_changed",
                "focus_reset",
            ]
            .as_slice(),
            "wizard_catalog_select_all_agents" => [
                "wizard_catalog_changed",
                "wizard_draft_changed",
                "focus_reset",
            ]
            .as_slice(),
            "wizard_development_enroll"
            | "wizard_development_test"
            | "wizard_development_rotate"
            | "wizard_development_reset"
            | "wizard_development_select_fixture"
            | "wizard_development_select_none"
            | "wizard_development_restart" => ["wizard_auth_changed", "focus_reset"].as_slice(),
            "cancel_wizard" => [
                "route_changed",
                "focus_reset",
                "wizard_draft_cleared",
                "wizard_catalog_reset",
                "development_auth_restarted",
            ]
            .as_slice(),
            _ => ["route_changed", "focus_reset"].as_slice(),
        };
        if transition
            .effects
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != expected_effects
        {
            return Err(WizardError::InvalidModel);
        }
        match event {
            FormalEvent::OpenWizard | FormalEvent::AutoOpenWizard
                if next.route == StartupRoute::Landing =>
            {
                next.route = StartupRoute::Wizard
            }
            FormalEvent::SetValue(value) if next.route == StartupRoute::Wizard => {
                next.wizard.set_value(value)?
            }
            FormalEvent::CatalogQuery(query) if next.route == StartupRoute::Wizard => {
                next.wizard.set_catalog_query(query)?
            }
            FormalEvent::CatalogMove(offset) if next.route == StartupRoute::Wizard => {
                next.wizard.move_catalog_cursor(offset)?
            }
            FormalEvent::CatalogSelect if next.route == StartupRoute::Wizard => {
                next.wizard.select_catalog_cursor()?
            }
            FormalEvent::CatalogSelectAllAgents if next.route == StartupRoute::Wizard => {
                next.wizard.select_all_agents()?
            }
            FormalEvent::DevelopmentEnroll if next.route == StartupRoute::Wizard => next
                .wizard
                .enroll_development_credential()
                .map_err(|_| WizardError::InvalidValue)?,
            FormalEvent::DevelopmentTest if next.route == StartupRoute::Wizard => next
                .wizard
                .test_development_credential()
                .map_err(|_| WizardError::InvalidValue)?,
            FormalEvent::DevelopmentRotate if next.route == StartupRoute::Wizard => next
                .wizard
                .rotate_development_credential()
                .map_err(|_| WizardError::InvalidValue)?,
            FormalEvent::DevelopmentReset if next.route == StartupRoute::Wizard => {
                next.wizard.reset_development_credential()
            }
            FormalEvent::DevelopmentSelectFixture if next.route == StartupRoute::Wizard => next
                .wizard
                .select_development_fixture()
                .map_err(|_| WizardError::InvalidValue)?,
            FormalEvent::DevelopmentSelectNone if next.route == StartupRoute::Wizard => next
                .wizard
                .select_development_none()
                .map_err(|_| WizardError::InvalidValue)?,
            FormalEvent::DevelopmentRestart if next.route == StartupRoute::Wizard => {
                next.wizard.restart_development_credential()
            }
            FormalEvent::Next if next.route == StartupRoute::Wizard => next.wizard.advance()?,
            FormalEvent::Back if next.route == StartupRoute::Wizard => next.wizard.back()?,
            FormalEvent::Complete if next.route == StartupRoute::Wizard => {
                next.wizard.complete()?;
                next.route = StartupRoute::Landing;
            }
            FormalEvent::Cancel if next.route == StartupRoute::Wizard => {
                next.wizard.cancel();
                next.route = StartupRoute::Landing;
            }
            _ => return Err(WizardError::InvalidModel),
        }
        *self = next;
        Ok(())
    }
    #[must_use]
    pub const fn route(&self) -> StartupRoute {
        self.route
    }
    #[must_use]
    pub const fn step(&self) -> Step {
        self.wizard.step()
    }
    #[must_use]
    pub const fn wizard(&self) -> &Wizard {
        &self.wizard
    }
}

#[derive(Deserialize)]
struct Model {
    routes: Vec<Route>,
    transitions: Vec<Transition>,
}
#[derive(Deserialize)]
struct Route {
    id: String,
}
#[derive(Deserialize)]
struct Transition {
    event: String,
    from: String,
    to: String,
    effects: Vec<String>,
}

fn validate_model() -> Result<(), WizardError> {
    let model: Model = serde_json::from_str(MODEL).map_err(|_| WizardError::InvalidModel)?;
    let routes: std::collections::BTreeSet<_> =
        model.routes.iter().map(|r| r.id.as_str()).collect();
    let required = ["wizard", "landing"];
    if required.iter().any(|route| !routes.contains(route)) {
        return Err(WizardError::InvalidModel);
    }
    for transition in model.transitions {
        if transition.event.is_empty()
            || !routes.contains(transition.from.as_str())
            || !routes.contains(transition.to.as_str())
            || transition.effects.is_empty()
        {
            return Err(WizardError::InvalidModel);
        }
    }
    Ok(())
}

#[must_use]
pub fn plain_text(wizard: &Wizard) -> String {
    format!(
        "ASB setup wizard\nstep: {}\nfield: {}\ncontrols: Enter next | Esc back | q cancel\n",
        step_title(wizard.step),
        element_id(wizard.step)
    )
}

pub fn render(frame: &mut Frame<'_>, wizard: &Wizard, policy: RenderPolicy) {
    let area = frame.area();
    let accent = Style::default().fg(if policy.unicode {
        Color::Cyan
    } else {
        Color::White
    });
    if area.width < 30 || area.height < 8 {
        frame.render_widget(Paragraph::new(plain_text(wizard)).style(accent), area);
        return;
    }
    let regions = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("ASB", accent),
            Span::raw(" setup wizard"),
        ]))
        .block(Block::default().borders(Borders::ALL).title(" Setup ")),
        regions[0],
    );
    let body = if let Some(catalog) = wizard.catalog() {
        let mut lines = vec![
            Line::from(Span::styled(
                format!("Step: {}", step_title(wizard.step)),
                accent,
            )),
            Line::from(step_prompt(wizard.step)),
            Line::from(format!("Search: {}", catalog.query())),
            Line::from(if catalog.kind() == OptionKind::Agent {
                format!(
                    "Agents selected: {}{}",
                    catalog.selected_ids(OptionKind::Agent).len(),
                    if catalog.all_agents_selected() {
                        " (all)"
                    } else {
                        ""
                    }
                )
            } else {
                "Choose one compatible option".into()
            }),
            Line::from(format!("Element: {}", element_id(wizard.step))),
        ];
        if wizard.step == Step::Agent && !wizard.adapter_options().is_empty() {
            lines.push(Line::from("Coding-agent adapters (selection-driven):"));
            for option in wizard.adapter_options() {
                lines.push(Line::from(format!(
                    "  {} {}{}",
                    if wizard.selected_adapter_id() == Some(option.id.as_str()) {
                        ">"
                    } else {
                        " "
                    },
                    option.label,
                    option
                        .reason
                        .map_or_else(String::new, |reason| format!(" ({reason})"))
                )));
            }
            lines.push(Line::from(if wizard.mode() == WizardMode::Development {
                "Development-only adapters may use generated fixtures; stable mode requires an authoritative catalog."
            } else {
                "Stable adapters require an authoritative catalog and compatible authentication."
            }));
        }
        for (index, option) in catalog.visible_options().into_iter().take(8).enumerate() {
            let cursor = catalog.cursor() == index;
            let selected = catalog
                .selected_ids(catalog.kind())
                .contains(&option.id.as_str());
            lines.push(Line::from(format!(
                "{}{} {}",
                if cursor { ">" } else { " " },
                if selected { "*" } else { " " },
                option.label
            )));
        }
        lines
    } else {
        vec![
            Line::from(Span::styled(
                format!("Step: {}", step_title(wizard.step)),
                accent,
            )),
            Line::from(step_prompt(wizard.step)),
            if wizard.step == Step::Authentication {
                let auth = wizard.development_auth();
                Line::from(format!(
                    "Development fixture: {:?} / {:?} / generation {}",
                    auth.status, auth.method, auth.generation
                ))
            } else {
                Line::from("")
            },
            Line::from(format!("Value: {}", wizard.current_value())),
            Line::from(format!("Element: {}", element_id(wizard.step))),
        ]
    };
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: true }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Current step "),
        ),
        regions[1],
    );
    frame.render_widget(
        Paragraph::new(if wizard.step == Step::Agent {
            "Space select/unselect | a all agents | Enter continue | Esc back | q cancel"
        } else if wizard.step == Step::Authentication {
            "F fixture | N no auth | E enroll | T test | R rotate | X reset | Z restart | Enter continue | Esc back | ? help"
        } else {
            "Enter select/continue | Esc back | ? help | q cancel"
        })
        .style(accent),
        regions[2],
    );
}

const fn step_title(step: Step) -> &'static str {
    match step {
        Step::Agent => "Agent",
        Step::Provider => "Provider",
        Step::Model => "Model",
        Step::Configuration => "Configuration",
        Step::Authentication => "Authentication",
        Step::Recording => "Recording",
        Step::Replay => "Offline replay",
        Step::Review => "Review",
    }
}
const fn step_prompt(step: Step) -> &'static str {
    match step {
        Step::Agent => "Choose the agent used for this benchmark.",
        Step::Provider => "Choose the model provider.",
        Step::Model => "Choose the provider model.",
        Step::Configuration => "Review benchmark configuration defaults.",
        Step::Authentication => {
            "Choose the local development fixture to enroll, test, rotate, or reset; it creates digest-only metadata and never a provider secret. Production enrollment still uses the approved keychain/helper and credential_helper:<endpoint_sha256>:<locator_sha256>; never paste an API key."
        }
        Step::Recording => "Choose whether to record benchmark activity.",
        Step::Replay => "Choose the offline replay policy.",
        Step::Review => "Review all choices before continuing.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_bound_wizard_flow_is_atomic() {
        let mut state = WizardFormalState::new().unwrap();
        assert!(state.apply(FormalEvent::Next).is_err());
        assert_eq!(state.route(), StartupRoute::Landing);
        state.apply(FormalEvent::OpenWizard).unwrap();
        for value in [
            "agent", "provider", "model", "config", "auth", "record", "replay",
        ] {
            state.apply(FormalEvent::SetValue(value.into())).unwrap();
            state.apply(FormalEvent::Next).unwrap();
        }
        assert_eq!(state.step(), Step::Review);
        state.apply(FormalEvent::Complete).unwrap();
        assert_eq!(state.route(), StartupRoute::Landing);
    }
    #[test]
    fn startup_route_is_fail_closed_and_manual_ready() {
        assert_eq!(startup_route(false), StartupRoute::Wizard);
        assert_eq!(startup_route(true), StartupRoute::Landing);
    }

    #[test]
    fn authentication_step_explains_secure_external_enrollment() {
        let prompt = step_prompt(Step::Authentication);
        assert!(prompt.contains("development fixture"));
        assert!(prompt.contains("digest-only metadata"));
        assert!(prompt.contains("approved keychain/helper"));
    }

    #[test]
    fn development_fixture_is_available_from_the_authentication_step() {
        let mut wizard = Wizard::default();
        wizard.enroll_development_credential().unwrap();
        let auth = wizard.development_auth();
        assert_eq!(auth.generation, 1);
        assert!(auth.credential_locator_sha256.is_some());
        assert!(
            wizard.values()[Step::Authentication as usize].starts_with("credential_reference:")
        );
    }

    #[test]
    fn formal_development_auth_actions_are_atomic_and_restartable() {
        let mut state = WizardFormalState::new().unwrap();
        state.apply(FormalEvent::OpenWizard).unwrap();
        for value in ["agent", "provider", "model", "config"] {
            state.apply(FormalEvent::SetValue(value.into())).unwrap();
            state.apply(FormalEvent::Next).unwrap();
        }
        state.apply(FormalEvent::DevelopmentEnroll).unwrap();
        assert_eq!(state.wizard().development_auth().generation, 1);
        state.apply(FormalEvent::DevelopmentTest).unwrap();
        assert_eq!(
            state.wizard().development_auth().status,
            crate::development_auth::DevelopmentAuthStatus::Tested
        );
        state.apply(FormalEvent::DevelopmentRotate).unwrap();
        assert_eq!(state.wizard().development_auth().generation, 2);
        state.apply(FormalEvent::DevelopmentReset).unwrap();
        assert!(
            state
                .wizard()
                .development_auth()
                .credential_locator_sha256
                .is_none()
        );
        assert!(state.wizard().values()[Step::Authentication as usize].is_empty());
    }

    #[test]
    fn enrollment_populates_a_configuration_safe_reference_for_completion() {
        let mut state = WizardFormalState::new().unwrap();
        state.apply(FormalEvent::OpenWizard).unwrap();
        for value in ["agent", "provider", "model", "config"] {
            state.apply(FormalEvent::SetValue(value.into())).unwrap();
            state.apply(FormalEvent::Next).unwrap();
        }
        state.apply(FormalEvent::DevelopmentEnroll).unwrap();
        state.apply(FormalEvent::Next).unwrap();
        assert_eq!(state.step(), Step::Recording);
        assert!(
            state.wizard().values()[Step::Authentication as usize]
                .starts_with("credential_reference:")
        );
    }

    #[test]
    fn selecting_none_or_cancelling_is_explicit_and_restartable() {
        let mut wizard = Wizard::default();
        wizard.select_development_none().unwrap();
        assert_eq!(wizard.values()[Step::Authentication as usize], "none");
        wizard.select_development_fixture().unwrap();
        assert!(wizard.values()[Step::Authentication as usize].is_empty());
        wizard.enroll_development_credential().unwrap();
        wizard.cancel();
        assert!(wizard.values()[Step::Authentication as usize].is_empty());
        assert_eq!(
            wizard.development_auth().status,
            crate::development_auth::DevelopmentAuthStatus::Unconfigured
        );
    }

    #[test]
    fn adapter_selection_exposes_only_compatible_tuple_choices() {
        let mut wizard = Wizard::default();
        assert_eq!(
            wizard
                .adapter_options()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["opencode", "opendesk"]
        );
        let choices = wizard.adapter_compatibility("opendesk").unwrap();
        assert!(choices.providers.iter().any(|item| item.id == "local"));
        assert!(
            choices
                .auth_methods
                .iter()
                .any(|item| item.id == "local_daemon")
        );
        wizard
            .select_adapter(AdapterSelection {
                adapter_id: "opendesk".into(),
                provider_id: "local".into(),
                model_id: "fixture-model".into(),
                auth: AuthMethod::LocalDaemon,
            })
            .unwrap();
        assert_eq!(wizard.selected_adapter_id(), Some("opendesk"));
    }

    #[test]
    fn openrouter_setup_is_checked_by_adapter_compatibility_before_completion() {
        let mut wizard = Wizard::default();
        for value in [
            "agent",
            "openrouter",
            "openai/gpt-4o",
            "defaults",
            "none",
            "record",
            "replay",
        ] {
            wizard.set_value(value).unwrap();
            wizard.advance().unwrap();
        }
        assert_eq!(wizard.step(), Step::Review);
        assert_eq!(wizard.complete(), Ok(()));
    }

    #[test]
    fn incompatible_openrouter_model_is_rejected_without_completion() {
        let mut wizard = Wizard::default();
        for value in [
            "agent",
            "openrouter",
            "unknown/model",
            "defaults",
            "none",
            "record",
            "replay",
        ] {
            wizard.set_value(value).unwrap();
            wizard.advance().unwrap();
        }
        assert!(matches!(wizard.complete(), Err(WizardError::Catalog(_))));
    }

    #[test]
    fn stable_wizard_never_uses_development_fixture_or_none_auth() {
        let mut wizard = Wizard::stable();
        for value in [
            "agent",
            "openrouter",
            "openai/gpt-4o",
            "defaults",
            "none",
            "record",
            "replay",
        ] {
            wizard.set_value(value).unwrap();
            wizard.advance().unwrap();
        }
        assert!(matches!(wizard.complete(), Err(WizardError::InvalidValue)));
    }

    #[test]
    fn stable_wizard_accepts_authoritative_catalog_and_reference_only() {
        let mut wizard = Wizard::stable_with_adapter_catalog(AdapterCatalog::development());
        for value in [
            "agent",
            "openrouter",
            "openai/gpt-4o",
            "defaults",
            "credential_reference:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "record",
            "replay",
        ] {
            wizard.set_value(value).unwrap();
            wizard.advance().unwrap();
        }
        assert_eq!(wizard.complete(), Ok(()));
    }

    #[test]
    fn development_catalog_cursor_query_and_provider_edit_paths_are_bounded() {
        let catalog = WizardCatalog::development().unwrap();
        let mut wizard = Wizard::with_catalog(catalog);
        assert!(wizard.connected_provider_options().is_err());
        wizard.set_provider_catalog(crate::provider_catalog::development_openrouter_catalog());
        assert!(wizard.connected_provider_options().is_ok());
        wizard.set_catalog_query("fake").unwrap();
        wizard.move_catalog_cursor(99).unwrap();
        wizard.select_catalog_cursor().unwrap();
        wizard.select_all_agents().unwrap();
        assert!(!wizard.current_value().is_empty());
        assert!(wizard.set_catalog_query("\u{7f}").is_err());
        assert!(wizard.set_value("x".repeat(257)).is_err());
        assert!(wizard.set_value("\u{1f}").is_err());
        wizard.back().unwrap_err();
        wizard.cancel();
        assert!(wizard.cancelled());
    }

    #[test]
    fn wizard_validation_covers_auth_modes_and_terminal_boundaries() {
        let mut wizard = Wizard::default();
        assert!(matches!(wizard.advance(), Err(WizardError::Missing)));
        assert!(matches!(
            wizard.select_openrouter_adapter(),
            Err(WizardError::InvalidValue)
        ));
        for value in ["agent", "openrouter", "openai/gpt-4o", "defaults"] {
            wizard.set_value(value).unwrap();
            wizard.advance().unwrap();
        }
        wizard.set_value("bad").unwrap();
        assert!(wizard.select_openrouter_adapter().is_err());
        wizard.set_value("credential_reference:deadbeef").unwrap();
        let _ = wizard.select_openrouter_adapter();
        wizard.set_value("local_daemon").unwrap();
        let _ = wizard.select_openrouter_adapter();
        wizard.set_value("none").unwrap();
        let _ = wizard.select_openrouter_adapter();
        for value in ["record", "replay"] {
            wizard.advance().unwrap();
            wizard.set_value(value).unwrap();
        }
        wizard.advance().unwrap();
        assert_eq!(wizard.step(), Step::Review);
        assert!(matches!(
            wizard.set_value("review"),
            Err(WizardError::InvalidValue)
        ));
        assert!(matches!(wizard.advance(), Err(WizardError::AtEnd)));
        assert_eq!(wizard.complete(), Ok(()));
    }
}
