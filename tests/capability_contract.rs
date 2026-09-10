// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::parse_capability_response;

const VALID: &str = r#"{"protocol":"asb-cli-capabilities","protocol_version":1,"asb_version":"0.1.0","capabilities":{"analysis":true,"artifacts":true,"cancel":true,"events":true,"history":true,"launch":true,"planning":true,"repeat":true}}"#;

#[test]
fn accepts_the_exact_closed_v1_contract() {
    assert!(parse_capability_response(VALID).is_ok());
}

#[test]
fn rejects_unknown_missing_duplicate_and_trailing_members() {
    let cases = [
        VALID.replace("\"asb_version\"", "\"unknown\":false,\"asb_version\""),
        VALID.replace("\"asb_version\":\"0.1.0\",", ""),
        VALID.replace("\"analysis\":true", "\"analysis\":true,\"analysis\":false"),
        format!("{VALID} trailing"),
        VALID.replace("\"repeat\":true", "\"repeat\":true,\"future\":false"),
    ];
    for input in cases {
        assert!(
            parse_capability_response(&input).is_err(),
            "accepted {input}"
        );
    }
}

#[test]
fn rejects_wrong_protocol_versions_and_json_types() {
    let cases = [
        VALID.replace("asb-cli-capabilities", "mutable-latest"),
        VALID.replace("\"protocol_version\":1", "\"protocol_version\":2"),
        VALID.replace("\"protocol_version\":1", "\"protocol_version\":true"),
        VALID.replace("\"analysis\":true", "\"analysis\":1"),
        VALID.replace("\"asb_version\":\"0.1.0\"", "\"asb_version\":\"\""),
        VALID.replace("\"0.1.0\"", &format!("\"{}\"", "x".repeat(65))),
    ];
    for input in cases {
        assert!(
            parse_capability_response(&input).is_err(),
            "accepted {input}"
        );
    }
}
