// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn local_clone_build_records_exact_provenance_and_cleans_workspace() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("asb-tui-clone-build-{nonce}"));
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rustup_home = env::var_os("RUSTUP_HOME").expect("test needs an explicit rustup home");
    let expected_commit = String::from_utf8(
        Command::new("git")
            .current_dir(&repository)
            .args(["rev-parse", "main"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();

    let output = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .env("ASB_TUI_DEV_REPOSITORY", &repository)
        .env("ASB_TUI_DEV_REF", "main")
        .env("ASB_TUI_DEV_RUSTUP_HOME", rustup_home)
        .env("ASB_TUI_DEV_INSTALL_ROOT", &root)
        .args(["tui", "install", "--channel", "dev", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["code"], "development_installed");
    assert_eq!(response["source_commit"], expected_commit);
    assert!(root.join("provenance.json").is_file());
    assert!(root.join("asb-tui").is_file());
    assert!(!fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with('.')
    }));
    fs::remove_dir_all(root).unwrap();
}
