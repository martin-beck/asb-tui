// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral dispatch for selected-agent fan-out.

use crate::{
    control_codec::{FanoutAdmission, FanoutMember},
    control_transport::{AuthenticatedBrokerSession, TransportError},
    live_projection::ControlProjection,
};

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
