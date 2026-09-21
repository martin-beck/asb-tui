// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::agent_catalog::{
    AgentAvailability, AgentCatalog, AgentCatalogEntry, AgentCatalogRequest, AgentPackage,
    AgentProvenance, AgentTarget, encode_agent_catalog_request, parse_agent_catalog_response,
};

fn valid() -> String {
    serde_json::json!({
        "jsonrpc":"2.0", "id":7,
        "result":{"kind":"operation","value":{"request_sha256":"f".repeat(64),"result":{"kind":"agent_catalog","value":{
            "runner_instance_id":"runner-1", "generation":3, "catalog_sha256":"decf62e9642eace3ac2e03cfe04da24e44f3504f9f8e96e1ff9fa925b7d3e590",
            "target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
            "agents":[{"agent_id":"agent-a","target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
              "package":{"package_id":"agent-package","version":"1.2.3","sha256":"a".repeat(64),"signature_sha256":"b".repeat(64),"signer":{"key_id":"release-key-1","principal":"asb-release"}},
              "provenance":{"source_revision":"c".repeat(40),"manifest_sha256":"d".repeat(64),"sbom_sha256":"e".repeat(64),"license_ref":"MIT"},"capabilities":["chat","tools"],"availability":{"status":"available"}}],"refreshed":false
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
    assert_eq!(
        catalog.catalog_sha256,
        "decf62e9642eace3ac2e03cfe04da24e44f3504f9f8e96e1ff9fa925b7d3e590"
    );
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
        "\"catalog_sha256\":\"decf62e9642eace3ac2e03cfe04da24e44f3504f9f8e96e1ff9fa925b7d3e590\",",
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

#[test]
fn request_encoding_validates_identity_and_timeout() {
    let request = AgentCatalogRequest {
        action: asb_tui::agent_catalog::AgentCatalogAction::Refresh,
        runner_instance_id: "runner-1".into(),
        known_generation: Some(3),
    };
    let value: serde_json::Value =
        serde_json::from_str(&encode_agent_catalog_request(12, 1000, &request).unwrap()).unwrap();
    assert_eq!(value["method"], "agent_catalog");
    assert_eq!(value["params"]["known_generation"], 3);
    assert!(encode_agent_catalog_request(12, 0, &request).is_err());
    assert!(encode_agent_catalog_request(12, 300_001, &request).is_err());
    assert!(
        encode_agent_catalog_request(
            12,
            1000,
            &AgentCatalogRequest {
                runner_instance_id: "bad value".into(),
                ..request
            }
        )
        .is_err()
    );
}

#[test]
fn catalog_validation_rejects_bounds_and_noncanonical_metadata() {
    let target = AgentTarget {
        operating_system: "linux".into(),
        architecture: "x86_64".into(),
        libc: "glibc".into(),
        libc_version: "2.35".into(),
    };
    let entry = |id: &str| AgentCatalogEntry {
        agent_id: id.into(),
        target: target.clone(),
        package: Some(AgentPackage {
            package_id: "pkg".into(),
            version: "1.0".into(),
            sha256: "a".repeat(64),
            signature_sha256: "b".repeat(64),
            signer: asb_tui::agent_catalog::AgentSigner {
                key_id: "release-key-1".into(),
                principal: "asb-release".into(),
            },
        }),
        provenance: Some(AgentProvenance {
            source_revision: "c".repeat(40),
            manifest_sha256: "d".repeat(64),
            sbom_sha256: "e".repeat(64),
            license_ref: "MIT".into(),
        }),
        capabilities: vec!["bench".into()],
        availability: AgentAvailability::Unavailable(
            asb_tui::agent_catalog::AgentUnavailableReason::PolicyDenied,
        ),
    };
    let mut catalog = AgentCatalog {
        runner_instance_id: "runner".into(),
        generation: 1,
        catalog_sha256: "e".repeat(64),
        target: target.clone(),
        agents: vec![entry("agent")],
        refreshed: true,
    };
    catalog.catalog_sha256 = catalog.computed_digest().unwrap();
    // Direct validation exercises the public boundary independently of the
    // JSON decoder and keeps unavailable catalog entries representable.
    assert!(catalog.validate().is_ok());
    catalog.target.libc_version = "bad value".into();
    assert!(catalog.validate().is_err());
    catalog.target = target;
    catalog.agents[0].capabilities = vec!["bench".into(), "bench".into()];
    assert!(catalog.validate().is_err());
    catalog.agents[0].capabilities = vec!["bad value".into()];
    assert!(catalog.validate().is_err());
    catalog.agents[0].capabilities = vec![];
    assert!(catalog.validate().is_err());
    catalog.agents[0]
        .provenance
        .as_mut()
        .unwrap()
        .source_revision = "C".repeat(40);
    assert!(catalog.validate().is_err());
}

#[test]
fn catalog_decoder_rejects_envelope_digest_and_empty_catalog() {
    let wrong_kind = valid().replace("\"kind\":\"agent_catalog\"", "\"kind\":\"other\"");
    assert!(parse_agent_catalog_response(&wrong_kind).is_err());
    let bad_digest = valid().replace(
        "\"request_sha256\":\"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\"",
        "\"request_sha256\":\"not-a-digest\"",
    );
    assert!(parse_agent_catalog_response(&bad_digest).is_err());
    let mut empty: serde_json::Value = serde_json::from_str(&valid()).unwrap();
    empty["result"]["value"]["result"]["value"]["agents"] = serde_json::json!([]);
    assert!(parse_agent_catalog_response(&empty.to_string()).is_err());
}

#[test]
fn catalog_digest_matches_asb_vector_and_rejects_content_tampering() {
    let catalog = parse_agent_catalog_response(&valid()).unwrap();
    assert_eq!(catalog.computed_digest().unwrap(), catalog.catalog_sha256);
    assert_eq!(
        catalog.catalog_sha256,
        "decf62e9642eace3ac2e03cfe04da24e44f3504f9f8e96e1ff9fa925b7d3e590"
    );

    let mut tampered: serde_json::Value = serde_json::from_str(&valid()).unwrap();
    tampered["result"]["value"]["result"]["value"]["generation"] = serde_json::Value::from(4);
    assert!(parse_agent_catalog_response(&tampered.to_string()).is_err());
}

#[test]
fn catalog_digest_is_independent_of_object_member_order_but_not_arrays() {
    let mut value: serde_json::Value = serde_json::from_str(&valid()).unwrap();
    let catalog = parse_agent_catalog_response(&valid()).unwrap();
    let reordered = serde_json::json!({
        "target": value["result"]["value"]["result"]["value"]["target"].clone(),
        "refreshed": false,
        "agents": value["result"]["value"]["result"]["value"]["agents"].clone(),
        "generation": 3,
        "runner_instance_id": "runner-1",
        "catalog_sha256": catalog.catalog_sha256,
    });
    value["result"]["value"]["result"]["value"] = reordered;
    assert_eq!(
        parse_agent_catalog_response(&value.to_string())
            .unwrap()
            .generation,
        3
    );

    let agents = &mut value["result"]["value"]["result"]["value"]["agents"];
    let entry = agents[0].clone();
    agents[0] = serde_json::json!({"agent_id":"agent-b", "target":entry["target"].clone(), "package":entry["package"].clone(), "provenance":entry["provenance"].clone(), "capabilities":entry["capabilities"].clone(), "availability":entry["availability"].clone()});
    assert!(parse_agent_catalog_response(&value.to_string()).is_err());
}

#[test]
fn catalog_digest_ignores_response_only_refresh_flag() {
    let refreshed = valid().replace("\"refreshed\":false", "\"refreshed\":true");
    let catalog = parse_agent_catalog_response(&valid()).unwrap();
    let refreshed_catalog = parse_agent_catalog_response(&refreshed).unwrap();
    assert_eq!(
        catalog.computed_digest().unwrap(),
        refreshed_catalog.computed_digest().unwrap()
    );
    assert_eq!(catalog.catalog_sha256, refreshed_catalog.catalog_sha256);
}

#[test]
fn catalog_validation_rejects_each_authenticated_identity_field() {
    let raw: serde_json::Value = serde_json::from_str(&valid()).unwrap();
    for (field, bad) in [
        ("runner_instance_id", "bad value"),
        ("catalog_sha256", "bad"),
        ("package_id", "bad value"),
        ("version", "bad value"),
        ("sha256", "bad"),
        ("signature_sha256", "bad"),
        ("manifest_sha256", "bad"),
    ] {
        let mut candidate = raw.clone();
        let target = &mut candidate["result"]["value"]["result"]["value"];
        if field == "runner_instance_id" || field == "catalog_sha256" {
            target[field] = serde_json::Value::String(bad.into());
        } else if field == "manifest_sha256" {
            target["agents"][0]["provenance"][field] = serde_json::Value::String(bad.into());
        } else {
            target["agents"][0]["package"][field] = serde_json::Value::String(bad.into());
        }
        assert!(parse_agent_catalog_response(&candidate.to_string()).is_err());
    }
    // Keep the base JSON value live so this test remains tied to the complete
    // nested ASB envelope rather than a hand-written fragment.
    assert_eq!(
        raw["result"]["value"]["result"]["value"]["runner_instance_id"],
        "runner-1"
    );
}
