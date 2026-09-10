// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use asb_tui::{
    delegated::{LifecycleRequest, execute, execute_input, read_request},
    lifecycle::FilesystemLifecycle,
};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
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
    assert_eq!(execute_input(&b"{}"[..]).code, "request_invalid");
}

#[test]
fn status_is_read_only_and_response_never_discloses_install_path() {
    let directory = PrivateDirectory::create();
    fs::create_dir(directory.path().join("versions")).unwrap();
    fs::set_permissions(
        directory.path().join("versions"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(directory.path().join(".lifecycle.lock"), b"").unwrap();
    fs::set_permissions(
        directory.path().join(".lifecycle.lock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
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
fn status_does_not_initialize_an_empty_install_root() {
    let directory = PrivateDirectory::create();
    let input = format!(
        "{{\"operation\":\"status\",\"schema_version\":1,\"install_root\":{:?}}}",
        directory.path().to_string_lossy()
    );
    let response = execute(read_request(input.as_bytes()).unwrap());
    assert_eq!(response.code, "install_root_unavailable");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
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
        .env("LLVM_PROFILE_FILE", directory.path().join("child.profraw"))
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
    assert_eq!(value["classification"], "source_only_unverified");
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
        artifacts: directory.path().join("artifacts"),
        target: "x86_64-unknown-linux-gnu".into(),
        asb_version: "99.0.0".into(),
        protocol_version: 1,
        expected_release: "v0.1.0".into(),
        expected_source_commit: "a".repeat(40),
        expected_source_tree: "b".repeat(40),
        expected_executable_sha256: "c".repeat(64),
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

fn install_request(
    directory: &PrivateDirectory,
    manifest: &Path,
    artifacts: &Path,
) -> LifecycleRequest {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    LifecycleRequest::Install {
        schema_version: 1,
        install_root: directory.path().join("install"),
        manifest: manifest.to_owned(),
        signature: repository.join("tests/fixtures/bundle/manifest.json.sig"),
        artifacts: artifacts.to_owned(),
        target: "x86_64-unknown-linux-gnu".into(),
        asb_version: "0.1.0".into(),
        protocol_version: 1,
        expected_release: "v0.1.0".into(),
        expected_source_commit: "a7ca8e07f177fc6a647b3297df624137cfb85e86".into(),
        expected_source_tree: "8556090e21336df0766263e6244084865f92d1a3".into(),
        expected_executable_sha256:
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
    }
}

#[test]
fn source_only_channel_rejects_authenticated_fixture_without_creating_root() {
    let directory = PrivateDirectory::create();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = repository.join("tests/fixtures/bundle/manifest.json");
    let artifacts = directory.path().join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    for name in ["asb-tui", "source", "licenses", "sbom", "provenance"] {
        fs::write(artifacts.join(name), b"hello").unwrap();
    }
    let response = execute(install_request(&directory, &manifest, &artifacts));
    assert_eq!(response.code, "release_channel_unverified");
    assert!(!directory.path().join("install").exists());

    let missing = directory.path().join("missing-manifest");
    let response = execute(install_request(&directory, &missing, &artifacts));
    assert_eq!(response.code, "manifest_unavailable");

    let link = directory.path().join("manifest-link");
    std::os::unix::fs::symlink(&manifest, &link).unwrap();
    let response = execute(install_request(&directory, &link, &artifacts));
    assert_eq!(response.code, "manifest_unavailable");

    let upgrade = execute(LifecycleRequest::Upgrade {
        schema_version: 1,
        install_root: directory.path().join("install"),
        manifest: directory.path().join("missing-upgrade-manifest"),
        signature: repository.join("tests/fixtures/bundle/manifest.json.sig"),
        artifacts,
        target: "x86_64-unknown-linux-gnu".into(),
        asb_version: "0.1.0".into(),
        protocol_version: 1,
        expected_release: "v0.1.0".into(),
        expected_source_commit: "a7ca8e07f177fc6a647b3297df624137cfb85e86".into(),
        expected_source_tree: "8556090e21336df0766263e6244084865f92d1a3".into(),
        expected_executable_sha256:
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into(),
    });
    assert_eq!(upgrade.code, "manifest_unavailable");
}

#[test]
fn authenticated_manifest_must_match_every_expected_release_identity() {
    let directory = PrivateDirectory::create();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = repository.join("tests/fixtures/bundle/manifest.json");
    let artifacts = directory.path().join("artifacts");
    fs::create_dir(&artifacts).unwrap();
    let mut request = install_request(&directory, &manifest, &artifacts);
    let LifecycleRequest::Install {
        expected_executable_sha256,
        ..
    } = &mut request
    else {
        unreachable!()
    };
    *expected_executable_sha256 = "0".repeat(64);
    let response = execute(request);
    assert_eq!(response.code, "request_release_identity_mismatch");
    assert!(!directory.path().join("install").exists());
}

#[test]
fn superseded_caller_authority_fields_are_rejected_before_mutation() {
    let directory = PrivateDirectory::create();
    let input = format!(
        concat!(
            "{{\"operation\":\"install\",\"schema_version\":1,",
            "\"install_root\":{:?},\"manifest\":\"/manifest\",",
            "\"signature\":\"/signature\",\"artifacts\":\"/artifacts\",",
            "\"allowed_signers\":\"/attacker\",\"now_unix\":1}}"
        ),
        directory.path().join("install").to_string_lossy()
    );
    assert_eq!(
        read_request(input.as_bytes()).unwrap_err(),
        "request_invalid"
    );
    assert!(!directory.path().join("install").exists());
}
