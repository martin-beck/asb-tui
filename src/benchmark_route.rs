// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral state machine for the guided benchmark journey.
//!
//! The TUI owns selection and intent only.  ASB remains the authority for
//! execution, recording, replay and report data.  In particular, offline
//! replay is an explicit mode and can never silently become a live run.

use crate::reports::{CompareError, Comparison, Report, RunId, compare};

const MAX_ITEMS: usize = 64;
const MAX_TEXT: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayMode {
    Live,
    Record,
    OfflineReplay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CampaignStage {
    Selection,
    Review,
    Running,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuidedCampaign {
    pub workload: String,
    pub agents: Vec<String>,
    pub measures: Vec<String>,
    pub replay: ReplayMode,
    pub stage: CampaignStage,
    catalog: Option<GuidedCatalog>,
}

/// The validated catalog projection supplied by ASB. The route never accepts
/// an identifier that is absent from this catalog when one is attached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuidedCatalog {
    pub workloads: Vec<String>,
    pub agents: Vec<String>,
    pub measures: Vec<String>,
}

impl GuidedCatalog {
    pub fn new(
        workloads: Vec<String>,
        agents: Vec<String>,
        measures: Vec<String>,
    ) -> Result<Self, CampaignError> {
        if workloads.is_empty()
            || agents.is_empty()
            || measures.is_empty()
            || workloads.len() > MAX_ITEMS
            || agents.len() > MAX_ITEMS
            || measures.len() > MAX_ITEMS
        {
            return Err(CampaignError::MissingWorkload);
        }
        for value in workloads.iter().chain(agents.iter()).chain(measures.iter()) {
            if bounded(value.clone())?.is_empty() {
                return Err(CampaignError::InvalidText);
            }
        }
        Ok(Self {
            workloads,
            agents,
            measures,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CampaignError {
    MissingWorkload,
    MissingAgent,
    MissingMeasure,
    TooManyItems,
    InvalidText,
    InvalidTransition,
    OfflineRecordingRequired,
    LiveFallbackForbidden,
    Comparison(CompareError),
}

impl GuidedCampaign {
    /// Start a new bounded selection-driven campaign. No backend call occurs.
    pub fn new(workload: impl Into<String>) -> Result<Self, CampaignError> {
        let workload = bounded(workload.into())?;
        if workload.is_empty() {
            return Err(CampaignError::MissingWorkload);
        }
        Ok(Self {
            workload,
            agents: Vec::new(),
            measures: Vec::new(),
            replay: ReplayMode::Live,
            stage: CampaignStage::Selection,
            catalog: None,
        })
    }

    pub fn with_catalog(
        workload: impl Into<String>,
        catalog: GuidedCatalog,
    ) -> Result<Self, CampaignError> {
        let mut campaign = Self::new(workload)?;
        if !catalog
            .workloads
            .iter()
            .any(|item| item == &campaign.workload)
        {
            return Err(CampaignError::MissingWorkload);
        }
        campaign.catalog = Some(catalog);
        Ok(campaign)
    }

    pub fn add_agent(&mut self, agent: impl Into<String>) -> Result<(), CampaignError> {
        self.ensure_selection()?;
        let agent = bounded(agent.into())?;
        if agent.is_empty() {
            return Err(CampaignError::MissingAgent);
        }
        if self
            .catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.agents.iter().any(|item| item == &agent))
        {
            return Err(CampaignError::MissingAgent);
        }
        push_unique(&mut self.agents, agent)
    }

    pub fn add_measure(&mut self, measure: impl Into<String>) -> Result<(), CampaignError> {
        self.ensure_selection()?;
        let measure = bounded(measure.into())?;
        if measure.is_empty() {
            return Err(CampaignError::MissingMeasure);
        }
        if self
            .catalog
            .as_ref()
            .is_some_and(|catalog| !catalog.measures.iter().any(|item| item == &measure))
        {
            return Err(CampaignError::MissingMeasure);
        }
        push_unique(&mut self.measures, measure)
    }

    pub fn set_replay_mode(&mut self, replay: ReplayMode) -> Result<(), CampaignError> {
        self.ensure_selection()?;
        self.replay = replay;
        Ok(())
    }

    pub fn review(&mut self) -> Result<(), CampaignError> {
        self.ensure_selection()?;
        if self.agents.is_empty() {
            return Err(CampaignError::MissingAgent);
        }
        if self.measures.is_empty() {
            return Err(CampaignError::MissingMeasure);
        }
        self.stage = CampaignStage::Review;
        Ok(())
    }

    /// Produce a backend-neutral launch intent. The caller must dispatch this
    /// through the existing ASB control seam; this method performs no I/O.
    pub fn start(&mut self) -> Result<LaunchIntent, CampaignError> {
        if self.stage != CampaignStage::Review {
            return Err(CampaignError::InvalidTransition);
        }
        if self.replay == ReplayMode::OfflineReplay {
            return Err(CampaignError::OfflineRecordingRequired);
        }
        self.stage = CampaignStage::Running;
        Ok(LaunchIntent {
            workload: self.workload.clone(),
            agents: self.agents.clone(),
            measures: self.measures.clone(),
            replay: self.replay,
        })
    }

    /// Produce an explicit strict-replay intent. There is no live fallback.
    pub fn start_offline_replay(
        &mut self,
        recording_id: impl Into<String>,
    ) -> Result<ReplayIntent, CampaignError> {
        if self.stage != CampaignStage::Review || self.replay != ReplayMode::OfflineReplay {
            return Err(CampaignError::InvalidTransition);
        }
        let recording_id = bounded(recording_id.into())?;
        if recording_id.is_empty() {
            return Err(CampaignError::OfflineRecordingRequired);
        }
        self.stage = CampaignStage::Running;
        Ok(ReplayIntent {
            workload: self.workload.clone(),
            agents: self.agents.clone(),
            measures: self.measures.clone(),
            recording_id,
            offline_only: true,
        })
    }

    pub fn finish(&mut self, success: bool) -> Result<(), CampaignError> {
        if self.stage != CampaignStage::Running {
            return Err(CampaignError::InvalidTransition);
        }
        self.stage = if success {
            CampaignStage::Complete
        } else {
            CampaignStage::Failed
        };
        Ok(())
    }

    pub fn compare(
        &self,
        reports: &[Report],
        selected: &[RunId],
    ) -> Result<Comparison, CampaignError> {
        if self.stage != CampaignStage::Complete {
            return Err(CampaignError::InvalidTransition);
        }
        compare(reports, selected).map_err(CampaignError::Comparison)
    }

    fn ensure_selection(&self) -> Result<(), CampaignError> {
        (self.stage == CampaignStage::Selection)
            .then_some(())
            .ok_or(CampaignError::InvalidTransition)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchIntent {
    pub workload: String,
    pub agents: Vec<String>,
    pub measures: Vec<String>,
    pub replay: ReplayMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayIntent {
    pub workload: String,
    pub agents: Vec<String>,
    pub measures: Vec<String>,
    pub recording_id: String,
    pub offline_only: bool,
}

fn bounded(value: String) -> Result<String, CampaignError> {
    if value.len() > MAX_TEXT || value.chars().any(char::is_control) {
        return Err(CampaignError::InvalidText);
    }
    Ok(value)
}

fn push_unique(values: &mut Vec<String>, value: String) -> Result<(), CampaignError> {
    if values.iter().any(|item| item == &value) {
        return Ok(());
    }
    if values.len() >= MAX_ITEMS {
        return Err(CampaignError::TooManyItems);
    }
    values.push(value);
    Ok(())
}
