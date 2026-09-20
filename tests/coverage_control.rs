// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Test-only matrix for the public control codec boundary.  These cases keep
//! the validation branches exercised without changing protocol or thresholds.

use asb_tui::{
    control_client::{
        AdoptedChannel, ClientError, ConnectionState, ControlLimits as ClientLimits,
        ControlVersion, CursorToken, MeasurementReason, NegotiateRequest, Negotiated,
        PeerCredentials, RequestClass, RequestId, RequestLedger, RunnerIdentity, V1_0, V1_3, V1_5,
        ValidationCategory, ValidationIssue, supported_versions,
    },
    control_codec::{ControlLimits, ControlRequest, ControlResponse},
};
use serde_json::json;

fn request(method: &str, params: serde_json::Value) -> ControlRequest {
    serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": 7, "timeout_ms": 1000,
        "method": method, "params": params
    }))
    .unwrap()
}

fn rejects(method: &str, params: serde_json::Value) {
    assert!(
        request(method, params)
            .validate(ControlLimits::default())
            .is_err(),
        "{method}"
    );
}

#[test]
fn request_validation_covers_setup_auth_and_recording_boundaries() {
    rejects(
        "auth_enroll",
        json!({"provider":"","endpoint_identity_sha256":"bad","credential_locator_sha256":"bad","idempotency_key":""}),
    );
    rejects("auth_status", json!({"provider":""}));
    rejects(
        "auth_rotate",
        json!({"provider":"p","credential_locator_sha256":"bad","idempotency_key":"k"}),
    );
    rejects("auth_revoke", json!({"provider":"p","idempotency_key":""}));
    rejects(
        "provider_catalog",
        json!({"action":"refresh","runner_instance_id":""}),
    );
    rejects("configuration_status", json!({"runner_instance_id":""}));
    rejects(
        "configuration_apply",
        json!({"idempotency_key":"k","expected_generation":0,"selection":{"agent_ids":["a"],"provider_id":"p","model_id":"m","auth_method":"none","credential_reference_sha256":null}}),
    );
    rejects(
        "recording_campaign_plan",
        json!({"idempotency_key":"k","expected_generation":0,"runner_instance_id":"r","provider_id":"p","model_id":"m","agent_ids":["a"],"workload_ids":["w"]}),
    );
    rejects(
        "recording_campaign_status",
        json!({"runner_instance_id":""}),
    );
    rejects(
        "recording_campaign_estimate",
        json!({"runner_instance_id":"r","provider_id":"","model_id":"m","agent_ids":["a"],"workload_ids":["w"]}),
    );
    for method in [
        "recording_campaign_execute",
        "recording_campaign_cancel",
        "recording_campaign_reconcile",
        "recording_campaign_offline_default",
    ] {
        rejects(
            method,
            json!({"idempotency_key":"k","expected_generation":0,"runner_instance_id":"r","campaign_id":"c"}),
        );
    }
    rejects(
        "recording_campaign_progress",
        json!({"runner_instance_id":"","campaign_id":"c"}),
    );
}

#[test]
fn request_validation_covers_core_call_limits_and_identity() {
    rejects(
        "negotiate",
        json!({"versions":[],"limits":ControlLimits::default()}),
    );
    rejects("history", json!({"after":null,"limit":0}));
    rejects("events", json!({"after":null,"limit":0}));
    rejects("analyze", json!({"run_ids":[]}));
    rejects("analyze", json!({"run_ids":["run","run"]}));
    rejects("artifact_metadata", json!({"run_id":"run","digest":"bad"}));
    rejects("create_plan", json!({"idempotency_key":"","definition":{}}));
    rejects("launch", json!({"idempotency_key":"k","plan_id":""}));
    rejects("status", json!({"run_id":""}));
    rejects("repeat", json!({"run_id":"","idempotency_key":"k"}));
}

#[test]
fn response_validation_rejects_public_failure_and_mismatch_shapes() {
    let limits = ControlLimits::default();
    for message in ["\u{0000}", &"x".repeat(4097)] {
        let response: ControlResponse = serde_json::from_value(json!({
            "jsonrpc":"2.0", "id":7, "error":{"code":-1,"message":message}
        }))
        .unwrap();
        assert!(response.validate(limits).is_err());
    }
}

#[test]
fn response_validation_covers_simple_operation_results() {
    let limits = ControlLimits::default();
    let cases = [
        json!({"kind":"acknowledged","value":{"accepted":true}}),
        json!({"kind":"plan","value":{"plan_id":"plan","plan_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}),
        json!({"kind":"analysis","value":{"run_count":1,"analysis_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}),
        json!({"kind":"artifact_metadata","value":{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size_bytes":1,"sensitivity":"public"}}),
        json!({"kind":"capabilities","value":{"validate_settings":true,"run_control":true,"repeat":true,"analysis":true,"events":true}}),
    ];
    for result in cases {
        let response: ControlResponse = serde_json::from_value(json!({
            "jsonrpc":"2.0", "id":7, "result":{"kind":"operation","value":{"request_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","result":result}}
        })).unwrap();
        assert!(response.validate(limits).is_ok());
    }
}

#[test]
fn client_limits_versions_identity_and_frame_boundaries_are_closed() {
    let bad = ClientLimits {
        max_frame_bytes: 0,
        ..ClientLimits::default()
    };
    assert_eq!(
        bad.validate(),
        Err(ClientError::InvalidLimit("max_frame_bytes"))
    );
    let bad = ClientLimits {
        max_timeout_ms: 0,
        ..ClientLimits::default()
    };
    assert!(bad.validate().is_err());
    assert!(
        ClientLimits::default()
            .intersect(ClientLimits::default())
            .is_ok()
    );
    assert!(
        NegotiateRequest::new(ClientLimits::default())
            .unwrap()
            .validate()
            .is_ok()
    );
    assert_eq!(
        asb_tui::control_client::select_version(&supported_versions(), &supported_versions())
            .unwrap(),
        V1_5
    );
    assert!(
        asb_tui::control_client::select_version(
            &[ControlVersion { major: 9, minor: 9 }]
                .into_iter()
                .collect(),
            &supported_versions()
        )
        .is_err()
    );
    assert!(RequestId::new("").is_err());
    assert!(CursorToken::parse("01").is_err());
    assert_eq!(CursorToken::parse("42").unwrap().revision(), 42);
    let identity = RunnerIdentity {
        uid: 1,
        pid: 2,
        service_generation: 3,
        runner_instance_id: "runner".into(),
    };
    let channel = AdoptedChannel::new(
        3,
        PeerCredentials {
            uid: 1,
            pid: 2,
            service_generation: 3,
        },
    )
    .unwrap();
    assert!(channel.authenticate(&identity).is_ok());
    assert!(
        AdoptedChannel::new(
            2,
            PeerCredentials {
                uid: 1,
                pid: 2,
                service_generation: 3
            }
        )
        .is_err()
    );
    assert!(asb_tui::control_client::encode_frame(&"ok", 1).is_err());
    assert!(asb_tui::control_client::decode_frame::<String>(&[0, 0, 0], 100).is_err());
    let frame = asb_tui::control_client::encode_frame(&"ok", 100).unwrap();
    assert_eq!(
        asb_tui::control_client::decode_frame::<String>(&frame, 100).unwrap(),
        "ok"
    );
    let mut ledger = RequestLedger::default();
    let id = RequestId::new("read").unwrap();
    ledger
        .begin(id.clone(), RequestClass::ReadOnly, ClientLimits::default())
        .unwrap();
    assert_eq!(ledger.complete(&id), Ok(RequestClass::ReadOnly));
    assert_eq!(ledger.complete(&id), Err(ClientError::UnknownRequest));
    assert_eq!(V1_0, ControlVersion { major: 1, minor: 0 });
    assert_eq!(V1_3, ControlVersion { major: 1, minor: 3 });
}

#[test]
fn client_connection_cursor_and_issue_validation_paths_are_exercised() {
    let identity = RunnerIdentity {
        uid: 1,
        pid: 2,
        service_generation: 3,
        runner_instance_id: "runner".into(),
    };
    let channel = AdoptedChannel::new(
        3,
        PeerCredentials {
            uid: 1,
            pid: 2,
            service_generation: 3,
        },
    )
    .unwrap();
    let negotiated = Negotiated {
        version: V1_5,
        limits: ClientLimits::default(),
        runner: identity,
        oldest_revision: 2,
        latest_revision: 5,
    };
    let offered = supported_versions();
    let mut state = ConnectionState::default()
        .establish(channel, negotiated, &offered, ClientLimits::default())
        .unwrap();
    assert!(state.advance_cursor(2).is_ok());
    assert!(state.advance_cursor(2).is_err());
    assert!(state.advance_cursor(6).is_err());
    assert!(matches!(
        state.disconnect(),
        ConnectionState::Disconnected { .. }
    ));
    assert!(ConnectionState::default().advance_cursor(1).is_err());
    assert!(
        asb_tui::control_client::validate_issue(
            V1_0,
            &ValidationIssue {
                category: ValidationCategory::MeasurementIssue,
                reason: None,
                measurement_id: None,
            }
        )
        .is_err()
    );
    assert!(
        asb_tui::control_client::validate_issue(
            V1_3,
            &ValidationIssue {
                category: ValidationCategory::MeasurementIssue,
                reason: None,
                measurement_id: None,
            }
        )
        .is_err()
    );
    assert!(
        asb_tui::control_client::validate_issue(
            V1_3,
            &ValidationIssue {
                category: ValidationCategory::MeasurementIssue,
                reason: Some(MeasurementReason::UnknownId),
                measurement_id: Some("m".into()),
            }
        )
        .is_err()
    );
    assert!(
        asb_tui::control_client::validate_issue(
            V1_3,
            &ValidationIssue {
                category: ValidationCategory::MeasurementIssue,
                reason: Some(MeasurementReason::Unavailable),
                measurement_id: Some("m".into()),
            }
        )
        .is_ok()
    );
    assert!(
        asb_tui::control_client::validate_issue(
            ControlVersion { major: 1, minor: 9 },
            &ValidationIssue {
                category: ValidationCategory::InvalidFormat,
                reason: None,
                measurement_id: None,
            }
        )
        .is_err()
    );
    let mut ledger = RequestLedger::default();
    let id = RequestId::new("mutation").unwrap();
    ledger
        .begin(id, RequestClass::Mutation, ClientLimits::default())
        .unwrap();
    let retry = ledger.reconnect_plan();
    assert!(retry.read_only.is_empty());
    assert_eq!(retry.abandoned_mutations.len(), 1);
}
