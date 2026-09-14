// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral projections for recent runs, reports, and comparisons.
//!
//! This module deliberately contains no terminal or widget code.  It is the
//! bounded state and validation seam that a renderer and the authenticated ASB
//! client can consume once the history protocol is available.

use std::collections::{BTreeMap, BTreeSet};

pub const MAX_PAGE_RUNS: usize = 100;
pub const MAX_RETAINED_PAGES: usize = 10;
pub const MAX_COMPARE_RUNS: usize = 256;
pub const MAX_MEASURES: usize = 512;
pub const MAX_TEXT_BYTES: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct RunId(String);

impl RunId {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_ascii_text("run_id", &value, 128)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Incomplete,
    Corrupt,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunSource {
    Live,
    LiveRecording,
    StrictReplay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Integrity {
    Verified,
    Unverified,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    pub id: RunId,
    /// Authoritative ASB time, in milliseconds since the Unix epoch.
    pub started_at_ms: u64,
    /// Opaque server cursor associated with this record.
    pub cursor: String,
    pub status: RunStatus,
    pub source: RunSource,
    pub platform: String,
    pub agents: Vec<String>,
    pub workload: String,
    pub integrity: Integrity,
    pub concise_result: Option<String>,
}

impl RunRecord {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_ascii_text("cursor", &self.cursor, 256)?;
        validate_ascii_text("platform", &self.platform, MAX_TEXT_BYTES)?;
        validate_ascii_text("workload", &self.workload, MAX_TEXT_BYTES)?;
        if self.agents.len() > 64 {
            return Err(ValidationError::TooMany("agents"));
        }
        for agent in &self.agents {
            validate_ascii_text("agent", agent, MAX_TEXT_BYTES)?;
        }
        if let Some(result) = &self.concise_result {
            validate_ascii_text("concise_result", result, MAX_TEXT_BYTES)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentPage {
    pub request_cursor: Option<String>,
    pub runs: Vec<RunRecord>,
    pub next_cursor: Option<String>,
    pub next_started_at_ms: Option<u64>,
}

impl RecentPage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.runs.len() > MAX_PAGE_RUNS {
            return Err(ValidationError::TooMany("page runs"));
        }
        if self.request_cursor.as_ref().is_some_and(|v| v.is_empty()) {
            return Err(ValidationError::ContradictoryCursor);
        }
        let mut ids = BTreeSet::new();
        let mut previous: Option<(u64, &RunId)> = None;
        for run in &self.runs {
            run.validate()?;
            if !ids.insert(run.id.clone()) {
                return Err(ValidationError::DuplicateRun);
            }
            if let Some((time, id)) = previous {
                // Newest first; IDs are the deterministic tie breaker.
                if (run.started_at_ms, &run.id) > (time, id) {
                    return Err(ValidationError::UnorderedPage);
                }
            }
            previous = Some((run.started_at_ms, &run.id));
        }
        match (
            &self.next_cursor,
            &self.next_started_at_ms,
            self.runs.last(),
        ) {
            (Some(cursor), Some(next_time), Some(last))
                if !cursor.is_empty() && *next_time <= last.started_at_ms =>
            {
                Ok(())
            }
            (None, None, _) | (Some(_), Some(_), None) => {
                if self.next_cursor.is_some() {
                    Err(ValidationError::ContradictoryCursor)
                } else {
                    Ok(())
                }
            }
            _ => Err(ValidationError::ContradictoryCursor),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecentRuns {
    pages: BTreeMap<String, RecentPage>,
    pub filter: String,
    pub selected: BTreeSet<RunId>,
}

impl RecentRuns {
    pub fn replace_page(
        &mut self,
        key: impl Into<String>,
        page: RecentPage,
    ) -> Result<(), ValidationError> {
        page.validate()?;
        let key = key.into();
        if key.is_empty() {
            return Err(ValidationError::ContradictoryCursor);
        }
        let incoming: BTreeSet<_> = page.runs.iter().map(|run| run.id.clone()).collect();
        if self
            .pages
            .iter()
            .filter(|(existing_key, _)| existing_key.as_str() != key)
            .flat_map(|(_, existing)| existing.runs.iter().map(|run| &run.id))
            .any(|id| incoming.contains(id))
        {
            return Err(ValidationError::DuplicateRun);
        }
        self.pages.insert(key, page);
        while self.pages.len() > MAX_RETAINED_PAGES {
            let first = self.pages.keys().next().cloned().unwrap();
            self.pages.remove(&first);
        }
        self.prune_selection();
        Ok(())
    }

    pub fn pages(&self) -> impl Iterator<Item = &RecentPage> {
        self.pages.values()
    }

    pub fn visible_runs(&self) -> Vec<&RunRecord> {
        let query = self.filter.to_ascii_lowercase();
        let mut runs: Vec<_> = self
            .pages
            .values()
            .flat_map(|p| p.runs.iter())
            .filter(|r| {
                query.is_empty()
                    || r.id.as_str().to_ascii_lowercase().contains(&query)
                    || r.workload.to_ascii_lowercase().contains(&query)
                    || r.platform.to_ascii_lowercase().contains(&query)
            })
            .collect();
        runs.sort_by(|a, b| {
            b.started_at_ms
                .cmp(&a.started_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        runs
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) -> Result<(), ValidationError> {
        let filter = filter.into();
        if filter.len() > MAX_TEXT_BYTES || filter.chars().any(char::is_control) {
            return Err(ValidationError::InvalidText("filter"));
        }
        self.filter = filter;
        Ok(())
    }

    pub fn select(&mut self, id: &RunId, selected: bool) -> Result<(), ValidationError> {
        if !self
            .pages
            .values()
            .any(|p| p.runs.iter().any(|r| &r.id == id))
        {
            return Err(ValidationError::UnknownRun);
        }
        if selected {
            self.selected.insert(id.clone());
        } else {
            self.selected.remove(id);
        }
        Ok(())
    }

    pub fn prune_selection(&mut self) {
        let available: BTreeSet<_> = self
            .pages
            .values()
            .flat_map(|p| p.runs.iter().map(|r| r.id.clone()))
            .collect();
        self.selected.retain(|id| available.contains(id));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub run_id: RunId,
    pub provenance: String,
    pub measures: BTreeMap<String, MeasureResult>,
    pub uncertainty: Option<String>,
    pub failures: Vec<String>,
    pub artifacts: Vec<Artifact>,
    pub command_argv: Vec<String>,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeasureResult {
    pub value: Option<f64>,
    pub unit: String,
    pub status: MeasureStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeasureStatus {
    Complete,
    Missing,
    Failed,
    Uncertain,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Artifact {
    pub name: String,
    pub available: bool,
}

impl Report {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_ascii_text("provenance", &self.provenance, MAX_TEXT_BYTES)?;
        if self.measures.len() > MAX_MEASURES {
            return Err(ValidationError::TooMany("measures"));
        }
        if self.command_argv.len() > 128
            || self
                .command_argv
                .iter()
                .any(|v| v.is_empty() || v.contains('\0'))
        {
            return Err(ValidationError::InvalidCommand);
        }
        for failure in &self.failures {
            validate_ascii_text("failure", failure, MAX_TEXT_BYTES)?;
        }
        Ok(())
    }

    pub fn is_conclusive(&self) -> bool {
        !self.stale
            && !self.measures.is_empty()
            && self.failures.is_empty()
            && self
                .measures
                .values()
                .all(|m| matches!(m.status, MeasureStatus::Complete) && m.value.is_some())
    }

    pub fn shell_command(&self) -> Result<String, ValidationError> {
        self.validate()?;
        if self.command_argv.iter().enumerate().any(|(index, arg)| {
            let lower = arg.to_ascii_lowercase();
            arg.starts_with('/')
                || arg.contains('$')
                || lower.contains("token")
                || lower.contains("password")
                || lower.contains("secret")
                || lower.contains("credential")
                || (index > 0 && self.command_argv[index - 1].starts_with("--") && arg.len() > 96)
        }) {
            return Err(ValidationError::SensitiveCommand);
        }
        Ok(self
            .command_argv
            .iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" "))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Comparison {
    pub run_ids: Vec<RunId>,
    pub compatible: bool,
    pub confounders: Vec<String>,
    pub excluded_measures: Vec<String>,
}

pub fn compare(reports: &[Report], selected: &[RunId]) -> Result<Comparison, CompareError> {
    if !(2..=MAX_COMPARE_RUNS).contains(&selected.len()) {
        return Err(CompareError::Cardinality);
    }
    let mut ids = BTreeSet::new();
    if selected.iter().any(|id| !ids.insert(id)) {
        return Err(CompareError::Duplicate);
    }
    let by_id: BTreeMap<_, _> = reports.iter().map(|r| (&r.run_id, r)).collect();
    let mut chosen = Vec::with_capacity(selected.len());
    for id in selected {
        let report = by_id.get(id).ok_or(CompareError::StaleOrDeleted)?;
        if report.stale {
            return Err(CompareError::StaleOrDeleted);
        }
        report.validate().map_err(CompareError::InvalidReport)?;
        chosen.push(*report);
    }
    let provenance = chosen[0].provenance.as_str();
    let mut confounders = Vec::new();
    if chosen.iter().any(|r| r.provenance != provenance) {
        confounders.push("provenance differs".into());
    }
    let common: BTreeSet<_> =
        chosen
            .iter()
            .skip(1)
            .fold(chosen[0].measures.keys().cloned().collect(), |set, r| {
                set.intersection(&r.measures.keys().cloned().collect())
                    .cloned()
                    .collect()
            });
    let all: BTreeSet<_> = chosen
        .iter()
        .flat_map(|r| r.measures.keys().cloned())
        .collect();
    let excluded_measures = all.difference(&common).cloned().collect();
    Ok(Comparison {
        run_ids: selected.to_vec(),
        compatible: confounders.is_empty() && chosen.iter().all(|r| r.is_conclusive()),
        confounders,
        excluded_measures,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    TooMany(&'static str),
    DuplicateRun,
    UnorderedPage,
    ContradictoryCursor,
    UnknownRun,
    InvalidText(&'static str),
    InvalidCommand,
    NonPublicText(&'static str),
    SensitiveCommand,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompareError {
    Cardinality,
    Duplicate,
    StaleOrDeleted,
    InvalidReport(ValidationError),
}

fn validate_ascii_text(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max {
        return Err(ValidationError::InvalidText(field));
    }
    if !value.is_ascii() || value.chars().any(char::is_control) {
        return Err(ValidationError::NonPublicText(field));
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/:=@%+".contains(&b))
    {
        return value.into();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}
