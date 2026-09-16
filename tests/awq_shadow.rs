// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::{fs, process::Command};

#[test]
fn shadow_workflow_runs_only_after_native_gates_and_stays_advisory() {
    let workflow = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/awq-shadow.yml"
    ))
    .unwrap();
    assert!(workflow.contains("uses: ./.github/workflows/quality.yml"));
    assert!(workflow.contains("concurrency-scope: awq-shadow"));
    assert!(workflow.contains("needs: native-gates"));
    assert!(workflow.contains("continue-on-error: true"));
    assert!(workflow.contains("AWQ v0.32.0 shadow classification"));
    for classification in ["pass", "failure", "skip", "unsupported", "tool-error"] {
        assert!(workflow.contains(classification));
    }
    assert!(!workflow.contains("push:\n    branches: [main]"));

    let native = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/quality.yml"
    ))
    .unwrap();
    assert!(native.contains("inputs.concurrency-scope || 'direct'"));
    assert!(native.contains("default: direct"));
}

#[test]
fn shadow_runner_rejects_tampered_wheel_before_python_execution() {
    let root = env!("CARGO_MANIFEST_DIR");
    let wheel = std::env::temp_dir().join(format!("asb-tui-awq-tampered-{}", std::process::id()));
    fs::write(&wheel, b"not the authenticated wheel").unwrap();
    let output = Command::new(format!("{root}/tools/run-awq-shadow.sh"))
        .args(["/path/that/must/not/run", wheel.to_str().unwrap(), "doctor"])
        .output()
        .unwrap();
    let _ = fs::remove_file(wheel);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wheel digest mismatch"));
}

#[test]
fn shadow_runner_fails_closed_on_missing_arguments_and_version_drift() {
    let root = env!("CARGO_MANIFEST_DIR");
    let runner = format!("{root}/tools/run-awq-shadow.sh");
    let output = Command::new(&runner).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let source = fs::read_to_string(runner).unwrap();
    assert!(source.contains("readonly AWQ_VERSION=0.32.0"));
    assert!(source.contains("installed version does not match v0.32.0"));
}

#[test]
fn awq_pin_matches_signed_dependency_provenance() {
    let lock = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/provenance/dependencies.lock.json"
    ))
    .unwrap();
    for value in [
        "v0.32.0",
        "d6b82279556871e2f9f3c3d691b8d3f50e1487ac",
        "f1c40859e9d10cacbc79100ed136c38bce48cd39",
        "d3e6109919fa857f20476d4faec181868b9d4050",
        "9430730c581af22d4d639413014fd46162f12fb06df957cca05d445c5fbb3ed9",
        "bf610d925de51dfd643e47c467bd82fcad128dc43793bae1a0aeebd5b5ea4848",
    ] {
        assert!(lock.contains(value));
    }
}
