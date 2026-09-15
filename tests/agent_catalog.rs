// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::agent_catalog::parse_agent_catalog;

fn valid() -> String {
    serde_json::json!({
        "protocol": "asb-agent-catalog", "protocol_version": 1,
        "runner_generation": "generation-7", "catalog_digest": "a".repeat(64),
        "authentication": {"status": "verified", "authorization": "agent-catalog-read"},
        "agents": [{
            "id": "local.echo", "name": "Echo Agent", "version": "1.2.3",
            "targets": [{"operating_system": "linux", "architecture": "x86_64"}],
            "package": {"format": "oci", "digest": "b".repeat(64), "signature": "sig-1", "provenance": "prov-1"},
            "capabilities": ["benchmark", "offline"],
            "availability": {"state": "available", "reason": null}
        }]
    }).to_string()
}

#[test]
fn accepts_verified_bounded_projection() {
    let catalog = parse_agent_catalog(&valid()).unwrap();
    assert_eq!(catalog.agents[0].id, "local.echo");
}

#[test]
fn rejects_authentication_and_shape_failures() {
    for replacement in [
        ("\"verified\"", "\"unverified\""),
        ("\"agent-catalog-read\"", "\"other\""),
        ("\"local.echo\"", "\"Local Echo\""),
        ("\"reason\":null", "\"reason\":\"why\""),
    ] {
        let input = valid().replace(replacement.0, replacement.1);
        assert!(parse_agent_catalog(&input).is_err(), "accepted {input}");
    }
}

#[test]
fn rejects_duplicates_unknown_fields_secrets_and_oversize() {
    let duplicate = valid().replace("\"local.echo\"", "\"local.echo\",\"id\":\"local.echo\"");
    assert!(parse_agent_catalog(&duplicate).is_err());
    let unknown = valid().replace(
        "\"protocol_version\":1",
        "\"future\":true,\"protocol_version\":1",
    );
    assert!(parse_agent_catalog(&unknown).is_err());
    let path = valid().replace("Echo Agent", "/secret/Echo Agent");
    assert!(parse_agent_catalog(&path).is_err());
    assert!(parse_agent_catalog(&format!("{}{}", valid(), " ".repeat(256 * 1024))).is_err());
}
