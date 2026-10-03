// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    control_codec::{
        AttemptId, ControlCall, ControlLimits, ControlRequest, FanoutAdmission, FanoutMember,
        FanoutParams, JSONRPC_VERSION, MAX_FRAME_BYTES, RequestId, RunId, V1_14, decode, encode,
    },
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
fn selected_fanout_requests_round_trip_over_v1_14_wire_shape() {
    let mut backend = FakeBackend::default();
    let mut projection = ControlProjection::default();
    let mut state = FanoutDispatchState::default();
    state
        .admit_selected(
            &mut backend,
            &mut projection,
            "wire-request".into(),
            FanoutSelection {
                agent_ids: vec!["opencode".into(), "opendesk".into()],
                workload_ids: vec!["workload-a".into()],
                provider_id: "openrouter".into(),
                model_id: "openai/gpt-4o".into(),
                catalog_digest: "a".repeat(64),
                workload_revision: "b".repeat(64),
                scorer_revision: "c".repeat(64),
            },
        )
        .unwrap();

    assert_eq!(backend.admitted[0].1.len(), 2);
    for request in &backend.admitted[0].1 {
        for field in [
            "agent_id",
            "provider_id",
            "model_id",
            "workload_id",
            "catalog_digest",
            "workload_revision",
            "scorer_revision",
            "mode",
            "limits",
        ] {
            assert!(request.get(field).is_some(), "missing {field}: {request}");
        }
    }

    let request = ControlRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: RequestId(1650),
        timeout_ms: 30_000,
        call: ControlCall::Fanout(FanoutParams {
            idempotency_key: "wire-request".into(),
            requests: backend.admitted[0].1.clone(),
        }),
    };
    assert_eq!(request.call.minimum_version(), Some(V1_14));
    request.validate(ControlLimits::default()).unwrap();
    let wire = encode(&request, MAX_FRAME_BYTES).unwrap();
    let decoded: ControlRequest = decode(&wire, MAX_FRAME_BYTES).unwrap();
    assert_eq!(decoded, request);
}

#[test]
fn report_projection_is_human_and_json_safe() {
    let report = FanoutDispatchState::default().report();
    assert!(report.human().contains("Status: unavailable"));
    assert!(report.json().contains("\"status\":\"unavailable\""));
}
