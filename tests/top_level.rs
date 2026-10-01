// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::process::Command;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_asb-tui"))
}

#[test]
fn tui_lifecycle_commands_require_explicit_development_mode() {
    let output = binary()
        .args(["tui", "status", "--format", "json"])
        .output()
        .expect("run asb-tui");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("development_marker_required"));
}

#[test]
fn tui_status_uses_the_development_router_and_keeps_request_closed() {
    let request = br#"{"router_version":1,"profile":"development","channel":"dev","current_main":{"asb_source_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","asb_source_tree":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","tui_source_commit":"cccccccccccccccccccccccccccccccccccccccc","tui_source_tree":"dddddddddddddddddddddddddddddddddddddddd"},"request":{"operation":"status","schema_version":1,"install_root":"/tmp/asb-tui-no-such-root"}}"#;
    let output = binary()
        .args(["tui", "status", "--development", "--format", "json"])
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
    let root = std::env::temp_dir().join(format!(
        "asb-tui-ar1579-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let run = |operation: &str| {
        binary()
            .args(["tui", operation, "--channel", "dev", "--format", "json"])
            .env("ASB_TUI_DEV_INSTALL_ROOT", &root)
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
}
