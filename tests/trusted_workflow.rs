// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::{fs, process::Command};

#[test]
fn trusted_runner_workflow_is_main_only_and_exact_revision_bound() {
    let workflow = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/trusted-main.yml"
    ))
    .unwrap();
    assert!(workflow.contains("branches: [main]"));
    assert!(workflow.contains("workflow_dispatch:"));
    assert!(!workflow.contains("pull_request"));
    assert!(workflow.contains("github.repository == 'martin-beck/asb-tui'"));
    assert!(workflow.contains("github.ref == 'refs/heads/main'"));
    assert!(
        workflow
            .contains("github.event_name == 'push' || github.event_name == 'workflow_dispatch'")
    );
    assert!(workflow.contains("ref: ${{ github.sha }}"));
    assert!(workflow.contains("persist-credentials: false"));
    assert!(workflow.contains("remote_main=$(git ls-remote --exit-code origin refs/heads/main"));
    assert!(workflow.contains("tools/verify-trusted-trigger.sh"));
    assert!(workflow.contains("runs-on: [asb-development-v1-x86_64-ubuntu2404]"));
    assert!(!workflow.contains("${{ secrets."));
    let preflight = workflow
        .find("Reject every untrusted trigger before checkout")
        .unwrap();
    let checkout = workflow.find("uses: actions/checkout@").unwrap();
    let binding = workflow
        .find("Bind checkout to event SHA and current public main")
        .unwrap();
    assert!(preflight < checkout && checkout < binding);

    let hosted = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/quality.yml"
    ))
    .unwrap();
    assert!(hosted.contains("pull_request:"));
    assert!(hosted.contains("runs-on: ubuntu-24.04"));
    assert!(!hosted.contains("asb-development-v1-x86_64-ubuntu2404"));
    for workflow in [&workflow, &hosted] {
        assert!(workflow.contains("python3 tools/validate-compatibility-schemas.py"));
    }
}

#[test]
fn trusted_trigger_validator_rejects_wrong_event_ref_repo_and_revision() {
    let verifier = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tools/verify-trusted-trigger.sh"
    );
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let valid = [
        "push",
        "martin-beck/asb-tui",
        "refs/heads/main",
        sha,
        sha,
        sha,
    ];
    assert!(
        Command::new(verifier)
            .args(valid)
            .status()
            .unwrap()
            .success()
    );
    for (index, value) in [
        (0, "pull_request"),
        (1, "fork/asb-tui"),
        (2, "refs/heads/feature"),
        (3, "not-a-sha"),
        (4, "ffffffffffffffffffffffffffffffffffffffff"),
        (5, "ffffffffffffffffffffffffffffffffffffffff"),
    ] {
        let mut hostile = valid;
        hostile[index] = value;
        assert!(
            !Command::new(verifier)
                .args(hostile)
                .status()
                .unwrap()
                .success()
        );
    }
}

#[test]
fn every_referenced_action_is_commit_sha_pinned() {
    for path in [
        "quality.yml",
        "development-runner-canary.yml",
        "trusted-main.yml",
    ] {
        let workflow = fs::read_to_string(format!(
            "{}/.github/workflows/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        for line in workflow
            .lines()
            .filter(|line| line.trim_start().starts_with("uses:"))
        {
            let revision = line
                .split_once('@')
                .expect("action reference must have @")
                .1;
            let revision = revision.split_whitespace().next().unwrap();
            assert_eq!(revision.len(), 40, "action is not commit pinned: {line}");
            assert!(
                revision
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
        }
    }
}

#[test]
fn every_checked_out_workflow_rejects_tracked_and_untracked_dirt() {
    for path in ["quality.yml", "trusted-main.yml"] {
        let workflow = fs::read_to_string(format!(
            "{}/.github/workflows/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(workflow.contains("CARGO_TARGET_DIR: ${{ runner.temp }}/asb-tui-target"));
        assert!(workflow.contains("git diff --exit-code"));
        assert!(workflow.contains("test -z \"$(git status --porcelain)\""));
    }
    let trusted = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/trusted-main.yml"
    ))
    .unwrap();
    assert!(trusted.contains("tools/run-coverage-clean.sh"));
}
