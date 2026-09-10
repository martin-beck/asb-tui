// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

use asb_tui::{
    compatibility::{
        COORDINATOR_COMMIT, COORDINATOR_VERSION, QUALITY_COMMIT, QUALITY_VERSION, evaluate,
    },
    delegated::{execute, read_request},
    system_probe::{LocalSystem, detect},
};
use std::{env, process::ExitCode};

const DIAGNOSTIC: &str = concat!(
    "{\"classification\":\"unverified_extension\",",
    "\"protocol\":\"asb-cli-capabilities\",",
    "\"protocol_version\":1,",
    "\"reason\":\"installed_asb_compatibility_not_verified\"}"
);

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments == ["lifecycle", "--format", "json"] {
        let response = match read_request(std::io::stdin().lock()) {
            Ok(request) => execute(request),
            Err(code) => asb_tui::delegated::LifecycleResponse {
                schema_version: 1,
                classification: "unverified_extension",
                ok: false,
                code,
                installed: None,
                verified: None,
                release: None,
                executable_sha256: None,
            },
        };
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize lifecycle response")
        );
        return ExitCode::from(if response.ok { 0 } else { 3 });
    }
    if let [command, release_flag, release, format_flag, format] = arguments.as_slice()
        && command == "lifecycle-self-test"
        && release_flag == "--release"
        && valid_release(release)
        && format_flag == "--format"
        && format == "json"
    {
        let report = evaluate(detect(&LocalSystem));
        let ready = report.bundle.is_some();
        println!(
            "{}",
            serde_json::json!({
                "schema_version": 1,
                "classification": "unverified_extension",
                "release": release,
                "protocol_version": 1,
                "coordinator_version": COORDINATOR_VERSION,
                "coordinator_commit": COORDINATOR_COMMIT,
                "quality_version": QUALITY_VERSION,
                "quality_commit": QUALITY_COMMIT,
                "ready": ready,
            })
        );
        return ExitCode::from(if ready { 0 } else { 3 });
    }
    if arguments == ["compatibility", "--format", "json"] {
        let report = evaluate(detect(&LocalSystem));
        println!(
            "{}",
            serde_json::to_string(&report).expect("serialize report")
        );
        return ExitCode::from(if report.bundle.is_some() { 0 } else { 3 });
    }
    if arguments == ["doctor", "--format", "json"] {
        println!("{DIAGNOSTIC}");
        return ExitCode::from(3);
    }
    eprintln!("usage: asb-tui (doctor|compatibility) --format json");
    ExitCode::from(2)
}

fn valid_release(value: &str) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    value.len() <= 32
        && version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::DIAGNOSTIC;

    #[test]
    fn diagnostic_is_content_free_and_explicitly_unverified() {
        assert!(DIAGNOSTIC.contains("unverified_extension"));
        assert!(DIAGNOSTIC.contains("installed_asb_compatibility_not_verified"));
        assert!(!DIAGNOSTIC.contains('/'));
        assert!(!DIAGNOSTIC.contains("token"));
    }
}
