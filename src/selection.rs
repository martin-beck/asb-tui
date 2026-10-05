// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral measurement selection state.
//!
//! This module deliberately contains no terminal or widget types.  It owns the
//! bounded, deterministic choice model that a renderer can project into a
//! measurement picker.

use std::collections::BTreeSet;

use crate::control_codec::Revision;

pub const MAX_ID_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 256;
pub const MAX_QUERY_BYTES: usize = 256;
pub const MAX_MEASUREMENTS: usize = 128;
pub const MAX_POOLS: usize = 16;
pub const MAX_BENCHMARK_GROUPS: usize = 64;
pub const MAX_BENCHMARKS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    NonPublicText(&'static str),
    DuplicateId,
    TooManyMeasurements,
    UnknownMeasure,
    UnknownGroup,
    UnknownPool,
    UnknownBenchmark,
    StaleCatalog,
    NoSelection,
    UnavailableMeasure,
}

/// A measure as published by the benchmark catalog. IDs are globally unique
/// so a campaign handoff cannot accidentally alias two nested nodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkMeasure {
    id: String,
    name: String,
    unit: String,
    available: bool,
}

impl BenchmarkMeasure {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        unit: impl Into<String>,
        available: bool,
    ) -> Result<Self, SelectionError> {
        let value = Self {
            id: id.into(),
            name: name.into(),
            unit: unit.into(),
            available,
        };
        validate_field(&value.id, "measure_id", MAX_ID_BYTES)?;
        validate_field(&value.name, "measure_name", MAX_TEXT_BYTES)?;
        validate_field(&value.unit, "measure_unit", MAX_TEXT_BYTES)?;
        Ok(value)
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }
    #[must_use]
    pub const fn available(&self) -> bool {
        self.available
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkDefinition {
    id: String,
    name: String,
    measures: Vec<BenchmarkMeasure>,
}

impl BenchmarkDefinition {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        measures: Vec<BenchmarkMeasure>,
    ) -> Result<Self, SelectionError> {
        let value = Self {
            id: id.into(),
            name: name.into(),
            measures,
        };
        validate_field(&value.id, "benchmark_id", MAX_ID_BYTES)?;
        validate_field(&value.name, "benchmark_name", MAX_TEXT_BYTES)?;
        if value.measures.is_empty() || value.measures.len() > MAX_MEASUREMENTS {
            return Err(SelectionError::TooManyMeasurements);
        }
        unique_measure_ids(&value.measures)?;
        Ok(value)
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn measures(&self) -> &[BenchmarkMeasure] {
        &self.measures
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkGroup {
    id: String,
    name: String,
    benchmarks: Vec<BenchmarkDefinition>,
}

impl BenchmarkGroup {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        benchmarks: Vec<BenchmarkDefinition>,
    ) -> Result<Self, SelectionError> {
        let value = Self {
            id: id.into(),
            name: name.into(),
            benchmarks,
        };
        validate_field(&value.id, "group_id", MAX_ID_BYTES)?;
        validate_field(&value.name, "group_name", MAX_TEXT_BYTES)?;
        if value.benchmarks.is_empty() {
            return Err(SelectionError::UnknownBenchmark);
        }
        unique_strings(value.benchmarks.iter().map(|v| v.id.as_str()))?;
        Ok(value)
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn benchmarks(&self) -> &[BenchmarkDefinition] {
        &self.benchmarks
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkPool {
    id: String,
    name: String,
    groups: Vec<BenchmarkGroup>,
}

impl BenchmarkPool {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        groups: Vec<BenchmarkGroup>,
    ) -> Result<Self, SelectionError> {
        let value = Self {
            id: id.into(),
            name: name.into(),
            groups,
        };
        validate_field(&value.id, "pool_id", MAX_ID_BYTES)?;
        validate_field(&value.name, "pool_name", MAX_TEXT_BYTES)?;
        if value.groups.is_empty() {
            return Err(SelectionError::UnknownGroup);
        }
        unique_strings(value.groups.iter().map(|v| v.id.as_str()))?;
        Ok(value)
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn groups(&self) -> &[BenchmarkGroup] {
        &self.groups
    }
}

/// Immutable, generation-bound nested catalog used by the picker and handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkCatalog {
    generation: Revision,
    digest: String,
    pools: Vec<BenchmarkPool>,
}

impl BenchmarkCatalog {
    pub fn new(
        generation: Revision,
        digest: impl Into<String>,
        pools: Vec<BenchmarkPool>,
    ) -> Result<Self, SelectionError> {
        if generation.0 == 0 || pools.is_empty() || pools.len() > MAX_POOLS {
            return Err(SelectionError::UnknownPool);
        }
        let digest = digest.into();
        validate_field(&digest, "catalog_digest", MAX_ID_BYTES)?;
        unique_strings(pools.iter().map(|v| v.id.as_str()))?;
        let mut measure_ids = BTreeSet::new();
        let mut benchmark_ids = BTreeSet::new();
        let mut group_count = 0;
        let mut benchmark_count = 0;
        for pool in &pools {
            group_count += pool.groups.len();
            for group in &pool.groups {
                for benchmark in &group.benchmarks {
                    benchmark_count += 1;
                    if !benchmark_ids.insert(benchmark.id()) {
                        return Err(SelectionError::DuplicateId);
                    }
                    for measure in &benchmark.measures {
                        if !measure_ids.insert(measure.id()) {
                            return Err(SelectionError::DuplicateId);
                        }
                    }
                }
            }
        }
        if group_count > MAX_BENCHMARK_GROUPS || benchmark_count > MAX_BENCHMARKS {
            return Err(SelectionError::TooManyMeasurements);
        }
        Ok(Self {
            generation,
            digest,
            pools,
        })
    }
    #[must_use]
    pub const fn generation(&self) -> Revision {
        self.generation
    }
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
    #[must_use]
    pub fn pools(&self) -> &[BenchmarkPool] {
        &self.pools
    }
    fn benchmark(&self, id: &str) -> Option<&BenchmarkDefinition> {
        self.pools
            .iter()
            .flat_map(|p| p.groups.iter())
            .flat_map(|g| g.benchmarks.iter())
            .find(|b| b.id() == id)
    }
    fn all_measure_ids(&self) -> impl Iterator<Item = &str> {
        self.pools
            .iter()
            .flat_map(|p| p.groups.iter())
            .flat_map(|g| g.benchmarks.iter())
            .flat_map(|b| b.measures.iter())
            .map(|m| m.id())
    }

    /// Validate a previously reviewed handoff against this exact catalog.
    /// This closes the boundary for callers that receive a serialized
    /// campaign rather than constructing it through `campaign_handoff`.
    pub fn validate_campaign(&self, campaign: &CampaignSelection) -> Result<(), SelectionError> {
        if campaign.generation != self.generation || campaign.catalog_digest != self.digest {
            return Err(SelectionError::StaleCatalog);
        }
        let pool = self
            .pools
            .iter()
            .find(|pool| pool.id() == campaign.pool_id)
            .ok_or(SelectionError::UnknownPool)?;
        let groups = pool
            .groups
            .iter()
            .map(|group| group.id())
            .collect::<BTreeSet<_>>();
        for id in &campaign.group_ids {
            if !groups.contains(&id.as_str()) {
                return Err(SelectionError::UnknownGroup);
            }
        }
        for id in &campaign.benchmark_ids {
            if !pool
                .groups
                .iter()
                .flat_map(|g| g.benchmarks())
                .any(|b| b.id() == id)
            {
                return Err(SelectionError::UnknownBenchmark);
            }
        }
        let measures = pool
            .groups
            .iter()
            .flat_map(|g| g.benchmarks())
            .flat_map(|b| b.measures())
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        for id in &campaign.measure_ids {
            if !seen.insert(id.as_str()) {
                return Err(SelectionError::DuplicateId);
            }
            let measure = measures
                .iter()
                .find(|measure| measure.id() == id)
                .ok_or(SelectionError::UnknownMeasure)?;
            if !measure.available() {
                return Err(SelectionError::UnavailableMeasure);
            }
        }
        if campaign.measure_ids.is_empty() {
            return Err(SelectionError::NoSelection);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeSelection {
    None,
    Partial,
    All,
}

fn node_state(selected: usize, visible: usize) -> NodeSelection {
    match selected {
        0 => NodeSelection::None,
        value if value == visible && visible > 0 => NodeSelection::All,
        _ => NodeSelection::Partial,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CampaignSelection {
    pub generation: Revision,
    pub catalog_digest: String,
    pub pool_id: String,
    pub group_ids: Vec<String>,
    pub benchmark_ids: Vec<String>,
    pub measure_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkSelection {
    catalog: BenchmarkCatalog,
    pool_id: String,
    selected_benchmarks: BTreeSet<String>,
    selected_measures: BTreeSet<String>,
    explicit_all: bool,
    query: String,
}

impl BenchmarkSelection {
    pub fn new(catalog: BenchmarkCatalog) -> Result<Self, SelectionError> {
        let pool_id = catalog
            .pools
            .first()
            .map(|p| p.id().to_owned())
            .ok_or(SelectionError::UnknownPool)?;
        Ok(Self {
            catalog,
            pool_id,
            selected_benchmarks: BTreeSet::new(),
            selected_measures: BTreeSet::new(),
            explicit_all: false,
            query: String::new(),
        })
    }
    #[must_use]
    pub fn catalog(&self) -> &BenchmarkCatalog {
        &self.catalog
    }
    #[must_use]
    pub fn pool_id(&self) -> &str {
        &self.pool_id
    }
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn set_query(&mut self, query: impl Into<String>) -> Result<(), SelectionError> {
        let query = query.into();
        if query.chars().count() > MAX_QUERY_BYTES {
            return Err(SelectionError::FieldTooLong("query"));
        }
        if query.chars().any(char::is_control) {
            return Err(SelectionError::NonPublicText("query"));
        }
        self.query = query;
        Ok(())
    }
    pub fn select_pool(&mut self, id: &str) -> Result<(), SelectionError> {
        if !self.catalog.pools.iter().any(|p| p.id() == id) {
            return Err(SelectionError::UnknownPool);
        }
        self.pool_id = id.to_owned();
        self.selected_benchmarks.clear();
        self.selected_measures.clear();
        self.explicit_all = false;
        Ok(())
    }

    /// Toggle the explicit all-workloads choice. An empty selection is never
    /// interpreted as all; callers must use this reviewed choice instead.
    pub fn set_explicit_all(&mut self, enabled: bool) {
        self.explicit_all = enabled;
        if enabled {
            self.selected_measures = self
                .active_measure_ids()
                .filter(|id| self.measure(id).is_some_and(BenchmarkMeasure::available))
                .map(str::to_owned)
                .collect();
            self.refresh_benchmark_flags();
        } else {
            self.selected_measures.clear();
            self.selected_benchmarks.clear();
        }
    }

    #[must_use]
    pub const fn explicit_all(&self) -> bool {
        self.explicit_all
    }
    pub fn set_benchmark_selected(
        &mut self,
        id: &str,
        selected: bool,
    ) -> Result<(), SelectionError> {
        self.explicit_all = false;
        let measure_ids = self
            .catalog
            .benchmark(id)
            .ok_or(SelectionError::UnknownBenchmark)?
            .measures()
            .iter()
            .map(|m| m.id().to_owned())
            .collect::<Vec<_>>();
        if !self.catalog.pools.iter().any(|p| {
            p.id() == self.pool_id
                && p.groups
                    .iter()
                    .any(|g| g.benchmarks.iter().any(|b| b.id() == id))
        }) {
            return Err(SelectionError::UnknownBenchmark);
        }
        if selected
            && measure_ids.iter().any(|measure_id| {
                !self.measure_in_active_pool(measure_id)
                    || self
                        .measure(measure_id)
                        .is_some_and(|measure| !measure.available())
            })
        {
            return Err(SelectionError::UnavailableMeasure);
        }
        for measure_id in &measure_ids {
            if selected {
                self.selected_measures.insert(measure_id.clone());
            } else {
                self.selected_measures.remove(measure_id);
            }
        }
        if selected {
            self.selected_benchmarks.insert(id.to_owned());
        } else {
            self.selected_benchmarks.remove(id);
        }
        Ok(())
    }
    pub fn set_measure_selected(&mut self, id: &str, selected: bool) -> Result<(), SelectionError> {
        self.explicit_all = false;
        if !self.measure_in_active_pool(id) {
            return Err(SelectionError::UnknownMeasure);
        }
        let measure = self.measure(id).ok_or(SelectionError::UnknownMeasure)?;
        if !measure.available() && selected {
            return Err(SelectionError::UnavailableMeasure);
        }
        if selected {
            self.selected_measures.insert(id.to_owned());
        } else {
            self.selected_measures.remove(id);
        }
        Ok(())
    }

    pub fn benchmark_state(&self, id: &str) -> Result<NodeSelection, SelectionError> {
        let benchmark = self
            .catalog
            .benchmark(id)
            .ok_or(SelectionError::UnknownBenchmark)?;
        if !self.benchmark_in_active_pool(id) {
            return Err(SelectionError::UnknownBenchmark);
        }
        let visible = benchmark
            .measures()
            .iter()
            .filter(|measure| measure.available() && self.measure_matches(measure.id()))
            .collect::<Vec<_>>();
        Ok(node_state(
            visible
                .iter()
                .filter(|measure| self.is_measure_selected(measure.id()))
                .count(),
            visible.len(),
        ))
    }

    pub fn group_state(&self, id: &str) -> Result<NodeSelection, SelectionError> {
        let group = self
            .catalog
            .pools
            .iter()
            .find(|pool| pool.id() == self.pool_id)
            .and_then(|pool| pool.groups.iter().find(|group| group.id() == id))
            .ok_or(SelectionError::UnknownGroup)?;
        let (selected, visible) = group
            .benchmarks()
            .iter()
            .flat_map(|benchmark| benchmark.measures())
            .filter(|measure| measure.available() && self.measure_matches(measure.id()))
            .fold((0, 0), |(selected, visible), measure| {
                (
                    selected + usize::from(self.is_measure_selected(measure.id())),
                    visible + 1,
                )
            });
        Ok(node_state(selected, visible))
    }

    pub fn set_group_selected(&mut self, id: &str, selected: bool) -> Result<(), SelectionError> {
        self.explicit_all = false;
        let measure_ids = self
            .catalog
            .pools
            .iter()
            .find(|pool| pool.id() == self.pool_id)
            .and_then(|pool| pool.groups.iter().find(|group| group.id() == id))
            .ok_or(SelectionError::UnknownGroup)?
            .benchmarks()
            .iter()
            .flat_map(|benchmark| benchmark.measures())
            .filter(|measure| measure.available() && self.measure_matches(measure.id()))
            .map(|measure| measure.id().to_owned())
            .collect::<Vec<_>>();
        for measure_id in measure_ids {
            if selected {
                self.selected_measures.insert(measure_id);
            } else {
                self.selected_measures.remove(&measure_id);
            }
        }
        self.refresh_benchmark_flags();
        Ok(())
    }
    #[must_use]
    pub fn is_measure_selected(&self, id: &str) -> bool {
        self.selected_measures.contains(id)
    }
    #[must_use]
    pub fn selected_measure_ids(&self) -> Vec<&str> {
        self.active_measure_ids()
            .filter(|id| self.selected_measures.contains(*id))
            .collect()
    }
    #[must_use]
    pub fn visible_measure_ids(&self) -> Vec<&str> {
        let query = self.query.to_ascii_lowercase();
        self.catalog
            .pools
            .iter()
            .filter(|p| p.id() == self.pool_id)
            .flat_map(|p| p.groups.iter())
            .flat_map(|g| {
                g.benchmarks
                    .iter()
                    .flat_map(move |b| b.measures.iter().map(move |m| (g, b, m)))
            })
            .filter(|(g, b, m)| {
                query.is_empty()
                    || [g.id(), g.name(), b.id(), b.name(), m.id(), m.name()]
                        .iter()
                        .any(|v| v.to_ascii_lowercase().contains(&query))
            })
            .map(|(_, _, m)| m.id())
            .collect()
    }

    fn measure(&self, id: &str) -> Option<&BenchmarkMeasure> {
        self.catalog
            .pools
            .iter()
            .flat_map(|p| p.groups.iter())
            .flat_map(|g| g.benchmarks.iter())
            .flat_map(|b| b.measures.iter())
            .find(|measure| measure.id() == id)
    }

    fn measure_in_active_pool(&self, id: &str) -> bool {
        self.catalog.pools.iter().any(|pool| {
            pool.id() == self.pool_id
                && pool
                    .groups
                    .iter()
                    .flat_map(|group| group.benchmarks.iter())
                    .flat_map(|benchmark| benchmark.measures.iter())
                    .any(|measure| measure.id() == id)
        })
    }

    fn benchmark_in_active_pool(&self, id: &str) -> bool {
        self.catalog.pools.iter().any(|pool| {
            pool.id() == self.pool_id
                && pool
                    .groups
                    .iter()
                    .flat_map(|group| group.benchmarks.iter())
                    .any(|benchmark| benchmark.id() == id)
        })
    }

    fn active_measure_ids(&self) -> impl Iterator<Item = &str> {
        self.catalog
            .pools
            .iter()
            .filter(|pool| pool.id() == self.pool_id)
            .flat_map(|pool| pool.groups.iter())
            .flat_map(|group| group.benchmarks.iter())
            .flat_map(|benchmark| benchmark.measures.iter())
            .map(|measure| measure.id())
    }

    fn measure_matches(&self, id: &str) -> bool {
        let query = self.query.to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }
        let Some(measure) = self.measure(id) else {
            return false;
        };
        self.catalog
            .pools
            .iter()
            .filter(|pool| pool.id() == self.pool_id)
            .flat_map(|pool| pool.groups.iter())
            .flat_map(|group| {
                group
                    .benchmarks
                    .iter()
                    .map(move |benchmark| (group, benchmark))
            })
            .find_map(|(group, benchmark)| {
                benchmark
                    .measures()
                    .iter()
                    .any(|candidate| candidate.id() == id)
                    .then_some(
                        [
                            group.id(),
                            group.name(),
                            benchmark.id(),
                            benchmark.name(),
                            measure.id(),
                            measure.name(),
                        ]
                        .iter()
                        .any(|value| value.to_ascii_lowercase().contains(&query)),
                    )
            })
            .unwrap_or(false)
    }

    fn refresh_benchmark_flags(&mut self) {
        self.selected_benchmarks = self
            .catalog
            .pools
            .iter()
            .filter(|pool| pool.id() == self.pool_id)
            .flat_map(|pool| pool.groups.iter())
            .flat_map(|group| group.benchmarks.iter())
            .filter(|benchmark| {
                benchmark
                    .measures()
                    .iter()
                    .filter(|measure| measure.available())
                    .count()
                    > 0
                    && benchmark
                        .measures()
                        .iter()
                        .filter(|measure| measure.available())
                        .all(|measure| self.is_measure_selected(measure.id()))
            })
            .map(|benchmark| benchmark.id().to_owned())
            .collect();
    }
    /// Build the exact immutable campaign input. The caller must provide the
    /// generation it reviewed; a refreshed catalog can never be used silently.
    pub fn campaign_handoff(
        &self,
        expected_generation: Revision,
    ) -> Result<CampaignSelection, SelectionError> {
        if expected_generation != self.catalog.generation {
            return Err(SelectionError::StaleCatalog);
        }
        let measure_ids = self
            .selected_measure_ids()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if measure_ids.is_empty() {
            return Err(SelectionError::NoSelection);
        }
        let mut groups = BTreeSet::new();
        let mut benchmarks = BTreeSet::new();
        for pool in &self.catalog.pools {
            if pool.id() != self.pool_id {
                continue;
            }
            for group in &pool.groups {
                for benchmark in &group.benchmarks {
                    let selected = benchmark
                        .measures()
                        .iter()
                        .any(|m| self.selected_measures.contains(m.id()));
                    if selected {
                        groups.insert(group.id().to_owned());
                        benchmarks.insert(benchmark.id().to_owned());
                    }
                }
            }
        }
        Ok(CampaignSelection {
            generation: self.catalog.generation,
            catalog_digest: self.catalog.digest.clone(),
            pool_id: self.pool_id.clone(),
            group_ids: groups.into_iter().collect(),
            benchmark_ids: benchmarks.into_iter().collect(),
            measure_ids,
        })
    }
    /// Replace only with a newer generation and preserve still-valid choices.
    pub fn replace_catalog(
        &mut self,
        catalog: BenchmarkCatalog,
    ) -> Result<Vec<String>, SelectionError> {
        if catalog.generation <= self.catalog.generation {
            return Err(SelectionError::StaleCatalog);
        }
        let previous = self.selected_measures.clone();
        let valid = catalog
            .all_measure_ids()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        self.catalog = catalog;
        self.pool_id = self
            .catalog
            .pools
            .first()
            .map(|p| p.id().to_owned())
            .ok_or(SelectionError::UnknownPool)?;
        self.selected_measures = previous
            .iter()
            .filter(|id| valid.contains(id.as_str()))
            .cloned()
            .collect();
        self.selected_benchmarks.clear();
        Ok(previous
            .into_iter()
            .filter(|id| !valid.contains(id.as_str()))
            .collect())
    }
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
    /// Construct state from an already-authenticated canonical catalog.
    ///
    /// The catalog publication owns ordering; preserving it here is important
    /// because filtering and serialized selections must not invent a second
    /// ordering that can disagree with the control contract.
    pub fn new(measurements: Vec<Measurement>) -> Result<Self, SelectionError> {
        if measurements.len() > MAX_MEASUREMENTS {
            return Err(SelectionError::TooManyMeasurements);
        }
        let mut ids = BTreeSet::new();
        if measurements
            .iter()
            .any(|measurement| !ids.insert(measurement.id.clone()))
        {
            return Err(SelectionError::DuplicateId);
        }
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
            if let Some(summary) = summaries
                .iter_mut()
                .find(|summary| summary.group == measurement.group)
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

fn unique_strings<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<(), SelectionError> {
    let mut ids = BTreeSet::new();
    for value in values {
        if !ids.insert(value) {
            return Err(SelectionError::DuplicateId);
        }
    }
    Ok(())
}

fn unique_measure_ids(values: &[BenchmarkMeasure]) -> Result<(), SelectionError> {
    unique_strings(values.iter().map(BenchmarkMeasure::id))
}
