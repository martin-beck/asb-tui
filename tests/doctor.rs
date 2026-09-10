// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
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

fn run_isolated(configure: impl FnOnce(&mut Command)) -> Output {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let before = profile_artifacts(repository);
    let directory = PrivateDirectory::create();
    let directory_path = directory.path().to_owned();
    let mut command = Command::new(env!("CARGO_BIN_EXE_asb-tui"));
    command.current_dir(directory.path());
    configure(&mut command);
    command.env("LLVM_PROFILE_FILE", directory.path().join("child.profraw"));
    let output = command.output().expect("run isolated asb-tui child");
    assert_eq!(profile_artifacts(repository), before);
    drop(directory);
    assert!(
        !directory_path.exists(),
        "private test directory was not removed"
    );
    output
}

#[test]
fn doctor_is_explicitly_unverified_and_content_free() {
    let output = run_isolated(|command| {
        command.args(["doctor", "--format", "json"]).env_clear();
    });
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 diagnostic"),
        concat!(
            "{\"classification\":\"source_only_unverified\",",
            "\"protocol\":\"asb-cli-capabilities\",",
            "\"protocol_version\":1,",
            "\"reason\":\"installed_asb_compatibility_not_verified\"}\n"
        )
    );
}

#[test]
fn unknown_arguments_fail_without_json_or_environment_output() {
    let output = run_isolated(|command| {
        command
            .arg("unknown")
            .env("PRIVATE_SENTINEL", "must-not-appear");
    });
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("UTF-8 usage"),
        "usage: asb-tui [run] | (doctor|compatibility) --format json | doctor --terminal\n"
    );
}

#[test]
fn terminal_doctor_normalizes_environment_without_disclosing_values() {
    let output = run_isolated(|command| {
        command
            .args(["doctor", "--terminal"])
            .env_clear()
            .env("TERM", "private-term-value")
            .env("TERM_PROGRAM", "private-program-value")
            .env("SSH_CONNECTION", "private-address-value")
            .env("NO_COLOR", "private-no-color-value")
            .env("COLUMNS", "80")
            .env("LINES", "24");
    });
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains("tier=plain tty=false channel=ssh size=80x24"));
    for private in [
        "private-term-value",
        "private-program-value",
        "private-address-value",
        "private-no-color-value",
    ] {
        assert!(!report.contains(private));
    }
}

#[test]
fn lifecycle_self_test_is_closed_and_fails_when_terminal_or_protocol_is_unavailable() {
    let output = run_isolated(|command| {
        command
            .args([
                "lifecycle-self-test",
                "--release",
                "v1.2.3",
                "--format",
                "json",
            ])
            .env_clear();
    });
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["classification"], "source_only_unverified");
    assert_eq!(value["release"], "v1.2.3");
    assert_eq!(value["target"], "x86_64-unknown-linux-gnu");
    assert_eq!(value["source_commit"], "");
    assert_eq!(value["source_tree"], "");
    assert_eq!(value["asb_version"], "0.1.0");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(value["ready"], false);
    assert_eq!(value.as_object().unwrap().len(), 13);
}

#[test]
fn lifecycle_command_rejects_invalid_input_with_one_generic_response() {
    let directory = PrivateDirectory::create();
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["lifecycle", "--format", "json"])
        .current_dir(directory.path())
        .env_clear()
        .env("LLVM_PROFILE_FILE", directory.path().join("child.profraw"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["code"], "request_invalid");
    assert_eq!(value["ok"], false);
}
