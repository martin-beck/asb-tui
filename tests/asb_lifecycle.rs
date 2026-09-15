// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::asb_lifecycle::{
    AgentInstallRequest, AgentLifecycleBinding, AgentLifecycleState, encode_install_request,
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
