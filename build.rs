// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::process::Command;

fn main() {
    for key in ["ASB_TUI_SOURCE_COMMIT", "ASB_TUI_SOURCE_TREE"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let commit = std::env::var("ASB_TUI_SOURCE_COMMIT")
        .ok()
        .or_else(|| git_rev_parse("HEAD"));
    let tree = std::env::var("ASB_TUI_SOURCE_TREE")
        .ok()
        .or_else(|| git_rev_parse("HEAD^{tree}"));
    if let Some(value) = commit {
        println!("cargo:rustc-env=ASB_TUI_SOURCE_COMMIT={value}");
    }
    if let Some(value) = tree {
        println!("cargo:rustc-env=ASB_TUI_SOURCE_TREE={value}");
    }
    println!("cargo:rerun-if-changed=.git/HEAD");
}

fn git_rev_parse(spec: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", spec])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then_some(value)
}
