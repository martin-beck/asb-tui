// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

use asb_tui::{
    app::AppState,
    compatibility::evaluate,
    delegated::execute_input,
    lifecycle::local_self_test_response,
    runtime::run_interactive,
    system_probe::{LocalSystem, detect},
    terminal::{RenderPolicy, TerminalEvidence},
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
    if arguments.is_empty() || arguments == ["run"] {
        return launch();
    }
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
    if let [
        command,
        release_flag,
        release,
        asb_flag,
        asb_version,
        protocol_flag,
        protocol_version,
        format_flag,
        format,
    ] = arguments.as_slice()
        && command == "lifecycle-self-test"
        && release_flag == "--release"
        && asb_flag == "--asb-version"
        && protocol_flag == "--protocol-version"
        && format_flag == "--format"
        && format == "json"
    {
        let Some(response) = protocol_version
            .parse()
            .ok()
            .and_then(|protocol| local_self_test_response(release, asb_version, protocol))
        else {
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
    eprintln!("usage: asb-tui [run] | (doctor|compatibility) --format json | doctor --terminal");
    ExitCode::from(2)
}

fn launch() -> ExitCode {
    let mut evidence = TerminalEvidence::from_environment();
    if evidence.tty
        && let Ok((columns, lines)) = crossterm::terminal::size()
        && columns > 0
        && lines > 0
    {
        evidence.columns = Some(columns);
        evidence.lines = Some(lines);
    }
    let Ok(policy) = RenderPolicy::from_evidence(&evidence) else {
        eprintln!("terminal capability check failed");
        return ExitCode::from(2);
    };
    let mut state =
        match AppState::new(evidence.columns.unwrap_or(80), evidence.lines.unwrap_or(24)) {
            Ok(state) => state,
            Err(_) => {
                eprintln!("terminal capability check failed");
                return ExitCode::from(2);
            }
        };
    if !policy.alternate_screen {
        print!("{}", state.plain_text());
        return ExitCode::SUCCESS;
    }
    match run_interactive(&mut state, policy) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("terminal application failed");
            ExitCode::from(2)
        }
    }
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
