// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use asb_tui::{
    compatibility::Architecture,
    delegated::{LifecycleRequest, execute, read_request},
    lifecycle::FilesystemLifecycle,
};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use support::PrivateDirectory;

#[test]
fn request_parser_is_closed_bounded_and_versioned() {
    let directory = PrivateDirectory::create();
    let valid = format!(
        "{{\"operation\":\"status\",\"schema_version\":1,\"install_root\":{:?}}}",
        directory.path().to_string_lossy()
    );
    assert!(read_request(valid.as_bytes()).is_ok());
    assert_eq!(read_request(&b"{}"[..]).unwrap_err(), "request_invalid");
    assert_eq!(
        read_request(valid.replace("}", ",\"host\":\"secret\"}").as_bytes()).unwrap_err(),
        "request_invalid"
    );
    assert_eq!(
        read_request(vec![b'x'; 65_537].as_slice()).unwrap_err(),
        "request_size_invalid"
    );
}

#[test]
fn status_is_read_only_and_response_never_discloses_install_path() {
    let directory = PrivateDirectory::create();
    drop(FilesystemLifecycle::open(directory.path()).unwrap());
    let input = format!(
        "{{\"operation\":\"status\",\"schema_version\":1,\"install_root\":{:?}}}",
        directory.path().to_string_lossy()
    );
    let before: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let response = execute(read_request(input.as_bytes()).unwrap());
    let after: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(!response.ok);
    assert_eq!(response.code, "extension_not_installed");
    assert_eq!(before, after);
    let output = serde_json::to_string(&response).unwrap();
    assert!(!output.contains(directory.path().to_string_lossy().as_ref()));
}

#[test]
fn delegated_binary_emits_one_closed_json_response() {
    let directory = PrivateDirectory::create();
    drop(FilesystemLifecycle::open(directory.path()).unwrap());
    let input = format!(
        "{{\"operation\":\"remove\",\"schema_version\":1,\"install_root\":{:?}}}",
        directory.path().to_string_lossy()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["lifecycle", "--format", "json"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["code"], "extension_removed");
    assert_eq!(value["classification"], "unverified_extension");
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains(directory.path().to_string_lossy().as_ref())
    );
}

#[test]
fn runtime_rejects_relative_paths_and_caller_asserted_compatibility() {
    let relative = execute(LifecycleRequest::Status {
        schema_version: 1,
        install_root: "relative".into(),
    });
    assert_eq!(relative.code, "request_path_invalid");
    let directory = PrivateDirectory::create();
    let invalid = execute(LifecycleRequest::Install {
        schema_version: 1,
        install_root: directory.path().join("install"),
        manifest: directory.path().join("manifest"),
        signature: directory.path().join("signature"),
        allowed_signers: directory.path().join("allowed_signers"),
        artifacts: directory.path().join("artifacts"),
        now_unix: 1,
        expected_bundle: "asb-tui-v1-linux-aarch64".into(),
        architecture: Architecture::X86_64,
        asb_version: "99.0.0".into(),
    });
    assert_eq!(invalid.code, "request_compatibility_invalid");
    assert!(!directory.path().join("install").exists());

    let unsupported = execute(LifecycleRequest::Status {
        schema_version: 2,
        install_root: directory.path().to_owned(),
    });
    assert_eq!(unsupported.code, "request_version_unsupported");

    let launch = execute(LifecycleRequest::Launch {
        schema_version: 1,
        install_root: directory.path().to_owned(),
    });
    assert_eq!(launch.code, "extension_not_installed");

    let remove = execute(LifecycleRequest::Remove {
        schema_version: 1,
        install_root: "relative".into(),
    });
    assert_eq!(remove.code, "request_path_invalid");
}
