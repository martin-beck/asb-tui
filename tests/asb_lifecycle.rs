// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::asb_lifecycle::{
    AgentCancelRequest, AgentInstallRequest, AgentLifecycleBinding, AgentLifecycleState,
    AgentRemoveRequest, AgentRetryRequest, AgentStatusRequest, encode_cancel_request,
    encode_install_request, encode_remove_request, encode_retry_request, encode_status_request,
    parse_lifecycle_response,
};

fn valid() -> String {
    serde_json::json!({"jsonrpc":"2.0","id":8,"result":{"kind":"operation","value":{"request_sha256":"a".repeat(64),"result":{"kind":"agent_lifecycle","value":{"binding":{"agent_id":"codex","runner_instance_id":"runner-1","catalog_sha256":"a".repeat(64)},"operation_id":"operation-1","state":"active","generation":3,"progress_percent":100,"failure":null}}}}}).to_string()
}

#[test]
fn accepts_exact_asb_v15_lifecycle_fixture_shape() {
    let value = parse_lifecycle_response(&valid()).unwrap();
    assert_eq!(value.operation_id, "operation-1");
    assert_eq!(value.generation, 3);
    assert_eq!(value.state, AgentLifecycleState::Active);
}

#[test]
fn consumes_the_checked_in_asb_v15_fixture() {
    let raw = include_str!("fixtures/asb-v1.5-agent-install-response.json");
    let value = parse_lifecycle_response(raw).unwrap();
    assert_eq!(value.binding.agent_id, "codex");
    assert_eq!(value.progress_percent, 100);

    // This digest is emitted by ASB's canonical v1.5 fixture at the pinned
    // lifecycle revision. Keep the response wrapper synchronized as well as
    // the nested lifecycle value.
    let envelope: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(
        envelope["result"]["value"]["request_sha256"],
        "51d58f15dd232726dbaa1b687110c0f889ecff4a1f5ced49e7d3306110d56d0f"
    );
}

#[test]
fn encodes_asb_v15_install_binding_and_idempotency() {
    let request = AgentInstallRequest {
        binding: AgentLifecycleBinding {
            agent_id: "codex".into(),
            runner_instance_id: "runner-1".into(),
            catalog_sha256: "a".repeat(64),
        },
        catalog_generation: 2,
        idempotency_key: "install-1".into(),
    };
    let value: serde_json::Value =
        serde_json::from_str(&encode_install_request(8, 1000, &request).unwrap()).unwrap();
    assert_eq!(value["method"], "agent_install");
    assert_eq!(value["params"]["catalog_generation"], 2);
    assert_eq!(value["params"]["binding"]["runner_instance_id"], "runner-1");
}

#[test]
fn rejects_wrong_generation_shape_unknown_fields_and_failure_contradictions() {
    assert!(
        parse_lifecycle_response(&valid().replace("\"generation\":3", "\"generation\":\"3\""))
            .is_err()
    );
    assert!(
        parse_lifecycle_response(
            &valid().replace("\"jsonrpc\":\"2.0\"", "\"future\":true,\"jsonrpc\":\"2.0\"")
        )
        .is_err()
    );
    assert!(
        parse_lifecycle_response(&valid().replace("\"failure\":null", "\"failure\":\"busy\""))
            .is_err()
    );
}

#[test]
fn rejects_partial_terminal_and_invalid_failure_responses() {
    assert!(
        parse_lifecycle_response(
            &valid().replace("\"progress_percent\":100", "\"progress_percent\":50")
        )
        .is_err()
    );
    let failed = valid().replace("\"state\":\"active\",\"generation\":3,\"progress_percent\":100,\"failure\":null", "\"state\":\"failed\",\"generation\":3,\"progress_percent\":0,\"failure\":\"self_test_failed\"");
    assert!(parse_lifecycle_response(&failed).is_ok());
}

#[test]
fn encodes_all_asb_v15_lifecycle_methods_with_exact_wire_names() {
    let binding = AgentLifecycleBinding {
        agent_id: "codex".into(),
        runner_instance_id: "runner-1".into(),
        catalog_sha256: "a".repeat(64),
    };
    let status: serde_json::Value = serde_json::from_str(
        &encode_status_request(
            9,
            1000,
            &AgentStatusRequest {
                binding: binding.clone(),
                operation_id: Some("operation-1".into()),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(status["method"], "agent_status");
    assert_eq!(status["params"]["operation_id"], "operation-1");
    let cancel: serde_json::Value = serde_json::from_str(
        &encode_cancel_request(
            10,
            1000,
            &AgentCancelRequest {
                binding: binding.clone(),
                operation_id: "operation-1".into(),
                idempotency_key: "cancel-1".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(cancel["method"], "agent_cancel");
    let retry: serde_json::Value = serde_json::from_str(
        &encode_retry_request(
            11,
            1000,
            &AgentRetryRequest {
                binding: binding.clone(),
                operation_id: "operation-1".into(),
                idempotency_key: "retry-1".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(retry["method"], "agent_retry");
    let remove: serde_json::Value = serde_json::from_str(
        &encode_remove_request(
            12,
            1000,
            &AgentRemoveRequest {
                binding,
                idempotency_key: "remove-1".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(remove["method"], "agent_remove");
}

#[test]
fn lifecycle_encoders_fail_closed_on_bad_binding_or_timeout() {
    let request = AgentRemoveRequest {
        binding: AgentLifecycleBinding {
            agent_id: "codex".into(),
            runner_instance_id: "runner-1".into(),
            catalog_sha256: "A".repeat(64),
        },
        idempotency_key: "remove-1".into(),
    };
    assert!(encode_remove_request(1, 1000, &request).is_err());
    let request = AgentInstallRequest {
        binding: AgentLifecycleBinding {
            agent_id: "codex".into(),
            runner_instance_id: "runner-1".into(),
            catalog_sha256: "a".repeat(64),
        },
        catalog_generation: 2,
        idempotency_key: "install-1".into(),
    };
    assert!(encode_install_request(1, 300_001, &request).is_err());
}

#[test]
fn request_validation_covers_optional_status_and_mutation_identities() {
    let binding = AgentLifecycleBinding {
        agent_id: "codex".into(),
        runner_instance_id: "runner-1".into(),
        catalog_sha256: "a".repeat(64),
    };
    assert!(
        AgentInstallRequest {
            binding: binding.clone(),
            catalog_generation: 4,
            idempotency_key: "key-1".into(),
        }
        .validate()
        .is_ok()
    );
    assert!(
        AgentInstallRequest {
            binding: binding.clone(),
            catalog_generation: 4,
            idempotency_key: "bad key".into(),
        }
        .validate()
        .is_err()
    );
    assert!(
        AgentStatusRequest {
            binding: binding.clone(),
            operation_id: None,
        }
        .validate()
        .is_ok()
    );
    assert!(
        AgentStatusRequest {
            binding: binding.clone(),
            operation_id: Some("bad key".into()),
        }
        .validate()
        .is_err()
    );
    assert!(
        AgentCancelRequest {
            binding: binding.clone(),
            operation_id: "op-1".into(),
            idempotency_key: "cancel-1".into(),
        }
        .validate()
        .is_ok()
    );
    assert!(
        AgentRetryRequest {
            binding: binding.clone(),
            operation_id: "".into(),
            idempotency_key: "retry-1".into(),
        }
        .validate()
        .is_err()
    );
    assert!(
        AgentRemoveRequest {
            binding,
            idempotency_key: "".into(),
        }
        .validate()
        .is_err()
    );
    assert!(
        AgentRemoveRequest {
            binding: AgentLifecycleBinding {
                agent_id: "codex".into(),
                runner_instance_id: "runner-1".into(),
                catalog_sha256: "a".repeat(64),
            },
            idempotency_key: "remove-1".into(),
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn all_lifecycle_response_states_have_exact_terminal_rules() {
    for (state, progress, failure) in [
        ("pending", 0, serde_json::Value::Null),
        ("staging", 25, serde_json::Value::Null),
        ("cancelled", 100, serde_json::Value::Null),
        ("removed", 100, serde_json::Value::Null),
        ("failed", 0, serde_json::json!("busy")),
    ] {
        let raw = valid()
            .replace("\"state\":\"active\"", &format!("\"state\":\"{state}\""))
            .replace(
                "\"progress_percent\":100",
                &format!("\"progress_percent\":{progress}"),
            )
            .replace("\"failure\":null", &format!("\"failure\":{failure}"));
        assert!(parse_lifecycle_response(&raw).is_ok(), "state {state}");
    }
    assert!(
        parse_lifecycle_response(
            &valid().replace("\"progress_percent\":100", "\"progress_percent\":101")
        )
        .is_err()
    );
    let failed_without_reason = valid().replace("\"state\":\"active\"", "\"state\":\"failed\"");
    assert!(parse_lifecycle_response(&failed_without_reason).is_err());
    let malformed_digest = valid().replace(
        "\"request_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
        "\"request_sha256\":\"bad\"",
    );
    assert!(parse_lifecycle_response(&malformed_digest).is_err());
}

#[test]
fn lifecycle_encoders_reject_each_mutation_and_binding_boundary() {
    let bad_binding = AgentLifecycleBinding {
        agent_id: "Bad".into(),
        runner_instance_id: "runner-1".into(),
        catalog_sha256: "a".repeat(64),
    };
    let status = AgentStatusRequest {
        binding: bad_binding.clone(),
        operation_id: None,
    };
    assert!(encode_status_request(1, 1000, &status).is_err());
    let binding = AgentLifecycleBinding {
        agent_id: "codex".into(),
        runner_instance_id: "runner 1".into(),
        catalog_sha256: "a".repeat(64),
    };
    let remove = AgentRemoveRequest {
        binding: binding.clone(),
        idempotency_key: "remove-1".into(),
    };
    assert!(encode_remove_request(1, 1000, &remove).is_err());
    let binding = AgentLifecycleBinding {
        agent_id: "codex".into(),
        runner_instance_id: "runner-1".into(),
        catalog_sha256: "a".repeat(64),
    };
    let cancel = AgentCancelRequest {
        binding: binding.clone(),
        operation_id: "op bad".into(),
        idempotency_key: "cancel-1".into(),
    };
    assert!(encode_cancel_request(1, 1000, &cancel).is_err());
    let retry = AgentRetryRequest {
        binding,
        operation_id: "op-1".into(),
        idempotency_key: "retry bad".into(),
    };
    assert!(encode_retry_request(1, 1000, &retry).is_err());
    let valid_status = AgentStatusRequest {
        binding: AgentLifecycleBinding {
            agent_id: "codex".into(),
            runner_instance_id: "runner-1".into(),
            catalog_sha256: "a".repeat(64),
        },
        operation_id: None,
    };
    assert!(encode_status_request(1, 0, &valid_status).is_err());
}

#[test]
fn lifecycle_decoder_rejects_wrong_envelope_and_binding_identities() {
    let wrong_kind = valid().replace("\"kind\":\"agent_lifecycle\"", "\"kind\":\"other\"");
    assert!(parse_lifecycle_response(&wrong_kind).is_err());
    let bad_runner = valid().replace(
        "\"runner_instance_id\":\"runner-1\"",
        "\"runner_instance_id\":\"runner instance\"",
    );
    assert!(parse_lifecycle_response(&bad_runner).is_err());
    let bad_digest = valid().replace(
        "\"catalog_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
        "\"catalog_sha256\":\"bad\"",
    );
    assert!(parse_lifecycle_response(&bad_digest).is_err());
}

#[test]
fn lifecycle_public_boundaries_reject_invalid_ids_at_each_entrypoint() {
    let valid_binding = AgentLifecycleBinding {
        agent_id: "codex".into(),
        runner_instance_id: "runner-1".into(),
        catalog_sha256: "a".repeat(64),
    };
    let bad_install = AgentInstallRequest {
        binding: valid_binding.clone(),
        catalog_generation: 1,
        idempotency_key: "bad key".into(),
    };
    assert!(encode_install_request(1, 1000, &bad_install).is_err());
    let bad_status = AgentStatusRequest {
        binding: valid_binding.clone(),
        operation_id: Some("bad operation".into()),
    };
    assert!(encode_status_request(1, 1000, &bad_status).is_err());
    let bad_remove = AgentRemoveRequest {
        binding: valid_binding,
        idempotency_key: "bad key".into(),
    };
    assert!(encode_remove_request(1, 1000, &bad_remove).is_err());
}
