// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    compatibility::{Multiplexer, RuntimeProbe, evaluate},
    system_probe::{ProbeSource, detect},
};
use std::collections::BTreeMap;

struct FakeSource {
    os: &'static str,
    architecture: &'static str,
    os_release: Option<Vec<u8>>,
    version: Option<Vec<u8>>,
    capabilities: Option<Vec<u8>>,
    dimensions: Option<(u16, u16)>,
    terminal: bool,
    resize_verified: bool,
    environment: BTreeMap<&'static str, String>,
    runtime: RuntimeProbe,
}

impl ProbeSource for FakeSource {
    fn os(&self) -> &str {
        self.os
    }
    fn architecture(&self) -> &str {
        self.architecture
    }
    fn os_release(&self) -> Option<Vec<u8>> {
        self.os_release.clone()
    }
    fn asb_version(&self) -> Option<Vec<u8>> {
        self.version.clone()
    }
    fn asb_capabilities(&self) -> Option<Vec<u8>> {
        self.capabilities.clone()
    }
    fn terminal_dimensions(&self) -> Option<(u16, u16)> {
        self.dimensions
    }
    fn stdin_is_terminal(&self) -> bool {
        self.terminal
    }
    fn resize_events_verified(&self) -> bool {
        self.resize_verified
    }
    fn environment(&self, name: &str) -> Option<String> {
        self.environment.get(name).cloned()
    }
    fn runtime(&self) -> RuntimeProbe {
        self.runtime.clone()
    }
}

fn source() -> FakeSource {
    FakeSource {
        os: "linux",
        architecture: "x86_64",
        os_release: Some(b"NAME=Ubuntu\nID=ubuntu\nVERSION_ID=\"24.04\"\n".to_vec()),
        version: Some(b"asb 0.1.0\n".to_vec()),
        capabilities: Some(br#"{"protocol":"asb-cli-capabilities","protocol_version":1,"asb_version":"0.1.0","capabilities":{"analysis":true,"artifacts":true,"cancel":true,"events":true,"history":true,"launch":true,"planning":true,"repeat":true}}"#.to_vec()),
        dimensions: Some((120, 40)),
        terminal: true,
        resize_verified: true,
        environment: BTreeMap::from([
            ("TERM", "xterm-256color".into()),
            ("LANG", "C.UTF-8".into()),
        ]),
        runtime: RuntimeProbe {
            config_writable: true,
            cache_writable: true,
            atomic_rename: true,
            executable_files: true,
            git_available: true,
            ssh_keygen_available: true,
        },
    }
}

#[test]
fn injected_source_proves_real_detection_path_can_select() {
    let report = evaluate(detect(&source()));
    assert_eq!(report.bundle, Some("asb-tui-v1-linux-x86_64"));
    assert!(report.reasons.is_empty());
}

#[test]
fn version_99_and_absent_or_mismatched_protocol_fail_closed() {
    let mut hostile = source();
    hostile.version = Some(b"asb 99.0.0\n".to_vec());
    hostile.capabilities = Some(br#"{"protocol":"asb-cli-capabilities","protocol_version":1,"asb_version":"99.0.0","capabilities":{"analysis":true,"artifacts":true,"cancel":true,"events":true,"history":true,"launch":true,"planning":true,"repeat":true}}"#.to_vec());
    assert_eq!(
        evaluate(detect(&hostile)).reasons,
        ["unsupported_asb_version"]
    );

    let mut absent = source();
    absent.capabilities = None;
    assert_eq!(evaluate(detect(&absent)).reasons, ["protocol_unavailable"]);

    let mut mismatch = source();
    mismatch.capabilities = Some(br#"{"protocol":"asb-cli-capabilities","protocol_version":1,"asb_version":"0.2.0","capabilities":{"analysis":true,"artifacts":true,"cancel":true,"events":true,"history":true,"launch":true,"planning":true,"repeat":true}}"#.to_vec());
    assert_eq!(
        evaluate(detect(&mismatch)).reasons,
        ["protocol_unavailable"]
    );
}

#[test]
fn missing_extra_or_false_required_capability_never_selects() {
    let original: serde_json::Value =
        serde_json::from_slice(source().capabilities.as_deref().unwrap()).unwrap();
    for capability in [
        "analysis",
        "artifacts",
        "cancel",
        "events",
        "history",
        "launch",
        "planning",
        "repeat",
    ] {
        let mut value = original.clone();
        value["capabilities"][capability] = false.into();
        let mut hostile = source();
        hostile.capabilities = Some(serde_json::to_vec(&value).unwrap());
        assert_eq!(evaluate(detect(&hostile)).reasons, ["protocol_unavailable"]);
    }

    let mut missing = original.clone();
    missing["capabilities"]
        .as_object_mut()
        .unwrap()
        .remove("launch");
    let mut hostile = source();
    hostile.capabilities = Some(serde_json::to_vec(&missing).unwrap());
    assert_eq!(evaluate(detect(&hostile)).reasons, ["protocol_unavailable"]);

    let mut extra = original;
    extra["capabilities"]["host_value"] = true.into();
    hostile.capabilities = Some(serde_json::to_vec(&extra).unwrap());
    assert_eq!(evaluate(detect(&hostile)).reasons, ["protocol_unavailable"]);
}

#[test]
fn hostile_os_release_command_outputs_and_unknown_platform_normalize_safely() {
    for bytes in [Some(vec![0xff]), Some(vec![b'x'; 16 * 1024 + 1]), None] {
        let mut hostile = source();
        hostile.os_release = bytes;
        assert!(
            evaluate(detect(&hostile))
                .reasons
                .contains(&"unsupported_distribution")
        );
    }
    let mut unknown = source();
    unknown.os = "private-system-name";
    unknown.architecture = "private-cpu-name";
    let json = serde_json::to_string(&evaluate(detect(&unknown))).unwrap();
    assert!(json.contains("unsupported_os"));
    assert!(json.contains("unsupported_architecture"));
    assert!(!json.contains("private"));
}

#[test]
fn terminal_channel_resize_and_boolean_only_session_context_are_detected() {
    let mut remote = source();
    remote
        .environment
        .insert("SSH_CONNECTION", "sensitive endpoint tuple".into());
    remote
        .environment
        .insert("TMUX", "sensitive socket path".into());
    let report = evaluate(detect(&remote));
    assert!(report.terminal.ssh);
    assert_eq!(report.terminal.multiplexer, Multiplexer::Tmux);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("sensitive"));

    remote.terminal = false;
    let report = evaluate(detect(&remote));
    assert!(report.reasons.contains(&"non_interactive_channel"));
    assert!(report.reasons.contains(&"resize_events_unavailable"));
    assert_eq!(report.terminal.columns, 0);
    assert_eq!(report.terminal.rows, 0);

    let mut unverified = source();
    unverified.resize_verified = false;
    assert_eq!(
        evaluate(detect(&unverified)).reasons,
        ["resize_events_unavailable"]
    );
}

#[test]
fn unavailable_oversized_non_utf8_and_timeout_equivalents_never_infer_facts() {
    for version in [None, Some(vec![0xff]), Some(vec![b'9'; 129])] {
        let mut hostile = source();
        hostile.version = version;
        assert!(
            evaluate(detect(&hostile))
                .reasons
                .contains(&"asb_version_unavailable")
        );
    }
    let mut unavailable = source();
    unavailable.dimensions = None;
    let report = evaluate(detect(&unavailable));
    assert!(report.reasons.contains(&"terminal_too_small"));
    assert!(report.reasons.contains(&"resize_events_unavailable"));
}
