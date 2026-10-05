// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::{
    env, fs,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn local_source_fixture() -> (PathBuf, String) {
    let source = env::temp_dir().join(format!(
        "asb-tui-clone-source-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(
        Command::new("git")
            .args([
                "clone",
                "--local",
                "--no-hardlinks",
                "--",
                repository.to_str().unwrap()
            ])
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&source)
            .args(["switch", "-c", "fixture"])
            .status()
            .unwrap()
            .success()
    );
    let commit = String::from_utf8(
        Command::new("git")
            .current_dir(&source)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    (source, commit)
}

fn trusted_tool(name: &str) -> String {
    String::from_utf8(Command::new("which").arg(name).output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned()
}

fn private_toolchain_tool(name: &str) -> String {
    let candidate = trusted_tool(name);
    let metadata = fs::symlink_metadata(&candidate).expect("tool metadata");
    if !metadata.file_type().is_symlink() {
        return candidate;
    }
    let output = Command::new("rustup")
        .args(["which", name])
        .output()
        .expect("resolve rustup toolchain tool");
    assert!(
        output.status.success(),
        "rustup which {name} failed: {output:?}"
    );
    let resolved = String::from_utf8(output.stdout)
        .expect("rustup tool path is utf-8")
        .trim()
        .to_owned();
    let resolved_metadata = fs::symlink_metadata(&resolved).expect("resolved tool metadata");
    assert!(
        resolved_metadata.is_file()
            && !resolved_metadata.file_type().is_symlink()
            && resolved_metadata.mode() & 0o022 == 0,
        "rustup resolved an unsafe tool: {resolved}"
    );
    resolved
}

#[test]
fn local_clone_build_records_exact_provenance_and_cleans_workspace() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("asb-tui-clone-build-{nonce}"));
    let (repository, expected_commit) = local_source_fixture();
    let cargo = private_toolchain_tool("cargo");
    let rustc = private_toolchain_tool("rustc");
    let cc = trusted_tool("cc");
    let ar = trusted_tool("ar");
    let ld = trusted_tool("ld");

    let run_install = |install_root: &PathBuf, manifest: Option<&PathBuf>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_asb-tui"));
        command
            .env("ASB_TUI_DEV_REPOSITORY", &repository)
            .env("ASB_TUI_DEV_REF", "fixture")
            .env("ASB_TUI_DEV_GIT", "/usr/bin/git")
            .env("ASB_TUI_DEV_SETSID", "/usr/bin/setsid")
            .env("ASB_TUI_DEV_CARGO", &cargo)
            .env("ASB_TUI_DEV_RUSTC", &rustc)
            .env("ASB_TUI_DEV_CC", &cc)
            .env("ASB_TUI_DEV_AR", &ar)
            .env("ASB_TUI_DEV_LD", &ld)
            .env("PATH", "/nonexistent")
            .env("ASB_TUI_DEV_INSTALL_ROOT", install_root)
            .args(["tui", "install", "--channel", "dev", "--format", "json"]);
        if let Some(manifest) = manifest {
            command.env("ASB_TUI_CHANNEL_MANIFEST", manifest);
        }
        command.output().unwrap()
    };
    let output = run_install(&root, None);
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
    let second_root = env::temp_dir().join(format!("asb-tui-clone-build-second-{nonce}"));
    let manifest_path = env::temp_dir().join(format!("asb-tui-clone-build-manifest-{nonce}.json"));
    let first_executable = fs::read(root.join("asb-tui")).unwrap();
    let manifest = serde_json::json!({
        "schema_version": 1,
        "channel": "dev",
        "development_only": true,
        "asb_repository": "https://github.com/martin-beck/agent-systems-benchmark.git",
        "asb_ref": "refs/heads/main",
        "asb_source_commit": "a".repeat(40),
        "asb_source_tree": "b".repeat(40),
        "tui_repository": "https://github.com/martin-beck/asb-tui.git",
        "tui_ref": "refs/heads/main",
        "tui_source_commit": response["source_commit"],
        "tui_source_tree": response["source_tree"],
        "executable_sha256": response["executable_sha256"],
        "executable_size": first_executable.len(),
        "built_unix": 1,
        "warnings": []
    });
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let second = run_install(&second_root, Some(&manifest_path));
    assert!(second.status.success(), "{second:?}");
    let second_response: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_response["code"], "development_installed");
    assert_eq!(second_response["verified"], true);
    assert_eq!(
        second_response["executable_sha256"],
        response["executable_sha256"]
    );
    assert_eq!(
        fs::read(second_root.join("asb-tui")).unwrap(),
        first_executable
    );
    fs::remove_dir_all(second_root).unwrap();
    fs::remove_file(manifest_path).unwrap();
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(repository).unwrap();
}
