// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral contextual help and fitted hot-key state for AR-1032.

use crate::{
    Capabilities,
    actions::{ActionDescriptor, ActionRegistry},
    shell::Route,
};

const MAX_QUERY_BYTES: usize = 128;
const MAX_VISIBLE_ACTIONS: usize = 32;

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
