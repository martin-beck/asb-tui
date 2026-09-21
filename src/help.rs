// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral contextual help and fitted hot-key state for AR-1032.

use crate::{
    Capabilities,
    actions::{ActionDescriptor, ActionRegistry},
    shell::Route,
};
use serde::Deserialize;
use std::sync::OnceLock;

const MAX_QUERY_BYTES: usize = 128;
const MAX_VISIBLE_ACTIONS: usize = 32;
const MAX_DOCUMENT_HELP_ENTRIES: usize = 128;
const MAX_DOCUMENT_HELP_ID_BYTES: usize = 128;
const MAX_DOCUMENT_HELP_TEXT_BYTES: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct DocumentHelpEntry {
    id: String,
    route: String,
    kind: String,
    label: String,
    help: DocumentHelpText,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct DocumentHelpText {
    summary: String,
    usage: String,
}

#[derive(Debug, Deserialize)]
struct HelpDocument {
    schema_version: u64,
    catalog: String,
    description: String,
    elements: Vec<DocumentHelpEntry>,
}

static DOCUMENT_HELP: OnceLock<Result<Box<[DocumentHelpEntry]>, ()>> = OnceLock::new();

fn document_help() -> &'static Result<Box<[DocumentHelpEntry]>, ()> {
    DOCUMENT_HELP.get_or_init(|| {
        let document: HelpDocument =
            serde_json::from_str(include_str!("../docs/ui-help.json")).map_err(|_| ())?;
        if document.schema_version != 1
            || document.catalog != "asb-tui.ui-help"
            || document.description.trim().is_empty()
            || document.elements.is_empty()
            || document.elements.len() > MAX_DOCUMENT_HELP_ENTRIES
        {
            return Err(());
        }
        let entries = document
            .elements
            .into_iter()
            .map(|mut entry| {
                let text = format!("{} {}", entry.help.summary, entry.help.usage);
                entry.help.summary = text;
                entry
            })
            .collect::<Vec<_>>();
        if entries.iter().any(|entry| {
            entry.id.is_empty()
                || entry.id.len() > MAX_DOCUMENT_HELP_ID_BYTES
                || entry.route.is_empty()
                || entry.kind.is_empty()
                || entry.label.is_empty()
                || entry.help.summary.split_whitespace().count() < 4
                || entry.help.summary.len() > MAX_DOCUMENT_HELP_TEXT_BYTES
                || entry.id.chars().any(char::is_control)
                || entry.help.summary.chars().any(char::is_control)
        }) {
            return Err(());
        }
        Ok(entries.into_boxed_slice())
    })
}

/// Document-backed contextual helper text for a stable UI element identifier.
///
/// The authoritative document-backed help catalog is embedded at build time,
/// so lookup has no file or environment dependency at runtime. Invalid or
/// over-sized catalog data fails closed as `None`; CI cross-checks that every
/// formal UI element has an entry in this catalog.
#[must_use]
pub fn document_help_text(element_id: &str) -> Option<&'static str> {
    document_help()
        .as_ref()
        .ok()?
        .iter()
        .find_map(|entry| (entry.id == element_id).then_some(entry.help.summary.as_str()))
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
    use super::document_help_text;
    use crate::actions::UiAction;

    #[test]
    fn document_catalog_resolves_known_elements_and_rejects_unknown_ids() {
        assert_eq!(
            document_help_text("measures.search"),
            Some(
                "Filter benchmark measures by group or name. Type a term to narrow the catalog, then move through matching measures and press Space to change selection."
            )
        );
        assert_eq!(document_help_text("missing.element"), None);
    }

    #[test]
    fn document_catalog_contains_meaningful_text_for_every_authored_entry() {
        for id in [
            "screen.landing",
            "measures.search",
            "measures.row",
            "measures.group_toggle",
            "configuration.entry",
            "configuration.search",
            "run.status",
            "run.start",
            "run.cancel",
            "runs.row",
            "runs.refresh",
            "reports.row",
            "reports.compare",
            "help.search",
            "help.entry",
            "navigation",
            "status.connection",
        ] {
            let text = document_help_text(id).expect("authored help entry");
            assert!(text.split_whitespace().count() >= 4);
        }
    }

    #[test]
    fn document_catalog_covers_every_registered_action() {
        for action in UiAction::ALL {
            let id = format!("action.{}", action.id());
            let text = document_help_text(&id).expect("registered action help entry");
            assert!(text.split_whitespace().count() >= 4, "short help for {id}");
        }
    }
}
