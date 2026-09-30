// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn onboarding_cli_returns_one_development_recovery_choice() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["onboarding", "--format", "json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            br#"{"schema_version":1,"profile":"development","bundle_available":true,"broker_available":false,"protocol_version":1}"#,
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["development_only"], true);
    assert_eq!(value["ready"], false);
    assert_eq!(value["recovery"], "retry_broker");
}
