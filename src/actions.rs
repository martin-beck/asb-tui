// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral action and help metadata.
//!
//! This module is deliberately data-only.  A renderer can use the descriptors
//! for a hot-key strip, a contextual help panel, or a searchable command
//! palette without this foundation owning widgets or terminal state.

use crate::{
    Capabilities,
    shell::{DisabledReason, Route, RouteAvailability},
};

/// A stable, typed operation understood by the frontend shell or a screen
/// extension.  The enum is the source of truth for help and hot-key metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum UiAction {
    Quit,
    GoBack,
    OpenLanding,
    OpenConfiguration,
    OpenMeasurementSelection,
    OpenRunControl,
    OpenRecentRuns,
    OpenReports,
    OpenHelp,
    Reconnect,
    FocusSearch,
    ToggleAllMeasures,
    ToggleMeasure,
    StartRun,
    CancelRun,
    RefreshRuns,
    CompareRuns,
    RefreshProviderCatalog,
    EstimateRecording,
    PlanRecording,
    ConfirmRecordingCapture,
    ProgressRecording,
    CancelRecording,
    ReconcileRecording,
    ActivateOfflineDefault,
}

impl UiAction {
    /// Every action, in stable display order.
    pub const ALL: [Self; 25] = [
        Self::Quit,
        Self::GoBack,
        Self::OpenLanding,
        Self::OpenConfiguration,
        Self::OpenMeasurementSelection,
        Self::OpenRunControl,
        Self::OpenRecentRuns,
        Self::OpenReports,
        Self::OpenHelp,
        Self::Reconnect,
        Self::FocusSearch,
        Self::ToggleAllMeasures,
        Self::ToggleMeasure,
        Self::StartRun,
        Self::CancelRun,
        Self::RefreshRuns,
        Self::CompareRuns,
        Self::RefreshProviderCatalog,
        Self::EstimateRecording,
        Self::PlanRecording,
        Self::ConfirmRecordingCapture,
        Self::ProgressRecording,
        Self::CancelRecording,
        Self::ReconcileRecording,
        Self::ActivateOfflineDefault,
    ];

    /// Stable machine-readable action identifier for recordings and help
    /// search.  It is not a renderer-specific key label.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::GoBack => "go_back",
            Self::OpenLanding => "open_landing",
            Self::OpenConfiguration => "open_configuration",
            Self::OpenMeasurementSelection => "open_measurement_selection",
            Self::OpenRunControl => "open_run_control",
            Self::OpenRecentRuns => "open_recent_runs",
            Self::OpenReports => "open_reports",
            Self::OpenHelp => "open_help",
            Self::Reconnect => "reconnect",
            Self::FocusSearch => "focus_search",
            Self::ToggleAllMeasures => "toggle_all_measures",
            Self::ToggleMeasure => "toggle_measure",
            Self::StartRun => "start_run",
            Self::CancelRun => "cancel_run",
            Self::RefreshRuns => "refresh_runs",
            Self::CompareRuns => "compare_runs",
            Self::RefreshProviderCatalog => "refresh_provider_catalog",
            Self::EstimateRecording => "estimate_recording",
            Self::PlanRecording => "plan_recording",
            Self::ConfirmRecordingCapture => "confirm_recording_capture",
            Self::ProgressRecording => "progress_recording",
            Self::CancelRecording => "cancel_recording",
            Self::ReconcileRecording => "reconcile_recording",
            Self::ActivateOfflineDefault => "activate_offline_default",
        }
    }
}

/// A portable key chord.  Renderers decide how to display it and how to map
/// terminal events to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum KeyChord {
    Char(char),
    Ctrl(char),
    Escape,
    Enter,
    Space,
}

impl KeyChord {
    /// User-facing, stable label for the hot-key window.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Char('?') => "?",
            Self::Char('/') => "/",
            Self::Char('b') => "b",
            Self::Char('c') => "c",
            Self::Char('h') => "h",
            Self::Char('m') => "m",
            Self::Char('r') => "r",
            Self::Char('s') => "s",
            Self::Char('q') => "q",
            Self::Char('R') => "R",
            Self::Char('a') => "a",
            Self::Char(' ') => "Space",
            Self::Char(_) => "key",
            Self::Ctrl('c') => "Ctrl-C",
            Self::Ctrl(_) => "Ctrl-key",
            Self::Escape => "Esc",
            Self::Enter => "Enter",
            Self::Space => "Space",
        }
    }
}

/// The screen context in which a descriptor is relevant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionContext {
    Global,
    Route(Route),
}

/// Why a known action is currently unavailable.  Disabled descriptors remain
/// discoverable, so help can explain what must change instead of hiding it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionDisabledReason {
    NotNegotiated,
    Analysis,
    Cancel,
    Events,
    History,
    Launch,
    Planning,
    WrongContext,
}

impl ActionDisabledReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotNegotiated => "connect to ASB first",
            Self::Analysis => "ASB analysis capability is unavailable",
            Self::Cancel => "ASB cancellation capability is unavailable",
            Self::Events => "ASB event capability is unavailable",
            Self::History => "ASB history capability is unavailable",
            Self::Launch => "ASB launch capability is unavailable",
            Self::Planning => "ASB planning capability is unavailable",
            Self::WrongContext => "not available in this screen",
        }
    }
}

/// Renderer-neutral metadata for one action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionDescriptor {
    pub action: UiAction,
    pub key: KeyChord,
    pub label: &'static str,
    pub description: &'static str,
    pub context: ActionContext,
    pub disabled: Option<ActionDisabledReason>,
}

impl ActionDescriptor {
    pub const fn enabled(self) -> bool {
        self.disabled.is_none()
    }
}

/// Immutable action registry used by hot-key and help renderers.
pub struct ActionRegistry;

impl ActionRegistry {
    /// Return all descriptors relevant to a visible context.  Global actions
    /// are included in every screen; route actions are included only on their
    /// owning screen.  Backend-gated actions remain visible with a reason.
    pub fn for_context(route: Route, capabilities: Option<&Capabilities>) -> Vec<ActionDescriptor> {
        UiAction::ALL
            .into_iter()
            .filter_map(|action| descriptor(action, route, capabilities))
            .collect()
    }

    /// Search all action metadata, including actions outside the current
    /// screen. Matching is deterministic, ASCII case-insensitive, and checks
    /// identifiers, labels, and descriptions.
    pub fn search(query: &str) -> Vec<ActionDescriptor> {
        let needle = query.trim().to_ascii_lowercase();
        UiAction::ALL
            .into_iter()
            .filter_map(|action| {
                let item = descriptor(action, action_context_route(action), None)?;
                let matches = needle.is_empty()
                    || [item.action.id(), item.label, item.description]
                        .into_iter()
                        .any(|value| value.to_ascii_lowercase().contains(&needle));
                matches.then_some(item)
            })
            .collect()
    }
}

const fn action_context_route(action: UiAction) -> Route {
    match action {
        UiAction::FocusSearch => Route::Help,
        UiAction::ToggleAllMeasures | UiAction::ToggleMeasure => Route::MeasurementSelection,
        UiAction::StartRun | UiAction::CancelRun => Route::RunControl,
        UiAction::EstimateRecording
        | UiAction::PlanRecording
        | UiAction::ConfirmRecordingCapture
        | UiAction::ProgressRecording
        | UiAction::CancelRecording
        | UiAction::ReconcileRecording
        | UiAction::ActivateOfflineDefault => Route::RunControl,
        UiAction::RefreshProviderCatalog => Route::Configuration,
        UiAction::RefreshRuns => Route::RecentRuns,
        UiAction::CompareRuns => Route::Reports,
        _ => Route::Landing,
    }
}

fn descriptor(
    action: UiAction,
    route: Route,
    capabilities: Option<&Capabilities>,
) -> Option<ActionDescriptor> {
    let (key, label, description, context, target) = match action {
        UiAction::Quit => (
            KeyChord::Char('q'),
            "Quit",
            "Close the frontend; the runner continues",
            ActionContext::Global,
            None,
        ),
        UiAction::GoBack => (
            KeyChord::Escape,
            "Back",
            "Return to the previous screen",
            ActionContext::Global,
            None,
        ),
        UiAction::OpenLanding => (
            KeyChord::Char('h'),
            "Home",
            "Open the landing screen",
            ActionContext::Global,
            Some(Route::Landing),
        ),
        UiAction::OpenConfiguration => (
            KeyChord::Char('c'),
            "Configuration",
            "Open frontend configuration",
            ActionContext::Global,
            Some(Route::Configuration),
        ),
        UiAction::OpenMeasurementSelection => (
            KeyChord::Char('m'),
            "Measures",
            "Choose benchmark groups and measures",
            ActionContext::Global,
            Some(Route::MeasurementSelection),
        ),
        UiAction::OpenRunControl => (
            KeyChord::Char('s'),
            "Run",
            "Start or control a benchmark run",
            ActionContext::Global,
            Some(Route::RunControl),
        ),
        UiAction::OpenRecentRuns => (
            KeyChord::Char('r'),
            "Recent runs",
            "Inspect recent runs and report inputs",
            ActionContext::Global,
            Some(Route::RecentRuns),
        ),
        UiAction::OpenReports => (
            KeyChord::Char('p'),
            "Reports",
            "Open report analysis and comparison",
            ActionContext::Global,
            Some(Route::Reports),
        ),
        UiAction::OpenHelp => (
            KeyChord::Char('?'),
            "Help",
            "Search actions and key bindings",
            ActionContext::Global,
            Some(Route::Help),
        ),
        UiAction::Reconnect => (
            KeyChord::Char('x'),
            "Reconnect",
            "Reconnect to the negotiated ASB endpoint",
            ActionContext::Global,
            None,
        ),
        UiAction::FocusSearch => (
            KeyChord::Char('/'),
            "Search",
            "Focus the current screen search field",
            ActionContext::Route(Route::Help),
            None,
        ),
        UiAction::ToggleAllMeasures => (
            KeyChord::Char('a'),
            "Toggle all",
            "Select or deselect every visible measure",
            ActionContext::Route(Route::MeasurementSelection),
            None,
        ),
        UiAction::ToggleMeasure => (
            KeyChord::Space,
            "Toggle measure",
            "Select or deselect the focused measure",
            ActionContext::Route(Route::MeasurementSelection),
            None,
        ),
        UiAction::StartRun => (
            KeyChord::Enter,
            "Start run",
            "Start the configured benchmark run",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::CancelRun => (
            KeyChord::Ctrl('c'),
            "Cancel run",
            "Request cancellation of the active run",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::RefreshRuns => (
            KeyChord::Char('f'),
            "Refresh",
            "Refresh recent run projections",
            ActionContext::Route(Route::RecentRuns),
            None,
        ),
        UiAction::CompareRuns => (
            KeyChord::Char('v'),
            "Compare",
            "Compare selected report runs",
            ActionContext::Route(Route::Reports),
            None,
        ),
        UiAction::RefreshProviderCatalog => (
            KeyChord::Char('f'),
            "Refresh providers",
            "Refresh connected provider and model catalog",
            ActionContext::Route(Route::Configuration),
            None,
        ),
        UiAction::EstimateRecording => (
            KeyChord::Char('e'),
            "Estimate capture",
            "Estimate the selected recording workload matrix",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::PlanRecording => (
            KeyChord::Char('P'),
            "Plan capture",
            "Create a bounded recording campaign plan",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::ConfirmRecordingCapture => (
            KeyChord::Char('C'),
            "Confirm capture",
            "Explicitly authorize provider response capture",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::ProgressRecording => (
            KeyChord::Char('G'),
            "Capture progress",
            "Refresh recording campaign progress",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::CancelRecording => (
            KeyChord::Char('X'),
            "Cancel capture",
            "Cancel the active recording campaign",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::ReconcileRecording => (
            KeyChord::Char('Y'),
            "Reconcile capture",
            "Reconcile an interrupted recording campaign",
            ActionContext::Route(Route::RunControl),
            None,
        ),
        UiAction::ActivateOfflineDefault => (
            KeyChord::Char('o'),
            "Use offline capture",
            "Activate offline defaults after complete coverage",
            ActionContext::Route(Route::RunControl),
            None,
        ),
    };
    let relevant = match context {
        ActionContext::Global => true,
        ActionContext::Route(owner) => owner == route,
    };
    if !relevant {
        return None;
    }
    let disabled = target
        .and_then(|target| route_status(target, capabilities))
        .or_else(|| action_backend_status(action, capabilities));
    Some(ActionDescriptor {
        action,
        key,
        label,
        description,
        context,
        disabled,
    })
}

fn route_status(route: Route, capabilities: Option<&Capabilities>) -> Option<ActionDisabledReason> {
    match route.availability(capabilities) {
        RouteAvailability::Available => None,
        RouteAvailability::Disabled(reason) => Some(map_reason(reason)),
    }
}

fn map_reason(reason: DisabledReason) -> ActionDisabledReason {
    match reason {
        DisabledReason::NotNegotiated => ActionDisabledReason::NotNegotiated,
        DisabledReason::Analysis => ActionDisabledReason::Analysis,
        DisabledReason::Cancel => ActionDisabledReason::Cancel,
        DisabledReason::Events => ActionDisabledReason::Events,
        DisabledReason::History => ActionDisabledReason::History,
        DisabledReason::Launch => ActionDisabledReason::Launch,
        DisabledReason::Planning => ActionDisabledReason::Planning,
    }
}

fn action_backend_status(
    action: UiAction,
    capabilities: Option<&Capabilities>,
) -> Option<ActionDisabledReason> {
    let Some(caps) = capabilities else {
        return match action {
            UiAction::Reconnect | UiAction::Quit | UiAction::GoBack => None,
            _ => Some(ActionDisabledReason::NotNegotiated),
        };
    };
    match action {
        UiAction::StartRun if !caps.launch => Some(ActionDisabledReason::Launch),
        UiAction::CancelRun if !caps.cancel => Some(ActionDisabledReason::Cancel),
        UiAction::CancelRun if !caps.events => Some(ActionDisabledReason::Events),
        UiAction::RefreshRuns | UiAction::CompareRuns if !caps.history => {
            Some(ActionDisabledReason::History)
        }
        UiAction::CompareRuns if !caps.analysis => Some(ActionDisabledReason::Analysis),
        UiAction::RefreshProviderCatalog
        | UiAction::EstimateRecording
        | UiAction::PlanRecording
        | UiAction::ConfirmRecordingCapture
        | UiAction::ProgressRecording
        | UiAction::CancelRecording
        | UiAction::ReconcileRecording
        | UiAction::ActivateOfflineDefault
            if !caps.planning =>
        {
            Some(ActionDisabledReason::Planning)
        }
        UiAction::ToggleAllMeasures | UiAction::ToggleMeasure if !caps.planning => {
            Some(ActionDisabledReason::Planning)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Capabilities {
        Capabilities {
            analysis: true,
            artifacts: true,
            cancel: true,
            events: true,
            history: true,
            launch: true,
            planning: true,
            repeat: true,
        }
    }

    #[test]
    fn registry_is_complete_and_context_filtered() {
        let descriptors = ActionRegistry::for_context(Route::MeasurementSelection, Some(&all()));
        assert_eq!(
            descriptors
                .iter()
                .filter(|d| d.context == ActionContext::Global)
                .count(),
            10
        );
        assert!(
            descriptors
                .iter()
                .any(|d| d.action == UiAction::ToggleMeasure)
        );
        assert!(!descriptors.iter().any(|d| d.action == UiAction::StartRun));
        assert_eq!(UiAction::ALL.len(), 25);
        for action in UiAction::ALL {
            assert!(
                ActionRegistry::search(action.id())
                    .iter()
                    .any(|d| d.action == action),
                "missing metadata for {}",
                action.id()
            );
        }
    }

    #[test]
    fn disabled_descriptors_explain_negotiation_and_capabilities() {
        let descriptors = ActionRegistry::for_context(Route::MeasurementSelection, None);
        let measure = descriptors
            .iter()
            .find(|d| d.action == UiAction::ToggleMeasure)
            .unwrap();
        assert_eq!(measure.disabled, Some(ActionDisabledReason::NotNegotiated));
        let no_planning = Capabilities {
            planning: false,
            ..all()
        };
        let descriptors =
            ActionRegistry::for_context(Route::MeasurementSelection, Some(&no_planning));
        assert_eq!(
            descriptors
                .iter()
                .find(|d| d.action == UiAction::ToggleMeasure)
                .unwrap()
                .disabled,
            Some(ActionDisabledReason::Planning)
        );
    }

    #[test]
    fn help_search_is_case_insensitive_and_searches_descriptions() {
        assert!(
            ActionRegistry::search("REPORT")
                .iter()
                .any(|d| d.action == UiAction::OpenReports)
        );
        assert!(
            ActionRegistry::search("benchmark groups")
                .iter()
                .any(|d| d.action == UiAction::OpenMeasurementSelection)
        );
        assert_eq!(ActionRegistry::search(" ").len(), UiAction::ALL.len());
    }

    #[test]
    fn visible_keys_are_unique() {
        for route in [
            Route::Landing,
            Route::Configuration,
            Route::MeasurementSelection,
            Route::RunControl,
            Route::RecentRuns,
            Route::Reports,
            Route::Help,
        ] {
            let descriptors = ActionRegistry::for_context(route, Some(&all()));
            for (index, left) in descriptors.iter().enumerate() {
                assert!(
                    !descriptors[index + 1..]
                        .iter()
                        .any(|right| right.key == left.key),
                    "duplicate key {} on {:?}",
                    left.key.label(),
                    route
                );
            }
        }
    }

    #[test]
    fn labels_and_backend_gates_cover_every_action_variant() {
        for action in UiAction::ALL {
            assert!(!action.id().is_empty());
            let descriptor = ActionRegistry::search(action.id())
                .into_iter()
                .find(|item| item.action == action)
                .unwrap();
            assert!(!descriptor.label.is_empty());
            assert!(!descriptor.description.is_empty());
        }
        for chord in [
            KeyChord::Char('?'),
            KeyChord::Char('/'),
            KeyChord::Char('b'),
            KeyChord::Char('c'),
            KeyChord::Char('h'),
            KeyChord::Char('m'),
            KeyChord::Char('r'),
            KeyChord::Char('s'),
            KeyChord::Char('q'),
            KeyChord::Char('R'),
            KeyChord::Char('a'),
            KeyChord::Char('z'),
            KeyChord::Ctrl('c'),
            KeyChord::Ctrl('z'),
            KeyChord::Escape,
            KeyChord::Enter,
            KeyChord::Space,
        ] {
            assert!(!chord.label().is_empty());
        }
        for reason in [
            ActionDisabledReason::NotNegotiated,
            ActionDisabledReason::Analysis,
            ActionDisabledReason::Cancel,
            ActionDisabledReason::Events,
            ActionDisabledReason::History,
            ActionDisabledReason::Launch,
            ActionDisabledReason::Planning,
            ActionDisabledReason::WrongContext,
        ] {
            assert!(!reason.label().is_empty());
        }
        let no_capabilities = Capabilities {
            analysis: false,
            artifacts: false,
            cancel: false,
            events: false,
            history: false,
            launch: false,
            planning: false,
            repeat: false,
        };
        for route in [
            Route::RunControl,
            Route::RecentRuns,
            Route::Reports,
            Route::Help,
        ] {
            let descriptors = ActionRegistry::for_context(route, Some(&no_capabilities));
            assert!(descriptors.iter().any(|item| item.disabled.is_some()));
        }
    }
}
