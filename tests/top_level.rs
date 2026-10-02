// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::process::Command;
use std::{
    env, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_asb-tui"))
}

fn local_source_fixture(label: &str) -> (PathBuf, PathBuf, String) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = env::temp_dir().join(format!("asb-tui-{label}-source-{nonce}"));
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let clone = Command::new("git")
        .args([
            "clone",
            "--local",
            "--no-hardlinks",
            "--",
            repository.to_str().unwrap(),
        ])
        .arg(&source)
        .status()
        .unwrap();
    assert!(clone.success());
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
    (source, repository, commit)
}

fn trusted_tool(name: &str) -> String {
    String::from_utf8(Command::new("which").arg(name).output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned()
}

fn isolated_channel_state(label: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "asb-tui-channel-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn tui_lifecycle_commands_default_to_development_mode() {
    let root = env::temp_dir().join(format!("asb-tui-default-dev-{}", std::process::id()));
    let state = isolated_channel_state("default");
    let _ = fs::remove_dir_all(&root);
    let output = binary()
        .args(["tui", "status", "--format", "json"])
        .env("ASB_TUI_DEV_INSTALL_ROOT", &root)
        .env("ASB_TUI_CHANNEL_STATE", &state)
        .output()
        .expect("run asb-tui");
    assert_eq!(output.status.code(), Some(3));
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["channel"], "dev");
    assert_eq!(response["code"], "development_not_installed");
    assert!(output.stderr.is_empty());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_file(state);
}

#[test]
fn tui_lifecycle_defaults_to_human_output_and_json_is_opt_in() {
    let state = isolated_channel_state("output");
    for extra in [vec![], vec!["--json"]] {
        let json = !extra.is_empty();
        let mut args = vec!["tui", "status", "--development"];
        args.extend(extra);
        let output = binary()
            .args(args)
            .env("ASB_TUI_CHANNEL_STATE", &state)
            .output()
            .expect("run asb-tui");
        assert_eq!(output.status.code(), Some(3));
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !json {
            assert!(stdout.contains("channel: dev"));
            assert!(stdout.contains("code: "));
            assert!(!stdout.trim_start().starts_with('{'));
        } else {
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["channel"], "dev");
            assert!(stdout.trim_start().starts_with('{'));
        }
    }
    let _ = fs::remove_file(state);
}

#[test]
fn selected_channel_survives_restart_without_fallback() {
    let state = env::temp_dir().join(format!("asb-tui-channel-state-{}", std::process::id()));
    let _ = fs::remove_file(&state);
    let selected = binary()
        .args(["tui", "status", "--channel", "nightly", "--json"])
        .env("ASB_TUI_CHANNEL_STATE", &state)
        .output()
        .unwrap();
    assert_eq!(selected.status.code(), Some(3));
    let selected_json: serde_json::Value = serde_json::from_slice(&selected.stdout).unwrap();
    assert_eq!(selected_json["channel"], "nightly");
    assert_eq!(selected_json["code"], "nightly_channel_unavailable");

    let restarted = binary()
        .args(["tui", "status", "--json"])
        .env("ASB_TUI_CHANNEL_STATE", &state)
        .output()
        .unwrap();
    assert_eq!(restarted.status.code(), Some(3));
    let restarted_json: serde_json::Value = serde_json::from_slice(&restarted.stdout).unwrap();
    assert_eq!(restarted_json["channel"], "nightly");
    assert_eq!(restarted_json["code"], "nightly_channel_unavailable");
    fs::remove_file(state).unwrap();
}

#[test]
fn explicit_development_marker_overrides_persisted_unavailable_channel() {
    let state = isolated_channel_state("development-override");
    let selected = binary()
        .args(["tui", "status", "--channel", "nightly", "--json"])
        .env("ASB_TUI_CHANNEL_STATE", &state)
        .output()
        .unwrap();
    assert_eq!(selected.status.code(), Some(3));
    let overridden = binary()
        .args(["tui", "status", "--development", "--json"])
        .env("ASB_TUI_CHANNEL_STATE", &state)
        .output()
        .unwrap();
    assert_eq!(overridden.status.code(), Some(3));
    let response: serde_json::Value = serde_json::from_slice(&overridden.stdout).unwrap();
    assert_eq!(response["channel"], "dev");
    assert_eq!(response["code"], "router_request_size_invalid");
    let _ = fs::remove_file(state);
}

#[test]
fn tui_status_uses_the_development_router_and_keeps_request_closed() {
    let state = isolated_channel_state("router");
    let request = br#"{"router_version":1,"profile":"development","channel":"dev","current_main":{"asb_source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","asb_source_tree":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","tui_source_commit":"cccccccccccccccccccccccccccccccccccccccc","tui_source_tree":"dddddddddddddddddddddddddddddddddddddddd"},"request":{"operation":"status","schema_version":1,"install_root":"/tmp/asb-tui-no-such-root"}}"#;
    let output = binary()
        .args(["tui", "status", "--development", "--format", "json"])
        .env("ASB_TUI_CHANNEL_STATE", &state)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(request)?;
            child.wait_with_output()
        })
        .expect("run development router");
    assert_eq!(output.status.code(), Some(3));
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["code"], "install_root_unavailable");
    assert_eq!(response["classification"], "source_only_unverified");
    assert_eq!(response["channel"], "dev");
    let _ = fs::remove_file(state);
}

#[test]
fn unavailable_channels_are_reported_without_falling_back_to_stable_or_dev() {
    let state = isolated_channel_state("unavailable");
    for operation in ["install", "status", "launch", "upgrade", "remove"] {
        for (channel, code) in [
            ("stable", "stable_channel_unavailable"),
            ("nightly", "nightly_channel_unavailable"),
            ("experimental", "experimental_channel_unavailable"),
        ] {
            let output = binary()
                .args(["tui", operation, "--channel", channel, "--format", "json"])
                .env("ASB_TUI_CHANNEL_STATE", &state)
                .output()
                .expect("run channel selector");
            assert_eq!(output.status.code(), Some(3));
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["channel"], channel);
            assert_eq!(response["code"], code);
            assert_eq!(response["ok"], false);
            assert!(output.stderr.is_empty());
        }
    }
    let _ = fs::remove_file(state);
}

#[test]
fn tui_rejects_production_marker_without_dispatching() {
    let output = binary()
        .args(["tui", "remove", "--production", "--format", "json"])
        .output()
        .expect("run asb-tui");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("production_profile_unsupported"));
}

#[test]
fn socket_launcher_rejects_missing_or_untrusted_endpoint() {
    let output = binary()
        .args(["run", "--socket", "/tmp/asb-tui-no-such-control.sock"])
        .output()
        .expect("run asb-tui");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("control socket connection failed"));
}

#[test]
fn dev_channel_materializes_status_launch_upgrade_and_remove_without_auth() {
    let (source, _repository, _commit) = local_source_fixture("top-level");
    let cargo = trusted_tool("cargo");
    let rustc = trusted_tool("rustc");
    let cc = trusted_tool("cc");
    let ar = trusted_tool("ar");
    let ld = trusted_tool("ld");
    let rustup_home = std::env::var_os("RUSTUP_HOME").expect("explicit rustup home");
    let root = std::env::temp_dir().join(format!(
        "asb-tui-ar1579-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let state = isolated_channel_state("materialize");
    let run = |operation: &str| {
        binary()
            .args(["tui", operation, "--channel", "dev", "--format", "json"])
            .env("ASB_TUI_DEV_INSTALL_ROOT", &root)
            .env("ASB_TUI_CHANNEL_STATE", &state)
            .env("ASB_TUI_DEV_REPOSITORY", &source)
            .env("ASB_TUI_DEV_REF", "fixture")
            .env("ASB_TUI_DEV_GIT", "/usr/bin/git")
            .env("ASB_TUI_DEV_SETSID", "/usr/bin/setsid")
            .env("ASB_TUI_DEV_CARGO", &cargo)
            .env("ASB_TUI_DEV_RUSTC", &rustc)
            .env("ASB_TUI_DEV_CC", &cc)
            .env("ASB_TUI_DEV_AR", &ar)
            .env("ASB_TUI_DEV_LD", &ld)
            .env("ASB_TUI_DEV_RUSTUP_HOME", &rustup_home)
            .output()
            .expect("run development lifecycle")
    };
    let installed = run("install");
    assert_eq!(installed.status.code(), Some(0));
    let installed_json: serde_json::Value = serde_json::from_slice(&installed.stdout).unwrap();
    assert_eq!(installed_json["code"], "development_installed");
    assert_eq!(installed_json["development_only"], true);
    assert!(root.join("provenance.json").is_file());
    assert_eq!(run("status").status.code(), Some(0));
    assert_eq!(run("launch").status.code(), Some(0));
    assert_eq!(run("upgrade").status.code(), Some(0));
    assert_eq!(run("remove").status.code(), Some(0));
    assert_eq!(run("status").status.code(), Some(3));
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(source).unwrap();
    fs::remove_file(state).unwrap();
}
