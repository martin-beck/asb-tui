// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral state machine for the guided benchmark journey.
//!
//! The TUI owns selection and intent only.  ASB remains the authority for
//! execution, recording, replay and report data.  In particular, offline
//! replay is an explicit mode and can never silently become a live run.

use crate::control_codec::Revision;
use crate::reports::{CompareError, Comparison, Report, RunId, compare};
use crate::selection::CampaignSelection;

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
    provider_profile_sha256: Option<String>,
    adapter_id: Option<String>,
}

/// The validated catalog projection supplied by ASB. The route never accepts
/// an identifier that is absent from this catalog when one is attached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuidedCatalog {
    pub workloads: Vec<String>,
    pub agents: Vec<String>,
    pub measures: Vec<String>,
}

/// The privacy-safe replay choices published by the authenticated runner.
///
/// The route receives compatibility metadata only; cassette contents never
/// enter the frontend state.  A replay can therefore be selected only from
/// the exact provider/agent catalog offered by ASB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayCatalog {
    pub provider_profile_sha256: String,
    pub agent_id: String,
    pub compatible: Vec<ReplayChoice>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReplayChoice {
    pub cassette_id: String,
    pub cassette_sha256: String,
}

/// Authenticated v1.12 cassette metadata returned by ASB.  Only identities
/// and digests cross into the frontend; cassette contents remain runner-owned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedCassetteCatalog {
    pub runner_instance_id: String,
    pub generation: Revision,
    pub campaign_id: String,
    pub entries: Vec<AuthenticatedCassetteEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedCassetteEntry {
    pub cassette_id: String,
    pub cassette_sha256: String,
    pub provider_profile_sha256: String,
    pub agent_id: String,
    pub workload_id: String,
    pub scorer_revision: String,
}

impl TryFrom<crate::control_codec::RecordingCassetteCatalog> for AuthenticatedCassetteCatalog {
    type Error = CampaignError;

    fn try_from(
        value: crate::control_codec::RecordingCassetteCatalog,
    ) -> Result<Self, Self::Error> {
        if value.generation.0 == 0
            || !valid_identifier(&value.runner_instance_id)
            || !valid_identifier(&value.campaign_id)
            || value.entries.is_empty()
            || value.entries.len() > MAX_ITEMS * 4
        {
            return Err(CampaignError::InvalidReplayCatalog);
        }
        let entries = value
            .entries
            .into_iter()
            .map(|entry| {
                if !valid_identifier(&entry.cassette_id)
                    || !valid_digest(&entry.cassette_sha256)
                    || !valid_digest(&entry.provider_profile_sha256)
                    || !valid_identifier(&entry.agent_id)
                    || !valid_identifier(&entry.workload_id)
                    || !valid_identifier(&entry.scorer_revision)
                {
                    return Err(CampaignError::InvalidReplayCatalog);
                }
                Ok(AuthenticatedCassetteEntry {
                    cassette_id: entry.cassette_id,
                    cassette_sha256: entry.cassette_sha256,
                    provider_profile_sha256: entry.provider_profile_sha256,
                    agent_id: entry.agent_id,
                    workload_id: entry.workload_id,
                    scorer_revision: entry.scorer_revision,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if entries.windows(2).any(|pair| {
            (
                pair[0].agent_id.as_str(),
                pair[0].workload_id.as_str(),
                pair[0].cassette_sha256.as_str(),
            ) >= (
                pair[1].agent_id.as_str(),
                pair[1].workload_id.as_str(),
                pair[1].cassette_sha256.as_str(),
            )
        }) {
            return Err(CampaignError::InvalidReplayCatalog);
        }
        Ok(Self {
            runner_instance_id: value.runner_instance_id,
            generation: value.generation,
            campaign_id: value.campaign_id,
            entries,
        })
    }
}

impl ReplayCatalog {
    pub fn new(
        provider_profile_sha256: impl Into<String>,
        agent_id: impl Into<String>,
        mut compatible: Vec<ReplayChoice>,
    ) -> Result<Self, CampaignError> {
        let provider_profile_sha256 = provider_profile_sha256.into();
        let agent_id = agent_id.into();
        if !valid_digest(&provider_profile_sha256) || !valid_identifier(&agent_id) {
            return Err(CampaignError::InvalidReplayCatalog);
        }
        compatible.sort();
        if compatible.windows(2).any(|pair| pair[0] == pair[1])
            || compatible.iter().any(|choice| {
                !valid_identifier(&choice.cassette_id) || !valid_digest(&choice.cassette_sha256)
            })
        {
            return Err(CampaignError::InvalidReplayCatalog);
        }
        Ok(Self {
            provider_profile_sha256,
            agent_id,
            compatible,
        })
    }
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
    StaleCatalog,
    EmptyCatalogSelection,
    InvalidReplayCatalog,
    ReplayUnavailable,
    ReplayAgentMismatch,
    ProviderProfileNotBound,
    ProviderProfileMismatch,
    ReplayGenerationMismatch,
    ReplayCampaignMismatch,
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
            provider_profile_sha256: None,
            adapter_id: None,
        })
    }

    /// Bind the exact authenticated provider profile selected by setup.
    /// Replay cannot proceed until this identity is present.
    pub fn bind_provider_profile_sha256(
        &mut self,
        provider_profile_sha256: impl Into<String>,
    ) -> Result<(), CampaignError> {
        let provider_profile_sha256 = provider_profile_sha256.into();
        if !valid_digest(&provider_profile_sha256) {
            return Err(CampaignError::InvalidReplayCatalog);
        }
        self.provider_profile_sha256 = Some(provider_profile_sha256);
        Ok(())
    }

    /// Bind the stable coding-agent adapter identity to every downstream route.
    pub fn bind_adapter_id(&mut self, adapter_id: impl Into<String>) -> Result<(), CampaignError> {
        let adapter_id = adapter_id.into();
        if !valid_identifier(&adapter_id) {
            return Err(CampaignError::InvalidText);
        }
        if crate::adapter_catalog::AdapterCatalog::development()
            .get(&adapter_id)
            .is_none()
        {
            return Err(CampaignError::InvalidText);
        }
        self.adapter_id = Some(adapter_id);
        Ok(())
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
            catalog_generation: None,
            catalog_digest: None,
            benchmark_ids: Vec::new(),
            adapter_id: self.adapter_id.clone(),
        })
    }

    /// Review and start a campaign from the exact nested benchmark-picker
    /// handoff. The generation and digest remain attached to the launch intent
    /// so a downstream dispatcher cannot silently substitute a refreshed
    /// catalog.
    pub fn start_with_handoff(
        &mut self,
        handoff: CampaignSelection,
        expected_generation: Revision,
    ) -> Result<LaunchIntent, CampaignError> {
        self.ensure_selection()?;
        if handoff.generation != expected_generation {
            return Err(CampaignError::StaleCatalog);
        }
        if handoff.catalog_digest.is_empty() || handoff.measure_ids.is_empty() {
            return Err(CampaignError::EmptyCatalogSelection);
        }
        self.measures = handoff.measure_ids.clone();
        self.review()?;
        self.stage = CampaignStage::Running;
        Ok(LaunchIntent {
            workload: self.workload.clone(),
            agents: self.agents.clone(),
            measures: self.measures.clone(),
            replay: self.replay,
            catalog_generation: Some(handoff.generation),
            catalog_digest: Some(handoff.catalog_digest),
            benchmark_ids: handoff.benchmark_ids,
            adapter_id: self.adapter_id.clone(),
        })
    }

    /// Produce an explicit strict-replay intent. There is no live fallback.
    fn start_offline_replay_selected(
        &mut self,
        recording_id: String,
    ) -> Result<ReplayIntent, CampaignError> {
        if self.stage != CampaignStage::Review || self.replay != ReplayMode::OfflineReplay {
            return Err(CampaignError::InvalidTransition);
        }
        let recording_id = bounded(recording_id)?;
        if recording_id.is_empty() {
            return Err(CampaignError::OfflineRecordingRequired);
        }
        self.stage = CampaignStage::Running;
        Ok(ReplayIntent {
            workload: self.workload.clone(),
            agents: self.agents.clone(),
            measures: self.measures.clone(),
            recording_id,
            cassette_sha256: None,
            provider_profile_sha256: None,
            offline_only: true,
            campaign_id: None,
            generation: None,
            adapter_id: self.adapter_id.clone(),
        })
    }

    /// Start replay only after selecting an exact runner-offered cassette.
    /// There is no live fallback when the catalog is stale or incompatible.
    pub fn start_offline_replay_from_catalog(
        &mut self,
        catalog: &ReplayCatalog,
        cassette_sha256: &str,
    ) -> Result<ReplayIntent, CampaignError> {
        if self.agents.len() != 1 || self.agents[0] != catalog.agent_id {
            return Err(CampaignError::ReplayAgentMismatch);
        }
        let provider_profile_sha256 = self
            .provider_profile_sha256
            .as_deref()
            .ok_or(CampaignError::ProviderProfileNotBound)?;
        if provider_profile_sha256 != catalog.provider_profile_sha256 {
            return Err(CampaignError::ProviderProfileMismatch);
        }
        let choice = catalog
            .compatible
            .iter()
            .find(|choice| choice.cassette_sha256 == cassette_sha256)
            .ok_or(CampaignError::ReplayUnavailable)?;
        let mut intent = self.start_offline_replay_selected(choice.cassette_id.clone())?;
        intent.cassette_sha256 = Some(choice.cassette_sha256.clone());
        intent.provider_profile_sha256 = Some(catalog.provider_profile_sha256.clone());
        Ok(intent)
    }

    /// Select an exact v1.12 runner cassette, preserving campaign generation
    /// and tuple identity. The returned intent is still only a request; the
    /// authenticated session must dispatch it through ASB's replay authority.
    pub fn start_offline_replay_from_authenticated_catalog(
        &mut self,
        catalog: &AuthenticatedCassetteCatalog,
        workload_id: &str,
        cassette_sha256: &str,
    ) -> Result<ReplayIntent, CampaignError> {
        if catalog.generation.0 == 0 || catalog.campaign_id.is_empty() {
            return Err(CampaignError::InvalidReplayCatalog);
        }
        if self.agents.len() != 1 {
            return Err(CampaignError::ReplayAgentMismatch);
        }
        let profile = self
            .provider_profile_sha256
            .as_deref()
            .ok_or(CampaignError::ProviderProfileNotBound)?;
        let entry = catalog
            .entries
            .iter()
            .find(|entry| {
                entry.agent_id == self.agents[0]
                    && entry.workload_id == workload_id
                    && entry.cassette_sha256 == cassette_sha256
                    && entry.provider_profile_sha256 == profile
            })
            .ok_or(CampaignError::ReplayUnavailable)?;
        let mut intent = self.start_offline_replay_selected(entry.cassette_id.clone())?;
        intent.cassette_sha256 = Some(entry.cassette_sha256.clone());
        intent.provider_profile_sha256 = Some(entry.provider_profile_sha256.clone());
        intent.campaign_id = Some(catalog.campaign_id.clone());
        intent.generation = Some(catalog.generation);
        intent.workload = workload_id.to_owned();
        Ok(intent)
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
    pub catalog_generation: Option<Revision>,
    pub catalog_digest: Option<String>,
    pub benchmark_ids: Vec<String>,
    pub adapter_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayIntent {
    pub workload: String,
    pub agents: Vec<String>,
    pub measures: Vec<String>,
    pub recording_id: String,
    pub cassette_sha256: Option<String>,
    pub provider_profile_sha256: Option<String>,
    pub offline_only: bool,
    pub campaign_id: Option<String>,
    pub generation: Option<Revision>,
    pub adapter_id: Option<String>,
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
