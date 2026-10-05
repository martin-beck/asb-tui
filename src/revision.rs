// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Typed revision lifecycle used by selection-driven wizard handoff.
//!
//! A revision is deliberately identified by the complete source tuple that
//! produced it.  Comparing only a catalog generation is insufficient: a
//! channel, provider, model, or source rebuild can invalidate an otherwise
//! matching local picker generation.  This module is renderer- and transport-
//! neutral so both wizard and handoff code can use the same fail-closed rules.

use serde::{Deserialize, Serialize};

const MAX_IDENTITY_PART: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionIdentity {
    pub catalog_generation: u64,
    pub channel_revision: String,
    pub provider_revision: String,
    pub model_revision: String,
    pub source_revision: String,
}

impl RevisionIdentity {
    pub fn new(
        catalog_generation: u64,
        channel_revision: impl Into<String>,
        provider_revision: impl Into<String>,
        model_revision: impl Into<String>,
        source_revision: impl Into<String>,
    ) -> Result<Self, RevisionError> {
        let identity = Self {
            catalog_generation,
            channel_revision: channel_revision.into(),
            provider_revision: provider_revision.into(),
            model_revision: model_revision.into(),
            source_revision: source_revision.into(),
        };
        identity.validate()
    }

    fn validate(&self) -> Result<Self, RevisionError> {
        if self.catalog_generation == 0
            || [
                &self.channel_revision,
                &self.provider_revision,
                &self.model_revision,
                &self.source_revision,
            ]
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_IDENTITY_PART || !value.is_ascii())
        {
            return Err(RevisionError::Unavailable);
        }
        Ok(self.clone())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionStatus {
    Provisional,
    Committed,
    Stale,
    Superseded,
    Unavailable,
}

impl RevisionStatus {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Provisional => "provisional",
            Self::Committed => "committed",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRecord {
    pub id: u64,
    pub identity: RevisionIdentity,
    pub status: RevisionStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevisionError {
    Unavailable,
    NotFound,
    NotProvisional,
    NotCommitted,
}

impl RevisionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "revision_unavailable",
            Self::NotFound => "revision_not_found",
            Self::NotProvisional => "revision_not_provisional",
            Self::NotCommitted => "revision_not_committed",
        }
    }
}

/// Single-writer lifecycle for revisions crossing wizard and runner handoff.
///
/// Only the current provisional revision may be committed, and only the
/// current committed revision may be admitted for execution.  A newer commit
/// marks the previous commit superseded; a mismatching identity is stale.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionLedger {
    next_id: u64,
    records: Vec<RevisionRecord>,
    provisional_id: Option<u64>,
    committed_id: Option<u64>,
}

impl RevisionLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    pub fn begin_edit(&mut self, identity: RevisionIdentity) -> Result<u64, RevisionError> {
        let identity = identity.validate()?;
        self.supersede_provisional();
        let id = self.allocate_id();
        self.records.push(RevisionRecord {
            id,
            identity,
            status: RevisionStatus::Provisional,
        });
        self.provisional_id = Some(id);
        Ok(id)
    }

    pub fn commit(&mut self, id: u64) -> Result<RevisionRecord, RevisionError> {
        if self.provisional_id != Some(id) {
            return Err(if self.records.iter().any(|record| record.id == id) {
                RevisionError::NotProvisional
            } else {
                RevisionError::NotFound
            });
        }
        if let Some(previous) = self.committed_id {
            self.set_status(previous, RevisionStatus::Superseded);
        }
        self.set_status(id, RevisionStatus::Committed);
        self.provisional_id = None;
        self.committed_id = Some(id);
        self.record(id)
    }

    /// Cancel an in-progress edit without reviving it after restart.
    pub fn cancel_edit(&mut self, id: u64) -> Result<(), RevisionError> {
        if self.provisional_id != Some(id) {
            return Err(RevisionError::NotProvisional);
        }
        self.set_status(id, RevisionStatus::Superseded);
        self.provisional_id = None;
        Ok(())
    }

    /// Restart preserves the committed revision but never the uncommitted draft.
    pub fn restart(&mut self) {
        if let Some(id) = self.provisional_id.take() {
            self.set_status(id, RevisionStatus::Superseded);
        }
    }

    pub fn admit(&self, id: u64, expected: &RevisionIdentity) -> Result<(), RevisionError> {
        let record = self.record(id)?;
        if record.status != RevisionStatus::Committed {
            return Err(RevisionError::NotCommitted);
        }
        if &record.identity != expected {
            return Err(RevisionError::NotCommitted);
        }
        Ok(())
    }

    #[must_use]
    pub fn classify(&self, identity: &RevisionIdentity) -> RevisionStatus {
        if identity.validate().is_err() {
            return RevisionStatus::Unavailable;
        }
        if let Some(record) = self
            .records
            .iter()
            .find(|record| &record.identity == identity)
        {
            return record.status;
        }
        if self.committed_id.is_some() {
            RevisionStatus::Stale
        } else {
            RevisionStatus::Unavailable
        }
    }

    pub fn record(&self, id: u64) -> Result<RevisionRecord, RevisionError> {
        self.records
            .iter()
            .find(|record| record.id == id)
            .cloned()
            .ok_or(RevisionError::NotFound)
    }

    #[must_use]
    pub fn committed(&self) -> Option<RevisionRecord> {
        self.committed_id.and_then(|id| self.record(id).ok())
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn set_status(&mut self, id: u64, status: RevisionStatus) {
        if let Some(record) = self.records.iter_mut().find(|record| record.id == id) {
            record.status = status;
        }
    }

    fn supersede_provisional(&mut self) {
        if let Some(id) = self.provisional_id.take() {
            self.set_status(id, RevisionStatus::Superseded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(generation: u64, model: &str) -> RevisionIdentity {
        RevisionIdentity::new(generation, "dev-1", "provider-1", model, "source-1").unwrap()
    }

    #[test]
    fn edit_commit_restart_and_admission_are_generation_and_tuple_fenced() {
        let mut ledger = RevisionLedger::new();
        let first = ledger.begin_edit(identity(1, "model-a")).unwrap();
        assert_eq!(
            ledger.record(first).unwrap().status,
            RevisionStatus::Provisional
        );
        let committed = ledger.commit(first).unwrap();
        assert_eq!(committed.status, RevisionStatus::Committed);
        ledger.admit(first, &identity(1, "model-a")).unwrap();
        assert_eq!(
            ledger.classify(&identity(2, "model-a")),
            RevisionStatus::Stale
        );
        assert_eq!(
            ledger.classify(&identity(1, "model-b")),
            RevisionStatus::Stale
        );

        let draft = ledger.begin_edit(identity(2, "model-b")).unwrap();
        ledger.restart();
        assert_eq!(
            ledger.record(draft).unwrap().status,
            RevisionStatus::Superseded
        );
        assert_eq!(ledger.committed().unwrap().id, first);
    }

    #[test]
    fn newer_commit_supersedes_old_and_cancel_never_revives_draft() {
        let mut ledger = RevisionLedger::new();
        let first_draft = ledger.begin_edit(identity(1, "model-a")).unwrap();
        let first = ledger.commit(first_draft).unwrap();
        let second = ledger.begin_edit(identity(2, "model-b")).unwrap();
        ledger.commit(second).unwrap();
        assert_eq!(
            ledger.record(first.id).unwrap().status,
            RevisionStatus::Superseded
        );
        assert_eq!(
            ledger.classify(&identity(1, "model-a")),
            RevisionStatus::Superseded
        );

        let third = ledger.begin_edit(identity(3, "model-c")).unwrap();
        ledger.cancel_edit(third).unwrap();
        assert_eq!(
            ledger.classify(&identity(3, "model-c")),
            RevisionStatus::Superseded
        );
        assert_eq!(ledger.committed().unwrap().id, second);
    }

    #[test]
    fn missing_identity_is_unavailable_and_cannot_be_admitted() {
        assert_eq!(
            RevisionIdentity::new(0, "dev", "provider", "model", "source"),
            Err(RevisionError::Unavailable)
        );
        let mut ledger = RevisionLedger::new();
        let draft = ledger.begin_edit(identity(1, "model")).unwrap();
        assert_eq!(
            ledger.admit(draft, &identity(1, "model")),
            Err(RevisionError::NotCommitted)
        );
        assert_eq!(
            ledger.classify(&RevisionIdentity {
                catalog_generation: 0,
                channel_revision: String::new(),
                provider_revision: String::new(),
                model_revision: String::new(),
                source_revision: String::new()
            }),
            RevisionStatus::Unavailable
        );
    }
}
