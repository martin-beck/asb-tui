// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! The interactive workspace: keyboard-first navigation and Ratatui projection.
//!
//! This module owns only presentation state. Benchmark execution and credentials remain in
//! the external ASB control plane.

use crate::{
    control_codec::{ConfigurationSelection, MeasurementCatalog, ProviderAuthMethod},
    help::document_help_text,
    live_projection::{Connection, LiveSnapshot},
    selection::{MAX_QUERY_BYTES, Measurement, MeasurementSelection},
    startup::{self, StartupInput},
    terminal::{CapabilityTier, RenderPolicy, ResponsiveLayout, frame_dimensions_are_safe},
    wizard::{self, FormalEvent, Wizard, WizardFormalState},
    wizard_catalog::{WizardCatalog, WizardOption},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Landing,
    Wizard,
    Measures,
    Configuration,
    Reports,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    None,
    Quit,
    Resize(u16, u16),
}

/// Result of the local configuration save action.  A failed save never clears
/// the draft, and an unavailable store is reported instead of pretending that
/// Ctrl-S applied anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationSaveState {
    Clean,
    Dirty,
    Saved,
    Unavailable,
    Failed,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceState {
    pub screen: Screen,
    pub help: bool,
    pub search: String,
    pub measure_cursor: usize,
    pub measures: Vec<MeasureRow>,
    pub config_cursor: usize,
    pub report_cursor: usize,
    configuration_draft: crate::configuration::ConfigurationDraft,
    configuration_path: Option<PathBuf>,
    configuration_save_state: ConfigurationSaveState,
    configuration_editing: bool,
    configuration_edit_buffer: String,
    configuration_edit_error: Option<String>,
    /// Last validated snapshot supplied by the authenticated control seam.
    pub live: Option<LiveSnapshot>,
    selection: Option<MeasurementSelection>,
    /// Local setup draft and its one-shot startup gate. This has no persistence
    /// or ASB side effects; those remain downstream integration seams.
    pub wizard: Wizard,
    wizard_formal: WizardFormalState,
    wizard_completion: Option<[String; 7]>,
    /// Last validated dimensions received from the terminal event stream.
    /// Rendering still uses the frame's authoritative area, so a missed
    /// event cannot make the renderer allocate from stale dimensions.
    layout: ResponsiveLayout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeasureRow {
    pub id: String,
    pub group: String,
    pub name: String,
    pub selected: bool,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            screen: Screen::Landing,
            wizard: Wizard::default(),
            wizard_formal: WizardFormalState::new().expect("authored wizard model must be valid"),
            wizard_completion: None,
            help: false,
            search: String::new(),
            measure_cursor: 0,
            measures: vec![
                MeasureRow {
                    id: "quality.correctness".into(),
                    group: "Quality".into(),
                    name: "Correctness".into(),
                    selected: true,
                },
                MeasureRow {
                    id: "quality.consistency".into(),
                    group: "Quality".into(),
                    name: "Consistency".into(),
                    selected: true,
                },
                MeasureRow {
                    id: "efficiency.latency".into(),
                    group: "Efficiency".into(),
                    name: "Latency".into(),
                    selected: true,
                },
                MeasureRow {
                    id: "efficiency.token_usage".into(),
                    group: "Efficiency".into(),
                    name: "Token usage".into(),
                    selected: false,
                },
                MeasureRow {
                    id: "safety.policy_adherence".into(),
                    group: "Safety".into(),
                    name: "Policy adherence".into(),
                    selected: false,
                },
            ],
            config_cursor: 0,
            report_cursor: 0,
            configuration_draft: crate::configuration::ConfigurationDraft::new(
                crate::configuration::Configuration::default(),
            )
            .expect("default configuration is valid"),
            configuration_path: None,
            configuration_save_state: ConfigurationSaveState::Unavailable,
            configuration_editing: false,
            configuration_edit_buffer: String::new(),
            configuration_edit_error: None,
            live: None,
            selection: None,
            layout: ResponsiveLayout::from_dimensions(None, None),
        }
    }
}

impl WorkspaceState {
    /// Load a bounded local configuration store for the configuration screen.
    /// This performs no ASB probing or backend acknowledgement.
    pub fn with_configuration_store(
        path: impl Into<PathBuf>,
    ) -> Result<Self, crate::configuration::ConfigError> {
        let path = path.into();
        let config = crate::configuration::ConfigurationStore::new(&path).load_or_default()?;
        Ok(Self {
            configuration_draft: crate::configuration::ConfigurationDraft::new(config)
                .expect("store configuration was validated on load"),
            configuration_path: Some(path),
            configuration_save_state: ConfigurationSaveState::Clean,
            ..Self::default()
        })
    }

    /// The current local configuration draft projected by the screen.
    #[must_use]
    pub fn configuration_draft(&self) -> &crate::configuration::ConfigurationDraft {
        &self.configuration_draft
    }

    /// Edit the local draft transactionally.  The store is not touched until
    /// [`Self::save_configuration`] succeeds.
    pub fn edit_configuration<F>(
        &mut self,
        edit: F,
    ) -> Result<(), crate::configuration::ConfigError>
    where
        F: FnOnce(&mut crate::configuration::Configuration),
    {
        self.configuration_draft.edit(edit)?;
        self.configuration_save_state = if self.configuration_path.is_some() {
            ConfigurationSaveState::Dirty
        } else {
            ConfigurationSaveState::Unavailable
        };
        Ok(())
    }

    /// Persist and commit the draft atomically through the local
    /// [`crate::configuration::ConfigurationStore`].
    pub fn save_configuration(&mut self) -> Result<(), crate::configuration::ConfigError> {
        // Validate the documented route/action transition before touching the
        // local store.  This keeps the keyboard action fail-closed if the
        // authored formal model ever drifts from the implementation.
        let mut formal = crate::formal_state::FormalUiState::new(80, 24).map_err(|error| {
            crate::configuration::ConfigError::Invalid(format!(
                "formal configuration save transition unavailable: {error:?}"
            ))
        })?;
        formal
            .apply(crate::formal_state::FormalEvent::OpenConfiguration, None)
            .and_then(|_| {
                formal.apply(
                    crate::formal_state::FormalEvent::Focus("configuration.entry"),
                    None,
                )
            })
            .and_then(|_| formal.apply(crate::formal_state::FormalEvent::SaveConfiguration, None))
            .map_err(|error| {
                crate::configuration::ConfigError::Invalid(format!(
                    "formal configuration save transition unavailable: {error:?}"
                ))
            })?;
        let Some(path) = self.configuration_path.as_ref() else {
            self.configuration_save_state = ConfigurationSaveState::Unavailable;
            return Err(crate::configuration::ConfigError::Invalid(
                "no local configuration store is configured".into(),
            ));
        };
        let store = crate::configuration::ConfigurationStore::new(path);
        if let Err(error) = store.save(self.configuration_draft.current()) {
            self.configuration_save_state = ConfigurationSaveState::Failed;
            return Err(error);
        }
        self.configuration_draft.apply()?;
        self.configuration_save_state = ConfigurationSaveState::Saved;
        Ok(())
    }

    #[must_use]
    pub const fn configuration_save_state(&self) -> ConfigurationSaveState {
        self.configuration_save_state
    }

    /// Attach a validated, already-authenticated catalog to the wizard. Catalog
    /// acquisition and installation remain outside the presentation layer.
    pub fn with_wizard_catalog(mut self, catalog: WizardCatalog) -> Self {
        if let Ok(formal) = WizardFormalState::new_with_catalog(catalog) {
            self.wizard_formal = formal;
            self.wizard = self.wizard_formal.wizard().clone();
        }
        self
    }

    /// Take a completed wizard draft. Completion is consumed exactly once so a
    /// retry cannot accidentally replay a configuration mutation.
    pub fn take_wizard_completion(&mut self) -> Option<[String; 7]> {
        self.wizard_completion.take()
    }

    /// Convert the bounded wizard draft to the backend's credential-free
    /// configuration selection. API keys never cross this presentation seam;
    /// credential auth is represented only by a 64-character SHA-256 digest.
    pub fn wizard_configuration_selection(
        values: &[String; 7],
    ) -> Result<ConfigurationSelection, &'static str> {
        let agent_ids: Vec<String> = values[0]
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        if agent_ids.is_empty() || agent_ids.len() > 64 {
            return Err("agent selection is required");
        }
        let provider_id = values[1].trim();
        let model_id = values[2].trim();
        if provider_id.is_empty() || model_id.is_empty() {
            return Err("provider and model are required");
        }
        let (auth_method, credential_reference_sha256) = match values[4].trim() {
            "none" => (ProviderAuthMethod::None, None),
            "local_daemon" => (ProviderAuthMethod::LocalDaemon, None),
            value
                if value
                    .strip_prefix("credential_reference:")
                    .is_some_and(|digest| {
                        digest.len() == 64
                            && digest
                                .chars()
                                .all(|character| character.is_ascii_hexdigit())
                    }) =>
            {
                (
                    ProviderAuthMethod::CredentialReference,
                    Some(value["credential_reference:".len()..].to_owned()),
                )
            }
            _ => {
                return Err(
                    "authentication must be none, local_daemon, or a credential reference digest",
                );
            }
        };
        Ok(ConfigurationSelection {
            agent_ids,
            provider_id: provider_id.to_owned(),
            model_id: model_id.to_owned(),
            auth_method,
            credential_reference_sha256,
        })
    }

    /// The bounded text currently visible in the focused configuration editor.
    pub fn configuration_edit_value(&self) -> Option<&str> {
        self.configuration_editing
            .then_some(self.configuration_edit_buffer.as_str())
    }

    fn apply_configuration_edit_buffer(&mut self) {
        let Some(id) = self.configuration_draft.focused() else {
            return;
        };
        match self
            .configuration_draft
            .set_value(id, &self.configuration_edit_buffer)
        {
            Ok(()) => {
                self.configuration_edit_error = None;
                self.configuration_save_state = if self.configuration_path.is_some() {
                    ConfigurationSaveState::Dirty
                } else {
                    ConfigurationSaveState::Unavailable
                };
            }
            Err(_error) => {
                self.configuration_edit_error =
                    Some("value rejected by configuration validation".into());
                self.configuration_save_state = ConfigurationSaveState::Invalid;
            }
        }
    }

    fn commit_configuration_edit(&mut self) -> Result<(), ()> {
        self.apply_configuration_edit_buffer();
        if self.configuration_edit_error.is_some() {
            return Err(());
        }
        self.configuration_editing = false;
        self.configuration_draft.focus(None);
        Ok(())
    }

    /// Create workspace state from an already-authoritative readiness result.
    /// An unconfigured result opens the wizard once; no probing or persistence
    /// is performed here.
    #[must_use]
    pub fn for_startup(asb_setup_ready: bool) -> Self {
        let mut state = Self::default();
        if matches!(
            wizard::startup_route(asb_setup_ready),
            wizard::StartupRoute::Wizard
        ) {
            state.open_wizard();
        }
        state
    }

    /// Create workspace state from the complete normalized readiness result.
    /// Only explicitly unconfigured or incomplete input opens the wizard;
    /// unavailable, malformed, stale, and unauthorized states stay closed.
    #[must_use]
    pub fn for_readiness(input: StartupInput) -> Self {
        let mut state = Self::default();
        let decision = startup::classify(input);
        if decision.auto_opens_wizard() {
            // Keep automatic first-run routing in the same checked transition
            // interpreter as manual wizard navigation.  The readiness facts
            // have already been normalized by the injected boundary above;
            // this call performs no I/O or persistence.
            if state
                .wizard_formal
                .apply(FormalEvent::AutoOpenWizard)
                .is_ok()
            {
                state.wizard = state.wizard_formal.wizard().clone();
                state.screen = Screen::Wizard;
            }
        }
        state
    }

    /// Open the wizard for a later manual reconfiguration.
    pub fn open_wizard(&mut self) {
        if self.wizard_formal.apply(FormalEvent::OpenWizard).is_ok() {
            self.screen = Screen::Wizard;
        }
    }

    /// Apply a live terminal resize without disturbing the current route,
    /// query, selection, or overlay state. Invalid dimensions are rejected
    /// atomically and leave the workspace unchanged.
    pub fn apply_resize(&mut self, columns: u16, lines: u16) -> bool {
        if !frame_dimensions_are_safe(columns, lines) {
            return false;
        }
        self.layout = ResponsiveLayout::from_dimensions(Some(columns), Some(lines));
        self.measure_cursor = self
            .measure_cursor
            .min(self.visible_indices().len().saturating_sub(1));
        self.config_cursor = self.config_cursor.min(3);
        self.report_cursor = self
            .report_cursor
            .min(self.report_entry_count().saturating_sub(1));
        true
    }

    /// The most recently observed responsive layout decision.
    pub const fn responsive_layout(&self) -> ResponsiveLayout {
        self.layout
    }

    /// Replace presentation data only after it has passed the typed control
    /// projection. No renderer input can mutate ASB state through this method.
    pub fn apply_live_snapshot(&mut self, snapshot: LiveSnapshot) {
        if let Some(catalog) = snapshot.measurement_catalog.as_ref()
            && let Some(selection) = selection_from_catalog(catalog, self.selection.as_ref())
        {
            self.selection = Some(selection);
            self.sync_measure_projection();
        }
        // Populate a first-run wizard from the authenticated control-plane
        // catalog so users select supported identifiers instead of typing
        // opaque values. Do not replace an in-progress draft during refresh.
        if self.wizard.catalog().is_none()
            && let Some(catalog) = wizard_catalog_from_snapshot(&snapshot)
        {
            self.wizard = Wizard::with_catalog(catalog.clone());
            self.wizard_formal = WizardFormalState::new_with_catalog(catalog)
                .expect("validated live wizard catalog must satisfy the state model");
        }
        // An authoritative, valid-but-unconfigured runner is the explicit
        // first-run signal. Route it into the wizard after the initial
        // authenticated refresh; disconnected or malformed states never open
        // a setup flow here.
        if snapshot
            .configuration
            .as_ref()
            .is_some_and(|configuration| !configuration.configured)
        {
            self.open_wizard();
        }
        self.live = Some(snapshot);
        self.report_cursor = 0;
        self.clamp_measure_cursor();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> UiAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return UiAction::Quit;
        }
        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h')
            ) {
                self.help = false;
            }
            return UiAction::None;
        }
        if self.screen == Screen::Wizard {
            return match key.code {
                KeyCode::Enter => {
                    let agent_selected = self.wizard.catalog().is_some_and(|catalog| {
                        catalog.kind() == crate::wizard_catalog::OptionKind::Agent
                            && !catalog
                                .selected_ids(crate::wizard_catalog::OptionKind::Agent)
                                .is_empty()
                    });
                    let event = if self.wizard.step() == wizard::Step::Review {
                        FormalEvent::Complete
                    } else if agent_selected {
                        FormalEvent::Next
                    } else if self.wizard.catalog().is_some() && catalog_step(self.wizard.step()) {
                        FormalEvent::CatalogSelect
                    } else {
                        FormalEvent::Next
                    };
                    if self.wizard_formal.apply(event.clone()).is_ok() {
                        self.wizard = self.wizard_formal.wizard().clone();
                        if matches!(event, FormalEvent::CatalogSelect)
                            && !agent_selected
                            && self.wizard_formal.apply(FormalEvent::Next).is_ok()
                        {
                            self.wizard = self.wizard_formal.wizard().clone();
                        }
                        if self.wizard_formal.route() == wizard::StartupRoute::Landing {
                            self.wizard_completion = Some(self.wizard.values());
                            self.screen = Screen::Landing;
                        }
                    }
                    UiAction::None
                }
                KeyCode::Up
                    if self.wizard.catalog().is_some() && catalog_step(self.wizard.step()) =>
                {
                    let _ = self.wizard_formal.apply(FormalEvent::CatalogMove(-1));
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Down
                    if self.wizard.catalog().is_some() && catalog_step(self.wizard.step()) =>
                {
                    let _ = self.wizard_formal.apply(FormalEvent::CatalogMove(1));
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Char(' ')
                    if self.wizard.step() == wizard::Step::Agent
                        && self.wizard.catalog().is_some() =>
                {
                    if self.wizard_formal.apply(FormalEvent::CatalogSelect).is_ok() {
                        self.wizard = self.wizard_formal.wizard().clone();
                    }
                    UiAction::None
                }
                KeyCode::Char('a')
                    if self.wizard.step() == wizard::Step::Agent
                        && self.wizard.catalog().is_some() =>
                {
                    if self
                        .wizard_formal
                        .apply(FormalEvent::CatalogSelectAllAgents)
                        .is_ok()
                    {
                        self.wizard = self.wizard_formal.wizard().clone();
                    }
                    UiAction::None
                }
                KeyCode::Esc => {
                    if self.wizard_formal.apply(FormalEvent::Back).is_err() {
                        self.screen = Screen::Landing;
                    } else {
                        self.wizard = self.wizard_formal.wizard().clone();
                    }
                    UiAction::None
                }
                KeyCode::Char('q') => {
                    if self.wizard_formal.apply(FormalEvent::Cancel).is_ok() {
                        self.wizard = self.wizard_formal.wizard().clone();
                        self.screen = Screen::Landing;
                    }
                    UiAction::None
                }
                KeyCode::Char('?') | KeyCode::Char('h') => {
                    self.help = true;
                    UiAction::None
                }
                KeyCode::Char(c) if !c.is_control() => {
                    if self.wizard.catalog().is_some() && catalog_step(self.wizard.step()) {
                        let mut query = self
                            .wizard
                            .catalog()
                            .map_or_else(String::new, |catalog| catalog.query().to_owned());
                        query.push(c);
                        if self
                            .wizard_formal
                            .apply(FormalEvent::CatalogQuery(query))
                            .is_ok()
                        {
                            self.wizard = self.wizard_formal.wizard().clone();
                        }
                        return UiAction::None;
                    }
                    let mut value = self.wizard.current_value().to_owned();
                    value.push(c);
                    if self
                        .wizard_formal
                        .apply(FormalEvent::SetValue(value))
                        .is_ok()
                    {
                        self.wizard = self.wizard_formal.wizard().clone();
                    }
                    UiAction::None
                }
                KeyCode::Backspace => {
                    if self.wizard.catalog().is_some() && catalog_step(self.wizard.step()) {
                        let mut query = self
                            .wizard
                            .catalog()
                            .map_or_else(String::new, |catalog| catalog.query().to_owned());
                        query.pop();
                        if self
                            .wizard_formal
                            .apply(FormalEvent::CatalogQuery(query))
                            .is_ok()
                        {
                            self.wizard = self.wizard_formal.wizard().clone();
                        }
                        return UiAction::None;
                    }
                    let mut value = self.wizard.current_value().to_owned();
                    value.pop();
                    if self
                        .wizard_formal
                        .apply(FormalEvent::SetValue(value))
                        .is_ok()
                    {
                        self.wizard = self.wizard_formal.wizard().clone();
                    }
                    UiAction::None
                }
                _ => UiAction::None,
            };
        }
        if self.screen == Screen::Configuration
            && key.code == KeyCode::Char('s')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            if self.configuration_editing && self.commit_configuration_edit().is_err() {
                return UiAction::None;
            }
            let _ = self.save_configuration();
            return UiAction::None;
        }
        match key.code {
            KeyCode::Char('q') => UiAction::Quit,
            KeyCode::Char('?') | KeyCode::Char('h') => {
                self.help = true;
                UiAction::None
            }
            KeyCode::Char('1') => {
                self.screen = Screen::Landing;
                UiAction::None
            }
            KeyCode::Char('2') => {
                self.screen = Screen::Measures;
                UiAction::None
            }
            KeyCode::Char('3') => {
                self.screen = Screen::Configuration;
                UiAction::None
            }
            KeyCode::Char('w') => {
                self.open_wizard();
                UiAction::None
            }
            KeyCode::Char('4') => {
                self.screen = Screen::Reports;
                UiAction::None
            }
            KeyCode::Tab | KeyCode::Right => {
                self.screen = next_screen(self.screen);
                UiAction::None
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.screen = previous_screen(self.screen);
                UiAction::None
            }
            KeyCode::Up => {
                self.move_cursor(-1);
                UiAction::None
            }
            KeyCode::Down => {
                self.move_cursor(1);
                UiAction::None
            }
            KeyCode::Enter if self.screen == Screen::Configuration => {
                if self.configuration_editing {
                    let _ = self.commit_configuration_edit();
                    return UiAction::None;
                }
                let setting = self
                    .configuration_draft
                    .visible_settings()
                    .get(self.config_cursor)
                    .map(|descriptor| descriptor.id);
                self.configuration_draft.focus(setting);
                self.configuration_editing = setting.is_some();
                self.configuration_edit_buffer = setting
                    .and_then(|id| self.configuration_draft.value(id))
                    .unwrap_or_default();
                self.configuration_edit_error = None;
                UiAction::None
            }
            KeyCode::Backspace
                if self.screen == Screen::Configuration && self.configuration_editing =>
            {
                self.configuration_edit_buffer.pop();
                self.apply_configuration_edit_buffer();
                UiAction::None
            }
            KeyCode::Char(c)
                if self.screen == Screen::Configuration
                    && self.configuration_editing
                    && !c.is_control() =>
            {
                if self.configuration_edit_buffer.chars().count()
                    < crate::configuration::MAX_STRING_SCALARS
                {
                    self.configuration_edit_buffer.push(c);
                    self.apply_configuration_edit_buffer();
                }
                UiAction::None
            }
            KeyCode::Char(' ') if self.screen == Screen::Measures => {
                self.toggle_current_measure();
                UiAction::None
            }
            KeyCode::Char('g') if self.screen == Screen::Measures => {
                self.toggle_visible_group();
                UiAction::None
            }
            KeyCode::Char('/') if self.screen == Screen::Measures => UiAction::None,
            KeyCode::Char(c) if self.screen == Screen::Measures && !c.is_control() => {
                let mut candidate = self.search.clone();
                candidate.push(c);
                if candidate.chars().count() <= MAX_QUERY_BYTES {
                    self.search = candidate;
                    if let Some(selection) = self.selection.as_mut() {
                        let _ = selection.set_query(self.search.clone());
                    }
                    self.sync_measure_projection();
                }
                self.measure_cursor = 0;
                UiAction::None
            }
            KeyCode::Backspace if self.screen == Screen::Measures => {
                self.search.pop();
                if let Some(selection) = self.selection.as_mut() {
                    let _ = selection.set_query(self.search.clone());
                }
                self.sync_measure_projection();
                self.measure_cursor = 0;
                UiAction::None
            }
            _ => UiAction::None,
        }
    }

    fn move_cursor(&mut self, delta: i8) {
        let max = match self.screen {
            Screen::Measures => self.visible_indices().len(),
            Screen::Configuration => self.configuration_draft.visible_settings().len(),
            Screen::Reports => 3,
            _ => 1,
        };
        if max == 0 {
            self.measure_cursor = 0;
            return;
        }
        let cursor = match self.screen {
            Screen::Measures => &mut self.measure_cursor,
            Screen::Configuration => &mut self.config_cursor,
            Screen::Reports => &mut self.report_cursor,
            _ => return,
        };
        if delta < 0 {
            *cursor = cursor.saturating_sub(1);
        } else {
            *cursor = (*cursor + 1).min(max - 1);
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self.search.to_ascii_lowercase();
        self.measures
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                query.is_empty()
                    || row.name.to_ascii_lowercase().contains(&query)
                    || row.group.to_ascii_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn clamp_measure_cursor(&mut self) {
        self.measure_cursor = self
            .measure_cursor
            .min(self.visible_indices().len().saturating_sub(1));
    }

    fn sync_measure_projection(&mut self) {
        if let Some(selection) = self.selection.as_ref() {
            self.measures = selection
                .measurements()
                .iter()
                .map(|measurement| MeasureRow {
                    id: measurement.id().to_owned(),
                    group: measurement.group().to_owned(),
                    name: measurement.name().to_owned(),
                    selected: selection.is_selected(measurement.id()),
                })
                .collect();
        }
        self.clamp_measure_cursor();
    }

    fn toggle_current_measure(&mut self) {
        let Some(index) = self.visible_indices().get(self.measure_cursor).copied() else {
            return;
        };
        if let Some(selection) = self.selection.as_mut() {
            let Some(measurement) = selection
                .measurements()
                .iter()
                .find(|measurement| measurement.id() == self.measures[index].id)
            else {
                return;
            };
            let id = measurement.id().to_owned();
            let selected = !selection.is_selected(&id);
            let _ = selection.set_measure_selected(&id, selected);
            self.sync_measure_projection();
        } else if let Some(row) = self.measures.get_mut(index) {
            row.selected = !row.selected;
        }
    }

    fn toggle_visible_group(&mut self) {
        let indices = self.visible_indices();
        let Some(&current) = indices.get(self.measure_cursor) else {
            return;
        };
        let group = self.measures[current].group.clone();
        if let Some(selection) = self.selection.as_mut() {
            let select = selection
                .visible_groups()
                .into_iter()
                .find(|summary| summary.group() == group)
                .is_some_and(|summary| summary.state() != crate::selection::GroupSelection::All);
            let _ = selection.set_visible_group_selected(&group, select);
            self.sync_measure_projection();
            return;
        }
        let members: Vec<usize> = indices
            .into_iter()
            .filter(|index| self.measures[*index].group == group)
            .collect();
        let select = members.iter().any(|index| !self.measures[*index].selected);
        for index in members {
            self.measures[index].selected = select;
        }
    }

    fn report_entry_count(&self) -> usize {
        self.live
            .as_ref()
            .map_or(2, |snapshot| snapshot.runs.len().max(1))
    }
}

const fn catalog_step(step: wizard::Step) -> bool {
    matches!(
        step,
        wizard::Step::Agent | wizard::Step::Provider | wizard::Step::Model
    )
}

/// Adapt validated live agent/provider/model data to the renderer-neutral
/// wizard catalog. The protocol currently exposes provider availability but
/// not per-agent provider compatibility, so available providers are offered
/// for every available agent. Duplicate model IDs are merged and retain all
/// provider compatibility references.
fn wizard_catalog_from_snapshot(snapshot: &LiveSnapshot) -> Option<WizardCatalog> {
    let agents = snapshot.agent_catalog.as_ref()?.agents.as_slice();
    let providers = snapshot.provider_catalog.as_ref()?.providers.as_slice();
    if agents.is_empty() || providers.is_empty() {
        return None;
    }
    let agent_ids: Vec<String> = agents.iter().map(|entry| entry.agent_id.clone()).collect();
    let agent_options = agents
        .iter()
        .map(|entry| {
            WizardOption::new(
                entry.agent_id.clone(),
                entry.agent_id.clone(),
                matches!(
                    entry.availability,
                    crate::agent_catalog::AgentAvailability::Available
                ),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let provider_options = providers
        .iter()
        .map(|entry| {
            WizardOption::new(
                entry.provider_id.clone(),
                entry.display_name.clone(),
                matches!(
                    entry.availability,
                    crate::control_codec::ProviderAvailability::Available
                ),
            )
            .and_then(|option| option.compatible_with(agent_ids.clone()))
        })
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let mut model_providers = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut model_available = std::collections::BTreeMap::<String, bool>::new();
    for provider in providers {
        for model in &provider.models {
            model_providers
                .entry(model.model_id.clone())
                .or_default()
                .push(provider.provider_id.clone());
            let available = matches!(
                provider.availability,
                crate::control_codec::ProviderAvailability::Available
            ) && matches!(
                model.availability,
                crate::control_codec::ProviderAvailability::Available
            );
            model_available
                .entry(model.model_id.clone())
                .and_modify(|current| *current |= available)
                .or_insert(available);
        }
    }
    let model_options = model_providers
        .into_iter()
        .map(|(model_id, provider_ids)| {
            let available = model_available.get(&model_id).copied().unwrap_or(false);
            WizardOption::new(model_id.clone(), model_id, available)
                .and_then(|option| option.compatible_with(provider_ids))
        })
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    WizardCatalog::new(agent_options, provider_options, model_options).ok()
}

fn selection_from_catalog(
    catalog: &MeasurementCatalog,
    previous: Option<&MeasurementSelection>,
) -> Option<MeasurementSelection> {
    let measurements = catalog
        .measurements
        .iter()
        .map(|definition| {
            let group = catalog
                .groups
                .iter()
                .find(|group| group.id == definition.group)
                .map(|group| group.label.clone())?;
            Measurement::new(
                definition.id.clone(),
                group,
                definition.name.clone(),
                definition.unit.clone(),
            )
            .ok()
        })
        .collect::<Option<Vec<_>>>()?;
    let mut selection = MeasurementSelection::new(measurements).ok()?;
    if let Some(previous) = previous {
        for id in previous.selected_ids() {
            let _ = selection.set_measure_selected(id, true);
        }
    }
    Some(selection)
}

fn next_screen(screen: Screen) -> Screen {
    match screen {
        Screen::Landing => Screen::Measures,
        Screen::Wizard => Screen::Measures,
        Screen::Measures => Screen::Configuration,
        Screen::Configuration => Screen::Reports,
        Screen::Reports | Screen::Help => Screen::Landing,
    }
}
fn previous_screen(screen: Screen) -> Screen {
    match screen {
        Screen::Landing => Screen::Reports,
        Screen::Wizard => Screen::Landing,
        Screen::Measures => Screen::Landing,
        Screen::Configuration => Screen::Measures,
        Screen::Reports | Screen::Help => Screen::Configuration,
    }
}

pub fn render(frame: &mut Frame<'_>, state: &WorkspaceState, policy: RenderPolicy) {
    let area = frame.area();
    if state.screen == Screen::Wizard {
        wizard::render(frame, &state.wizard, policy);
        if state.help {
            wizard_help_overlay(frame, area, policy, wizard::element_id(state.wizard.step()));
        }
        return;
    }
    if area.width < 38 || area.height < 8 {
        render_compact(frame, area, policy);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(area);
    let titles = ["1 Home", "2 Measures", "3 Configure", "4 Reports"];
    let selected = match state.screen {
        Screen::Landing => 0,
        Screen::Wizard => 2,
        Screen::Measures => 1,
        Screen::Configuration => 2,
        Screen::Reports => 3,
        Screen::Help => 0,
    };
    frame.render_widget(
        Tabs::new(titles)
            .select(selected)
            .block(
                Block::default()
                    .title(" ASB - Agent Systems Benchmark ")
                    .borders(Borders::ALL),
            )
            .highlight_style(accent(policy)),
        chunks[0],
    );
    match state.screen {
        Screen::Landing => landing(frame, chunks[1], state, policy),
        Screen::Wizard => unreachable!("wizard is rendered before workspace layout"),
        Screen::Measures => measures(frame, chunks[1], state, policy),
        Screen::Configuration => configuration(frame, chunks[1], state, policy),
        Screen::Reports => reports(frame, chunks[1], state, policy),
        Screen::Help => landing(frame, chunks[1], state, policy),
    }
    footer(frame, chunks[2], state, policy);
    if state.help {
        help_overlay(frame, area, policy);
    }
}

fn landing(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);
    let connection = state.live.as_ref().map_or(
        "Connection: waiting for ASB control plane".to_string(),
        |snapshot| match snapshot.connection {
            Connection::Negotiated => format!(
                "Connection: authenticated runner {}",
                snapshot.runner_instance_id.as_deref().unwrap_or("unknown")
            ),
            Connection::Disconnected => "Connection: disconnected".to_string(),
        },
    );
    frame.render_widget(
        Paragraph::new("Welcome to ASB")
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center),
        inner[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Review benchmark plans, select measures, and compare recent runs."),
            Line::from(connection),
            Line::from("The runner remains external; this workspace is a safe control surface."),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(panel(" Ready ", policy)),
        inner[1],
    );
    frame.render_widget(
        Paragraph::new("Press 2 to choose measures  |  3 to configure  |  4 to inspect reports")
            .alignment(Alignment::Center)
            .style(muted(policy)),
        inner[2],
    );
}

fn measures(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(area);
    let search = if state.search.is_empty() {
        "Search measures (type to filter)"
    } else {
        &state.search
    };
    frame.render_widget(
        Paragraph::new(search)
            .block(panel(" / Search ", policy))
            .style(if state.search.is_empty() {
                muted(policy)
            } else {
                accent(policy)
            }),
        cols[0],
    );
    let list_area = Rect {
        x: cols[0].x,
        y: cols[0].y.saturating_add(2),
        width: cols[0].width,
        height: cols[0].height.saturating_sub(2),
    };
    let items: Vec<ListItem> = state
        .visible_indices()
        .iter()
        .map(|i| {
            let row = &state.measures[*i];
            ListItem::new(Line::from(vec![
                Span::styled(if row.selected { "[x] " } else { "[ ] " }, accent(policy)),
                Span::styled(format!("{} / {}", row.group, row.name), Style::default()),
            ]))
        })
        .collect();
    let mut ls = ListState::default();
    ls.select((state.measure_cursor < items.len()).then_some(state.measure_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Measures - Space item, g group ", policy))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        list_area,
        &mut ls,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Grouped selection"),
            Line::from(""),
            Line::from(format!(
                "{} selected",
                state.measures.iter().filter(|m| m.selected).count()
            )),
            Line::from(""),
            Line::from("Up/Down navigate   Space item   g group"),
            Line::from("Type to search"),
        ])
        .block(panel(" Selection ", policy))
        .wrap(Wrap { trim: true }),
        cols[1],
    );
}

fn configuration(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    let entries = state.configuration_draft.visible_settings();
    let items: Vec<ListItem> = entries
        .iter()
        .map(|setting| {
            let value = if state.configuration_editing
                && state.configuration_draft.focused() == Some(setting.id)
            {
                state.configuration_edit_buffer.clone()
            } else {
                state
                    .configuration_draft
                    .value(setting.id)
                    .unwrap_or_default()
            };
            ListItem::new(format!(
                "{} = {} [{}]",
                setting.label, value, setting.category
            ))
        })
        .collect();
    let mut ls = ListState::default();
    ls.select((state.config_cursor < items.len()).then_some(state.config_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Configuration - Enter focus, Ctrl-S apply ", policy))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        columns[0],
        &mut ls,
    );
    let status = match state.configuration_save_state {
        ConfigurationSaveState::Clean => "No local changes",
        ConfigurationSaveState::Dirty => "Unsaved local changes",
        ConfigurationSaveState::Saved => "Applied to local configuration",
        ConfigurationSaveState::Unavailable => "Save unavailable: no local store",
        ConfigurationSaveState::Failed => "Save failed: draft retained",
        ConfigurationSaveState::Invalid => "Invalid value: correct it before applying",
    };
    let footer = Rect {
        x: columns[0].x,
        y: columns[0]
            .y
            .saturating_add(columns[0].height.saturating_sub(2)),
        width: columns[0].width,
        height: 2.min(columns[0].height),
    };
    let detail = state.configuration_edit_error.as_deref().unwrap_or("");
    frame.render_widget(
        Paragraph::new(format!("{status}  {detail}")).style(muted(policy)),
        footer,
    );
    let authoritative = state.live.as_ref().map_or_else(
        || {
            vec![
                Line::from("Runner setup: not connected"),
                Line::from("Provider/model catalog unavailable"),
            ]
        },
        |snapshot| {
            let mut lines = vec![Line::from("Runner-authoritative setup")];
            if let Some(config) = &snapshot.configuration {
                lines.push(Line::from(format!(
                    "Provider: {}",
                    config.provider_id.as_deref().unwrap_or("not configured")
                )));
                lines.push(Line::from(format!(
                    "Model: {}",
                    config.model_id.as_deref().unwrap_or("not configured")
                )));
                lines.push(Line::from(format!("Agents: {}", config.agent_ids.len())));
            } else {
                lines.push(Line::from("Configuration status unavailable"));
            }
            if let Some(auth) = &snapshot.auth_status {
                lines.push(Line::from(format!(
                    "Authentication: {} (provider {})",
                    auth.status, auth.provider
                )));
            } else {
                lines.push(Line::from("Authentication: unavailable"));
            }
            if let Some(campaign) = &snapshot.recording_campaign {
                lines.push(Line::from(format!(
                    "Recording: {} ({} tuples)",
                    campaign.state, campaign.tuple_count
                )));
                lines.push(Line::from(if campaign.offline_ready {
                    "Offline default: ready"
                } else {
                    "Offline default: not ready"
                }));
            } else {
                lines.push(Line::from("Recording: no campaign"));
                lines.push(Line::from("Offline default: not ready"));
            }
            if let Some(catalog) = &snapshot.provider_catalog {
                lines.push(Line::from(format!(
                    "Providers: {} available",
                    catalog.providers.len()
                )));
            }
            lines
        },
    );
    frame.render_widget(
        Paragraph::new(authoritative)
            .block(panel(" ASB control status ", policy))
            .wrap(Wrap { trim: true }),
        columns[1],
    );
}

fn reports(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let entries: Vec<String> = state.live.as_ref().map_or_else(
        || {
            vec![
                "No authoritative history loaded".to_string(),
                "Connect to inspect recent runs".to_string(),
            ]
        },
        |snapshot| {
            snapshot
                .runs
                .iter()
                .map(|run| {
                    format!(
                        "{} | {:?} | revision {}",
                        run.run_id.0, run.state, run.revision.0
                    )
                })
                .collect()
        },
    );
    let items: Vec<ListItem> = entries.into_iter().map(ListItem::new).collect();
    let mut ls = ListState::default();
    ls.select(Some(state.report_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Recent runs - Reports ", policy))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut ls,
    );
}

fn footer(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let help = if state.help {
        "Esc close help"
    } else {
        "? help"
    };
    let context = match state.screen {
        Screen::Landing => "w setup   2 measures   3 configure   4 reports",
        Screen::Wizard => "Enter next   Esc back   q cancel",
        Screen::Measures => "Up/Down move   Space item   g group   type search",
        Screen::Configuration => "Up/Down move   Enter edit   Ctrl-S save",
        Screen::Reports => "Up/Down move   Enter open/compare",
        Screen::Help => "Esc close help",
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{}    {}    Tab/Left/Right navigate    q quit",
            help, context
        ))
        .alignment(Alignment::Center)
        .style(muted(policy)),
        area,
    );
}
fn help_overlay(frame: &mut Frame<'_>, area: Rect, policy: RenderPolicy) {
    let popup = centered(area, 70, 65);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("Keyboard help", accent(policy))),
            Line::from("1-4  switch workspace"),
            Line::from("Tab / arrows  navigate"),
            Line::from("Up/Down  move selection"),
            Line::from("Space  toggle measure"),
            Line::from("g  toggle the current measure group"),
            Line::from("Type / Backspace  search measures"),
            Line::from("Enter  open, edit, or compare the focused item"),
            Line::from("q / Ctrl-C  quit"),
            Line::from("Esc  close this window"),
        ])
        .block(panel(" Help ", policy))
        .wrap(Wrap { trim: true }),
        popup,
    );
}

fn wizard_help_overlay(frame: &mut Frame<'_>, area: Rect, policy: RenderPolicy, element: &str) {
    let popup = centered(area, 70, 65);
    frame.render_widget(Clear, popup);
    let context = document_help_text(element)
        .unwrap_or("Edit the active wizard value and use Enter to continue.");
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("Wizard help", accent(policy))),
            Line::from(context),
            Line::from("Type to append | Backspace to correct"),
            Line::from("Enter next or complete | Esc back | q cancel"),
            Line::from("? / h / Esc close this window"),
        ])
        .block(panel(" Contextual help ", policy))
        .wrap(Wrap { trim: true }),
        popup,
    );
}
fn render_compact(frame: &mut Frame<'_>, area: Rect, policy: RenderPolicy) {
    frame.render_widget(
        Paragraph::new("ASB | 1 Home 2 Measures 3 Config 4 Reports\n? help | q quit")
            .style(accent(policy)),
        area,
    );
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}
fn panel(title: &'static str, policy: RenderPolicy) -> Block<'static> {
    Block::default().title(title).borders(if policy.unicode {
        Borders::ALL
    } else {
        Borders::NONE
    })
}
fn accent(policy: RenderPolicy) -> Style {
    if matches!(policy.tier, CapabilityTier::Plain) {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }
}
fn muted(policy: RenderPolicy) -> Style {
    if matches!(policy.tier, CapabilityTier::Plain) {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::ConfigurationStore;
    use crate::control_codec::{AttemptId, PublicRunState, Revision, RunId, RunSummary};
    use crossterm::event::KeyEventKind;
    use ratatui::{Terminal, backend::TestBackend};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::CONTROL, KeyEventKind::Press)
    }

    fn private_temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "asb-tui-ui-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }
    fn policy() -> RenderPolicy {
        RenderPolicy {
            tier: CapabilityTier::TrueColor,
            unicode: true,
            mouse: false,
            focus: false,
            bracketed_paste: false,
            synchronized_output: false,
            alternate_screen: true,
        }
    }
    #[test]
    fn navigation_and_selection_are_keyboard_first() {
        let mut s = WorkspaceState::default();
        assert_eq!(s.handle_key(key(KeyCode::Char('2'))), UiAction::None);
        assert_eq!(s.screen, Screen::Measures);
        s.handle_key(key(KeyCode::Down));
        s.handle_key(key(KeyCode::Char(' ')));
        assert!(!s.measures[1].selected);
        s.handle_key(key(KeyCode::Char('g')));
        assert!(s.measures[0].selected && s.measures[1].selected);
        s.handle_key(key(KeyCode::Char('c')));
        assert_eq!(s.search, "c");
        s.handle_key(key(KeyCode::Char('?')));
        assert!(s.help);
        s.handle_key(key(KeyCode::Esc));
        assert!(!s.help);
    }

    #[test]
    fn configuration_screen_save_is_truthful_and_atomic() {
        let directory = private_temp_dir();
        let path = directory.join("config.json");
        let mut state = WorkspaceState::with_configuration_store(&path).unwrap();
        state.screen = Screen::Configuration;
        state
            .edit_configuration(|configuration| configuration.frontend.contrast = true)
            .unwrap();
        assert_eq!(
            state.configuration_save_state(),
            ConfigurationSaveState::Dirty
        );
        state.handle_key(ctrl_key(KeyCode::Char('s')));
        assert_eq!(
            state.configuration_save_state(),
            ConfigurationSaveState::Saved
        );
        assert!(
            ConfigurationStore::new(&path)
                .load()
                .unwrap()
                .frontend
                .contrast
        );
        assert!(!state.configuration_draft().is_dirty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn configuration_value_editing_shows_draft_and_rejects_invalid_text() {
        let mut state = WorkspaceState {
            screen: Screen::Configuration,
            ..WorkspaceState::default()
        };
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.configuration_edit_value(), Some("dark"));
        state.handle_key(key(KeyCode::Char('x')));
        assert_eq!(
            state.configuration_save_state(),
            ConfigurationSaveState::Invalid
        );
        assert_eq!(state.configuration_edit_value(), Some("darkx"));
        state.handle_key(key(KeyCode::Backspace));
        assert_eq!(
            state.configuration_save_state(),
            ConfigurationSaveState::Unavailable
        );
        assert_eq!(state.configuration_edit_value(), Some("dark"));
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.configuration_edit_value(), None);
    }

    #[test]
    fn configuration_save_without_store_reports_unavailable_and_keeps_draft() {
        let mut state = WorkspaceState {
            screen: Screen::Configuration,
            ..WorkspaceState::default()
        };
        state
            .edit_configuration(|configuration| configuration.frontend.contrast = true)
            .unwrap();
        state.handle_key(ctrl_key(KeyCode::Char('s')));
        assert_eq!(
            state.configuration_save_state(),
            ConfigurationSaveState::Unavailable
        );
        assert!(state.configuration_draft().is_dirty());
    }

    #[test]
    fn configuration_save_failure_keeps_dirty_draft_and_previous_file() {
        let directory = private_temp_dir();
        let path = directory.join("config.json");
        let mut state = WorkspaceState::with_configuration_store(&path).unwrap();
        state
            .edit_configuration(|configuration| configuration.frontend.contrast = true)
            .unwrap();
        fs::remove_dir_all(&directory).unwrap();
        assert!(state.save_configuration().is_err());
        assert_eq!(
            state.configuration_save_state(),
            ConfigurationSaveState::Failed
        );
        assert!(state.configuration_draft().is_dirty());
    }

    #[test]
    fn authoritative_empty_catalog_replaces_built_in_measure_rows() {
        let mut state = WorkspaceState::default();
        let snapshot = LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner-catalog".into()),
            latest_revision: None,
            capabilities: None,
            measurement_catalog: Some(MeasurementCatalog {
                schema_version: 1,
                catalog_sha256: String::new(),
                groups: Vec::new(),
                measurements: Vec::new(),
            }),
            auth_status: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            configuration: None,
            recording_campaign: None,
            runs: Vec::new(),
        };
        state.apply_live_snapshot(snapshot);
        assert!(state.measures.is_empty());
        assert_eq!(state.measure_cursor, 0);
    }

    #[test]
    fn live_setup_catalog_populates_wizard_choices_without_secrets() {
        let agent = crate::agent_catalog::AgentCatalogEntry {
            agent_id: "codex".into(),
            target: crate::agent_catalog::AgentTarget {
                operating_system: "linux".into(),
                architecture: "x86_64".into(),
                libc: "glibc".into(),
                libc_version: "2.39".into(),
            },
            package: crate::agent_catalog::AgentPackage {
                package_id: "codex-package".into(),
                version: "1".into(),
                sha256: "a".repeat(64),
                signature_sha256: "b".repeat(64),
            },
            provenance: crate::agent_catalog::AgentProvenance {
                source_revision: "source".into(),
                manifest_sha256: "c".repeat(64),
            },
            capabilities: vec!["coding".into()],
            availability: crate::agent_catalog::AgentAvailability::Available,
        };
        let snapshot = LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner".into()),
            latest_revision: Some(Revision(1)),
            capabilities: None,
            measurement_catalog: None,
            auth_status: None,
            agent_catalog: Some(crate::agent_catalog::AgentCatalog {
                runner_instance_id: "runner".into(),
                generation: 1,
                catalog_sha256: "d".repeat(64),
                target: agent.target.clone(),
                agents: vec![agent],
                refreshed: true,
            }),
            agent_lifecycle: None,
            provider_catalog: Some(crate::control_codec::ProviderCatalog {
                runner_instance_id: "runner".into(),
                generation: Revision(1),
                catalog_sha256: "e".repeat(64),
                providers: vec![crate::control_codec::ProviderCatalogEntry {
                    provider_id: "openai".into(),
                    display_name: "OpenAI".into(),
                    auth_methods: vec![ProviderAuthMethod::CredentialReference],
                    models: vec![crate::control_codec::ProviderModel {
                        model_id: "gpt-test".into(),
                        revision: "pinned".into(),
                        availability: crate::control_codec::ProviderAvailability::Available,
                    }],
                    availability: crate::control_codec::ProviderAvailability::Available,
                }],
                refreshed: true,
            }),
            configuration: None,
            recording_campaign: None,
            runs: Vec::new(),
        };
        let mut state = WorkspaceState::default();
        state.apply_live_snapshot(snapshot);
        let catalog = state.wizard.catalog().expect("live catalog attached");
        assert_eq!(
            catalog
                .catalog()
                .options(crate::wizard_catalog::OptionKind::Agent)
                .len(),
            1
        );
        assert_eq!(
            catalog
                .catalog()
                .options(crate::wizard_catalog::OptionKind::Provider)
                .len(),
            1
        );
        assert_eq!(
            catalog
                .catalog()
                .options(crate::wizard_catalog::OptionKind::Model)
                .len(),
            1
        );
        assert!(
            WorkspaceState::wizard_configuration_selection(&[
                "codex".into(),
                "openai".into(),
                "gpt-test".into(),
                "".into(),
                "none".into(),
                "".into(),
                "".into(),
            ])
            .is_ok()
        );
    }

    #[test]
    fn startup_and_manual_wizard_route_are_local_and_one_shot() {
        let mut state = WorkspaceState::for_startup(false);
        assert_eq!(state.screen, Screen::Wizard);
        state.handle_key(key(KeyCode::Char('q')));
        assert_eq!(state.screen, Screen::Landing);
        state.handle_key(key(KeyCode::Char('w')));
        assert_eq!(state.screen, Screen::Wizard);
        state.handle_key(key(KeyCode::Char('a')));
        assert_eq!(state.wizard.step(), crate::wizard::Step::Agent);
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.wizard.step(), crate::wizard::Step::Provider);
        state.handle_key(key(KeyCode::Esc));
        assert_eq!(state.wizard.step(), crate::wizard::Step::Agent);
        let configured = WorkspaceState::for_startup(true);
        assert_eq!(configured.screen, Screen::Landing);
    }

    #[test]
    fn authoritative_unconfigured_runner_opens_first_run_wizard() {
        let mut state = WorkspaceState::default();
        state.apply_live_snapshot(LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner".into()),
            latest_revision: Some(Revision(1)),
            capabilities: None,
            measurement_catalog: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            configuration: Some(crate::control_codec::ConfigurationSnapshot {
                runner_instance_id: "runner".into(),
                generation: Revision(1),
                configured: false,
                agent_ids: Vec::new(),
                provider_id: None,
                model_id: None,
                auth_method: None,
                credential_reference_sha256: None,
            }),
            auth_status: None,
            recording_campaign: None,
            runs: Vec::new(),
        });
        assert_eq!(state.screen, Screen::Wizard);
    }

    #[test]
    fn readiness_injection_opens_only_for_explicitly_unconfigured_states() {
        let base = StartupInput {
            configuration_present: true,
            configuration_complete: true,
            endpoint_available: true,
            configuration_malformed: false,
            configuration_stale: false,
            authorized: true,
        };
        assert_eq!(WorkspaceState::for_readiness(base).screen, Screen::Landing);
        assert_eq!(
            WorkspaceState::for_readiness(StartupInput {
                configuration_present: false,
                ..base
            })
            .screen,
            Screen::Wizard
        );
        assert_eq!(
            WorkspaceState::for_readiness(StartupInput {
                endpoint_available: false,
                ..base
            })
            .screen,
            Screen::Landing
        );
        assert_eq!(
            WorkspaceState::for_readiness(StartupInput {
                configuration_malformed: true,
                ..base
            })
            .screen,
            Screen::Landing
        );
        assert_eq!(
            WorkspaceState::for_readiness(StartupInput {
                configuration_stale: true,
                ..base
            })
            .screen,
            Screen::Landing
        );
        assert_eq!(
            WorkspaceState::for_readiness(StartupInput {
                authorized: false,
                ..base
            })
            .screen,
            Screen::Landing
        );
    }

    #[test]
    fn review_enter_applies_formal_completion_and_returns_to_landing() {
        let mut state = WorkspaceState::for_startup(false);
        for _ in 0..7 {
            state.handle_key(key(KeyCode::Char('x')));
            state.handle_key(key(KeyCode::Enter));
        }
        assert_eq!(state.wizard.step(), crate::wizard::Step::Review);
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.screen, Screen::Landing);
        assert_eq!(state.wizard.step(), crate::wizard::Step::Review);
    }

    #[test]
    fn wizard_editing_appends_backspaces_and_opens_contextual_help() {
        let mut state = WorkspaceState::for_startup(false);
        state.handle_key(key(KeyCode::Char('a')));
        state.handle_key(key(KeyCode::Char('b')));
        assert_eq!(state.wizard.current_value(), "ab");
        state.handle_key(key(KeyCode::Backspace));
        assert_eq!(state.wizard.current_value(), "a");
        state.handle_key(key(KeyCode::Char('?')));
        assert!(state.help);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Wizard help"));
        assert!(text.contains("Choose which supported agent"));
        state.handle_key(key(KeyCode::Esc));
        assert!(!state.help);
    }

    #[test]
    fn failed_formal_completion_does_not_change_wizard_route() {
        let mut state = WorkspaceState {
            screen: Screen::Wizard,
            ..WorkspaceState::default()
        };
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.screen, Screen::Wizard);
        assert_eq!(state.wizard.step(), crate::wizard::Step::Agent);
    }

    #[test]
    fn wizard_route_hands_rendering_to_wizard_at_normal_and_tiny_sizes() {
        let mut state = WorkspaceState::for_startup(false);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("setup wizard"));
        terminal.backend_mut().resize(24, 6);
        state.apply_resize(24, 6);
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("ASB setup wizard"));
    }
    #[test]
    fn test_backend_snapshot_contains_contextual_controls() {
        let mut s = WorkspaceState::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| render(f, &s, policy())).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Welcome to ASB"));
        assert!(text.contains("? help"));
        s.screen = Screen::Measures;
        terminal.draw(|f| render(f, &s, policy())).unwrap();
        let measures_text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(measures_text.contains("Space item"));
        assert!(measures_text.contains("g group"));
    }

    #[test]
    fn renders_every_workspace_route_and_terminal_fallback() {
        let snapshot = LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner-test".into()),
            latest_revision: Some(Revision(7)),
            capabilities: None,
            measurement_catalog: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            configuration: None,
            auth_status: None,
            recording_campaign: None,
            runs: vec![RunSummary {
                run_id: RunId("run-7".into()),
                attempt_id: AttemptId("attempt-7".into()),
                state: PublicRunState::Completed,
                created_revision: Revision(7),
                revision: Revision(7),
                plan_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            }],
        };
        let mut state = WorkspaceState::default();
        state.apply_live_snapshot(snapshot);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        for screen in [
            Screen::Landing,
            Screen::Measures,
            Screen::Configuration,
            Screen::Reports,
            Screen::Help,
        ] {
            state.screen = screen;
            terminal
                .draw(|frame| render(frame, &state, policy()))
                .unwrap();
        }
        state.help = true;
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
        state.help = false;
        terminal
            .draw(|frame| {
                render(
                    frame,
                    &state,
                    RenderPolicy {
                        tier: CapabilityTier::Plain,
                        unicode: false,
                        ..policy()
                    },
                )
            })
            .unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    &state,
                    RenderPolicy {
                        tier: CapabilityTier::Plain,
                        ..policy()
                    },
                )
            })
            .unwrap();
        state.screen = Screen::Configuration;
        state.handle_key(key(KeyCode::Down));
        state.screen = Screen::Reports;
        state.handle_key(key(KeyCode::Down));
        state.screen = Screen::Landing;
        state.handle_key(key(KeyCode::Left));
        assert_eq!(state.screen, Screen::Reports);
    }

    #[test]
    fn key_handling_covers_help_navigation_and_search_edges() {
        let mut state = WorkspaceState::default();
        assert_eq!(state.handle_key(key(KeyCode::Char('h'))), UiAction::None);
        assert!(state.help);
        state.handle_key(key(KeyCode::Char('h')));
        assert!(!state.help);
        state.screen = Screen::Measures;
        state.handle_key(key(KeyCode::Char('/')));
        state.handle_key(key(KeyCode::Char('e')));
        state.handle_key(key(KeyCode::Backspace));
        state.handle_key(key(KeyCode::Char(' ')));
        state.handle_key(key(KeyCode::Char('g')));
        state.handle_key(key(KeyCode::Char('1')));
        state.handle_key(key(KeyCode::Char('3')));
        state.handle_key(key(KeyCode::Char('4')));
        state.handle_key(key(KeyCode::Tab));
        state.handle_key(key(KeyCode::BackTab));
        assert_eq!(state.handle_key(key(KeyCode::Char('q'))), UiAction::Quit);
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL,)),
            UiAction::Quit
        );
    }

    #[test]
    fn live_resize_reflows_without_losing_route_focus_query_or_overlay() {
        let mut state = WorkspaceState {
            screen: Screen::Measures,
            search: "qual".into(),
            measure_cursor: 1,
            help: true,
            ..WorkspaceState::default()
        };
        assert!(state.apply_resize(120, 40));
        assert_eq!(
            state.responsive_layout().class,
            crate::terminal::LayoutClass::Wide
        );
        assert_eq!(state.screen, Screen::Measures);
        assert_eq!(state.search, "qual");
        assert_eq!(state.measure_cursor, 1);
        assert!(state.help);

        assert!(state.apply_resize(30, 7));
        assert_eq!(
            state.responsive_layout().class,
            crate::terminal::LayoutClass::Compact
        );
        assert_eq!(state.screen, Screen::Measures);
        assert_eq!(state.search, "qual");
        assert!(state.help);
    }

    #[test]
    fn invalid_resize_is_atomic_and_resize_clamps_each_focusable_route() {
        let mut state = WorkspaceState {
            screen: Screen::Configuration,
            config_cursor: usize::MAX,
            report_cursor: usize::MAX,
            ..WorkspaceState::default()
        };
        let before = state.clone();
        assert!(!state.apply_resize(0, 24));
        assert_eq!(state, before);

        assert!(state.apply_resize(80, 24));
        assert_eq!(state.config_cursor, 3);
        state.screen = Screen::Reports;
        assert_eq!(state.report_cursor, 1);
    }

    #[test]
    fn test_backend_reflow_survives_resize_across_routes_and_overlay() {
        let mut state = WorkspaceState::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        for screen in [
            Screen::Landing,
            Screen::Measures,
            Screen::Configuration,
            Screen::Reports,
        ] {
            state.screen = screen;
            state.help = false;
            terminal
                .draw(|frame| render(frame, &state, policy()))
                .unwrap();
            terminal.backend_mut().resize(32, 8);
            assert!(state.apply_resize(32, 8));
            terminal
                .draw(|frame| render(frame, &state, policy()))
                .unwrap();
            state.help = true;
            terminal
                .draw(|frame| render(frame, &state, policy()))
                .unwrap();
            state.help = false;
            terminal.backend_mut().resize(100, 24);
            assert!(state.apply_resize(100, 24));
            terminal
                .draw(|frame| render(frame, &state, policy()))
                .unwrap();
        }
    }

    #[test]
    fn wizard_selection_is_credential_free_and_closed() {
        let values = [
            "codex,goose".into(),
            "openai".into(),
            "gpt-5.2".into(),
            "shared".into(),
            format!("credential_reference:{}", "a".repeat(64)),
            "record".into(),
            "offline".into(),
        ];
        let selection = WorkspaceState::wizard_configuration_selection(&values).unwrap();
        assert_eq!(selection.agent_ids, ["codex", "goose"]);
        assert_eq!(
            selection.auth_method,
            ProviderAuthMethod::CredentialReference
        );
        assert_eq!(
            selection.credential_reference_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );

        let mut invalid = values;
        invalid[4] = "sk-live-secret".into();
        assert!(WorkspaceState::wizard_configuration_selection(&invalid).is_err());
    }

    #[test]
    fn wizard_selection_rejects_missing_identity_and_supports_each_nonsecret_auth_mode() {
        let mut values = [
            "".into(),
            "provider".into(),
            "model".into(),
            "config".into(),
            "none".into(),
            "record".into(),
            "replay".into(),
        ];
        assert!(WorkspaceState::wizard_configuration_selection(&values).is_err());
        values[0] = "agent".into();
        values[1].clear();
        assert!(WorkspaceState::wizard_configuration_selection(&values).is_err());
        values[1] = "provider".into();
        values[2].clear();
        assert!(WorkspaceState::wizard_configuration_selection(&values).is_err());
        values[2] = "model".into();
        values[4] = "local_daemon".into();
        let selection = WorkspaceState::wizard_configuration_selection(&values).unwrap();
        assert_eq!(selection.auth_method, ProviderAuthMethod::LocalDaemon);
        assert!(selection.credential_reference_sha256.is_none());
        values[4] = "none".into();
        let selection = WorkspaceState::wizard_configuration_selection(&values).unwrap();
        assert_eq!(selection.auth_method, ProviderAuthMethod::None);
    }

    #[test]
    fn wizard_completion_is_consumed_once() {
        let mut state = WorkspaceState {
            wizard_completion: Some(Default::default()),
            ..WorkspaceState::default()
        };
        assert!(state.take_wizard_completion().is_some());
        assert!(state.take_wizard_completion().is_none());
    }
}
