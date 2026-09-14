// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
use std::process::{Command, Stdio};

#[test]
fn broker_entrypoint_fails_closed_without_an_inherited_socket() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["run", "--broker"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn asb-tui broker entrypoint");
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .expect("wait for broker entrypoint");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stdout.is_empty(),
        "broker path must not emit lifecycle JSON"
    );
    assert_eq!(output.stderr, b"broker channel adoption failed\n");
}
