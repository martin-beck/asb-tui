// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

use asb_tui::{
    app::AppState,
    broker_adoption::{receive_from_stdin, validate_channel_shape},
    compatibility::evaluate,
    control_codec::ControlLimits,
    control_transport::AuthenticatedBrokerSession,
    delegated::execute_input,
    development_journey::execute_input as execute_development_journey_input,
    development_onboarding::execute_input as execute_development_onboarding_input,
    development_router::execute_input as execute_development_router_input,
    lifecycle::{local_self_test_response, run_self_test_supervisor},
    output_contract::{Route as OutputRoute, render as render_output},
    runtime::{
        run_interactive, run_interactive_with_control, run_interactive_with_control_context,
    },
    system_probe::{LocalSystem, detect},
    terminal::{RenderPolicy, TerminalEvidence},
    terminal_handoff::{
        preflight_terminal_path, redirect_stdin_to_controlling_terminal,
        redirect_stdin_to_development_terminal, redirect_stdin_to_terminal_path,
    },
    top_level::{self, TuiCommand},
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
    if let Some(arguments) = arguments.strip_prefix(&["__self-test-supervisor".to_owned()]) {
        return ExitCode::from(if run_self_test_supervisor(arguments).is_ok() {
            0
        } else {
            126
        });
    }
    if arguments == ["run", "--broker"] {
        return launch_broker_entry(false);
    }
    if let Some(route) = development_broker_route(&arguments) {
        return launch_development_broker_entry(route);
    }
    if arguments.len() == 3 && arguments[0] == "run" && arguments[1] == "--socket" {
        return launch_socket_entry(&arguments[2]);
    }
    if let Some(tui_arguments) = arguments.strip_prefix(&["tui".to_owned()]) {
        return launch_tui_command(tui_arguments);
    }
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
    if arguments == ["preflight", "--format", "json"] || arguments == ["preflight", "--json"] {
        let report = asb_tui::development_preflight::run();
        println!(
            "{}",
            serde_json::to_string(&report).expect("serialize preflight report")
        );
        return ExitCode::from(if report.ok { 0 } else { 3 });
    }
    if arguments == ["preflight"] {
        let report = asb_tui::development_preflight::run();
        for line in report.human_lines() {
            println!("{line}");
        }
        return ExitCode::from(if report.ok { 0 } else { 3 });
    }
    if let Some(route) = arguments
        .first()
        .and_then(|value| OutputRoute::parse(value))
        && arguments.as_slice() != ["doctor", "--format", "json"]
        && arguments.len() <= 3
    {
        let json = match arguments.get(1).map(String::as_str) {
            None => false,
            Some("--json") | Some("--format")
                if arguments.get(2).is_some_and(|value| value == "json") =>
            {
                true
            }
            Some("--json") if arguments.len() == 2 => true,
            _ => return usage(),
        };
        print!("{}", render_output(route, json));
        return ExitCode::from(3);
    }
    if arguments == ["lifecycle", "--format", "json"] {
        let response = execute_input(std::io::stdin().lock());
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize lifecycle response")
        );
        return ExitCode::from(if response.ok { 0 } else { 3 });
    }
    if arguments == ["router", "--format", "json"] {
        let response = execute_development_router_input(std::io::stdin().lock());
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize development router response")
        );
        return ExitCode::from(if response.lifecycle.ok { 0 } else { 3 });
    }
    if arguments == ["onboarding", "--format", "json"] {
        let response = execute_development_onboarding_input(std::io::stdin().lock());
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize development onboarding response")
        );
        return ExitCode::from(if response.ready { 0 } else { 3 });
    }
    if arguments == ["journey", "--format", "json"] {
        let response = execute_development_journey_input(std::io::stdin().lock());
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize development journey response")
        );
        return ExitCode::from(if response.ready { 0 } else { 3 });
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
    eprintln!(
        "usage: asb-tui [run|run --broker|run --broker --development [--live-provider|--dynamic-catalog]|run --socket PATH] | tui | tui <install|upgrade|status|launch|remove> --channel dev [--json|--format json] | tui <install|upgrade|status|launch|remove> --development [--json|--format json] | (doctor|compatibility|lifecycle|router|onboarding|journey) --format json | doctor --terminal"
    );
    ExitCode::from(2)
}

fn development_broker_route(arguments: &[String]) -> Option<Option<&'static str>> {
    if arguments.len() == 3
        && arguments[0..3]
            == [
                "run".to_owned(),
                "--broker".to_owned(),
                "--development".to_owned(),
            ]
    {
        return Some(None);
    }
    if arguments.len() != 4
        || arguments[0..3]
            != [
                "run".to_owned(),
                "--broker".to_owned(),
                "--development".to_owned(),
            ]
    {
        return None;
    }
    match arguments[3].as_str() {
        "--live-provider" => Some(Some("live-provider")),
        "--dynamic-catalog" => Some(Some("dynamic-catalog")),
        _ => None,
    }
}

fn print_lifecycle_response<T: serde::Serialize>(response: &T, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(response).expect("serialize lifecycle response")
        );
        return;
    }
    let value = serde_json::to_value(response).expect("serialize lifecycle response");
    for key in [
        "channel",
        "code",
        "ok",
        "classification",
        "development_only",
        "installed",
        "verified",
        "source_commit",
        "source_tree",
    ] {
        if let Some(value) = value.get(key).filter(|value| !value.is_null()) {
            let rendered = value
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| value.to_string());
            println!("{key}: {rendered}");
        }
    }
    if let Some(warnings) = value.get("warnings") {
        println!("warnings: {warnings}");
    }
}

fn launch_tui_command(arguments: &[String]) -> ExitCode {
    let json = arguments
        .windows(2)
        .any(|window| window == ["--format", "json"])
        || arguments.iter().any(|argument| argument == "--json");
    let command = match top_level::parse(arguments) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{}: {}", error.code(), top_level::usage());
            return ExitCode::from(2);
        }
    };
    // An explicit selector is durable across restart and reconfiguration. A
    // failed write is surfaced as a diagnostic instead of pretending the
    // choice was retained.
    if arguments.windows(2).any(|window| window[0] == "--channel")
        && let TuiCommand::Lifecycle { selection, .. } = command
        && let Err(code) =
            asb_tui::channel_selection::ChannelSelection::persist(selection.requested)
    {
        let response = asb_tui::delegated::LifecycleResponse::result(false, code)
            .with_channel(selection.requested.as_str());
        print_lifecycle_response(&response, json);
        return ExitCode::from(3);
    }
    match command {
        TuiCommand::LaunchUi => launch(),
        TuiCommand::Lifecycle {
            operation,
            channel,
            selection,
            ..
        } => {
            if let Some(code) = selection.warning {
                let response = asb_tui::delegated::LifecycleResponse::result(false, code)
                    .with_channel(selection.requested.as_str());
                print_lifecycle_response(&response, json);
                return ExitCode::from(3);
            }
            if let TuiCommand::Lifecycle {
                channel_dev: true, ..
            } = command
            {
                let response = asb_tui::development_lifecycle::execute(operation.as_str());
                print_lifecycle_response(&response, json);
                ExitCode::from(if response.ok { 0 } else { 3 })
            } else {
                let response =
                    top_level::execute_lifecycle(operation, channel, std::io::stdin().lock());
                print_lifecycle_response(&response, json);
                ExitCode::from(if response.ok { 0 } else { 3 })
            }
        }
    }
}

/// Consume the broker's inherited fd-0 handoff without entering the UI.
///
/// The received channel is intentionally not treated as authenticated merely
/// because it arrived over SCM_RIGHTS: the typed generation/identity
/// negotiation still has to be implemented by the control client. Keeping
/// this path fail-closed also ensures lifecycle JSON remains exclusively on
/// `lifecycle --format json` and can never be confused with broker traffic.
fn launch_broker_entry(development_mode: bool) -> ExitCode {
    let received = match receive_from_stdin() {
        Ok(received) => received,
        Err(_) => {
            eprintln!("broker channel adoption failed");
            return ExitCode::from(2);
        }
    };
    if validate_channel_shape(received.channel()).is_err() {
        eprintln!("broker channel adoption failed");
        return ExitCode::from(2);
    }
    let mut control =
        match AuthenticatedBrokerSession::establish_from_broker(received, ControlLimits::default())
        {
            Ok(control) => control,
            Err(_) => {
                eprintln!("broker control handshake failed");
                return ExitCode::from(2);
            }
        };
    let terminal_result = if development_mode {
        match development_terminal_path(
            development_mode,
            env::var_os("ASB_TUI_DEVELOPMENT_TERMINAL_PATH"),
        ) {
            Some(path) => {
                let path = std::path::Path::new(&path);
                if preflight_terminal_path(path).is_err() {
                    eprintln!("environment_path_invalid");
                    return ExitCode::from(2);
                }
                redirect_stdin_to_terminal_path(path)
            }
            None => redirect_stdin_to_development_terminal(),
        }
    } else {
        redirect_stdin_to_controlling_terminal()
    };
    if terminal_result.is_err() {
        if development_mode && env::var_os("ASB_TUI_DEVELOPMENT_TERMINAL_PATH").is_some() {
            eprintln!("environment_path_invalid");
        } else {
            eprintln!("controlling terminal unavailable");
        }
        return ExitCode::from(2);
    }
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
    if !policy.alternate_screen {
        eprintln!("interactive terminal required for broker mode");
        return ExitCode::from(2);
    }
    let mut state =
        match AppState::new(evidence.columns.unwrap_or(80), evidence.lines.unwrap_or(24)) {
            Ok(state) => state,
            Err(_) => {
                eprintln!("terminal capability check failed");
                return ExitCode::from(2);
            }
        };
    if development_mode {
        let preflight = asb_tui::development_preflight::run_for_launch();
        if !preflight.ok {
            for line in preflight.human_lines() {
                eprintln!("{line}");
            }
            return ExitCode::from(2);
        }
    }
    match run_interactive_with_control_context(&mut state, policy, &mut control, development_mode) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("terminal application failed");
            ExitCode::from(2)
        }
    }
}

fn development_terminal_path(
    development_mode: bool,
    configured: Option<std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    development_mode.then_some(configured).flatten()
}

fn launch_socket_entry(path: &str) -> ExitCode {
    let Ok(mut control) =
        AuthenticatedBrokerSession::connect(std::path::Path::new(path), ControlLimits::default())
    else {
        eprintln!("control socket connection failed");
        return ExitCode::from(2);
    };
    launch_authenticated_control(&mut control)
}

fn launch_authenticated_control(control: &mut AuthenticatedBrokerSession) -> ExitCode {
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
    if !policy.alternate_screen {
        eprintln!("interactive terminal required for control socket mode");
        return ExitCode::from(2);
    }
    let mut state =
        match AppState::new(evidence.columns.unwrap_or(80), evidence.lines.unwrap_or(24)) {
            Ok(state) => state,
            Err(_) => {
                eprintln!("terminal capability check failed");
                return ExitCode::from(2);
            }
        };
    match run_interactive_with_control(&mut state, policy, control) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("terminal application failed");
            ExitCode::from(2)
        }
    }
}

fn launch_development_broker_entry(route: Option<&str>) -> ExitCode {
    if asb_tui::development_broker::DevelopmentBrokerDescriptor::from_env().is_err() {
        eprintln!("development broker descriptor rejected");
        return ExitCode::from(3);
    }
    // The paired ASB router uses these route markers to distinguish its
    // lifecycle variants. Both variants intentionally enter the same
    // authenticated broker startup: the first projection refresh requests
    // the authoritative provider catalog and the existing readiness logic
    // selects the appropriate wizard/live handoff without a fallback.
    let _ = route;
    launch_broker_entry(true)
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
    use super::{DIAGNOSTIC, development_broker_route, development_terminal_path};
    use asb_tui::terminal_handoff::preflight_terminal_path;
    use std::ffi::OsString;

    #[test]
    fn diagnostic_is_content_free_and_explicitly_unverified() {
        assert!(DIAGNOSTIC.contains("source_only_unverified"));
        assert!(DIAGNOSTIC.contains("installed_asb_compatibility_not_verified"));
        assert!(!DIAGNOSTIC.contains('/'));
        assert!(!DIAGNOSTIC.contains("token"));
    }

    #[test]
    fn stable_broker_mode_ignores_development_terminal_path() {
        let configured = Some(OsString::from("/dev/pts/7"));
        assert_eq!(development_terminal_path(false, configured), None);
        assert_eq!(
            development_terminal_path(true, Some(OsString::from("/dev/pts/7"))),
            Some(OsString::from("/dev/pts/7"))
        );
    }

    #[test]
    fn development_terminal_preflight_accepts_only_numeric_pts_paths() {
        assert!(preflight_terminal_path(std::path::Path::new("/dev/pts/7")).is_ok());
        for value in [
            "",
            "/dev/null",
            "relative",
            "/dev/pts/+1",
            "/dev/pts/../null",
        ] {
            assert_eq!(
                preflight_terminal_path(std::path::Path::new(value)),
                Err("environment_path_invalid")
            );
        }
    }

    #[test]
    fn development_broker_accepts_paired_frontend_routes() {
        let args = |values: &[&str]| -> Vec<String> {
            values.iter().map(|value| (*value).to_owned()).collect()
        };
        assert_eq!(
            development_broker_route(&args(&["run", "--broker", "--development"])),
            Some(None)
        );
        assert_eq!(
            development_broker_route(&args(&[
                "run",
                "--broker",
                "--development",
                "--live-provider"
            ])),
            Some(Some("live-provider"))
        );
        assert_eq!(
            development_broker_route(&args(&[
                "run",
                "--broker",
                "--development",
                "--dynamic-catalog"
            ])),
            Some(Some("dynamic-catalog"))
        );
        assert_eq!(
            development_broker_route(&args(&["run", "--broker", "--development", "--unexpected"])),
            None
        );
    }
}
