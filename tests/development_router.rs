// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn router_cli_exposes_explicit_development_provenance() {
    let root = std::env::temp_dir().join(format!(
        "asb-tui-development-router-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir(root.join("versions")).unwrap();
    std::fs::set_permissions(
        root.join("versions"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::write(root.join(".lifecycle.lock"), b"").unwrap();
    std::fs::set_permissions(
        root.join(".lifecycle.lock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let request = format!(
        r#"{{"router_version":1,"profile":"development","request":{{"operation":"status","schema_version":1,"install_root":{root:?}}}}}"#,
        root = root.to_string_lossy()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["router", "--format", "json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["profile"], "development");
    assert_eq!(value["development_only"], true);
    assert_eq!(value["lifecycle"]["code"], "extension_not_installed");
    std::fs::remove_dir_all(root).unwrap();
}
