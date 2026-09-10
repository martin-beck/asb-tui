// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::process::Command;

#[test]
fn doctor_is_explicitly_unverified_and_content_free() {
    let output = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["doctor", "--format", "json"])
        .env_clear()
        .output()
        .expect("run asb-tui doctor");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 diagnostic"),
        concat!(
            "{\"classification\":\"unverified_extension\",",
            "\"protocol\":\"asb-cli-capabilities\",",
            "\"protocol_version\":1,",
            "\"reason\":\"installed_asb_compatibility_not_verified\"}\n"
        )
    );
}

#[test]
fn unknown_arguments_fail_without_json_or_environment_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .arg("run")
        .env("PRIVATE_SENTINEL", "must-not-appear")
        .output()
        .expect("run invalid asb-tui command");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("UTF-8 usage"),
        "usage: asb-tui doctor --format json\n"
    );
}
