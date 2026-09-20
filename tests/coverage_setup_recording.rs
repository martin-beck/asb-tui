// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Coverage-only matrix for the setup and recording control boundary.

use asb_tui::control_codec::{ControlLimits, ControlRequest, ControlResponse};
use serde_json::json;

fn request(method: &str, params: serde_json::Value) -> ControlRequest {
    serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": 7, "timeout_ms": 1000,
        "method": method, "params": params
    }))
    .expect("closed request fixture")
}

fn rejected(method: &str, params: serde_json::Value) {
    assert!(
        request(method, params)
            .validate(ControlLimits::default())
            .is_err(),
        "invalid {method} request must fail closed"
    );
}

#[test]
fn setup_and_recording_requests_cover_invalid_identity_and_generation_paths() {
    rejected("auth_status", json!({"provider": ""}));
    rejected(
        "auth_enroll",
        json!({
            "provider": "provider",
            "endpoint_identity_sha256": "bad",
            "credential_locator_sha256": "bad",
            "idempotency_key": ""
        }),
    );
    rejected(
        "provider_catalog",
        json!({"action": "status", "runner_instance_id": ""}),
    );
    rejected("configuration_status", json!({"runner_instance_id": ""}));
    rejected(
        "configuration_apply",
        json!({
            "idempotency_key": "apply",
            "expected_generation": 0,
            "selection": {
                "agent_ids": ["agent"], "provider_id": "provider",
                "model_id": "model", "auth_method": "none",
                "credential_reference_sha256": null
            }
        }),
    );
    rejected(
        "recording_campaign_estimate",
        json!({
            "runner_instance_id": "runner", "provider_id": "",
            "model_id": "model", "agent_ids": ["agent"], "workload_ids": ["workload"]
        }),
    );
    rejected(
        "recording_campaign_plan",
        json!({
            "idempotency_key": "plan", "expected_generation": 0,
            "runner_instance_id": "runner", "provider_id": "provider",
            "model_id": "model", "agent_ids": ["agent"], "workload_ids": ["workload"]
        }),
    );
    for method in [
        "recording_campaign_execute",
        "recording_campaign_cancel",
        "recording_campaign_reconcile",
        "recording_campaign_offline_default",
    ] {
        rejected(
            method,
            json!({
                "idempotency_key": "mutation", "expected_generation": 0,
                "runner_instance_id": "runner", "campaign_id": "campaign"
            }),
        );
    }
    rejected(
        "recording_campaign_progress",
        json!({"runner_instance_id": "", "campaign_id": "campaign"}),
    );
}

#[test]
fn public_failure_responses_reject_unbounded_content() {
    for message in ["\u{0000}".to_owned(), "x".repeat(4097)] {
        let response: ControlResponse = serde_json::from_value(json!({
            "jsonrpc": "2.0", "id": 7,
            "error": {"code": -1, "message": message}
        }))
        .expect("failure fixture shape");
        assert!(response.validate(ControlLimits::default()).is_err());
    }
}
