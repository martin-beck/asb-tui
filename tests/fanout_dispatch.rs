// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    control_codec::{AttemptId, FanoutAdmission, FanoutMember, RunId},
    control_transport::TransportError,
    fanout_dispatch::{FanoutBackend, FanoutDispatchState, FanoutSelection, FanoutStatus},
    live_projection::ControlProjection,
};

#[derive(Default)]
struct FakeBackend {
    admission: Option<FanoutAdmission>,
    admitted: Vec<(String, Vec<serde_json::Value>)>,
    cancelled: Vec<(String, Vec<FanoutMember>)>,
}

impl FanoutBackend for FakeBackend {
    fn admit_fanout(
        &mut self,
        _projection: &mut ControlProjection,
        idempotency_key: String,
        requests: Vec<serde_json::Value>,
    ) -> Result<FanoutAdmission, TransportError> {
        self.admitted.push((idempotency_key, requests));
        Ok(self.admission.clone().unwrap_or(FanoutAdmission {
            idempotency_key: "fanout-1".into(),
            members: vec![FanoutMember {
                run_id: RunId("run-1".into()),
                attempt_id: AttemptId("attempt-1".into()),
            }],
        }))
    }

    fn cancel_fanout(
        &mut self,
        _projection: &mut ControlProjection,
        idempotency_key: String,
        members: Vec<FanoutMember>,
    ) -> Result<(), TransportError> {
        self.cancelled.push((idempotency_key, members));
        Ok(())
    }
}

#[test]
fn selected_scope_expands_in_stable_order_and_cancel_uses_exact_members() {
    let mut backend = FakeBackend::default();
    let mut projection = ControlProjection::default();
    let mut state = FanoutDispatchState::default();
    state
        .admit_selected(
            &mut backend,
            &mut projection,
            "request-1".into(),
            FanoutSelection {
                agent_ids: vec!["agent-b".into(), "agent-a".into()],
                workload_ids: vec!["workload-b".into(), "workload-a".into()],
                provider_id: "provider".into(),
                model_id: "model".into(),
                catalog_digest: "0".repeat(64),
                workload_revision: "0".repeat(64),
                scorer_revision: "0".repeat(64),
            },
        )
        .unwrap();
    assert_eq!(state.status(), FanoutStatus::Admitted);
    assert_eq!(backend.admitted[0].0, "request-1");
    assert_eq!(backend.admitted[0].1.len(), 4);
    state
        .cancel(&mut backend, &mut projection, "cancel-1".into())
        .unwrap();
    assert_eq!(state.status(), FanoutStatus::Cancelled);
    assert_eq!(backend.cancelled[0].0, "cancel-1");
    assert_eq!(backend.cancelled[0].1, state.admission().unwrap().members);
}

#[test]
fn report_projection_is_human_and_json_safe() {
    let report = FanoutDispatchState::default().report();
    assert!(report.human().contains("Status: unavailable"));
    assert!(report.json().contains("\"status\":\"unavailable\""));
}
