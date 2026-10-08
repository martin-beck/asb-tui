// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! The interactive workspace: keyboard-first navigation and Ratatui projection.
//!
//! This module owns only presentation state. Benchmark execution and credentials remain in
//! the external ASB control plane.

use crate::{
    agent_catalog::AgentCatalog,
    control_codec::{
        ConfigurationSelection, MeasurementCatalog, ProviderAuthMethod, ProviderCatalog,
    },
    help::document_help_text,
    live_projection::{Connection, LiveSnapshot},
    selection::{
        BenchmarkCatalog, BenchmarkDefinition, BenchmarkGroup, BenchmarkMeasure, BenchmarkPool,
        BenchmarkSelection, MAX_QUERY_BYTES, Measurement, MeasurementSelection, NodeSelection,
    },
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
    DevelopmentHandoff,
    Wizard,
    Measures,
    Configuration,
    RunControl,
    Reports,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    None,
    Quit,
    Resize(u16, u16),
    Control(crate::actions::UiAction),
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
    pub development_handoff: crate::development_handoff::HandoffProjection,
    pub help: bool,
    pub search: String,
    pub measure_cursor: usize,
    pub measures: Vec<MeasureRow>,
    /// Generation-bound nested benchmark catalog used by the production
    /// picker; the flat rows are a rendering projection only.
    benchmark_selection: Option<BenchmarkSelection>,
    /// Raw control revision associated with the benchmark catalog projection.
    ///
    /// The renderer-neutral picker requires a non-zero local generation, so a
    /// provisional control revision zero is normalized in the selection. Keep
    /// the wire revision separately for stale-catalog checks; otherwise zero
    /// and the first durable revision one would alias.
    benchmark_control_revision: Option<crate::control_codec::Revision>,
    pub config_cursor: usize,
    /// Secret-free materialization preflight shown before configuration apply.
    pub preflight: Option<crate::configuration_materialization::PreflightSummary>,
    preflight_bundle: Option<crate::configuration_materialization::MaterializedBundle>,
    preflight_error: Option<String>,
    reviewed_provider_setup: Option<crate::provider_setup::ProviderSetupDraft>,
    /// Generation-bound launch state; populated only from a reviewed
    /// materialization and retained across reconnect/cancellation.
    pub(crate) launch_state: Option<crate::launch_statistics::LaunchState>,
    /// Last bounded launch/control error shown on the run-control screen.
    /// Errors remain in the UI so a provider failure does not terminate the
    /// frontend or look like a successful local/mock run.
    pub(crate) run_error: Option<String>,
    pub report_cursor: usize,
    /// Explicit report rows selected for comparison.
    pub report_selection: Vec<crate::control_codec::RunId>,
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
    wizard_adapter_completion: Option<String>,
    /// Single-use apply gate for a completed provider setup review. Opening a
    /// new wizard restarts this gate; a failed apply cannot be replayed.
    pub(crate) provider_setup_apply: crate::provider_setup::AtomicProviderSetup,
    development_catalog_fallback: bool,
    authoritative_provider_catalog_seen: bool,
    /// Last validated dimensions received from the terminal event stream.
    /// Rendering still uses the frame's authoritative area, so a missed
    /// event cannot make the renderer allocate from stale dimensions.
    layout: ResponsiveLayout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeasureRow {
    pub id: String,
    /// Stable catalog identifiers used for actions; labels below are display-only.
    pub group_id: String,
    pub benchmark_id: String,
    pub group: String,
    pub benchmark: String,
    pub name: String,
    pub selected: bool,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        let benchmark_selection = default_benchmark_selection();
        Self {
            screen: Screen::Landing,
            development_handoff: Default::default(),
            // Live setup is stable by default; development fixtures are
            // installed only by ensure_development_catalog after an explicit
            // development readiness decision.
            wizard: Wizard::stable(),
            wizard_formal: WizardFormalState::new().expect("authored wizard model must be valid"),
            wizard_completion: None,
            wizard_adapter_completion: None,
            provider_setup_apply: Default::default(),
            development_catalog_fallback: false,
            authoritative_provider_catalog_seen: false,
            help: false,
            search: String::new(),
            measure_cursor: 0,
            measures: vec![
                MeasureRow {
                    id: "quality.correctness".into(),
                    group_id: "quality".into(),
                    benchmark_id: "quality".into(),
                    group: "Quality".into(),
                    benchmark: "Quality".into(),
                    name: "Correctness".into(),
                    selected: true,
                },
                MeasureRow {
                    id: "quality.consistency".into(),
                    group_id: "quality".into(),
                    benchmark_id: "quality".into(),
                    group: "Quality".into(),
                    benchmark: "Quality".into(),
                    name: "Consistency".into(),
                    selected: true,
                },
                MeasureRow {
                    id: "efficiency.latency".into(),
                    group_id: "efficiency".into(),
                    benchmark_id: "efficiency".into(),
                    group: "Efficiency".into(),
                    benchmark: "Efficiency".into(),
                    name: "Latency".into(),
                    selected: true,
                },
                MeasureRow {
                    id: "efficiency.token_usage".into(),
                    group_id: "efficiency".into(),
                    benchmark_id: "efficiency".into(),
                    group: "Efficiency".into(),
                    benchmark: "Efficiency".into(),
                    name: "Token usage".into(),
                    selected: false,
                },
                MeasureRow {
                    id: "safety.policy_adherence".into(),
                    group_id: "safety".into(),
                    benchmark_id: "safety".into(),
                    group: "Safety".into(),
                    benchmark: "Safety".into(),
                    name: "Policy adherence".into(),
                    selected: false,
                },
            ],
            config_cursor: 0,
            preflight: None,
            preflight_bundle: None,
            preflight_error: None,
            reviewed_provider_setup: None,
            launch_state: None,
            run_error: None,
            report_cursor: 0,
            report_selection: Vec::new(),
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
            benchmark_selection: Some(benchmark_selection),
            benchmark_control_revision: None,
            layout: ResponsiveLayout::from_dimensions(None, None),
        }
    }
}

impl WorkspaceState {
    /// Restore the private workspace configuration advertised by the
    /// lifecycle handoff. Missing handoff state is normal for an uninstalled
    /// development checkout and therefore returns the ordinary default.
    pub fn from_persisted_environment() -> Self {
        let Some(root) = std::env::var_os("ASB_TUI_WORKSPACE_CONFIG_ROOT") else {
            return Self::default();
        };
        let path = std::path::PathBuf::from(root).join("config.json");
        Self::with_configuration_store(path).unwrap_or_default()
    }

    /// Return the exact bounded fan-out selection reviewed by the operator.
    /// Agent identities come from the authenticated configuration snapshot;
    /// workload identities come from the generation-bound benchmark picker.
    /// The runtime must never infer either set from display rows.
    pub(crate) fn fanout_selection(
        &self,
    ) -> Result<crate::fanout_dispatch::FanoutSelection, String> {
        let live = self
            .live
            .as_ref()
            .ok_or_else(|| "fan-out requires an applied configuration".to_owned())?;
        let configuration = live
            .configuration
            .as_ref()
            .ok_or_else(|| "fan-out requires an applied configuration".to_owned())?;
        let agents = configuration.agent_ids.clone();
        let provider_id = configuration
            .provider_id
            .clone()
            .ok_or_else(|| "fan-out requires a selected provider".to_owned())?;
        let model_id = configuration
            .model_id
            .clone()
            .ok_or_else(|| "fan-out requires a selected model".to_owned())?;
        if self.benchmark_selection.is_none() {
            return Err("fan-out requires a benchmark catalog".to_owned());
        }
        let campaign = self
            .current_benchmark_campaign_handoff()
            .map_err(|error| format!("fan-out workload selection invalid: {error:?}"))?;
        let catalog_digest = live
            .provider_catalog
            .as_ref()
            .map(|catalog| catalog.catalog_sha256.clone())
            .unwrap_or_else(|| "0".repeat(64));
        Ok(crate::fanout_dispatch::FanoutSelection {
            agent_ids: agents,
            workload_ids: campaign.benchmark_ids,
            provider_id,
            model_id,
            catalog_digest,
            workload_revision: campaign.catalog_digest.clone(),
            scorer_revision: campaign.catalog_digest,
        })
    }

    pub(crate) fn fanout_execution_mode(&self) -> crate::fanout_dispatch::FanoutExecutionMode {
        match self.execution_mode_for_values(&self.wizard.values()) {
            crate::configuration_materialization::ExecutionMode::Live => {
                crate::fanout_dispatch::FanoutExecutionMode::Live
            }
            crate::configuration_materialization::ExecutionMode::LocalMock => {
                crate::fanout_dispatch::FanoutExecutionMode::LocalMock
            }
        }
    }

    pub(crate) fn fanout_credential_reference(&self) -> Option<&str> {
        self.live
            .as_ref()
            .and_then(|snapshot| snapshot.configuration.as_ref())
            .and_then(|configuration| configuration.credential_reference_sha256.as_deref())
    }

    /// Return the report run currently under the cursor and the bounded pair
    /// used by the explicit live/offline comparison action.
    pub fn operator_run_selection(
        &self,
    ) -> (
        Option<crate::control_codec::RunId>,
        Vec<crate::control_codec::RunId>,
    ) {
        let Some(snapshot) = self.live.as_ref() else {
            return (None, Vec::new());
        };
        let selected = snapshot
            .runs
            .get(self.report_cursor)
            .map(|run| run.run_id.clone());
        let comparison = snapshot
            .runs
            .iter()
            .filter(|run| self.report_selection.contains(&run.run_id))
            .map(|run| run.run_id.clone())
            .collect();
        (selected, comparison)
    }

    /// Return the explicit workload scope selected on the benchmark screen.
    ///
    /// `All` is available only through the explicit all-workloads toggle.
    /// Empty or partially selected scopes remain distinct and fail closed.
    pub(crate) fn recording_workload_scope(
        &self,
    ) -> Result<crate::recording_campaign::WorkloadScope, String> {
        let Some(selection) = self.benchmark_selection.as_ref() else {
            return Err("workload selection is unavailable".to_owned());
        };
        if selection.explicit_all() {
            return Ok(crate::recording_campaign::WorkloadScope::All);
        }
        let selected = selection.selected_measure_ids();
        if selected.is_empty() {
            return Err("workload selection is empty; choose measures or explicit all".to_owned());
        }
        let handoff = selection
            .campaign_handoff(selection.catalog().generation())
            .map_err(|error| format!("workload selection is no longer valid: {error:?}"))?;
        Ok(crate::recording_campaign::WorkloadScope::Selected(
            handoff.benchmark_ids,
        ))
    }

    /// Keep run-control key handling coupled to the executable state model.
    /// The runtime owns the durable launch state; this bounded check ensures a
    /// UI action is only emitted when its documented route transition exists.
    fn run_control_formal(&self, event: crate::formal_state::FormalEvent) -> bool {
        let Ok(mut formal) = crate::formal_state::FormalUiState::new(100, 30) else {
            return false;
        };
        formal
            .apply(
                crate::formal_state::FormalEvent::OpenMeasurementSelection,
                None,
            )
            .and_then(|_| formal.apply(crate::formal_state::FormalEvent::OpenRunControl, None))
            .and_then(|_| formal.apply(event, None))
            .is_ok()
    }

    /// Load a bounded local configuration store for the configuration screen.
    /// This performs no ASB probing or backend acknowledgement.
    pub fn with_configuration_store(
        path: impl Into<PathBuf>,
    ) -> Result<Self, crate::configuration::ConfigError> {
        let path = path.into();
        let config = crate::configuration::ConfigurationStore::new(&path).load_or_default()?;
        let mut state = Self {
            configuration_draft: crate::configuration::ConfigurationDraft::new(config)
                .expect("store configuration was validated on load"),
            configuration_path: Some(path),
            configuration_save_state: ConfigurationSaveState::Clean,
            ..Self::default()
        };
        let materialized_root = state
            .configuration_path
            .as_ref()
            .and_then(|value| value.parent())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("materialized");
        if let Some(bundle) =
            crate::configuration_materialization::MaterializedBundleStore::new(materialized_root)
                .load()
                .map_err(|error| {
                    crate::configuration::ConfigError::Invalid(format!(
                        "materialized configuration could not be restored: {error:?}"
                    ))
                })?
        {
            state
                .restore_materialized_bundle(&bundle)
                .map_err(crate::configuration::ConfigError::Invalid)?;
        }
        Ok(state)
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
            self.development_catalog_fallback = false;
        }
        self
    }

    /// Take a completed wizard draft. Completion is consumed exactly once so a
    /// retry cannot accidentally replay a configuration mutation.
    pub fn take_wizard_completion(&mut self) -> Option<[String; 7]> {
        self.wizard_completion.take()
    }

    pub fn take_wizard_adapter_completion(&mut self) -> Option<String> {
        self.wizard_adapter_completion.take()
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

            value
                if value
                    .strip_prefix("credential_helper:")
                    .is_some_and(|payload| {
                        let parts: Vec<_> = payload.split(':').collect();
                        parts.len() == 2
                            && parts.iter().all(|digest| {
                                digest.len() == 64
                                    && digest
                                        .chars()
                                        .all(|character| character.is_ascii_hexdigit())
                            })
                    }) =>
            {
                let locator = value
                    .strip_prefix("credential_helper:")
                    .and_then(|payload| payload.split(':').nth(1))
                    .expect("validated helper receipt");
                (
                    ProviderAuthMethod::CredentialReference,
                    Some(locator.to_owned()),
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

    /// Convert the wizard's recording choice to an explicit provider
    /// execution mode.  The setup flow is warning-only and accepts the live
    /// choice even when authentication is unavailable; the runner performs
    /// the credential check only when the operator starts the live run.
    pub fn wizard_execution_mode(
        values: &[String; 7],
    ) -> crate::configuration_materialization::ExecutionMode {
        match values[5].trim().to_ascii_lowercase().as_str() {
            "live" | "online" | "live-record" | "live_record" => {
                crate::configuration_materialization::ExecutionMode::Live
            }
            _ => crate::configuration_materialization::ExecutionMode::LocalMock,
        }
    }

    fn execution_mode_for_values(
        &self,
        values: &[String; 7],
    ) -> crate::configuration_materialization::ExecutionMode {
        if values[5].trim().is_empty() {
            self.preflight_bundle
                .as_ref()
                .map(|bundle| bundle.document.provider.execution_mode)
                .unwrap_or_default()
        } else {
            Self::wizard_execution_mode(values)
        }
    }

    /// Convert a completed wizard into the catalog-bound typed setup draft.
    /// Callers should use this seam when a live or development catalog is
    /// available; the legacy selection helper above remains for protocol
    /// compatibility with older fixture paths.
    pub fn wizard_provider_setup_draft(
        values: &[String; 7],
        agents: &AgentCatalog,
        providers: &ProviderCatalog,
    ) -> Result<crate::provider_setup::ProviderSetupDraft, crate::provider_setup::ProviderSetupError>
    {
        crate::provider_setup::ProviderSetupDraft::from_wizard_values(values, agents, providers)
    }

    /// Decode the optional digest-only helper receipt embedded in the wizard
    /// auth field. Raw credentials cannot match this syntax.
    pub fn wizard_credential_helper_receipt(
        values: &[String; 7],
    ) -> Result<Option<crate::credential_helper::CredentialEnrollmentReceipt>, &'static str> {
        let value = values[4].trim();
        let Some(payload) = value.strip_prefix("credential_helper:") else {
            return Ok(None);
        };
        let Some((endpoint, locator)) = payload.split_once(':') else {
            return Err("credential helper receipt must contain endpoint and locator digests");
        };
        let json = serde_json::json!({
            "provider": values[1].trim(),
            "endpoint_identity_sha256": endpoint,
            "credential_locator_sha256": locator,
        });
        crate::credential_helper::CredentialEnrollmentReceipt::decode(json.to_string().as_bytes())
            .map(Some)
            .map_err(|_| "credential helper receipt is invalid")
    }

    /// The bounded text currently visible in the focused configuration editor.
    pub fn configuration_edit_value(&self) -> Option<&str> {
        self.configuration_editing
            .then_some(self.configuration_edit_buffer.as_str())
    }

    /// Replace the visible preflight projection after a validated bundle has
    /// been built. The projection contains identifiers and counts only.
    pub fn set_preflight(
        &mut self,
        summary: crate::configuration_materialization::PreflightSummary,
    ) {
        self.preflight = Some(summary);
    }

    pub fn clear_preflight(&mut self) {
        self.preflight = None;
        self.preflight_bundle = None;
        self.preflight_error = None;
        self.launch_state = None;
    }

    /// Restore the exact durable provider/model/mode draft and reviewed
    /// preflight. The materialized bundle is already digest-validated by the
    /// store, but the integrity check is repeated at this public seam so a
    /// caller cannot inject an unreviewed bundle.
    pub fn restore_materialized_bundle(
        &mut self,
        bundle: &crate::configuration_materialization::MaterializedBundle,
    ) -> Result<(), String> {
        bundle
            .validate_integrity()
            .map_err(|error| format!("materialized configuration invalid: {error:?}"))?;
        let provider = &bundle.document.provider;
        let auth = match (
            provider.auth_method,
            provider.credential_reference_sha256.as_deref(),
        ) {
            (crate::control_codec::ProviderAuthMethod::None, None) => "none".into(),
            (crate::control_codec::ProviderAuthMethod::LocalDaemon, None) => "local_daemon".into(),
            (crate::control_codec::ProviderAuthMethod::CredentialReference, Some(digest)) => {
                format!("credential_reference:{digest}")
            }
            _ => return Err("materialized authentication reference is inconsistent".into()),
        };
        let values = [
            provider.agent_ids.join(","),
            provider.provider_id.clone(),
            provider.model_id.clone(),
            "restored defaults".into(),
            auth,
            provider.execution_mode.label().into(),
            "strict offline replay".into(),
        ];
        self.wizard
            .restore_values(values)
            .map_err(|error| format!("materialized wizard draft invalid: {error:?}"))?;
        self.preflight = Some(bundle.preflight_summary());
        self.preflight_bundle = Some(bundle.clone());
        self.preflight_error = None;
        Ok(())
    }

    /// Supply the catalog-bound provider review produced by the authenticated
    /// setup seam. Development fallback is used only when this is absent.
    pub fn set_reviewed_provider_setup(
        &mut self,
        draft: crate::provider_setup::ProviderSetupDraft,
    ) {
        self.reviewed_provider_setup = Some(draft);
        self.clear_preflight();
    }

    /// Expose the validated bundle to the authenticated launch seam without
    /// exposing any credential material or allowing mutation of the bundle.
    pub(crate) fn preflight_bundle(
        &self,
    ) -> Option<&crate::configuration_materialization::MaterializedBundle> {
        self.preflight_bundle.as_ref()
    }

    pub(crate) fn launch_state_mut(
        &mut self,
    ) -> Option<&mut crate::launch_statistics::LaunchState> {
        self.launch_state.as_mut()
    }

    pub(crate) fn launch_state(&self) -> Option<&crate::launch_statistics::LaunchState> {
        self.launch_state.as_ref()
    }

    pub(crate) fn install_launch_state(&mut self) -> Result<(), String> {
        let bundle = self
            .preflight_bundle
            .as_ref()
            .ok_or_else(|| "prepare configuration preflight before launching".to_owned())?;
        self.launch_state = Some(crate::launch_statistics::LaunchState::new(
            bundle.launch_binding(),
        ));
        self.run_error = None;
        Ok(())
    }

    /// Preserve a typed, bounded control failure for the run-control view.
    /// The runtime owns the transport error; the workspace owns only its
    /// privacy-safe display projection.
    pub(crate) fn set_run_error(&mut self, error: impl Into<String>) {
        let mut value = error.into();
        value.truncate(512);
        self.run_error = Some(value);
    }

    pub(crate) fn clear_run_error(&mut self) {
        self.run_error = None;
    }

    /// Build a fresh digest-bound bundle from the current reviewed wizard and
    /// nested benchmark selections. No file is changed by this operation.
    pub fn prepare_preflight(&mut self) -> Result<(), String> {
        let values = self.wizard.values();
        let selection = Self::wizard_configuration_selection(&values)?;
        let draft = if let Some(draft) = self.reviewed_provider_setup.clone() {
            draft
        } else {
            if self.authoritative_provider_catalog_seen {
                return Err(
                    "review the current provider catalog before preparing preflight".into(),
                );
            }
            crate::provider_setup::ProviderSetupDraft::from_selection_for_development(
                selection.clone(),
            )
            .map_err(|error| format!("provider review invalid: {error:?}"))?
        };
        if draft.selection() != &selection {
            return Err("provider review no longer matches wizard selection".into());
        }
        let benchmark_selection = self
            .benchmark_selection
            .as_ref()
            .ok_or_else(|| "benchmark catalog unavailable".to_owned())?;
        let campaign = self
            .current_benchmark_campaign_handoff()
            .map_err(|error| format!("benchmark selection invalid: {error:?}"))?;
        let input = crate::configuration_materialization::MaterializationInput {
            provider: selection,
            provider_catalog_generation: draft.catalog_generation(),
            provider_catalog_digest: draft.catalog_digest().to_owned(),
            benchmark: campaign,
            asb_protocol: "asb-control".into(),
            asb_version: "development".into(),
            execution_mode: self.execution_mode_for_values(&values),
        };
        let bundle = crate::configuration_materialization::MaterializedBundle::build(
            input,
            &draft,
            benchmark_selection.catalog(),
            draft.catalog_generation(),
            benchmark_selection.catalog().generation(),
        )
        .map_err(|error| format!("preflight rejected: {error:?}"))?;
        self.preflight = Some(bundle.preflight_summary());
        self.preflight_bundle = Some(bundle);
        self.preflight_error = None;
        Ok(())
    }

    /// Apply the pending preflight bundle to the private materialization store
    /// beside the configured frontend file.
    pub fn apply_preflight(&mut self) -> Result<(), String> {
        let mut formal = crate::formal_state::FormalUiState::new(80, 24)
            .map_err(|error| format!("formal preflight model unavailable: {error:?}"))?;
        formal
            .apply(crate::formal_state::FormalEvent::OpenConfiguration, None)
            .and_then(|_| formal.apply(crate::formal_state::FormalEvent::ApplyPreflight, None))
            .map_err(|error| format!("formal preflight transition unavailable: {error:?}"))?;
        let bundle = self
            .preflight_bundle
            .as_ref()
            .ok_or_else(|| "prepare preflight before applying".to_owned())?;
        let path = self
            .configuration_path
            .as_ref()
            .ok_or_else(|| "no local configuration store is configured".to_owned())?;
        let root = path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("materialized");
        let store = crate::configuration_materialization::MaterializedBundleStore::new(root);
        store
            .apply(bundle)
            .map_err(|error| format!("preflight apply failed: {error:?}"))?;
        Ok(())
    }

    /// Apply a validated materialized bundle through its atomic store and only
    /// then expose the secret-free preflight projection to the renderer.
    pub fn apply_materialized_bundle(
        &mut self,
        bundle: &crate::configuration_materialization::MaterializedBundle,
        store: &crate::configuration_materialization::MaterializedBundleStore,
    ) -> Result<(), crate::configuration_materialization::MaterializationError> {
        store.apply(bundle)?;
        self.preflight = Some(bundle.preflight_summary());
        self.preflight_bundle = Some(bundle.clone());
        Ok(())
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
            state.ensure_development_catalog();
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
        self.provider_setup_apply.restart();
        if self.wizard_formal.apply(FormalEvent::OpenWizard).is_ok() {
            self.screen = Screen::Wizard;
        }
    }

    fn ensure_development_catalog(&mut self) {
        if self.wizard.catalog().is_none()
            && let Ok(catalog) = crate::wizard_catalog::WizardCatalog::development()
            && let Ok(formal) = WizardFormalState::new_with_catalog(catalog)
        {
            self.wizard_formal = formal;
            self.wizard = self.wizard_formal.wizard().clone();
            self.development_catalog_fallback = true;
        }
    }

    /// Select the explicit development-only wizard context for the runtime
    /// development channel when no authoritative provider catalog is present.
    pub fn use_development_context(&mut self) {
        if !self.authoritative_provider_catalog_seen {
            self.ensure_development_catalog();
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

    /// Return the exact nested benchmark handoff currently reviewed by the
    /// user. The generation check prevents a refreshed catalog from changing
    /// a campaign between review and dispatch.
    pub fn benchmark_campaign_handoff(
        &self,
        expected_control_revision: crate::control_codec::Revision,
    ) -> Result<crate::selection::CampaignSelection, crate::selection::SelectionError> {
        if let Some(actual) = self.benchmark_control_revision
            && actual != expected_control_revision
        {
            return Err(crate::selection::SelectionError::StaleCatalog);
        }
        let selection = self
            .benchmark_selection
            .as_ref()
            .ok_or(crate::selection::SelectionError::NoSelection)?;
        // BenchmarkSelection carries a non-zero renderer-local generation;
        // the raw control revision above is the authoritative stale fence.
        selection.campaign_handoff(selection.catalog().generation())
    }

    fn current_benchmark_campaign_handoff(
        &self,
    ) -> Result<crate::selection::CampaignSelection, crate::selection::SelectionError> {
        let expected = self
            .benchmark_control_revision
            .or_else(|| {
                self.benchmark_selection
                    .as_ref()
                    .map(|selection| selection.catalog().generation())
            })
            .unwrap_or(crate::control_codec::Revision(1));
        self.benchmark_campaign_handoff(expected)
    }

    /// Replace presentation data only after it has passed the typed control
    /// projection. No renderer input can mutate ASB state through this method.
    pub fn apply_live_snapshot(&mut self, snapshot: LiveSnapshot) {
        self.authoritative_provider_catalog_seen =
            snapshot.agent_catalog.is_some() || snapshot.provider_catalog.is_some();
        if let Some(catalog) = snapshot.measurement_catalog.as_ref()
            && let Some(selection) = selection_from_catalog(catalog, self.selection.as_ref())
        {
            self.selection = Some(selection);
            self.benchmark_selection = nested_selection_from_catalog(
                catalog,
                self.benchmark_selection.as_ref(),
                snapshot
                    .latest_revision
                    .unwrap_or(crate::control_codec::Revision(1)),
            );
            self.benchmark_control_revision = snapshot.latest_revision;
            self.sync_measure_projection();
        }
        // Populate a first-run wizard from the authenticated control-plane
        // catalog so users select supported identifiers instead of typing
        // opaque values. Do not replace an in-progress draft during refresh.
        if (self.wizard.catalog().is_none() || self.development_catalog_fallback)
            && let Some(catalog) = wizard_catalog_from_snapshot(&snapshot)
        {
            let adapter_catalog = snapshot.provider_catalog.as_ref().and_then(|provider| {
                crate::adapter_catalog::AdapterCatalog::from_provider_catalog(provider).ok()
            });
            self.wizard_formal = adapter_catalog
                .map_or_else(
                    || WizardFormalState::new_with_catalog(catalog.clone()),
                    |adapter| {
                        WizardFormalState::new_with_catalog_and_adapter(catalog.clone(), adapter)
                    },
                )
                .expect("validated live wizard catalog must satisfy the state model");
            self.wizard = self.wizard_formal.wizard().clone();
            if let Some(bundle) = self.preflight_bundle.clone() {
                // A catalog refresh replaces the renderer catalog, but never
                // the durable provider/model/mode draft selected previously.
                let _ = self.restore_materialized_bundle(&bundle);
            }
            self.development_catalog_fallback = false;
        }
        if self.authoritative_provider_catalog_seen
            && let (Some(agents), Some(providers)) = (
                snapshot.agent_catalog.as_ref(),
                snapshot.provider_catalog.as_ref(),
            )
        {
            match Self::wizard_provider_setup_draft(&self.wizard.values(), agents, providers) {
                Ok(draft) => self.set_reviewed_provider_setup(draft),
                Err(_) => self.reviewed_provider_setup = None,
            }
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
            self.ensure_development_catalog();
            self.open_wizard();
        }
        self.report_selection = self
            .report_selection
            .iter()
            .filter(|run_id| snapshot.runs.iter().any(|run| &run.run_id == *run_id))
            .cloned()
            .collect();
        self.live = Some(snapshot);
        self.report_cursor = 0;
        self.clamp_measure_cursor();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> UiAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.screen == Screen::RunControl {
                if self.run_control_formal(crate::formal_state::FormalEvent::CancelRun) {
                    return UiAction::Control(crate::actions::UiAction::CancelRun);
                }
                return UiAction::None;
            }
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
                KeyCode::Char('E') if self.wizard.step() == wizard::Step::Authentication => {
                    let _ = self.wizard_formal.apply(FormalEvent::DevelopmentEnroll);
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Char('T') if self.wizard.step() == wizard::Step::Authentication => {
                    let _ = self.wizard_formal.apply(FormalEvent::DevelopmentTest);
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Char('R') if self.wizard.step() == wizard::Step::Authentication => {
                    let _ = self.wizard_formal.apply(FormalEvent::DevelopmentRotate);
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Char('X') if self.wizard.step() == wizard::Step::Authentication => {
                    let _ = self.wizard_formal.apply(FormalEvent::DevelopmentReset);
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Char('F') if self.wizard.step() == wizard::Step::Authentication => {
                    let _ = self
                        .wizard_formal
                        .apply(FormalEvent::DevelopmentSelectFixture);
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Char('N') if self.wizard.step() == wizard::Step::Authentication => {
                    let _ = self.wizard_formal.apply(FormalEvent::DevelopmentSelectNone);
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
                KeyCode::Char('Z') if self.wizard.step() == wizard::Step::Authentication => {
                    let _ = self.wizard_formal.apply(FormalEvent::DevelopmentRestart);
                    self.wizard = self.wizard_formal.wizard().clone();
                    UiAction::None
                }
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
                            self.wizard_adapter_completion =
                                self.wizard.selected_adapter_id().map(str::to_owned);
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
        if self.screen == Screen::DevelopmentHandoff {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('1') => {
                    self.screen = Screen::Landing;
                    UiAction::None
                }
                KeyCode::Char(' ') | KeyCode::Enter
                    if self.development_handoff.phase
                        == crate::development_handoff::Phase::Ready =>
                {
                    self.development_handoff.begin();
                    UiAction::Control(crate::actions::UiAction::MaterializeDevelopment)
                }
                KeyCode::Char('x') => {
                    self.development_handoff.cancel();
                    UiAction::Control(crate::actions::UiAction::CancelDevelopment)
                }
                KeyCode::Char('r') => {
                    self.development_handoff.begin_retry();
                    UiAction::Control(crate::actions::UiAction::RetryDevelopment)
                }
                KeyCode::Char('?') | KeyCode::Char('h') => {
                    self.help = true;
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
        if self.screen == Screen::Configuration && key.code == KeyCode::Char('V') {
            if let Ok(mut formal) = crate::formal_state::FormalUiState::new(80, 24) {
                let _ = formal.apply(crate::formal_state::FormalEvent::OpenConfiguration, None);
                let _ = formal.apply(crate::formal_state::FormalEvent::OpenPreflight, None);
            }
            if let Err(error) = self.prepare_preflight() {
                self.preflight_error = Some(error);
            }
            return UiAction::Control(crate::actions::UiAction::OpenPreflight);
        }
        if self.screen == Screen::Configuration && key.code == KeyCode::Char('A') {
            if let Err(error) = self.apply_preflight() {
                self.preflight_error = Some(error);
            } else {
                self.preflight_error = None;
            }
            return UiAction::Control(crate::actions::UiAction::ApplyPreflight);
        }
        if self.screen == Screen::RunControl && key.code == KeyCode::Char('x') {
            if self.run_control_formal(crate::formal_state::FormalEvent::Reconnect) {
                if let Some(launch) = self.launch_state_mut() {
                    launch.reconnect_started();
                }
                return UiAction::Control(crate::actions::UiAction::Reconnect);
            }
            return UiAction::None;
        }
        if self.screen == Screen::RunControl && key.code == KeyCode::Char('A') {
            return UiAction::Control(crate::actions::UiAction::AdmitFanout);
        }
        if self.screen == Screen::RunControl
            && key.code == KeyCode::Char('z')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return UiAction::Control(crate::actions::UiAction::CancelFanout);
        }
        if self.screen == Screen::Reports && key.code == KeyCode::Char(' ') {
            if let Some(run_id) = self
                .live
                .as_ref()
                .and_then(|snapshot| snapshot.runs.get(self.report_cursor))
                .map(|run| run.run_id.clone())
            {
                if let Some(position) = self
                    .report_selection
                    .iter()
                    .position(|selected| *selected == run_id)
                {
                    self.report_selection.remove(position);
                } else if self.report_selection.len() < 2 {
                    self.report_selection.push(run_id);
                }
            }
            return UiAction::None;
        }
        if self.screen == Screen::Reports && key.code == KeyCode::Char('t') {
            return UiAction::Control(crate::actions::UiAction::RetryRun);
        }
        if self.screen == Screen::Reports && key.code == KeyCode::Char('v') {
            return UiAction::Control(crate::actions::UiAction::CompareLiveOffline);
        }
        if self.screen == Screen::RunControl && key.code == KeyCode::Char('S') {
            return UiAction::Control(crate::actions::UiAction::SealRecording);
        }
        if self.screen == Screen::RunControl && key.code == KeyCode::Char('U') {
            return UiAction::Control(crate::actions::UiAction::ReopenRecording);
        }
        if self.screen == Screen::RunControl && key.code == KeyCode::Char('D') {
            return UiAction::Control(crate::actions::UiAction::RemoveRecordingCassette);
        }
        match key.code {
            KeyCode::Char('d') if self.screen == Screen::Landing => {
                self.screen = Screen::DevelopmentHandoff;
                UiAction::None
            }
            KeyCode::Char('f') if self.screen == Screen::Configuration => {
                UiAction::Control(crate::actions::UiAction::RefreshProviderCatalog)
            }
            KeyCode::Char('e') if self.screen == Screen::Reports => {
                UiAction::Control(crate::actions::UiAction::EstimateRecording)
            }
            KeyCode::Char('P') if self.screen != Screen::Measures => {
                UiAction::Control(crate::actions::UiAction::PlanRecording)
            }
            KeyCode::Char('C') if self.screen == Screen::Reports => {
                UiAction::Control(crate::actions::UiAction::CompareLiveOffline)
            }
            KeyCode::Char('C') => {
                UiAction::Control(crate::actions::UiAction::ConfirmRecordingCapture)
            }
            KeyCode::Char('G') => UiAction::Control(crate::actions::UiAction::ProgressRecording),
            KeyCode::Char('X') => UiAction::Control(crate::actions::UiAction::CancelRecording),
            KeyCode::Char('Y') => UiAction::Control(crate::actions::UiAction::ReconcileRecording),
            KeyCode::Char('o') => {
                UiAction::Control(crate::actions::UiAction::ActivateOfflineDefault)
            }
            KeyCode::Char('J') if self.screen == Screen::RunControl => {
                UiAction::Control(crate::actions::UiAction::ReplaySelected)
            }
            KeyCode::Char(']') if self.screen == Screen::RunControl => {
                UiAction::Control(crate::actions::UiAction::SelectOfflineCassette)
            }
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
            KeyCode::Char('s') => {
                self.screen = Screen::RunControl;
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
            KeyCode::Enter if self.screen == Screen::RunControl => {
                if self.run_control_formal(crate::formal_state::FormalEvent::StartRun)
                    && self.install_launch_state().is_ok()
                    && self
                        .launch_state_mut()
                        .is_some_and(|launch| launch.begin_launch().is_ok())
                {
                    UiAction::Control(crate::actions::UiAction::StartRun)
                } else {
                    UiAction::None
                }
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
            KeyCode::Char('P') if self.screen == Screen::Measures => {
                self.select_next_benchmark_pool();
                UiAction::None
            }
            KeyCode::Char('b') if self.screen == Screen::Measures => {
                self.toggle_current_benchmark();
                UiAction::None
            }
            KeyCode::Char('g') if self.screen == Screen::Measures => {
                self.toggle_visible_group();
                UiAction::None
            }
            KeyCode::Char('a') if self.screen == Screen::Measures && self.search.is_empty() => {
                self.toggle_all_measures();
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
                    if let Some(selection) = self.benchmark_selection.as_mut() {
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
                if let Some(selection) = self.benchmark_selection.as_mut() {
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
        if let Some(selection) = self.benchmark_selection.as_ref() {
            let visible = selection.visible_measure_ids();
            return self
                .measures
                .iter()
                .enumerate()
                .filter(|(_, row)| visible.iter().any(|id| *id == row.id))
                .map(|(index, _)| index)
                .collect();
        }
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
                    group_id: measurement.group().to_ascii_lowercase(),
                    benchmark_id: measurement.group().to_ascii_lowercase(),
                    group: measurement.group().to_owned(),
                    benchmark: measurement.group().to_owned(),
                    name: measurement.name().to_owned(),
                    selected: selection.is_selected(measurement.id()),
                })
                .collect();
        }
        if let Some(selection) = self.benchmark_selection.as_ref() {
            for row in &mut self.measures {
                if let Some((group_id, group_name, benchmark_id, benchmark_name, measure_name)) =
                    nested_measure_metadata(selection, &row.id)
                {
                    row.group_id = group_id;
                    row.benchmark_id = benchmark_id;
                    row.group = group_name;
                    row.benchmark = benchmark_name;
                    row.name = measure_name;
                }
                row.selected = selection.is_measure_selected(&row.id);
            }
        }
        self.clamp_measure_cursor();
    }

    fn toggle_current_measure(&mut self) {
        let Some(index) = self.visible_indices().get(self.measure_cursor).copied() else {
            return;
        };
        if let Some(selection) = self.benchmark_selection.as_mut() {
            let id = self.measures[index].id.clone();
            let selected = !selection.is_measure_selected(&id);
            let _ = selection.set_measure_selected(&id, selected);
            self.sync_measure_projection();
        } else if let Some(selection) = self.selection.as_mut() {
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
        if let Some(selection) = self.benchmark_selection.as_mut() {
            let group = self.measures[current].group_id.clone();
            let select = selection.group_state(&group) != Ok(NodeSelection::All);
            let _ = selection.set_group_selected(&group, select);
            self.sync_measure_projection();
            return;
        }
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

    fn toggle_all_measures(&mut self) {
        if let Some(selection) = self.benchmark_selection.as_mut() {
            selection.set_explicit_all(!selection.explicit_all());
            self.sync_measure_projection();
        }
    }

    fn toggle_current_benchmark(&mut self) {
        let Some(index) = self.visible_indices().get(self.measure_cursor).copied() else {
            return;
        };
        let benchmark = self.measures[index].benchmark_id.clone();
        let Some(selection) = self.benchmark_selection.as_mut() else {
            return;
        };
        let select = selection.benchmark_state(&benchmark) != Ok(NodeSelection::All);
        let _ = selection.set_benchmark_selected(&benchmark, select);
        self.sync_measure_projection();
    }

    fn select_next_benchmark_pool(&mut self) {
        let Some(selection) = self.benchmark_selection.as_mut() else {
            return;
        };
        let pools = selection.catalog().pools();
        if pools.len() < 2 {
            return;
        }
        let current = pools
            .iter()
            .position(|pool| pool.id() == selection.pool_id())
            .unwrap_or(0);
        let next = pools[(current + 1) % pools.len()].id().to_owned();
        let _ = selection.select_pool(&next);
        self.sync_measure_projection();
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

fn nested_selection_from_catalog(
    catalog: &MeasurementCatalog,
    previous: Option<&BenchmarkSelection>,
    generation: crate::control_codec::Revision,
) -> Option<BenchmarkSelection> {
    let groups = catalog
        .groups
        .iter()
        .map(|group| {
            let group_id = measurement_group_id(group.id);
            let measures = catalog
                .measurements
                .iter()
                .filter(|definition| definition.group == group.id)
                .map(|definition| {
                    let available = matches!(
                        definition.live,
                        crate::control_codec::MeasurementModeSupport::Supported
                    );
                    BenchmarkMeasure::new(
                        definition.id.clone(),
                        definition.name.clone(),
                        definition.unit.clone(),
                        available,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let benchmark =
                BenchmarkDefinition::new(group_id.clone(), group.label.clone(), measures).ok()?;
            BenchmarkGroup::new(group_id, group.label.clone(), vec![benchmark]).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    let pool = BenchmarkPool::new("authoritative", "Authoritative benchmark pool", groups).ok()?;
    // A freshly negotiated control stream legitimately has revision zero
    // until its first durable event. The renderer-neutral selection model
    // uses a non-zero generation as its local catalog identity, so normalize
    // only at this UI/model boundary. Control revisions remain unchanged in
    // the live projection and transport.
    let catalog_generation = crate::control_codec::Revision(generation.0.max(1));
    let mut next = BenchmarkSelection::new(
        BenchmarkCatalog::new(
            catalog_generation,
            catalog.catalog_sha256.clone(),
            vec![pool],
        )
        .ok()?,
    )
    .ok()?;
    if let Some(previous) = previous {
        for id in previous.selected_measure_ids() {
            let _ = next.set_measure_selected(id, true);
        }
    }
    Some(next)
}

/// Resolve a rendered row through the nested catalog. Labels are deliberately
/// returned alongside canonical IDs because users may see labels that do not
/// resemble their machine identifiers (for example, "Quality Reliability" vs
/// `quality_reliability`).
fn nested_measure_metadata(
    selection: &BenchmarkSelection,
    measure_id: &str,
) -> Option<(String, String, String, String, String)> {
    selection.catalog().pools().iter().find_map(|pool| {
        pool.groups().iter().find_map(|group| {
            group.benchmarks().iter().find_map(|benchmark| {
                benchmark
                    .measures()
                    .iter()
                    .find(|measure| measure.id() == measure_id)
                    .map(|measure| {
                        (
                            group.id().to_owned(),
                            group.name().to_owned(),
                            benchmark.id().to_owned(),
                            benchmark.name().to_owned(),
                            measure.name().to_owned(),
                        )
                    })
            })
        })
    })
}

fn measurement_group_id(group: crate::control_codec::MeasurementGroupId) -> String {
    match group {
        crate::control_codec::MeasurementGroupId::SystemResources => "system_resources",
        crate::control_codec::MeasurementGroupId::SchedulingContention => "scheduling_contention",
        crate::control_codec::MeasurementGroupId::Latency => "latency",
        crate::control_codec::MeasurementGroupId::QualityReliability => "quality_reliability",
        crate::control_codec::MeasurementGroupId::Fairness => "fairness",
        crate::control_codec::MeasurementGroupId::Cost => "cost",
        crate::control_codec::MeasurementGroupId::Provenance => "provenance",
    }
    .into()
}

fn default_benchmark_selection() -> BenchmarkSelection {
    let measure = |id: &str, name: &str| {
        BenchmarkMeasure::new(id, name, "development", true)
            .expect("development benchmark measure is bounded")
    };
    let pool = BenchmarkPool::new(
        "development",
        "Development benchmark pool",
        vec![
            BenchmarkGroup::new(
                "quality",
                "Quality",
                vec![
                    BenchmarkDefinition::new(
                        "quality",
                        "Quality benchmark",
                        vec![
                            measure("quality.correctness", "Correctness"),
                            measure("quality.consistency", "Consistency"),
                        ],
                    )
                    .expect("development benchmark is bounded"),
                ],
            )
            .expect("development group is bounded"),
            BenchmarkGroup::new(
                "efficiency",
                "Efficiency",
                vec![
                    BenchmarkDefinition::new(
                        "efficiency",
                        "Efficiency benchmark",
                        vec![
                            measure("efficiency.latency", "Latency"),
                            measure("efficiency.token_usage", "Token usage"),
                        ],
                    )
                    .expect("development benchmark is bounded"),
                ],
            )
            .expect("development group is bounded"),
            BenchmarkGroup::new(
                "safety",
                "Safety",
                vec![
                    BenchmarkDefinition::new(
                        "safety",
                        "Safety benchmark",
                        vec![measure("safety.policy_adherence", "Policy adherence")],
                    )
                    .expect("development benchmark is bounded"),
                ],
            )
            .expect("development group is bounded"),
        ],
    )
    .expect("development pool is bounded");
    let catalog = BenchmarkCatalog::new(
        crate::control_codec::Revision(1),
        "development-benchmark-catalog-v1",
        vec![pool],
    )
    .expect("development catalog is bounded");
    let mut selection = BenchmarkSelection::new(catalog).expect("development selection is valid");
    for id in [
        "quality.correctness",
        "quality.consistency",
        "efficiency.latency",
    ] {
        selection
            .set_measure_selected(id, true)
            .expect("development measure is selectable");
    }
    selection
}

fn next_screen(screen: Screen) -> Screen {
    match screen {
        Screen::Landing => Screen::Measures,
        Screen::DevelopmentHandoff => Screen::Landing,
        Screen::Wizard => Screen::Measures,
        Screen::Measures => Screen::Configuration,
        Screen::Configuration => Screen::RunControl,
        Screen::RunControl => Screen::Reports,
        Screen::Reports | Screen::Help => Screen::Landing,
    }
}
fn previous_screen(screen: Screen) -> Screen {
    match screen {
        Screen::Landing => Screen::Reports,
        Screen::DevelopmentHandoff => Screen::Landing,
        Screen::Wizard => Screen::Landing,
        Screen::Measures => Screen::Landing,
        Screen::Configuration => Screen::Measures,
        Screen::RunControl => Screen::Configuration,
        Screen::Reports => Screen::RunControl,
        Screen::Help => Screen::Configuration,
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
    if state.screen == Screen::DevelopmentHandoff {
        render_development_handoff(frame, area, state, policy);
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
    let titles = ["1 Home", "2 Measures", "3 Configure", "s Run", "4 Reports"];
    let selected = match state.screen {
        Screen::Landing => 0,
        Screen::DevelopmentHandoff => 0,
        Screen::Wizard => 2,
        Screen::Measures => 1,
        Screen::Configuration => 2,
        Screen::RunControl => 3,
        Screen::Reports => 4,
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
        Screen::DevelopmentHandoff => unreachable!(),
        Screen::Wizard => unreachable!("wizard is rendered before workspace layout"),
        Screen::Measures => measures(frame, chunks[1], state, policy),
        Screen::Configuration => configuration(frame, chunks[1], state, policy),
        Screen::RunControl => run_control(frame, chunks[1], state, policy),
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
        Paragraph::new(
            "Press d for development handoff  |  2 measures  |  3 configure  |  4 reports",
        )
        .alignment(Alignment::Center)
        .style(muted(policy)),
        inner[2],
    );
}

fn render_development_handoff(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &WorkspaceState,
    policy: RenderPolicy,
) {
    let lines = state
        .development_handoff
        .human_lines()
        .into_iter()
        .map(Line::from)
        .chain([
            Line::from(""),
            Line::from("Enter/Space start | x cancel | r retry | Esc back"),
        ])
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(panel(" Development channel handoff ", policy)),
        area,
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
                Span::styled(
                    format!("{} / {} / {}", row.group, row.benchmark, row.name),
                    Style::default(),
                ),
            ]))
        })
        .collect();
    let mut ls = ListState::default();
    ls.select((state.measure_cursor < items.len()).then_some(state.measure_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(
                " Pool > Group > Benchmark > Measure - Space item, b benchmark, g group, P pool ",
                policy,
            ))
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
            Line::from("Up/Down navigate   Space measure"),
            Line::from("b benchmark   g group   P pool"),
            Line::from("Type to search nested catalog"),
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
    let detail = state
        .preflight_error
        .as_deref()
        .or(state.configuration_edit_error.as_deref())
        .unwrap_or("");
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
            if let Some(dynamic) = &snapshot.dynamic_provider_catalog {
                match dynamic {
                    crate::live_projection::DynamicProviderCatalogState::UnsupportedVersion => {
                        lines.push(Line::from(
                            "OpenRouter catalog: unavailable (control_version_unsupported)",
                        ));
                    }
                    crate::live_projection::DynamicProviderCatalogState::Catalog(dynamic) => {
                        let mode = match dynamic.openrouter.mode {
                            crate::control_codec::ProviderCatalogMode::Static => "static",
                            crate::control_codec::ProviderCatalogMode::Dynamic => "dynamic",
                            crate::control_codec::ProviderCatalogMode::Unavailable => "unavailable",
                        };
                        lines.push(Line::from(format!(
                            "OpenRouter catalog: {mode} ({} models)",
                            dynamic.openrouter.models.len()
                        )));
                        if let Some(diagnostic) = &dynamic.openrouter.diagnostic {
                            lines.push(Line::from(format!(
                                "Catalog diagnostic: {}",
                                diagnostic.code()
                            )));
                        }
                    }
                }
            }
            if let Some(preflight) = &state.preflight {
                lines.push(Line::from("Preflight: ready (review before apply)"));
                lines.push(Line::from(format!(
                    "  {} / {}  agents={} pool={} measures={}",
                    preflight.provider_id,
                    preflight.model_id,
                    preflight.agent_count,
                    preflight.pool_id,
                    preflight.measure_count
                )));
                lines.push(Line::from(format!(
                    "  execution: {}{}",
                    preflight.execution_mode.label(),
                    if matches!(
                        preflight.execution_mode,
                        crate::configuration_materialization::ExecutionMode::Live
                    ) {
                        " (provider network; key checked at run)"
                    } else {
                        " (credential-free)"
                    }
                )));
                lines.push(Line::from(format!(
                    "  digest={}{}",
                    &preflight.digest_sha256[..8.min(preflight.digest_sha256.len())],
                    if preflight.development_only {
                        " (development-only)"
                    } else {
                        ""
                    }
                )));
            } else {
                lines.push(Line::from("Preflight: not prepared (V prepare, A apply)"));
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

fn run_control(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let lines = if let Some(launch) = state.launch_state.as_ref() {
        let binding = launch.binding();
        let mut lines = vec![Line::from(format!(
            "State: {:?}  connection: {:?}",
            launch.state(),
            launch.connection()
        ))];
        lines.push(Line::from(format!(
            "Provider: {}  model: {}  mode: {}",
            binding.provider_id,
            binding.model_id,
            binding.execution_mode.label()
        )));
        if let Some(error) = state.run_error.as_deref() {
            lines.push(Line::from(format!("Run error: {error}")));
        }
        if let Some(stats) = launch.statistics() {
            lines.push(Line::from(format!(
                "Run {} / attempt {} | {}% ({}/{}) complete | failed={}",
                stats.run_id.0,
                stats.attempt_id.0,
                stats.bounded_progress_percent(),
                stats.completed_measures,
                stats.total_measures,
                stats.failed_measures
            )));
            lines.push(Line::from(format!(
                "Throughput: {} | latency: {} | provenance: {:?}",
                stats
                    .throughput_per_second
                    .map_or_else(|| "unavailable".into(), |value| format!("{value}/s")),
                stats
                    .latency_millis
                    .map_or_else(|| "unavailable".into(), |value| format!("{value} ms")),
                stats.provenance
            )));
            if let Some(reason) = &stats.unavailable_reason {
                lines.push(Line::from(format!("Unavailable: {reason}")));
            }
        } else {
            lines.push(Line::from("No authoritative run event received yet."));
        }
        if let Some(fanout) = state
            .live
            .as_ref()
            .and_then(|snapshot| snapshot.fanout.as_ref())
        {
            lines.push(Line::from(format!(
                "Fan-out: admitted {} members (idempotency {})",
                fanout.members.len(),
                fanout.idempotency_key
            )));
        }
        lines
    } else {
        vec![
            Line::from("No reviewed configuration is ready to launch."),
            Line::from("Open Configuration, press V to prepare and A to apply preflight."),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(
                " Run control - Enter start | A fan-out | Ctrl-C cancel | Ctrl-Z cancel fan-out | x reconnect ",
                policy,
            ))
            .wrap(Wrap { trim: true }),
        area,
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
            let mut entries: Vec<String> = snapshot
                .runs
                .iter()
                .map(|run| {
                    format!(
                        "{} | {:?} | revision {} | plan {}",
                        run.run_id.0,
                        run.state,
                        run.revision.0,
                        &run.plan_sha256[..run.plan_sha256.len().min(12)]
                    )
                })
                .collect();
            if let Some(analysis) = &snapshot.analysis {
                entries.push(format!(
                    "Analysis: {} runs | digest {}",
                    analysis.run_count,
                    &analysis.analysis_sha256[..analysis.analysis_sha256.len().min(12)]
                ));
            }
            entries
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
        Screen::DevelopmentHandoff => "Enter start   x cancel   r retry   Esc back",
        Screen::Wizard => "Enter next   Esc back   q cancel",
        Screen::Measures => "Up/Down move   Space item   g group   type search",
        Screen::Configuration => "Up/Down move   Enter edit   Ctrl-S save",
        Screen::RunControl => "Enter start   Ctrl-C cancel   x reconnect",
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
            Line::from("1-4 / s  switch workspace"),
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
        Paragraph::new("ASB | 1 Home 2 Measures 3 Config s Run 4 Reports\n? help | q quit")
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
    fn nested_benchmark_picker_keys_drive_pool_group_benchmark_and_measure_state() {
        let mut state = WorkspaceState::default();
        state.handle_key(key(KeyCode::Char('2')));
        // The production picker is backed by the nested catalog, not the
        // legacy flat row list. Benchmark toggle clears both quality measures.
        state.handle_key(key(KeyCode::Char('b')));
        assert!(!state.measures[0].selected);
        assert!(!state.measures[1].selected);
        // Group toggle restores only the active group's visible measures.
        state.handle_key(key(KeyCode::Char('g')));
        assert!(state.measures[0].selected && state.measures[1].selected);
        // Search narrows the nested projection without dropping hidden state.
        for character in "latency".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }
        assert_eq!(state.visible_indices().len(), 1);
        assert!(state.measures[2].selected);
        // Pool selection is a handled production action even when only one
        // development pool is available; it never mutates the campaign.
        assert_eq!(state.handle_key(key(KeyCode::Char('P'))), UiAction::None);
        let handoff = state
            .benchmark_campaign_handoff(crate::control_codec::Revision(1))
            .expect("nested picker produces an exact campaign handoff");
        assert_eq!(handoff.pool_id, "development");
        assert_eq!(handoff.measure_ids.len(), 3);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Pool > Group > Benchmark > Measure"));
    }

    #[test]
    fn recording_scope_defaults_to_all_and_tracks_selected_workloads() {
        let mut state = WorkspaceState::default();
        assert_eq!(
            state.recording_workload_scope().unwrap(),
            crate::recording_campaign::WorkloadScope::Selected(vec![
                "efficiency".into(),
                "quality".into(),
            ])
        );

        state.screen = Screen::Measures;
        // Select only the quality benchmark. The dispatcher must receive the
        // benchmark identity, not a display label or the whole catalog.
        state.handle_key(key(KeyCode::Char('b')));
        let scope = state.recording_workload_scope().unwrap();
        assert_eq!(
            scope,
            crate::recording_campaign::WorkloadScope::Selected(vec!["efficiency".into()])
        );
        state.handle_key(key(KeyCode::Char('a')));
        assert_eq!(
            state.recording_workload_scope().unwrap(),
            crate::recording_campaign::WorkloadScope::All
        );
        state.handle_key(key(KeyCode::Char('a')));
        assert!(state.recording_workload_scope().is_err());
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
            benchmark_catalog: None,
            auth_status: None,
            auth_unavailable: false,
            auth_development_only: false,
            auth_unavailable_reason: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            dynamic_provider_catalog: None,
            configuration: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            fanout: None,
            runs: Vec::new(),
            analysis: None,
        };
        state.apply_live_snapshot(snapshot);
        assert!(state.measures.is_empty());
        assert_eq!(state.measure_cursor, 0);
    }

    #[test]
    fn live_catalog_keeps_nested_picker_when_control_revision_is_zero() {
        let mut state = WorkspaceState::default();
        let snapshot = LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner-nested".into()),
            // Initial negotiation can expose control revision zero before
            // the runner has emitted its first durable event.
            latest_revision: Some(Revision(0)),
            capabilities: None,
            measurement_catalog: Some(MeasurementCatalog {
                schema_version: 1,
                catalog_sha256: "live-digest".into(),
                groups: vec![crate::control_codec::MeasurementGroup {
                    id: crate::control_codec::MeasurementGroupId::QualityReliability,
                    label: "Quality Reliability".into(),
                    description: "quality".into(),
                }],
                measurements: vec![crate::control_codec::MeasurementDefinition {
                    id: "quality.reliability".into(),
                    name: "Reliability".into(),
                    description: "reliability score".into(),
                    group: crate::control_codec::MeasurementGroupId::QualityReliability,
                    quantity: crate::control_codec::MeasurementQuantity::Ratio,
                    unit: "ratio".into(),
                    aggregation: crate::control_codec::MeasurementAggregation::Gauge,
                    scope: crate::control_codec::MeasurementScope::Attempt,
                    provenance: crate::control_codec::MeasurementProvenance {
                        source: crate::control_codec::MeasurementSource::AsbRunnerJournal,
                        qualification: crate::control_codec::MeasurementQualification::Implemented,
                    },
                    source_identity:
                        crate::control_codec::MeasurementSourceIdentity::ProcfsProcessStat,
                    resolution_ns: 1,
                    overhead: crate::control_codec::MeasurementOverhead {
                        class: crate::control_codec::MeasurementOverheadClass::Low,
                        minimum_interval_ns: 1,
                        requires_privilege: false,
                    },
                    live: crate::control_codec::MeasurementModeSupport::Supported,
                    replay: crate::control_codec::MeasurementModeSupport::Supported,
                    platforms: vec![],
                    evidence_limits: vec![],
                }],
            }),
            benchmark_catalog: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            dynamic_provider_catalog: None,
            configuration: None,
            auth_status: None,
            auth_unavailable: false,
            auth_development_only: false,
            auth_unavailable_reason: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            fanout: None,
            runs: Vec::new(),
            analysis: None,
        };
        state.apply_live_snapshot(snapshot);
        state.screen = Screen::Measures;
        assert_eq!(state.measures[0].group_id, "quality_reliability");
        assert_eq!(state.measures[0].group, "Quality Reliability");
        state.handle_key(key(KeyCode::Char('g')));
        let handoff = state
            .benchmark_campaign_handoff(Revision(0))
            .expect("live catalog remains nested and handoffable");
        assert_eq!(handoff.catalog_digest, "live-digest");
        assert_eq!(handoff.measure_ids, ["quality.reliability"]);
        assert_eq!(
            state.benchmark_campaign_handoff(Revision(1)),
            Err(crate::selection::SelectionError::StaleCatalog)
        );
    }

    #[test]
    fn provisional_zero_revision_cannot_alias_first_durable_revision() {
        let mut state = WorkspaceState::default();
        let catalog = MeasurementCatalog {
            schema_version: 1,
            catalog_sha256: "zero-digest".into(),
            groups: vec![crate::control_codec::MeasurementGroup {
                id: crate::control_codec::MeasurementGroupId::QualityReliability,
                label: "Quality Reliability".into(),
                description: "quality".into(),
            }],
            measurements: vec![crate::control_codec::MeasurementDefinition {
                id: "quality.reliability".into(),
                name: "Reliability".into(),
                description: "reliability score".into(),
                group: crate::control_codec::MeasurementGroupId::QualityReliability,
                quantity: crate::control_codec::MeasurementQuantity::Ratio,
                unit: "ratio".into(),
                aggregation: crate::control_codec::MeasurementAggregation::Gauge,
                scope: crate::control_codec::MeasurementScope::Attempt,
                provenance: crate::control_codec::MeasurementProvenance {
                    source: crate::control_codec::MeasurementSource::AsbRunnerJournal,
                    qualification: crate::control_codec::MeasurementQualification::Implemented,
                },
                source_identity: crate::control_codec::MeasurementSourceIdentity::ProcfsProcessStat,
                resolution_ns: 1,
                overhead: crate::control_codec::MeasurementOverhead {
                    class: crate::control_codec::MeasurementOverheadClass::Low,
                    minimum_interval_ns: 1,
                    requires_privilege: false,
                },
                live: crate::control_codec::MeasurementModeSupport::Supported,
                replay: crate::control_codec::MeasurementModeSupport::Supported,
                platforms: vec![],
                evidence_limits: vec![],
            }],
        };
        let snapshot = |revision: Revision, digest: &str| LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner-revision-fence".into()),
            latest_revision: Some(revision),
            capabilities: None,
            measurement_catalog: Some(MeasurementCatalog {
                catalog_sha256: digest.into(),
                ..catalog.clone()
            }),
            benchmark_catalog: None,
            auth_status: None,
            auth_unavailable: false,
            auth_development_only: false,
            auth_unavailable_reason: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            dynamic_provider_catalog: None,
            configuration: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            fanout: None,
            runs: Vec::new(),
            analysis: None,
        };
        state.apply_live_snapshot(snapshot(Revision(0), "zero-digest"));
        state.screen = Screen::Measures;
        state.handle_key(key(KeyCode::Char('g')));
        assert!(state.benchmark_campaign_handoff(Revision(0)).is_ok());
        state.apply_live_snapshot(snapshot(Revision(1), "one-digest"));
        assert_eq!(
            state.benchmark_campaign_handoff(Revision(0)),
            Err(crate::selection::SelectionError::StaleCatalog)
        );
        assert!(state.benchmark_campaign_handoff(Revision(1)).is_ok());
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
            package: Some(crate::agent_catalog::AgentPackage {
                package_id: "codex-package".into(),
                version: "1".into(),
                sha256: "a".repeat(64),
                signature_sha256: "b".repeat(64),
                signer: crate::agent_catalog::AgentSigner {
                    key_id: "key-1".into(),
                    principal: "asb-release".into(),
                },
            }),
            provenance: Some(crate::agent_catalog::AgentProvenance {
                source_revision: "source".into(),
                manifest_sha256: "c".repeat(64),
                sbom_sha256: "d".repeat(64),
                license_ref: "MIT".into(),
            }),
            capabilities: vec!["coding".into()],
            availability: crate::agent_catalog::AgentAvailability::Available,
        };
        let snapshot = LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner".into()),
            latest_revision: Some(Revision(1)),
            capabilities: None,
            measurement_catalog: None,
            benchmark_catalog: None,
            auth_status: None,
            auth_unavailable: false,
            auth_development_only: false,
            auth_unavailable_reason: None,
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
            dynamic_provider_catalog: None,
            configuration: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            fanout: None,
            runs: Vec::new(),
            analysis: None,
        };
        let mut state = WorkspaceState::for_readiness(StartupInput {
            configuration_present: false,
            configuration_complete: false,
            endpoint_available: true,
            configuration_malformed: false,
            configuration_stale: false,
            authorized: true,
        });
        assert_eq!(
            state
                .wizard
                .catalog()
                .expect("development fallback")
                .catalog()
                .options(crate::wizard_catalog::OptionKind::Provider)[0]
                .id,
            "development"
        );
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
    fn automatic_unconfigured_route_has_development_catalog_fallback() {
        let state = WorkspaceState::for_readiness(StartupInput {
            configuration_present: false,
            configuration_complete: false,
            endpoint_available: true,
            configuration_malformed: false,
            configuration_stale: false,
            authorized: true,
        });
        assert_eq!(state.screen, Screen::Wizard);
        let catalog = state.wizard.catalog().expect("development catalog");
        assert_eq!(
            catalog
                .catalog()
                .options(crate::wizard_catalog::OptionKind::Provider)[0]
                .id,
            "development"
        );
        assert_eq!(state.wizard.mode(), crate::wizard::WizardMode::Development);
    }

    #[test]
    fn workspace_default_uses_stable_wizard_context() {
        assert_eq!(
            WorkspaceState::default().wizard.mode(),
            crate::wizard::WizardMode::Stable
        );
    }

    #[test]
    fn fanout_selection_requires_applied_configuration_and_catalog_context() {
        let mut state = WorkspaceState::default();
        assert!(state.fanout_selection().is_err());
        state.live = Some(LiveSnapshot {
            connection: Connection::Negotiated,
            runner_instance_id: Some("runner".into()),
            latest_revision: Some(Revision(1)),
            capabilities: None,
            measurement_catalog: None,
            benchmark_catalog: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            dynamic_provider_catalog: None,
            configuration: None,
            auth_status: None,
            auth_unavailable: false,
            auth_development_only: true,
            auth_unavailable_reason: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            fanout: None,
            runs: Vec::new(),
            analysis: None,
        });
        assert!(state.fanout_selection().is_err());
        state.live.as_mut().unwrap().configuration =
            Some(crate::control_codec::ConfigurationSnapshot {
                runner_instance_id: "runner".into(),
                generation: Revision(1),
                configured: true,
                agent_ids: vec!["agent".into()],
                provider_id: None,
                model_id: None,
                auth_method: None,
                credential_reference_sha256: None,
            });
        assert!(state.fanout_selection().is_err());
    }

    #[test]
    fn explicit_development_runtime_context_installs_fixture_catalog() {
        let mut state = WorkspaceState::default();
        assert_eq!(state.wizard.mode(), crate::wizard::WizardMode::Stable);
        state.use_development_context();
        assert_eq!(state.wizard.mode(), crate::wizard::WizardMode::Development);
        assert!(state.wizard.catalog().is_some());
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
            benchmark_catalog: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            dynamic_provider_catalog: None,
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
            auth_unavailable: false,
            auth_development_only: false,
            auth_unavailable_reason: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            fanout: None,
            runs: Vec::new(),
            analysis: None,
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
            benchmark_catalog: None,
            agent_catalog: None,
            agent_lifecycle: None,
            provider_catalog: None,
            dynamic_provider_catalog: None,
            configuration: None,
            auth_status: None,
            auth_unavailable: false,
            auth_development_only: false,
            auth_unavailable_reason: None,
            recording_campaign: None,
            recording_estimate: None,
            recording_campaign_lifecycle: None,
            fanout: None,
            runs: vec![RunSummary {
                run_id: RunId("run-7".into()),
                attempt_id: AttemptId("attempt-7".into()),
                state: PublicRunState::Completed,
                created_revision: Revision(7),
                revision: Revision(7),
                plan_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            }],
            analysis: None,
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
    fn renders_routes_across_all_layout_tiers_and_overlays() {
        let mut state = WorkspaceState::default();
        let policies = [
            policy(),
            RenderPolicy {
                tier: CapabilityTier::BasicColor,
                unicode: true,
                ..policy()
            },
            RenderPolicy {
                tier: CapabilityTier::Plain,
                unicode: false,
                ..policy()
            },
        ];
        for (width, height) in [(120, 40), (80, 24), (30, 8)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for screen in [
                Screen::Landing,
                Screen::Measures,
                Screen::Configuration,
                Screen::Reports,
                Screen::Help,
            ] {
                state.screen = screen;
                for render_policy in policies {
                    terminal
                        .draw(|frame| render(frame, &state, render_policy))
                        .unwrap();
                }
                state.help = true;
                terminal
                    .draw(|frame| render(frame, &state, policy()))
                    .unwrap();
                state.help = false;
            }
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        for (width, height, tier) in [
            (120, 40, CapabilityTier::TrueColor),
            (60, 18, CapabilityTier::BasicColor),
            (24, 6, CapabilityTier::Plain),
        ] {
            terminal.backend_mut().resize(width, height);
            state.apply_resize(width, height);
            for screen in [
                Screen::Landing,
                Screen::DevelopmentHandoff,
                Screen::Measures,
                Screen::Configuration,
                Screen::RunControl,
                Screen::Reports,
                Screen::Help,
            ] {
                state.screen = screen;
                terminal
                    .draw(|frame| {
                        let mut render_policy = policy();
                        render_policy.tier = tier;
                        render(frame, &state, render_policy)
                    })
                    .unwrap();
            }
        }
        state.screen = Screen::Wizard;
        state.help = false;
        terminal.backend_mut().resize(120, 40);
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
        state.help = true;
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
        state.help = false;
        state.screen = Screen::Landing;
        state.help = true;
        terminal
            .draw(|frame| render(frame, &state, policy()))
            .unwrap();
    }

    #[test]
    fn development_handoff_navigation_and_projections_render_all_outcomes() {
        let mut state = WorkspaceState::default();
        assert_eq!(state.handle_key(key(KeyCode::Char('d'))), UiAction::None);
        assert_eq!(state.screen, Screen::DevelopmentHandoff);
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            UiAction::Control(crate::actions::UiAction::MaterializeDevelopment)
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        for phase in [
            crate::development_handoff::Phase::Cloning,
            crate::development_handoff::Phase::Building,
            crate::development_handoff::Phase::Installing,
            crate::development_handoff::Phase::Installed,
            crate::development_handoff::Phase::RolledBack,
            crate::development_handoff::Phase::Failed,
            crate::development_handoff::Phase::Incompatible,
        ] {
            state.development_handoff.phase = phase;
            state.development_handoff.source_commit = Some("a".repeat(40));
            state.development_handoff.source_tree = Some("b".repeat(40));
            state.development_handoff.executable_sha256 = Some("c".repeat(64));
            terminal
                .draw(|frame| render(frame, &state, policy()))
                .unwrap();
        }
        state.development_handoff.phase = crate::development_handoff::Phase::Ready;
        assert_eq!(
            state.handle_key(key(KeyCode::Char(' '))),
            UiAction::Control(crate::actions::UiAction::MaterializeDevelopment)
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('x'))),
            UiAction::Control(crate::actions::UiAction::CancelDevelopment)
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('r'))),
            UiAction::Control(crate::actions::UiAction::RetryDevelopment)
        );
        assert_eq!(state.handle_key(key(KeyCode::Esc)), UiAction::None);
        assert_eq!(state.screen, Screen::Landing);
    }

    #[test]
    fn keyboard_matrix_exercises_each_workspace_route_without_panicking() {
        let keys = [
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Char('3'),
            KeyCode::Char('4'),
            KeyCode::Char('d'),
            KeyCode::Char('f'),
            KeyCode::Char('g'),
            KeyCode::Char('p'),
            KeyCode::Char('r'),
            KeyCode::Char('s'),
            KeyCode::Char('w'),
            KeyCode::Char('/'),
            KeyCode::Char('?'),
            KeyCode::Char(' '),
            KeyCode::Char('a'),
            KeyCode::Char('b'),
            KeyCode::Char('e'),
            KeyCode::Char('o'),
            KeyCode::Char('v'),
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Backspace,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
        ];
        let mut state = WorkspaceState::default();
        for screen in [
            Screen::Landing,
            Screen::DevelopmentHandoff,
            Screen::Measures,
            Screen::Configuration,
            Screen::RunControl,
            Screen::Reports,
            Screen::Help,
        ] {
            state.screen = screen;
            for code in keys {
                let _ = state.handle_key(key(code));
            }
        }
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
        state.screen = Screen::Configuration;
        assert_eq!(
            state.handle_key(key(KeyCode::Char('f'))),
            UiAction::Control(crate::actions::UiAction::RefreshProviderCatalog)
        );
        state.screen = Screen::Reports;
        for (key_code, action) in [
            ('e', crate::actions::UiAction::EstimateRecording),
            ('P', crate::actions::UiAction::PlanRecording),
            ('C', crate::actions::UiAction::CompareLiveOffline),
            ('G', crate::actions::UiAction::ProgressRecording),
            ('X', crate::actions::UiAction::CancelRecording),
            ('Y', crate::actions::UiAction::ReconcileRecording),
            ('o', crate::actions::UiAction::ActivateOfflineDefault),
        ] {
            assert_eq!(
                state.handle_key(key(KeyCode::Char(key_code))),
                UiAction::Control(action)
            );
        }
        state.screen = Screen::RunControl;
        assert_eq!(
            state.handle_key(key(KeyCode::Char('J'))),
            UiAction::Control(crate::actions::UiAction::ReplaySelected)
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char(']'))),
            UiAction::Control(crate::actions::UiAction::SelectOfflineCassette)
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('A'))),
            UiAction::Control(crate::actions::UiAction::AdmitFanout)
        );
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            UiAction::Control(crate::actions::UiAction::CancelFanout)
        );
        state.screen = Screen::Reports;
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
    fn report_operator_actions_use_explicit_cursor_selection() {
        let mut state = WorkspaceState {
            screen: Screen::Reports,
            ..WorkspaceState::default()
        };
        state.report_selection = vec![
            crate::control_codec::RunId("run-a".into()),
            crate::control_codec::RunId("run-b".into()),
        ];
        assert!(matches!(
            state.handle_key(key(KeyCode::Char(' '))),
            UiAction::None
        ));
        assert_eq!(state.report_selection.len(), 2);
        state.report_cursor = 1;
        state.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(state.report_selection.len(), 2);
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('v'))),
            UiAction::Control(crate::actions::UiAction::CompareLiveOffline)
        ));
        assert!(matches!(
            state.handle_key(key(KeyCode::Char('t'))),
            UiAction::Control(crate::actions::UiAction::RetryRun)
        ));
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
    fn wizard_helper_receipt_enrolls_only_digest_metadata() {
        let values = [
            "codex".into(),
            "openai".into(),
            "gpt-5.2".into(),
            "shared".into(),
            format!("credential_helper:{}:{}", "a".repeat(64), "b".repeat(64)),
            "record".into(),
            "offline".into(),
        ];
        let receipt = WorkspaceState::wizard_credential_helper_receipt(&values)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.provider, "openai");
        assert_eq!(receipt.endpoint_identity_sha256, "a".repeat(64));
        assert_eq!(receipt.credential_locator_sha256, "b".repeat(64));
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
    fn wizard_live_choice_is_explicit_but_authentication_remains_warning_only() {
        let mut values = [
            "opencode,opendesk".into(),
            "openrouter".into(),
            "cohere/north-mini-code:free".into(),
            "shared defaults".into(),
            "none".into(),
            "live".into(),
            "offline replay after capture".into(),
        ];
        assert_eq!(
            WorkspaceState::wizard_execution_mode(&values),
            crate::configuration_materialization::ExecutionMode::Live
        );
        // Setup stays non-blocking: the same live intent can be reviewed even
        // before a helper receipt is available. ASB owns the explicit run-time
        // credential error when the operator starts the live run.
        assert!(WorkspaceState::wizard_configuration_selection(&values).is_ok());
        values[5] = "mock".into();
        assert_eq!(
            WorkspaceState::wizard_execution_mode(&values),
            crate::configuration_materialization::ExecutionMode::LocalMock
        );
    }

    #[test]
    fn run_control_keeps_live_identity_and_typed_failure_visible() {
        let mut state = WorkspaceState {
            screen: Screen::RunControl,
            ..WorkspaceState::default()
        };
        state.launch_state = Some(crate::launch_statistics::LaunchState::new(
            crate::configuration_materialization::LaunchBinding {
                materialization_digest_sha256: "a".repeat(64),
                provider_catalog_generation: Revision(1),
                provider_catalog_digest: "b".repeat(64),
                benchmark_catalog_generation: Revision(2),
                benchmark_catalog_digest: "c".repeat(64),
                agent_ids: vec!["opencode".into(), "opendesk".into()],
                provider_id: "openrouter".into(),
                model_id: "free-model".into(),
                pool_id: "default".into(),
                group_ids: vec!["quality".into()],
                benchmark_ids: vec!["quality".into()],
                measure_ids: vec!["quality.correctness".into()],
                development_only: true,
                execution_mode: crate::configuration_materialization::ExecutionMode::Live,
            },
        ));
        state.set_run_error("live provider unavailable (code -32001)");
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
        assert!(text.contains("Provider: openrouter"));
        assert!(text.contains("model: free-model"));
        assert!(text.contains("mode: live"));
        assert!(text.contains("Run error: live provider unavailable (code -32001)"));
    }

    #[test]
    fn run_error_projection_is_bounded_and_clearable() {
        let mut state = WorkspaceState::default();
        state.set_run_error("x".repeat(1024));
        assert_eq!(state.run_error.as_ref().map(String::len), Some(512));
        state.clear_run_error();
        assert!(state.run_error.is_none());
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
