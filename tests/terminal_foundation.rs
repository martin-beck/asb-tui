// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Real pseudo-terminal checks for the standalone Crossterm lifecycle.
#![cfg(unix)]

mod support;

use asb_tui::{
    app::{Action, AppState},
    lifecycle::{FrontendLauncher, Installation, ProcessLauncher},
    renderer,
    runtime::TerminalSession,
    terminal::{CapabilityTier, RenderPolicy},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    process::{Command, Stdio},
};
use support::PrivateDirectory;

const CHILD_MODE: &str = "ASB_TUI_TERMINAL_FOUNDATION_CHILD";

fn under_pty(test: &str, mode: &str) -> std::process::Output {
    let executable = std::env::current_exe().unwrap();
    let command = format!("{} --exact {test} --nocapture", executable.display());
    Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .env(CHILD_MODE, mode)
        .output()
        .unwrap()
}

fn assert_terminal_boundaries(output: &[u8]) {
    for sequence in [
        b"\x1b[?1049h".as_slice(),
        b"\x1b[?1049l".as_slice(),
        b"\x1b[?25l".as_slice(),
        b"\x1b[?25h".as_slice(),
        b"\x1b[?2004h".as_slice(),
        b"\x1b[?2004l".as_slice(),
    ] {
        assert!(
            output
                .windows(sequence.len())
                .any(|window| window == sequence),
            "missing terminal lifecycle sequence {sequence:?}"
        );
    }
}

fn interactive_policy() -> RenderPolicy {
    RenderPolicy {
        tier: CapabilityTier::IndexedColor,
        unicode: true,
        mouse: false,
        focus: false,
        bracketed_paste: true,
        synchronized_output: false,
        alternate_screen: true,
    }
}

fn installation() -> Installation {
    Installation {
        schema_version: 1,
        release: "v0.1.0".into(),
        executable_sha256: "a".repeat(64),
        source_commit: "b".repeat(40),
        source_tree: "c".repeat(40),
        target: "x86_64-unknown-linux-gnu".into(),
        bundle: "linux".into(),
        asb_version: "0.1.0".into(),
        protocol_version: 1,
        coordinator_version: "0.3.5".into(),
        coordinator_commit: "d".repeat(40),
        quality_version: "0.23.0".into(),
        quality_commit: "e".repeat(40),
        classification: "verified_extension".into(),
    }
}

fn draw_once(state: &AppState, policy: RenderPolicy) {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| renderer::render(frame, state, policy))
        .unwrap();
}

#[test]
fn pseudo_terminal_normal_exit_restores_screen_cursor_paste_and_raw_mode() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("normal") {
        let policy = interactive_policy();
        let mut state = AppState::new(80, 24).unwrap();
        let mut session = TerminalSession::enter(policy).unwrap();
        draw_once(&state, policy);
        state.apply(Action::Quit).unwrap();
        assert!(state.should_quit());
        session.restore().unwrap();
        session.restore().unwrap();
        return;
    }
    let output = under_pty(
        "pseudo_terminal_normal_exit_restores_screen_cursor_paste_and_raw_mode",
        "normal",
    );
    assert!(output.status.success(), "nested PTY test failed");
    assert_terminal_boundaries(&output.stdout);
}

#[test]
fn pseudo_terminal_panic_unwind_restores_terminal() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("panic") {
        let policy = interactive_policy();
        let state = AppState::new(80, 24).unwrap();
        let _session = TerminalSession::enter(policy).unwrap();
        draw_once(&state, policy);
        panic!("intentional terminal restoration probe");
    }
    let output = under_pty("pseudo_terminal_panic_unwind_restores_terminal", "panic");
    assert!(
        !output.status.success(),
        "nested panic probe unexpectedly passed"
    );
    assert_terminal_boundaries(&output.stdout);
}

#[test]
fn actual_binary_draws_accepts_quit_and_restores_terminal() {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let command = format!("stty cols 80 rows 24; exec {binary}");
    let mut child = Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("COLUMNS", "80")
        .env("LINES", "24")
        .env_remove("NO_COLOR")
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_TTY")
        .env_remove("TMUX")
        .env_remove("STY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"q").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "interactive binary failed");
    assert_terminal_boundaries(&output.stdout);
    for label in [b"Agent".as_slice(), b"Systems", b"Benchmark", b"Connection"] {
        assert!(
            output
                .stdout
                .windows(label.len())
                .any(|window| window == label),
            "missing rendered label {label:?}"
        );
    }
}

#[test]
fn dumb_tty_uses_plain_output_without_ansi_or_box_drawing() {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let output = Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", binary, "/dev/null"])
        .env("TERM", "dumb")
        .env_remove("COLORTERM")
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert!(output.status.success(), "plain binary failed");
    assert!(!output.stdout.contains(&0x1b));
    assert!(!String::from_utf8_lossy(&output.stdout).contains('┌'));
    assert!(String::from_utf8_lossy(&output.stdout).contains("runner ownership remains external"));
}

#[test]
fn process_launcher_keeps_json_stdout_separate_from_the_controlling_terminal() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("handoff") {
        let response_path = std::env::var_os("ASB_TUI_TEST_RESPONSE").unwrap();
        let response = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(response_path)
            .unwrap();
        rustix::stdio::dup2_stdout(&response).unwrap();
        let executable = fs::read(env!("CARGO_BIN_EXE_asb-tui")).unwrap();
        ProcessLauncher
            .launch_frontend(&installation(), &executable)
            .unwrap();
        io::stdout()
            .write_all(b"{\"code\":\"frontend_exited\"}\n")
            .unwrap();
        io::stdout().flush().unwrap();
        std::process::exit(0);
    }

    let directory = PrivateDirectory::create();
    let response = directory.path().join("response.json");
    let executable = std::env::current_exe().unwrap();
    let command = format!(
        "stty cols 80 rows 24; exec {} --exact process_launcher_keeps_json_stdout_separate_from_the_controlling_terminal --nocapture >/dev/null",
        executable.display()
    );
    let mut child = Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .env(CHILD_MODE, "handoff")
        .env("ASB_TUI_TEST_RESPONSE", &response)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env_remove("NO_COLOR")
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_TTY")
        .env_remove("TMUX")
        .env_remove("STY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"q").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "terminal handoff child failed");
    assert_terminal_boundaries(&output.stdout);
    assert!(
        output
            .stdout
            .windows(b"Connection".len())
            .any(|window| window == b"Connection")
    );
    let response_bytes = fs::read(response).unwrap();
    assert_eq!(response_bytes, b"{\"code\":\"frontend_exited\"}\n");
    let parsed: serde_json::Value = serde_json::from_slice(&response_bytes).unwrap();
    assert_eq!(parsed["code"], "frontend_exited");
}
