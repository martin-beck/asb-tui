// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded executable interpreter for the documented TUI state graph.
//!
//! This layer has no renderer dependency. It consumes the checked-in JSON
//! model, applies one event at a time, and rejects transitions whose route or
//! element is not explicitly documented or whose capability guard is closed.

use crate::{
    Capabilities,
    app::AppError,
    shell::{ConnectionState, Route},
};
use serde::Deserialize;

const MODEL_JSON: &str = include_str!("../docs/ui-state-model.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct Document {
    schema_version: u64,
    model: String,
    initial: String,
    routes: Vec<DocumentRoute>,
    elements: Vec<DocumentElement>,
    transitions: Vec<DocumentTransition>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct DocumentRoute {
    id: String,
    elements: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct DocumentElement {
    id: String,
    route: String,
    role: String,
    help_id: String,
    focusable: bool,
    hoverable: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct DocumentTransition {
    event: String,
    from: String,
    to: String,
    effects: Vec<String>,
}

/// Events represented by the formal model.  Screen-specific actions can be
/// added only by extending this closed enum and the document together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormalEvent {
    OpenLanding,
    OpenConfiguration,
    OpenMeasurementSelection,
    OpenRunControl,
    OpenRecentRuns,
    OpenReports,
    OpenHelp,
    GoBack,
    SaveConfiguration,
    Resize { columns: u16, lines: u16 },
    Focus(&'static str),
}

impl FormalEvent {
    const fn id(self) -> Option<&'static str> {
        match self {
            Self::OpenLanding => Some("open_landing"),
            Self::OpenConfiguration => Some("open_configuration"),
            Self::OpenMeasurementSelection => Some("open_measurement_selection"),
            Self::OpenRunControl => Some("open_run_control"),
            Self::OpenRecentRuns => Some("open_recent_runs"),
            Self::OpenReports => Some("open_reports"),
            Self::OpenHelp => Some("open_help"),
            Self::GoBack => Some("go_back"),
            Self::SaveConfiguration => Some("save_configuration"),
            Self::Resize { .. } | Self::Focus(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormalError {
    InvalidModel(String),
    UnknownElement(String),
    ElementNotFocusable(String),
    InvalidDimensions(AppError),
    UnknownTransition { route: Route, event: String },
    RouteUnavailable(Route),
}

/// The executable state required to check route, focus, and underlying state
/// changes. Fields are intentionally bounded and renderer-independent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormalUiState {
    route: Route,
    return_route: Route,
    focus: Option<String>,
    columns: u16,
    lines: u16,
    connection: ConnectionState,
}

impl FormalUiState {
    pub fn new(columns: u16, lines: u16) -> Result<Self, FormalError> {
        validate_model().map_err(FormalError::InvalidModel)?;
        crate::app::AppState::new(columns, lines).map_err(FormalError::InvalidDimensions)?;
        Ok(Self {
            route: Route::Landing,
            return_route: Route::Landing,
            focus: None,
            columns,
            lines,
            connection: ConnectionState::Disconnected,
        })
    }

    pub fn apply(
        &mut self,
        event: FormalEvent,
        capabilities: Option<&Capabilities>,
    ) -> Result<(), FormalError> {
        let document = parse_model().map_err(FormalError::InvalidModel)?;
        match event {
            FormalEvent::Resize { columns, lines } => {
                crate::app::AppState::new(columns, lines)
                    .map_err(FormalError::InvalidDimensions)?;
                self.columns = columns;
                self.lines = lines;
            }
            FormalEvent::Focus(element) => {
                let id = element.to_owned();
                let Some(item) = document.elements.iter().find(|item| {
                    item.id == id && (item.route == route_id(self.route) || item.route == "global")
                }) else {
                    return Err(FormalError::UnknownElement(id));
                };
                if !item.focusable {
                    return Err(FormalError::ElementNotFocusable(id));
                }
                self.focus = Some(id);
            }
            event => {
                let event_id = event.id().expect("navigation events have IDs");
                let from = route_id(self.route);
                let Some(transition) = document
                    .transitions
                    .iter()
                    .find(|item| item.from == from && item.event == event_id)
                else {
                    return Err(FormalError::UnknownTransition {
                        route: self.route,
                        event: event_id.to_owned(),
                    });
                };
                let expected_effects: &[&str] = if event == FormalEvent::SaveConfiguration {
                    &["configuration_persisted", "focus_reset"]
                } else if event == FormalEvent::GoBack {
                    &["route_restored", "focus_reset"]
                } else if event == FormalEvent::OpenHelp {
                    &["return_route_saved", "route_changed", "focus_reset"]
                } else {
                    &["route_changed", "focus_reset"]
                };
                if transition
                    .effects
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    != expected_effects
                {
                    return Err(FormalError::InvalidModel(format!(
                        "effects for {} do not match interpreter semantics",
                        transition.event
                    )));
                }
                let destination = route_from_id(&transition.to).ok_or_else(|| {
                    FormalError::InvalidModel(format!("unknown target {}", transition.to))
                })?;
                if !destination.available_or_unnegotiated(capabilities) {
                    return Err(FormalError::RouteUnavailable(destination));
                }
                if destination == Route::Help {
                    self.return_route = self.route;
                }
                self.route = if event == FormalEvent::GoBack && self.route == Route::Help {
                    self.return_route
                } else {
                    destination
                };
                self.focus = None;
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn route(&self) -> Route {
        self.route
    }
    #[must_use]
    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }
    #[must_use]
    pub const fn size(&self) -> (u16, u16) {
        (self.columns, self.lines)
    }
    #[must_use]
    pub const fn connection(&self) -> ConnectionState {
        self.connection
    }
}

fn parse_model() -> Result<Document, String> {
    serde_json::from_str(MODEL_JSON).map_err(|error| format!("invalid UI model: {error}"))
}

fn validate_model() -> Result<(), String> {
    let model = parse_model()?;
    validate_document(&model)
}

fn validate_document(model: &Document) -> Result<(), String> {
    if model.schema_version != 1 || model.model != "asb-tui.ui-state" {
        return Err("unsupported UI model identity".into());
    }
    if model.initial != "landing" || model.routes.is_empty() {
        return Err("invalid UI model entry".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for route in &model.routes {
        if !ids.insert(route.id.as_str()) {
            return Err(format!("duplicate route {}", route.id));
        }
    }
    for element in &model.elements {
        if element.id.is_empty() || element.help_id.trim().is_empty() || element.role.is_empty() {
            return Err(format!("incomplete element {}", element.id));
        }
        if !model
            .routes
            .iter()
            .any(|route| route.elements.iter().any(|id| id == &element.id))
        {
            return Err(format!("element {} is not placed", element.id));
        }
    }
    for route in &model.routes {
        for element in &route.elements {
            if element != "navigation" && !model.elements.iter().any(|item| item.id == *element) {
                return Err(format!(
                    "route {} references unknown element {}",
                    route.id, element
                ));
            }
        }
    }
    for transition in &model.transitions {
        if !ids.contains(transition.from.as_str())
            || !ids.contains(transition.to.as_str())
            || transition.event.is_empty()
            || transition.effects.is_empty()
        {
            return Err(format!("invalid transition {}", transition.event));
        }
        let expected = if transition.event == "save_configuration" {
            ["configuration_persisted", "focus_reset"].as_slice()
        } else if matches!(
            transition.event.as_str(),
            "edit_configuration" | "backspace_configuration"
        ) {
            ["configuration_draft_changed"].as_slice()
        } else if transition.event == "commit_configuration_edit" {
            ["configuration_draft_changed", "focus_reset"].as_slice()
        } else if matches!(transition.event.as_str(), "wizard_next" | "wizard_back") {
            ["wizard_step_changed", "focus_reset"].as_slice()
        } else if transition.event == "wizard_set_value" {
            ["wizard_draft_changed", "focus_reset"].as_slice()
        } else if matches!(
            transition.event.as_str(),
            "wizard_catalog_query" | "wizard_catalog_move"
        ) {
            ["wizard_catalog_changed", "focus_reset"].as_slice()
        } else if matches!(
            transition.event.as_str(),
            "wizard_catalog_select" | "wizard_catalog_select_all_agents"
        ) {
            [
                "wizard_catalog_changed",
                "wizard_draft_changed",
                "focus_reset",
            ]
            .as_slice()
        } else if transition.event == "go_back" {
            ["route_restored", "focus_reset"].as_slice()
        } else if transition.event == "open_help" {
            ["return_route_saved", "route_changed", "focus_reset"].as_slice()
        } else {
            ["route_changed", "focus_reset"].as_slice()
        };
        if transition
            .effects
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != expected
        {
            return Err(format!(
                "invalid effects for transition {}",
                transition.event
            ));
        }
    }
    let mut reached = std::collections::BTreeSet::from([model.initial.as_str()]);
    loop {
        let before = reached.len();
        for transition in &model.transitions {
            if reached.contains(transition.from.as_str()) {
                reached.insert(transition.to.as_str());
            }
        }
        if reached.len() == before {
            break;
        }
    }
    if reached.len() != model.routes.len() {
        return Err("UI model contains unreachable routes".into());
    }
    Ok(())
}

fn route_id(route: Route) -> &'static str {
    match route {
        Route::Landing => "landing",
        Route::Configuration => "configuration",
        Route::MeasurementSelection => "measurement_selection",
        Route::RunControl => "run_control",
        Route::RecentRuns => "recent_runs",
        Route::Reports => "reports",
        Route::Help => "help",
    }
}
fn route_from_id(id: &str) -> Option<Route> {
    Some(match id {
        "landing" => Route::Landing,
        "configuration" => Route::Configuration,
        "measurement_selection" => Route::MeasurementSelection,
        "run_control" => Route::RunControl,
        "recent_runs" => Route::RecentRuns,
        "reports" => Route::Reports,
        "help" => Route::Help,
        _ => return None,
    })
}
trait RouteAvailability {
    fn available_or_unnegotiated(self, capabilities: Option<&Capabilities>) -> bool;
}
impl RouteAvailability for Route {
    fn available_or_unnegotiated(self, capabilities: Option<&Capabilities>) -> bool {
        matches!(
            self.availability(capabilities),
            crate::shell::RouteAvailability::Available
        )
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
    fn interpreter_reaches_every_documented_route_and_returns_from_help() {
        let mut state = FormalUiState::new(80, 24).unwrap();
        state.apply(FormalEvent::OpenConfiguration, None).unwrap();
        state
            .apply(FormalEvent::OpenMeasurementSelection, Some(&all()))
            .unwrap();
        state
            .apply(FormalEvent::OpenRunControl, Some(&all()))
            .unwrap();
        state
            .apply(FormalEvent::OpenRecentRuns, Some(&all()))
            .unwrap();
        state.apply(FormalEvent::OpenReports, Some(&all())).unwrap();
        state.apply(FormalEvent::OpenHelp, Some(&all())).unwrap();
        assert_eq!(state.route(), Route::Help);
        state.apply(FormalEvent::GoBack, Some(&all())).unwrap();
        assert_eq!(state.route(), Route::Reports);
    }

    #[test]
    fn interpreter_rejects_capability_gated_route_without_mutation() {
        let mut state = FormalUiState::new(80, 24).unwrap();
        let before = state.clone();
        let capabilities = Capabilities {
            planning: false,
            ..all()
        };
        assert_eq!(
            state.apply(FormalEvent::OpenMeasurementSelection, Some(&capabilities)),
            Err(FormalError::RouteUnavailable(Route::MeasurementSelection))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn interpreter_preserves_focus_only_for_documented_current_elements() {
        let mut state = FormalUiState::new(80, 24).unwrap();
        state
            .apply(FormalEvent::Focus("landing.primary"), None)
            .unwrap();
        assert_eq!(state.focus(), Some("landing.primary"));
        assert!(matches!(
            state.apply(FormalEvent::Focus("reports.compare"), None),
            Err(FormalError::UnknownElement(_))
        ));
    }

    #[test]
    fn interpreter_rejects_documented_but_non_focusable_elements() {
        let mut state = FormalUiState::new(80, 24).unwrap();
        let before = state.clone();
        assert_eq!(
            state.apply(FormalEvent::Focus("status.connection"), None),
            Err(FormalError::ElementNotFocusable("status.connection".into()))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn resize_is_atomic_and_rejects_zero_dimensions() {
        let mut state = FormalUiState::new(80, 24).unwrap();
        let before = state.clone();
        assert!(
            state
                .apply(
                    FormalEvent::Resize {
                        columns: 0,
                        lines: 24
                    },
                    None
                )
                .is_err()
        );
        assert_eq!(state, before);
    }

    #[test]
    fn model_rejects_missing_or_wrong_transition_effects() {
        let mut document = parse_model().unwrap();
        document.transitions[0].effects.clear();
        assert!(validate_document(&document).is_err());
        let mut document = parse_model().unwrap();
        document.transitions[0].effects = vec!["route_changed".into()];
        assert!(validate_document(&document).is_err());
    }
}
