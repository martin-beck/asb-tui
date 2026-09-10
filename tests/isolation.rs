// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::{fs, process::Command};

#[test]
fn standalone_binary_runs_without_asb_or_ambient_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["doctor", "--format", "json"])
        .env_clear()
        .env("PATH", "/nonexistent")
        .output()
        .expect("run standalone binary");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("unverified_extension")
    );
}

#[test]
fn standalone_dependency_graph_has_no_asb_core_or_workspace_path() {
    let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let lock = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
    let dependencies = manifest
        .split_once("[dependencies]")
        .expect("dependency table")
        .1
        .split("\n[")
        .next()
        .unwrap();
    assert!(!dependencies.contains("path ="));
    assert!(!manifest.contains("[workspace]"));
    for prohibited in ["asb-core", "asb-cli", "agent-systems-benchmark"] {
        assert!(!dependencies.contains(prohibited));
        assert!(!lock.contains(prohibited));
    }
}
