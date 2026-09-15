// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::agent_catalog::parse_agent_catalog_response;

fn valid() -> String {
    serde_json::json!({
        "jsonrpc":"2.0", "id":7,
        "result":{"kind":"operation","value":{"request_sha256":"f".repeat(64),"result":{"kind":"agent_catalog","value":{
            "runner_instance_id":"runner-1", "generation":3, "catalog_sha256":"e".repeat(64),
            "target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
            "agents":[{"agent_id":"agent-a","target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
              "package":{"package_id":"agent-package","version":"1.2.3","sha256":"a".repeat(64),"signature_sha256":"b".repeat(64)},
              "provenance":{"source_revision":"c".repeat(40),"manifest_sha256":"d".repeat(64)},"capabilities":["chat","tools"],"availability":{"status":"available"}}],"refreshed":false
        }}}}
    }).to_string()
}

#[test]
fn accepts_exact_asb_v14_response() {
    let catalog = parse_agent_catalog_response(&valid()).unwrap();
    assert_eq!(catalog.runner_instance_id, "runner-1");
    assert_eq!(catalog.generation, 3);
    assert_eq!(catalog.agents[0].agent_id, "agent-a");
}

#[test]
fn consumes_the_checked_in_asb_v14_fixture() {
    let catalog = parse_agent_catalog_response(include_str!(
        "fixtures/asb-v1.4-agent-catalog-response.json"
    ))
    .unwrap();
    assert_eq!(catalog.catalog_sha256, "e".repeat(64));
    assert_eq!(catalog.target.libc, "glibc");
}

#[test]
fn rejects_old_provisional_envelope_and_unknown_fields() {
    let old = valid().replace(
        "\"jsonrpc\":\"2.0\"",
        "\"protocol\":\"asb-agent-catalog\",\"protocol_version\":1",
    );
    assert!(parse_agent_catalog_response(&old).is_err());
    let unknown = valid().replace("\"jsonrpc\":\"2.0\"", "\"future\":true,\"jsonrpc\":\"2.0\"");
    assert!(parse_agent_catalog_response(&unknown).is_err());
}

#[test]
fn rejects_unsorted_duplicate_incomplete_and_wrong_target_catalogs() {
    let duplicate = valid().replace("\"agent-a\"", "\"agent-b\",\"agent_id\":\"agent-a\"");
    assert!(parse_agent_catalog_response(&duplicate).is_err());
    let missing = valid().replace(
        "\"catalog_sha256\":\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\",",
        "",
    );
    assert!(parse_agent_catalog_response(&missing).is_err());
    let mut wrong_target: serde_json::Value = serde_json::from_str(&valid()).unwrap();
    wrong_target["result"]["value"]["result"]["value"]["agents"][0]["target"]["architecture"] =
        serde_json::Value::String("aarch64".into());
    let wrong_target = wrong_target.to_string();
    assert!(parse_agent_catalog_response(&wrong_target).is_err());
    let mut unsorted: serde_json::Value = serde_json::from_str(&valid()).unwrap();
    let entry = unsorted["result"]["value"]["result"]["value"]["agents"][0].clone();
    let mut second = entry.clone();
    second["agent_id"] = serde_json::Value::String("agent-b".into());
    unsorted["result"]["value"]["result"]["value"]["agents"] =
        serde_json::Value::Array(vec![second, entry]);
    assert!(parse_agent_catalog_response(&unsorted.to_string()).is_err());
}

#[test]
fn rejects_agent_identifiers_outside_asb_lowercase_slug_form() {
    let invalid = valid().replace("\"agent-a\"", "\"Agent_A\"");
    assert!(parse_agent_catalog_response(&invalid).is_err());
}
