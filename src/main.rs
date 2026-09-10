// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

use asb_tui::{
    compatibility::evaluate,
    delegated::execute_input,
    lifecycle::local_self_test_response,
    system_probe::{LocalSystem, detect},
};
use std::{env, process::ExitCode};

const DIAGNOSTIC: &str = concat!(
    "{\"classification\":\"source_only_unverified\",",
    "\"protocol\":\"asb-cli-capabilities\",",
    "\"protocol_version\":1,",
    "\"reason\":\"installed_asb_compatibility_not_verified\"}"
);

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments == ["doctor", "--terminal"] {
        match asb_tui::terminal::doctor(asb_tui::terminal::TerminalEvidence::from_environment()) {
            Ok(report) => {
                println!("{report}");
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("terminal doctor failed: {error}");
                return ExitCode::from(2);
            }
        }
    }
    if arguments == ["lifecycle", "--format", "json"] {
        let response = execute_input(std::io::stdin().lock());
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize lifecycle response")
        );
        return ExitCode::from(if response.ok { 0 } else { 3 });
    }
    if let [command, release_flag, release, format_flag, format] = arguments.as_slice()
        && command == "lifecycle-self-test"
        && release_flag == "--release"
        && format_flag == "--format"
        && format == "json"
    {
        let Some(response) = local_self_test_response(release) else {
            return usage();
        };
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize self-test response")
        );
        return ExitCode::from(if response.ready { 0 } else { 3 });
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
    usage()
}

fn usage() -> ExitCode {
    eprintln!("usage: asb-tui (doctor|compatibility) --format json | doctor --terminal");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::DIAGNOSTIC;

    #[test]
    fn diagnostic_is_content_free_and_explicitly_unverified() {
        assert!(DIAGNOSTIC.contains("source_only_unverified"));
        assert!(DIAGNOSTIC.contains("installed_asb_compatibility_not_verified"));
        assert!(!DIAGNOSTIC.contains('/'));
        assert!(!DIAGNOSTIC.contains("token"));
    }
}
