// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use asb_tui::compatibility::{
    Architecture, CompatibilityProbe, Distribution, Multiplexer, OperatingSystem, TerminalChannel,
    evaluate, parse_probe,
};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
use support::PrivateDirectory;

const COMPATIBLE: &str = include_str!("fixtures/compatibility/compatible.json");
const MISMATCH: &str = include_str!("fixtures/compatibility/mismatch.json");
const MALFORMED: &str = include_str!("fixtures/compatibility/malformed.json");

fn compatible() -> CompatibilityProbe {
    parse_probe(COMPATIBLE).unwrap()
}

fn run_cli(input: &str) -> std::process::Output {
    let directory = PrivateDirectory::create();
    let path = directory.path().to_owned();
    let mut child = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["compatibility", "--format", "json"])
        .env_clear()
        .current_dir(directory.path())
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
    drop(directory);
    assert!(!path.exists());
    output
}

#[test]
fn selects_only_normalized_compatible_bundles_deterministically() {
    let x86 = evaluate(compatible());
    assert_eq!(x86.classification, "compatible");
    assert_eq!(x86.bundle, Some("asb-tui-v1-linux-x86_64"));
    assert!(x86.reasons.is_empty());
    assert_eq!(
        serde_json::to_string(&x86).unwrap(),
        serde_json::to_string(&evaluate(compatible())).unwrap()
    );

    let mut arm = compatible();
    arm.platform.architecture = Architecture::Aarch64;
    assert_eq!(evaluate(arm).bundle, Some("asb-tui-v1-linux-aarch64"));
}

#[test]
fn mismatch_fixture_fails_closed_with_fixed_privacy_safe_reasons() {
    let report = evaluate(parse_probe(MISMATCH).unwrap());
    assert_eq!(report.classification, "unsupported");
    assert_eq!(report.bundle, None);
    assert_eq!(
        report.reasons,
        [
            "filesystem_incompatible",
            "inconsistent_platform",
            "non_interactive_channel",
            "release_verifier_unavailable",
            "resize_events_unavailable",
            "storage_unavailable",
            "terminal_features_unavailable",
            "terminal_too_small",
            "tooling_identity_mismatch",
            "unsupported_architecture",
            "unsupported_os",
            "unsupported_protocol",
        ]
    );
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("00000000"));
    assert!(!json.contains("host"));
}

#[test]
fn resize_and_channel_boundaries_are_exact() {
    for (columns, rows, expected) in [(79, 24, false), (80, 23, false), (80, 24, true)] {
        let mut probe = compatible();
        probe.terminal.columns = columns;
        probe.terminal.rows = rows;
        assert_eq!(evaluate(probe).bundle.is_some(), expected);
    }
    let mut resized = compatible();
    resized.terminal.columns = 20;
    assert_eq!(evaluate(resized.clone()).reasons, ["terminal_too_small"]);
    resized.terminal.columns = 100;
    assert!(evaluate(resized).reasons.is_empty());

    let mut pipe = compatible();
    pipe.terminal.channel = TerminalChannel::Pipe;
    assert_eq!(evaluate(pipe).reasons, ["non_interactive_channel"]);
}

#[test]
fn ssh_tmux_and_screen_are_normalized_without_identifiers() {
    for multiplexer in [Multiplexer::None, Multiplexer::Tmux, Multiplexer::Screen] {
        let mut probe = compatible();
        probe.terminal.ssh = true;
        probe.terminal.multiplexer = multiplexer;
        let report = evaluate(probe);
        assert!(report.terminal.ssh);
        assert_eq!(report.terminal.multiplexer, multiplexer);
        assert!(report.reasons.is_empty());
    }
}

#[test]
fn every_runtime_requirement_and_identity_mismatch_blocks_selection() {
    for field in 0..6 {
        let mut probe = compatible();
        match field {
            0 => probe.runtime.config_writable = false,
            1 => probe.runtime.cache_writable = false,
            2 => probe.runtime.atomic_rename = false,
            3 => probe.runtime.executable_files = false,
            4 => probe.runtime.git_available = false,
            5 => probe.runtime.ssh_keygen_available = false,
            _ => unreachable!(),
        }
        assert!(evaluate(probe).bundle.is_none());
    }
    for field in 0..4 {
        let mut probe = compatible();
        match field {
            0 => probe.dependencies.coordinator_version = "v0.0.0".into(),
            1 => probe
                .dependencies
                .coordinator_commit
                .replace_range(..1, "0"),
            2 => probe.dependencies.quality_version = "v0.0.0".into(),
            3 => probe.dependencies.quality_commit.replace_range(..1, "0"),
            _ => unreachable!(),
        }
        assert_eq!(evaluate(probe).reasons, ["tooling_identity_mismatch"]);
    }
}

#[test]
fn unsupported_platforms_and_inconsistent_distributions_fail_closed() {
    for (os, distribution) in [
        (OperatingSystem::Macos, Distribution::NotApplicable),
        (OperatingSystem::Windows, Distribution::NotApplicable),
        (OperatingSystem::Linux, Distribution::Other),
        (OperatingSystem::Linux, Distribution::NotApplicable),
    ] {
        let mut probe = compatible();
        probe.platform.os = os;
        probe.platform.distribution = distribution;
        assert!(evaluate(probe).bundle.is_none());
    }
}

#[test]
fn malformed_unknown_oversized_and_hostile_versions_are_generic() {
    let error = parse_probe(MALFORMED).unwrap_err();
    assert_eq!(error, "invalid compatibility probe");
    assert!(!error.contains("must-not-echo"));
    assert_eq!(
        parse_probe(&" ".repeat(65_537)).unwrap_err(),
        "compatibility probe exceeds size limit"
    );
    for version in [
        "",
        "1",
        "1.2",
        "1.2.3-",
        "1.2.3+build",
        "1.2.3/host",
        "1.2.3\nsecret",
    ] {
        let mut value: serde_json::Value = serde_json::from_str(COMPATIBLE).unwrap();
        value["asb"]["version"] = version.into();
        assert_eq!(
            parse_probe(&serde_json::to_string(&value).unwrap()).unwrap_err(),
            "invalid ASB version"
        );
    }
}

#[test]
fn executable_report_uses_stdin_and_stable_exit_classes() {
    let compatible = run_cli(COMPATIBLE);
    assert_eq!(compatible.status.code(), Some(0));
    assert!(compatible.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&compatible.stdout).unwrap();
    assert_eq!(report["classification"], "compatible");

    let mismatch = run_cli(MISMATCH);
    assert_eq!(mismatch.status.code(), Some(3));
    assert!(mismatch.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&mismatch.stdout).unwrap()["classification"],
        "unsupported"
    );

    let malformed = run_cli(MALFORMED);
    assert_eq!(malformed.status.code(), Some(2));
    assert!(malformed.stdout.is_empty());
    assert_eq!(malformed.stderr, b"invalid compatibility probe\n");
    assert!(
        !String::from_utf8(malformed.stderr)
            .unwrap()
            .contains("must-not-echo")
    );
}

#[test]
fn schemas_and_fixtures_are_closed_json_documents() {
    for path in [
        "protocol/v1/capabilities.schema.json",
        "protocol/v1/compatibility-probe.schema.json",
        "protocol/v1/compatibility-report.schema.json",
        "tests/fixtures/compatibility/compatible.json",
        "tests/fixtures/compatibility/mismatch.json",
        "tests/fixtures/compatibility/malformed.json",
    ] {
        let value: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap(),
        )
        .unwrap();
        assert!(value.is_object());
    }
    let schema = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/protocol/v1/compatibility-probe.schema.json"
    ))
    .unwrap();
    assert_eq!(schema.matches("\"additionalProperties\": false").count(), 6);
    let report_schema = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/protocol/v1/compatibility-report.schema.json"
    ))
    .unwrap();
    assert_eq!(
        report_schema
            .matches("\"additionalProperties\": false")
            .count(),
        3
    );
}
