// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Authenticated-control response projection for the interactive frontend.
//!
//! This is deliberately a transport seam, rather than a transport
//! implementation.  A broker adapter supplies a typed request/response pair;
//! this module validates the pair at the JSON-RPC boundary and publishes only
//! bounded, renderer-safe state.  It cannot launch, retry, or cancel work.

use crate::control_codec::{
    ControlCall, ControlLimits, ControlRequest, ControlResponse, ControlResult, ControlSuccess,
    Negotiated, Revision, RunSummary,
};
use std::{collections::BTreeMap, fmt};

pub const MAX_PROJECTED_RUNS: usize = 256;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Connection {
    #[default]
    Disconnected,
    Negotiated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSnapshot {
    pub connection: Connection,
    pub runner_instance_id: Option<String>,
    pub latest_revision: Option<Revision>,
    pub capabilities: Option<crate::control_codec::Capabilities>,
    pub runs: Vec<RunSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    InvalidResponse,
    UnexpectedResult,
    StaleRun,
    TooManyRuns,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidResponse => "invalid control response",
            Self::UnexpectedResult => "response does not match projection request",
            Self::StaleRun => "run revision moved backwards",
            Self::TooManyRuns => "projected run limit exceeded",
        })
    }
}
impl std::error::Error for ProjectionError {}

/// State owned by the frontend projection.  The runner remains authoritative.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ControlProjection {
    connection: Connection,
    negotiated: Option<Negotiated>,
    capabilities: Option<crate::control_codec::Capabilities>,
    runs: BTreeMap<String, RunSummary>,
}

impl ControlProjection {
    pub fn apply(
        &mut self,
        request: &ControlRequest,
        response: &ControlResponse,
        limits: ControlLimits,
    ) -> Result<(), ProjectionError> {
        response
            .validate_for(request, limits)
            .map_err(|_| ProjectionError::InvalidResponse)?;
        let ControlResponse::Success(success) = response else {
            return Ok(());
        };
        if !matches!(&request.call, ControlCall::Negotiate(_))
            && !matches!(self.connection, Connection::Negotiated)
        {
            return Err(ProjectionError::UnexpectedResult);
        }
        match (&request.call, &success.result) {
            (ControlCall::Negotiate(_), ControlSuccess::Negotiated(negotiated)) => {
                self.connection = Connection::Negotiated;
                self.negotiated = Some(negotiated.clone());
            }
            (_, ControlSuccess::Operation(bound)) => {
                self.apply_result(&request.call, &bound.result)?
            }
            _ => return Err(ProjectionError::UnexpectedResult),
        }
        Ok(())
    }

    fn apply_result(
        &mut self,
        call: &ControlCall,
        result: &ControlResult,
    ) -> Result<(), ProjectionError> {
        match (call, result) {
            (ControlCall::Capabilities, ControlResult::Capabilities(value)) => {
                self.capabilities = Some(value.clone());
            }
            (ControlCall::History(_), ControlResult::History(page)) => {
                if self.runs.len() + page.items.len() > MAX_PROJECTED_RUNS {
                    return Err(ProjectionError::TooManyRuns);
                }
                for run in &page.items {
                    self.insert_run(run.clone())?;
                }
            }
            (ControlCall::Status { .. }, ControlResult::Status(run))
            | (ControlCall::Launch(_), ControlResult::Launch(run)) => {
                self.insert_run(run.clone())?;
            }
            _ => {}
        }
        Ok(())
    }

    fn insert_run(&mut self, run: RunSummary) -> Result<(), ProjectionError> {
        let key = run.run_id.0.clone();
        if let Some(previous) = self.runs.get(&key)
            && run.revision < previous.revision
        {
            return Err(ProjectionError::StaleRun);
        }
        if !self.runs.contains_key(&key) && self.runs.len() >= MAX_PROJECTED_RUNS {
            return Err(ProjectionError::TooManyRuns);
        }
        self.runs.insert(key, run);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.connection = Connection::Disconnected;
    }

    #[must_use]
    pub fn snapshot(&self) -> LiveSnapshot {
        let mut runs: Vec<_> = self.runs.values().cloned().collect();
        runs.sort_by(|left, right| {
            right
                .revision
                .cmp(&left.revision)
                .then_with(|| left.run_id.0.cmp(&right.run_id.0))
        });
        LiveSnapshot {
            connection: self.connection,
            runner_instance_id: self
                .negotiated
                .as_ref()
                .map(|value| value.runner_instance_id.clone()),
            latest_revision: self.negotiated.as_ref().map(|value| value.latest_revision),
            capabilities: self.capabilities.clone(),
            runs,
        }
    }

    #[must_use]
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_codec::{
        BoundResult, ControlVersion, Negotiated, Page, PublicRunState, RequestId, SuccessResponse,
    };

    fn request(call: ControlCall, id: u64) -> ControlRequest {
        ControlRequest {
            jsonrpc: "2.0".into(),
            id: RequestId(id),
            timeout_ms: 1_000,
            call,
        }
    }
    fn run(id: &str, revision: u64) -> RunSummary {
        RunSummary {
            run_id: crate::control_codec::RunId(id.into()),
            attempt_id: crate::control_codec::AttemptId(format!("{id}-a")),
            state: PublicRunState::Completed,
            created_revision: Revision(revision),
            revision: Revision(revision),
            plan_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        }
    }
    fn response(id: u64, result: ControlResult) -> ControlResponse {
        ControlResponse::Success(SuccessResponse {
            jsonrpc: "2.0".into(),
            id: RequestId(id),
            result: ControlSuccess::Operation(BoundResult {
                request_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
                result,
            }),
        })
    }

    fn negotiate_response(id: u64) -> ControlResponse {
        ControlResponse::Success(SuccessResponse {
            jsonrpc: "2.0".into(),
            id: RequestId(id),
            result: ControlSuccess::Negotiated(Negotiated {
                version: ControlVersion { major: 1, minor: 3 },
                limits: ControlLimits::default(),
                runner_instance_id: "runner".into(),
                oldest_revision: Revision(1),
                latest_revision: Revision(5),
            }),
        })
    }

    fn connected_projection() -> ControlProjection {
        let call = ControlCall::Negotiate(crate::control_codec::NegotiateParams {
            versions: [ControlVersion { major: 1, minor: 3 }]
                .into_iter()
                .collect(),
            limits: ControlLimits::default(),
        });
        let mut projection = ControlProjection::default();
        projection
            .apply(
                &request(call, 99),
                &negotiate_response(99),
                ControlLimits::default(),
            )
            .unwrap();
        projection
    }

    #[test]
    fn history_response_becomes_newest_first_snapshot() {
        let call = ControlCall::History(crate::control_codec::PageParams {
            after: None,
            limit: 2,
        });
        let req = request(call.clone(), 1);
        let page = Page {
            items: vec![run("old", 2), run("new", 5)],
            next: None,
            has_more: false,
        };
        let mut projection = connected_projection();
        projection
            .apply(
                &req,
                &response(1, ControlResult::History(page)),
                ControlLimits::default(),
            )
            .unwrap();
        let snapshot = projection.snapshot();
        assert_eq!(
            snapshot
                .runs
                .iter()
                .map(|v| v.run_id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "old"]
        );
    }

    #[test]
    fn stale_status_cannot_replace_authoritative_run() {
        let call = ControlCall::Status {
            run_id: crate::control_codec::RunId("run".into()),
        };
        let mut projection = connected_projection();
        let req = request(call.clone(), 1);
        projection
            .apply(
                &req,
                &response(1, ControlResult::Status(run("run", 3))),
                ControlLimits::default(),
            )
            .unwrap();
        let req = request(call, 2);
        assert_eq!(
            projection.apply(
                &req,
                &response(2, ControlResult::Status(run("run", 2))),
                ControlLimits::default()
            ),
            Err(ProjectionError::StaleRun)
        );
    }

    #[test]
    fn mismatched_response_id_is_rejected_before_projection() {
        let call = ControlCall::History(crate::control_codec::PageParams {
            after: None,
            limit: 1,
        });
        let req = request(call, 8);
        let page = Page {
            items: vec![],
            next: None,
            has_more: false,
        };
        assert_eq!(
            ControlProjection::default().apply(
                &req,
                &response(9, ControlResult::History(page)),
                ControlLimits::default()
            ),
            Err(ProjectionError::InvalidResponse)
        );
    }
}
