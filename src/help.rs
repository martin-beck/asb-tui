// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral contextual help and fitted hot-key state for AR-1032.

use crate::{
    Capabilities,
    actions::{ActionDescriptor, ActionRegistry, UiAction},
    shell::Route,
};

const MAX_QUERY_BYTES: usize = 128;
const MAX_VISIBLE_ACTIONS: usize = 32;

/// Stable identifiers for focusable/hoverable workspace regions.
///
/// These identifiers are intentionally independent of Ratatui widget values
/// or screen coordinates, so a renderer can resolve help after a resize or a
/// layout change. `MeasureRow` denotes the currently focused catalog row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiElement {
    LandingPrimary,
    MeasureSearch,
    MeasureRow,
    ConfigurationEntry,
    RecentRun,
    ReportRun,
    HelpSearch,
    Navigation,
}

impl UiElement {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::LandingPrimary => "landing.primary",
            Self::MeasureSearch => "measures.search",
            Self::MeasureRow => "measures.row",
            Self::ConfigurationEntry => "configuration.entry",
            Self::RecentRun => "recent_runs.row",
            Self::ReportRun => "reports.row",
            Self::HelpSearch => "help.search",
            Self::Navigation => "navigation",
        }
    }
}

/// The catalog entry resolved for the currently focused or hovered element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextualHelp {
    pub element: UiElement,
    pub title: &'static str,
    pub description: &'static str,
    pub actions: Vec<ActionDescriptor>,
}

/// Bounded help state. Rendering and terminal input remain outside this module.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HelpModel {
    query: String,
    selected: usize,
}

impl HelpModel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the search query, rejecting control characters and oversized input.
    pub fn set_query(&mut self, query: &str) -> Result<(), HelpError> {
        if query.len() > MAX_QUERY_BYTES || !query.chars().all(|c| !c.is_control()) {
            return Err(HelpError::InvalidQuery);
        }
        self.query = query.to_owned();
        self.selected = 0;
        Ok(())
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Return all matching actions in deterministic order, bounded for rendering.
    #[must_use]
    pub fn entries(
        &self,
        route: Route,
        capabilities: Option<&Capabilities>,
    ) -> Vec<ActionDescriptor> {
        let mut entries = if self.query.is_empty() {
            ActionRegistry::for_context(route, capabilities)
        } else {
            ActionRegistry::search(&self.query)
        };
        entries.truncate(MAX_VISIBLE_ACTIONS);
        entries
    }

    /// Resolve the help catalog for one stable UI element in the current
    /// route. Action descriptors retain capability-disabled explanations.
    #[must_use]
    pub fn contextual_help(
        &self,
        route: Route,
        capabilities: Option<&Capabilities>,
        element: UiElement,
    ) -> ContextualHelp {
        let (title, description, actions): (_, _, &[UiAction]) = match element {
            UiElement::LandingPrimary => (
                "Landing primary action",
                "Open the next recommended benchmark workspace.",
                &[UiAction::OpenMeasurementSelection, UiAction::OpenReports],
            ),
            UiElement::MeasureSearch => (
                "Measure search",
                "Type to filter the authoritative measurement catalog.",
                &[UiAction::FocusSearch],
            ),
            UiElement::MeasureRow => (
                "Focused measure",
                "Select or deselect this measure; use group selection for its visible group.",
                &[UiAction::ToggleMeasure, UiAction::ToggleAllMeasures],
            ),
            UiElement::ConfigurationEntry => (
                "Configuration entry",
                "Inspect or edit the focused setting, then save a validated configuration.",
                &[UiAction::OpenConfiguration],
            ),
            UiElement::RecentRun => (
                "Recent benchmark run",
                "Open a run summary and refresh the authoritative recent-run projection.",
                &[UiAction::OpenRecentRuns, UiAction::RefreshRuns],
            ),
            UiElement::ReportRun => (
                "Report run",
                "Select runs for an evidence-backed comparison.",
                &[UiAction::CompareRuns, UiAction::OpenReports],
            ),
            UiElement::HelpSearch => (
                "Help search",
                "Search action names, shortcuts, and descriptions.",
                &[UiAction::FocusSearch, UiAction::GoBack],
            ),
            UiElement::Navigation => (
                "Workspace navigation",
                "Move between the landing, measures, configuration, and reports screens.",
                &[
                    UiAction::OpenLanding,
                    UiAction::OpenMeasurementSelection,
                    UiAction::OpenConfiguration,
                    UiAction::OpenReports,
                ],
            ),
        };
        let available = ActionRegistry::for_context(route, capabilities);
        let actions = available
            .into_iter()
            .filter(|descriptor| actions.contains(&descriptor.action))
            .collect();
        ContextualHelp {
            element,
            title,
            description,
            actions,
        }
    }

    /// Select the next entry, wrapping within the current visible set.
    pub fn move_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.rem_euclid(count as isize) as usize;
    }

    #[must_use]
    pub fn selected(&self, count: usize) -> usize {
        if count == 0 {
            0
        } else {
            self.selected.min(count - 1)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpError {
    InvalidQuery,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contextual_catalog_resolves_element_and_actions() {
        let model = HelpModel::new();
        let help = model.contextual_help(Route::MeasurementSelection, None, UiElement::MeasureRow);
        assert_eq!(help.element.id(), "measures.row");
        assert!(help.description.contains("deselect"));
        let toggle = help
            .actions
            .iter()
            .find(|item| item.action == UiAction::ToggleMeasure)
            .expect("focused measure action");
        assert_eq!(
            toggle.disabled,
            Some(crate::actions::ActionDisabledReason::NotNegotiated)
        );
    }

    #[test]
    fn contextual_catalog_keeps_element_ids_stable() {
        let expected = [
            (UiElement::LandingPrimary, "landing.primary"),
            (UiElement::MeasureSearch, "measures.search"),
            (UiElement::MeasureRow, "measures.row"),
            (UiElement::ConfigurationEntry, "configuration.entry"),
            (UiElement::RecentRun, "recent_runs.row"),
            (UiElement::ReportRun, "reports.row"),
            (UiElement::HelpSearch, "help.search"),
            (UiElement::Navigation, "navigation"),
        ];
        for (element, id) in expected {
            assert_eq!(element.id(), id);
        }
    }
}
