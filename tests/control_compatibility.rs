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
