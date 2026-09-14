// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Versioned, renderer-independent description of the TUI state graph.
//!
//! The manifest is intentionally data-only.  It gives screen implementations
//! a stable inventory of routes and elements, and gives CI a deterministic
//! place to validate references and reachability before a renderer is changed.

use serde::{Deserialize, Serialize};

/// Current manifest schema version.
pub const SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UiModel {
    pub schema_version: u64,
    pub routes: Vec<RouteSpec>,
    pub elements: Vec<ElementSpec>,
    pub transitions: Vec<TransitionSpec>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteSpec {
    pub id: String,
    pub title: String,
    pub entry: bool,
    pub availability: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ElementSpec {
    pub id: String,
    pub route: String,
    pub kind: String,
    pub label: String,
    pub help_id: String,
    pub visibility: String,
    pub state_bindings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionSpec {
    pub id: String,
    pub from: String,
    pub event: String,
    pub to: String,
    pub guard: String,
    pub effects: Vec<String>,
}

impl UiModel {
    /// Return the checked-in model for the current application shell.
    #[must_use]
    pub fn current() -> Self {
        let routes = [
            ("landing", "Landing", true, "always"),
            ("configuration", "Configuration", false, "always"),
            (
                "measurement_selection",
                "Measurement selection",
                false,
                "planning",
            ),
            ("run_control", "Run control", false, "launch+cancel+events"),
            ("recent_runs", "Recent runs", false, "history"),
            ("reports", "Reports", false, "analysis+history"),
            ("help", "Help", false, "always"),
        ]
        .into_iter()
        .map(|(id, title, entry, availability)| RouteSpec {
            id: id.into(),
            title: title.into(),
            entry,
            availability: availability.into(),
        })
        .collect();

        let elements = [
            (
                "landing.primary_action",
                "landing",
                "action",
                "Next action",
                "landing.primary_action",
                "always",
                &["primary_action"] as &[&str],
            ),
            (
                "landing.recent_runs",
                "landing",
                "list",
                "Recent runs",
                "landing.recent_runs",
                "always",
                &["recent_activity"],
            ),
            (
                "landing.destinations",
                "landing",
                "list",
                "Workspaces",
                "landing.destinations",
                "always",
                &["route_availability"],
            ),
            (
                "configuration.provider",
                "configuration",
                "field",
                "Provider",
                "configuration.provider",
                "always",
                &["provider"],
            ),
            (
                "configuration.model",
                "configuration",
                "field",
                "Model",
                "configuration.model",
                "always",
                &["model"],
            ),
            (
                "configuration.defaults",
                "configuration",
                "field",
                "Benchmark defaults",
                "configuration.defaults",
                "always",
                &["defaults"],
            ),
            (
                "measurement_selection.search",
                "measurement_selection",
                "search",
                "Filter measures",
                "measurement_selection.search",
                "always",
                &["query"],
            ),
            (
                "measurement_selection.groups",
                "measurement_selection",
                "list",
                "Measure groups",
                "measurement_selection.groups",
                "always",
                &["group_selection"],
            ),
            (
                "measurement_selection.measures",
                "measurement_selection",
                "list",
                "Measures",
                "measurement_selection.measures",
                "always",
                &["selected_ids", "cursor"],
            ),
            (
                "run_control.status",
                "run_control",
                "status",
                "Run status",
                "run_control.status",
                "always",
                &["run_status"],
            ),
            (
                "run_control.controls",
                "run_control",
                "actions",
                "Run controls",
                "run_control.controls",
                "always",
                &["launch", "cancel"],
            ),
            (
                "recent_runs.runs",
                "recent_runs",
                "list",
                "Recent runs",
                "recent_runs.runs",
                "always",
                &["runs", "cursor"],
            ),
            (
                "reports.summary",
                "reports",
                "panel",
                "Report summary",
                "reports.summary",
                "always",
                &["report"],
            ),
            (
                "reports.comparison",
                "reports",
                "panel",
                "Comparison",
                "reports.comparison",
                "always",
                &["comparison"],
            ),
            (
                "help.search",
                "help",
                "search",
                "Search help",
                "help.search",
                "always",
                &["query"],
            ),
            (
                "help.entries",
                "help",
                "list",
                "Available actions",
                "help.entries",
                "always",
                &["entries", "cursor"],
            ),
        ]
        .into_iter()
        .map(
            |(id, route, kind, label, help_id, visibility, bindings)| ElementSpec {
                id: id.into(),
                route: route.into(),
                kind: kind.into(),
                label: label.into(),
                help_id: help_id.into(),
                visibility: visibility.into(),
                state_bindings: bindings
                    .iter()
                    .map(|binding| (*binding).to_owned())
                    .collect(),
            },
        )
        .collect();

        let mut transitions = Vec::new();
        for (from, event, to) in [
            ("landing", "open_configuration", "configuration"),
            (
                "landing",
                "open_measurement_selection",
                "measurement_selection",
            ),
            ("landing", "open_help", "help"),
            ("configuration", "open_landing", "landing"),
            (
                "configuration",
                "open_measurement_selection",
                "measurement_selection",
            ),
            ("configuration", "open_help", "help"),
            ("measurement_selection", "open_landing", "landing"),
            ("measurement_selection", "open_run_control", "run_control"),
            ("measurement_selection", "open_help", "help"),
            ("run_control", "open_landing", "landing"),
            ("run_control", "open_recent_runs", "recent_runs"),
            ("run_control", "open_help", "help"),
            ("recent_runs", "open_landing", "landing"),
            ("recent_runs", "open_reports", "reports"),
            ("recent_runs", "open_help", "help"),
            ("reports", "open_landing", "landing"),
            ("reports", "open_help", "help"),
            ("help", "go_back", "landing"),
        ] {
            transitions.push(TransitionSpec {
                id: format!("{from}.{event}"),
                from: from.into(),
                event: event.into(),
                to: to.into(),
                guard: "route_available".into(),
                effects: vec!["route_changed".into(), "focus_reset".into()],
            });
        }
        Self {
            schema_version: SCHEMA_VERSION,
            routes,
            elements,
            transitions,
        }
    }

    /// Validate the model as a closed graph with deterministic diagnostics.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.schema_version != SCHEMA_VERSION {
            errors.push(format!(
                "unsupported schema version {}",
                self.schema_version
            ));
        }
        let route_ids: std::collections::BTreeSet<_> =
            self.routes.iter().map(|r| r.id.as_str()).collect();
        if route_ids.len() != self.routes.len() {
            errors.push("duplicate route id".into());
        }
        if self.routes.iter().filter(|r| r.entry).count() != 1 {
            errors.push("model must have exactly one entry route".into());
        }
        let Some(entry) = self.routes.iter().find(|r| r.entry).map(|r| r.id.as_str()) else {
            errors.push("missing entry route".into());
            return Err(errors);
        };
        let mut element_ids = std::collections::BTreeSet::new();
        for element in &self.elements {
            if !route_ids.contains(element.route.as_str()) {
                errors.push(format!(
                    "element {} references unknown route {}",
                    element.id, element.route
                ));
            }
            if element.label.trim().is_empty() || element.help_id.trim().is_empty() {
                errors.push(format!("element {} lacks label/help id", element.id));
            }
            if element.state_bindings.is_empty() {
                errors.push(format!("element {} lacks state binding", element.id));
            }
            if !element_ids.insert(element.id.as_str()) {
                errors.push(format!("duplicate element id {}", element.id));
            }
        }
        for route in &route_ids {
            if !self.elements.iter().any(|e| e.route == *route) {
                errors.push(format!("route {} has no elements", route));
            }
        }
        let mut transition_ids = std::collections::BTreeSet::new();
        let mut reachable = std::collections::BTreeSet::from([entry]);
        for transition in &self.transitions {
            if !route_ids.contains(transition.from.as_str()) {
                errors.push(format!("transition {} has unknown source", transition.id));
            }
            if !route_ids.contains(transition.to.as_str()) {
                errors.push(format!("transition {} has unknown target", transition.id));
            }
            if transition.event.trim().is_empty()
                || transition.guard.trim().is_empty()
                || transition.effects.is_empty()
            {
                errors.push(format!("transition {} is incomplete", transition.id));
            }
            if !transition_ids.insert(transition.id.as_str()) {
                errors.push(format!("duplicate transition id {}", transition.id));
            }
            if reachable.contains(transition.from.as_str()) {
                reachable.insert(transition.to.as_str());
            }
        }
        for route in &route_ids {
            if !reachable.contains(route) {
                errors.push(format!("route {} is unreachable from {}", route, entry));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Serialize the model with stable indentation for review and CI checks.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_model_is_closed_and_reachable() {
        UiModel::current().validate().unwrap();
    }

    #[test]
    fn validator_rejects_unknown_element_route() {
        let mut model = UiModel::current();
        model.elements[0].route = "missing".into();
        let errors = model.validate().unwrap_err().join("; ");
        assert!(errors.contains("unknown route"));
    }

    #[test]
    fn json_round_trip_is_deterministic() {
        let model = UiModel::current();
        let json = model.to_json().unwrap();
        assert_eq!(UiModel::current(), serde_json::from_str(&json).unwrap());
        assert!(json.contains("\"schema_version\": 1"));
    }
}
