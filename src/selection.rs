// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral measurement selection state.
//!
//! This module deliberately contains no terminal or widget types.  It owns the
//! bounded, deterministic choice model that a renderer can project into a
//! measurement picker.

use std::collections::BTreeSet;

pub const MAX_ID_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 256;
pub const MAX_QUERY_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    NonPublicText(&'static str),
    DuplicateId,
    UnknownMeasure,
    UnknownGroup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Measurement {
    id: String,
    group: String,
    name: String,
    unit: String,
}

impl Measurement {
    pub fn new(
        id: impl Into<String>,
        group: impl Into<String>,
        name: impl Into<String>,
        unit: impl Into<String>,
    ) -> Result<Self, SelectionError> {
        let measurement = Self {
            id: id.into(),
            group: group.into(),
            name: name.into(),
            unit: unit.into(),
        };
        validate_field(&measurement.id, "id", MAX_ID_BYTES)?;
        validate_field(&measurement.group, "group", MAX_TEXT_BYTES)?;
        validate_field(&measurement.name, "name", MAX_TEXT_BYTES)?;
        validate_field(&measurement.unit, "unit", MAX_TEXT_BYTES)?;
        Ok(measurement)
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn group(&self) -> &str {
        &self.group
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupSelection {
    None,
    Partial,
    All,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupSummary {
    group: String,
    visible_count: usize,
    selected_count: usize,
    state: GroupSelection,
}

impl GroupSummary {
    #[must_use]
    pub fn group(&self) -> &str {
        &self.group
    }
    #[must_use]
    pub const fn visible_count(&self) -> usize {
        self.visible_count
    }
    #[must_use]
    pub const fn selected_count(&self) -> usize {
        self.selected_count
    }
    #[must_use]
    pub const fn state(&self) -> GroupSelection {
        self.state
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeasurementSelection {
    measurements: Vec<Measurement>,
    selected: BTreeSet<String>,
    query: String,
}

impl MeasurementSelection {
    /// Construct canonical state. Measurements are sorted by group then ID.
    pub fn new(mut measurements: Vec<Measurement>) -> Result<Self, SelectionError> {
        let mut ids = BTreeSet::new();
        if measurements
            .iter()
            .any(|measurement| !ids.insert(measurement.id.clone()))
        {
            return Err(SelectionError::DuplicateId);
        }
        measurements.sort_by(|left, right| {
            left.group
                .cmp(&right.group)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(Self {
            measurements,
            selected: BTreeSet::new(),
            query: String::new(),
        })
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> Result<(), SelectionError> {
        let query = query.into();
        // The public query bound is expressed in Unicode scalar values so
        // non-ASCII search text is not penalized for its UTF-8 width.
        if query.chars().count() > MAX_QUERY_BYTES {
            return Err(SelectionError::FieldTooLong("query"));
        }
        if query.chars().any(char::is_control) {
            return Err(SelectionError::NonPublicText("query"));
        }
        self.query = query;
        Ok(())
    }

    #[must_use]
    pub fn measurements(&self) -> &[Measurement] {
        &self.measurements
    }

    /// Visible order is canonical catalog order, filtered case-insensitively.
    #[must_use]
    pub fn visible_measurements(&self) -> Vec<&Measurement> {
        let query = self.query.to_lowercase();
        self.measurements
            .iter()
            .filter(|measurement| matches_query(measurement, &query))
            .collect()
    }

    #[must_use]
    pub fn is_selected(&self, id: &str) -> bool {
        self.selected.contains(id)
    }

    pub fn set_measure_selected(&mut self, id: &str, selected: bool) -> Result<(), SelectionError> {
        if !self
            .measurements
            .iter()
            .any(|measurement| measurement.id == id)
        {
            return Err(SelectionError::UnknownMeasure);
        }
        if selected {
            self.selected.insert(id.to_owned());
        } else {
            self.selected.remove(id);
        }
        Ok(())
    }

    /// Set every measure in a group, irrespective of the active search query.
    pub fn set_group_selected(
        &mut self,
        group: &str,
        selected: bool,
    ) -> Result<(), SelectionError> {
        if !self
            .measurements
            .iter()
            .any(|measurement| measurement.group == group)
        {
            return Err(SelectionError::UnknownGroup);
        }
        for measurement in self
            .measurements
            .iter()
            .filter(|measurement| measurement.group == group)
        {
            if selected {
                self.selected.insert(measurement.id.clone());
            } else {
                self.selected.remove(&measurement.id);
            }
        }
        Ok(())
    }

    /// Set only the currently visible measures in a group. Selections hidden
    /// by a search remain untouched.
    pub fn set_visible_group_selected(
        &mut self,
        group: &str,
        selected: bool,
    ) -> Result<(), SelectionError> {
        if !self
            .measurements
            .iter()
            .any(|measurement| measurement.group == group)
        {
            return Err(SelectionError::UnknownGroup);
        }
        let query = self.query.to_lowercase();
        for measurement in self
            .measurements
            .iter()
            .filter(|measurement| measurement.group == group && matches_query(measurement, &query))
        {
            if selected {
                self.selected.insert(measurement.id.clone());
            } else {
                self.selected.remove(&measurement.id);
            }
        }
        Ok(())
    }

    /// Summaries include only visible measures, while selected counts remain
    /// restricted to those visible measures so tri-state controls are stable
    /// under search.
    #[must_use]
    pub fn visible_groups(&self) -> Vec<GroupSummary> {
        let mut summaries: Vec<GroupSummary> = Vec::new();
        for measurement in self.visible_measurements() {
            if let Some(summary) = summaries.last_mut()
                && summary.group == measurement.group
            {
                summary.visible_count += 1;
                summary.selected_count += usize::from(self.is_selected(&measurement.id));
                summary.state = group_state(summary.selected_count, summary.visible_count);
                continue;
            }
            let selected_count = usize::from(self.is_selected(&measurement.id));
            summaries.push(GroupSummary {
                group: measurement.group.clone(),
                visible_count: 1,
                selected_count,
                state: group_state(selected_count, 1),
            });
        }
        summaries
    }

    #[must_use]
    pub fn selected_ids(&self) -> Vec<&str> {
        self.measurements
            .iter()
            .filter_map(|measurement| {
                self.selected
                    .contains(&measurement.id)
                    .then_some(measurement.id.as_str())
            })
            .collect()
    }
}

fn group_state(selected: usize, visible: usize) -> GroupSelection {
    match selected {
        0 => GroupSelection::None,
        value if value == visible => GroupSelection::All,
        _ => GroupSelection::Partial,
    }
}

fn matches_query(measurement: &Measurement, query: &str) -> bool {
    query.is_empty()
        || measurement.id.to_ascii_lowercase().contains(query)
        || measurement.group.to_ascii_lowercase().contains(query)
        || measurement.name.to_ascii_lowercase().contains(query)
        || measurement.unit.to_ascii_lowercase().contains(query)
}

fn validate_field(value: &str, field: &'static str, limit: usize) -> Result<(), SelectionError> {
    if value.is_empty() {
        return Err(SelectionError::EmptyField(field));
    }
    if value.len() > limit {
        return Err(SelectionError::FieldTooLong(field));
    }
    if !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(SelectionError::NonPublicText(field));
    }
    Ok(())
}
