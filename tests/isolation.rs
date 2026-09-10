// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use support::PrivateDirectory;

fn profile_artifacts(root: &Path) -> Vec<PathBuf> {
    let mut artifacts: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("default_") && name.ends_with(".profraw"))
        })
        .collect();
    artifacts.sort();
    artifacts
}

#[test]
fn standalone_binary_runs_without_asb_or_ambient_environment() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let before = profile_artifacts(repository);
    let directory = PrivateDirectory::create();
    let directory_path = directory.path().to_owned();
    let output = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["doctor", "--format", "json"])
        .env_clear()
        .env("PATH", "/nonexistent")
        .current_dir(directory.path())
        .output()
        .expect("run standalone binary");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("source_only_unverified")
    );
    assert_eq!(profile_artifacts(repository), before);
    drop(directory);
    assert!(
        !directory_path.exists(),
        "private test directory was not removed"
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
