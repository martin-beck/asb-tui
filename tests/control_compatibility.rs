// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Cross-repository wire smoke tests for the ASB control contract.
//!
//! The fixture is copied from ASB AR-1036's v1.2 request contract.  This test
//! intentionally checks only the stable envelope and operation discriminator;
//! full catalog semantics remain an ASB-owned validation responsibility.

use asb_tui::control_codec::{
    ControlCall, ControlLimits, ControlRequest, JSONRPC_VERSION, RequestId, V1_2,
};
use asb_tui::protocol_compatibility::{
    MATRIX_SCHEMA_VERSION, operation_matrix, supported_versions, validate_schema, validate_version,
};
use std::collections::BTreeSet;

#[test]
fn asb_v12_measurement_catalog_request_is_accepted_exactly() {
    let request: ControlRequest = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":7,"timeout_ms":30000,"method":"measurement_catalog"}"#,
    )
    .expect("AR-1036 request fixture must decode");
    assert_eq!(request.jsonrpc, JSONRPC_VERSION);
    assert_eq!(request.id, RequestId(7));
    assert_eq!(request.timeout_ms, 30_000);
    assert!(matches!(request.call, ControlCall::MeasurementCatalog));
    request.validate(ControlLimits::default()).unwrap();
    assert_eq!(V1_2.minor, 2);
}

#[test]
fn measurement_catalog_request_rejects_lifecycle_field_widening() {
    let result = serde_json::from_str::<ControlRequest>(
        r#"{"jsonrpc":"2.0","id":7,"timeout_ms":30000,"method":"measurement_catalog","runner_path":"/private"}"#,
    );
    assert!(result.is_err());
}

#[test]
fn development_fixture_is_explicit_and_matches_the_matrix() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/control-compatibility/development.json"
    ))
    .unwrap();
    assert_eq!(fixture["schema_version"], MATRIX_SCHEMA_VERSION);
    assert_eq!(fixture["development_only"], true);
    let minors: BTreeSet<_> = fixture["supported_control_minors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap() as u16)
        .collect();
    assert_eq!(
        minors,
        supported_versions()
            .into_iter()
            .map(|version| version.minor)
            .collect()
    );
    assert!(
        fixture["current_main"]["asb_source_commit"]
            .as_str()
            .is_some_and(|value| value.len() == 40)
    );
    assert_eq!(fixture["lifecycle_operations"].as_array().unwrap().len(), 5);
    assert!(validate_schema(MATRIX_SCHEMA_VERSION).is_ok());
    assert!(validate_version(V1_2).is_ok());
}

#[test]
fn unsupported_schema_and_minor_are_fail_closed() {
    assert!(validate_schema(MATRIX_SCHEMA_VERSION + 1).is_err());
    assert!(
        validate_version(asb_tui::control_codec::ControlVersion { major: 1, minor: 9 }).is_err()
    );
    assert!(
        operation_matrix()
            .iter()
            .any(|row| row.name == "agent_cancel" && row.minimum.minor == 5)
    );
}
