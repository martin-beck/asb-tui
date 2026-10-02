// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral dispatch for selected-agent fan-out.

use crate::{
    control_codec::{FanoutAdmission, FanoutMember},
    control_transport::{AuthenticatedBrokerSession, TransportError},
    live_projection::ControlProjection,
};
use serde::Serialize;
use std::fmt;

const MAX_SELECTION_ITEMS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FanoutSelection {
    pub agent_ids: Vec<String>,
    pub workload_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FanoutSelectionError {
    Empty(&'static str),
    TooMany(&'static str),
    Invalid(&'static str),
}

#[derive(Debug, Eq, PartialEq)]
pub enum FanoutDispatchError {
    Selection(FanoutSelectionError),
    Transport(TransportError),
}

impl fmt::Display for FanoutDispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selection(error) => error.fmt(f),
            Self::Transport(error) => write!(f, "fan-out control route: {error:?}"),
        }
    }
}

impl fmt::Display for FanoutSelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(field) => write!(f, "fan-out {field} selection is empty"),
            Self::TooMany(field) => write!(f, "fan-out {field} selection is too large"),
            Self::Invalid(field) => write!(f, "fan-out {field} selection is invalid"),
        }
    }
}

impl FanoutSelection {
    pub fn canonicalize(mut self) -> Result<Self, FanoutSelectionError> {
        for (name, values) in [
            ("agents", &mut self.agent_ids),
            ("workloads", &mut self.workload_ids),
        ] {
            if values.is_empty() {
                return Err(FanoutSelectionError::Empty(name));
            }
            if values.len() > MAX_SELECTION_ITEMS {
                return Err(FanoutSelectionError::TooMany(name));
            }
            if values.iter().any(|value| {
                value.is_empty()
                    || value.len() > 128
                    || !value.is_ascii()
                    || value.chars().any(char::is_control)
            }) {
                return Err(FanoutSelectionError::Invalid(name));
            }
            values.sort();
            values.dedup();
        }
        Ok(self)
    }

    fn requests(&self) -> Vec<serde_json::Value> {
        self.agent_ids
            .iter()
            .flat_map(|agent_id| {
                self.workload_ids.iter().map(move |workload_id| {
                    serde_json::json!({"agent_id": agent_id, "workload_id": workload_id})
                })
            })
            .collect()
    }
}

pub trait FanoutBackend {
    fn admit_fanout(
        &mut self,
        projection: &mut ControlProjection,
        idempotency_key: String,
        requests: Vec<serde_json::Value>,
    ) -> Result<FanoutAdmission, TransportError>;
    fn cancel_fanout(
        &mut self,
        projection: &mut ControlProjection,
        idempotency_key: String,
        members: Vec<FanoutMember>,
    ) -> Result<(), TransportError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FanoutStatus {
    #[default]
    Unavailable,
    Pending,
    Admitted,
    CancelRequested,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FanoutDispatchReport {
    pub status: &'static str,
    pub agent_ids: Vec<String>,
    pub workload_ids: Vec<String>,
    pub member_count: usize,
}

impl FanoutDispatchReport {
    pub fn human(&self) -> String {
        format!(
            "fan-out\nStatus: {}\nAgents: {}\nWorkloads: {}\nMembers: {}\n",
            self.status,
            self.agent_ids.join(", "),
            self.workload_ids.join(", "),
            self.member_count
        )
    }

    pub fn json(&self) -> String {
        serde_json::to_string(self).expect("fan-out report is serializable")
    }
}

impl FanoutBackend for AuthenticatedBrokerSession {
    fn admit_fanout(
        &mut self,
        projection: &mut ControlProjection,
        idempotency_key: String,
        requests: Vec<serde_json::Value>,
    ) -> Result<FanoutAdmission, TransportError> {
        Self::admit_fanout(self, projection, idempotency_key, requests)
    }

    fn cancel_fanout(
        &mut self,
        projection: &mut ControlProjection,
        idempotency_key: String,
        members: Vec<FanoutMember>,
    ) -> Result<(), TransportError> {
        Self::cancel_fanout(self, projection, idempotency_key, members)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FanoutDispatchState {
    admission: Option<FanoutAdmission>,
    status: FanoutStatus,
    selection: Option<FanoutSelection>,
}

impl FanoutDispatchState {
    #[must_use]
    pub const fn admission(&self) -> Option<&FanoutAdmission> {
        self.admission.as_ref()
    }

    #[must_use]
    pub const fn status(&self) -> FanoutStatus {
        self.status
    }

    pub fn report(&self) -> FanoutDispatchReport {
        let selection = self.selection.clone().unwrap_or(FanoutSelection {
            agent_ids: Vec::new(),
            workload_ids: Vec::new(),
        });
        FanoutDispatchReport {
            status: match self.status {
                FanoutStatus::Unavailable => "unavailable",
                FanoutStatus::Pending => "pending",
                FanoutStatus::Admitted => "admitted",
                FanoutStatus::CancelRequested => "cancel_requested",
                FanoutStatus::Cancelled => "cancelled",
            },
            agent_ids: selection.agent_ids,
            workload_ids: selection.workload_ids,
            member_count: self.admission.as_ref().map_or(0, |a| a.members.len()),
        }
    }

    pub fn admit_selected<B: FanoutBackend>(
        &mut self,
        backend: &mut B,
        projection: &mut ControlProjection,
        idempotency_key: String,
        selection: FanoutSelection,
    ) -> Result<(), FanoutDispatchError> {
        let selection = selection
            .canonicalize()
            .map_err(FanoutDispatchError::Selection)?;
        self.admit(backend, projection, idempotency_key, selection.requests())
            .map_err(FanoutDispatchError::Transport)?;
        self.selection = Some(selection);
        Ok(())
    }

    pub fn admit<B: FanoutBackend>(
        &mut self,
        backend: &mut B,
        projection: &mut ControlProjection,
        idempotency_key: String,
        requests: Vec<serde_json::Value>,
    ) -> Result<(), TransportError> {
        self.status = FanoutStatus::Pending;
        let admission = match backend.admit_fanout(projection, idempotency_key, requests) {
            Ok(admission) => admission,
            Err(error) => {
                self.status = FanoutStatus::Unavailable;
                return Err(error);
            }
        };
        self.admission = Some(admission);
        self.status = FanoutStatus::Admitted;
        Ok(())
    }

    pub fn cancel<B: FanoutBackend>(
        &mut self,
        backend: &mut B,
        projection: &mut ControlProjection,
        idempotency_key: String,
    ) -> Result<(), TransportError> {
        let members = self
            .admission
            .as_ref()
            .ok_or(TransportError::Projection)?
            .members
            .clone();
        self.status = FanoutStatus::CancelRequested;
        if let Err(error) = backend.cancel_fanout(projection, idempotency_key, members) {
            self.status = FanoutStatus::Admitted;
            return Err(error);
        }
        self.status = FanoutStatus::Cancelled;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingBackend(TransportError);
    impl FanoutBackend for FailingBackend {
        fn admit_fanout(
            &mut self,
            _projection: &mut ControlProjection,
            _idempotency_key: String,
            _requests: Vec<serde_json::Value>,
        ) -> Result<FanoutAdmission, TransportError> {
            Err(std::mem::replace(&mut self.0, TransportError::Projection))
        }

        fn cancel_fanout(
            &mut self,
            _projection: &mut ControlProjection,
            _idempotency_key: String,
            _members: Vec<FanoutMember>,
        ) -> Result<(), TransportError> {
            Err(TransportError::Projection)
        }
    }

    #[test]
    fn selected_scope_is_canonical_and_expands_deterministically() {
        let selection = FanoutSelection {
            agent_ids: vec!["agent-b".into(), "agent-a".into(), "agent-a".into()],
            workload_ids: vec!["workload-b".into(), "workload-a".into()],
        }
        .canonicalize()
        .unwrap();
        assert_eq!(selection.agent_ids, ["agent-a", "agent-b"]);
        assert_eq!(selection.workload_ids, ["workload-a", "workload-b"]);
        assert_eq!(
            selection.requests(),
            vec![
                serde_json::json!({"agent_id":"agent-a","workload_id":"workload-a"}),
                serde_json::json!({"agent_id":"agent-a","workload_id":"workload-b"}),
                serde_json::json!({"agent_id":"agent-b","workload_id":"workload-a"}),
                serde_json::json!({"agent_id":"agent-b","workload_id":"workload-b"}),
            ]
        );
    }

    #[test]
    fn empty_and_hostile_scopes_fail_before_control_submission() {
        for selection in [
            FanoutSelection {
                agent_ids: vec![],
                workload_ids: vec!["w".into()],
            },
            FanoutSelection {
                agent_ids: vec!["a".into()],
                workload_ids: vec![],
            },
            FanoutSelection {
                agent_ids: vec!["a\n".into()],
                workload_ids: vec!["w".into()],
            },
        ] {
            assert!(selection.canonicalize().is_err());
        }
    }

    #[test]
    fn report_is_stable_and_secret_free_in_human_and_json_forms() {
        let report = FanoutDispatchState::default().report();
        assert!(report.human().contains("Status: unavailable"));
        let json = report.json();
        assert!(json.contains("\"status\":\"unavailable\""));
        assert!(!json.contains("api_key"));
    }

    #[test]
    fn admission_preserves_reconciliation_and_transport_error_classes() {
        let selection = FanoutSelection {
            agent_ids: vec!["agent".into()],
            workload_ids: vec!["workload".into()],
        };
        let mut state = FanoutDispatchState::default();
        let mut projection = ControlProjection::default();
        let error = state
            .admit_selected(
                &mut FailingBackend(TransportError::RemoteFailureCode(-33008)),
                &mut projection,
                "key".into(),
                selection.clone(),
            )
            .unwrap_err();
        assert_eq!(
            error,
            FanoutDispatchError::Transport(TransportError::RemoteFailureCode(-33008))
        );
        if let FanoutDispatchError::Transport(transport) = &error {
            assert!(transport.is_reconciliation_required());
        } else {
            panic!("expected transport error");
        }
        let invalid = state
            .admit_selected(
                &mut FailingBackend(TransportError::RemoteFailure),
                &mut projection,
                "key".into(),
                FanoutSelection {
                    agent_ids: vec![],
                    ..selection
                },
            )
            .unwrap_err();
        assert_eq!(
            invalid,
            FanoutDispatchError::Selection(FanoutSelectionError::Empty("agents"))
        );
    }
}
