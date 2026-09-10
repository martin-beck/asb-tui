// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Real pseudo-terminal checks for the standalone Crossterm lifecycle.
#![cfg(unix)]

mod support;

use asb_tui::{
    app::{Action, AppState},
    lifecycle::{FrontendLauncher, Installation, ProcessLauncher},
    renderer,
    runtime::{RuntimeError, TerminalSession},
    terminal::{CapabilityTier, RenderPolicy},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{FileTypeExt, MetadataExt},
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::PrivateDirectory;

const CHILD_MODE: &str = "ASB_TUI_TERMINAL_FOUNDATION_CHILD";

fn under_pty(test: &str, mode: &str) -> std::process::Output {
    let executable = std::env::current_exe().unwrap();
    let command = format!(
        "before=$(stty -g) || exit 90; {} --exact {test} --nocapture; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf '\\nASB_TERMIOS_RESTORED\\n'; exit $status",
        executable.display()
    );
    Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .env(CHILD_MODE, mode)
        .output()
        .unwrap()
}

fn assert_termios_restored(output: &[u8]) {
    assert!(
        output
            .windows(b"ASB_TERMIOS_RESTORED".len())
            .any(|window| window == b"ASB_TERMIOS_RESTORED"),
        "post-exit termios differs from the pre-entry state: {}",
        String::from_utf8_lossy(output)
    );
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
            "missing terminal lifecycle sequence {sequence:?}: {}",
            String::from_utf8_lossy(output)
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
    assert_termios_restored(&output.stdout);
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
    assert_termios_restored(&output.stdout);
}

#[test]
fn pseudo_terminal_error_unwind_restores_terminal_and_termios() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("error") {
        let result: Result<(), RuntimeError> = (|| {
            let policy = interactive_policy();
            let state = AppState::new(80, 24).unwrap();
            let _session = TerminalSession::enter(policy)?;
            draw_once(&state, policy);
            Err(io::Error::other("intentional runtime error").into())
        })();
        assert!(result.is_err());
        return;
    }
    let output = under_pty(
        "pseudo_terminal_error_unwind_restores_terminal_and_termios",
        "error",
    );
    assert!(output.status.success(), "nested error probe failed");
    assert_terminal_boundaries(&output.stdout);
    assert_termios_restored(&output.stdout);
}

#[test]
fn actual_binary_draws_accepts_quit_and_restores_terminal() {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let command = format!(
        "stty cols 80 rows 24; before=$(stty -g) || exit 90; {binary}; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf '\\nASB_TERMIOS_RESTORED\\n'; exit $status"
    );
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
    assert_termios_restored(&output.stdout);
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

fn signal_binary(signal_script: &str) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let command = format!(
        "ulimit -c 0; before=$(stty -g) || exit 90; wait_raw() {{ i=0; while test \"$(stty -g)\" = \"$before\"; do i=$((i+1)); test $i -lt 500 || exit 94; sleep 0.01; done; }}; {binary} </dev/tty >/dev/tty 2>/dev/tty & pid=$!; wait_raw; {signal_script}; wait $pid; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf '\\nASB_TERMIOS_RESTORED\\n'; exit $status"
    );
    Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env_remove("NO_COLOR")
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_TTY")
        .env_remove("TMUX")
        .env_remove("STY")
        .output()
        .unwrap()
}

#[test]
fn termination_signals_restore_termios_before_default_exit() {
    for signal in ["TERM", "HUP", "INT", "QUIT"] {
        for repetition in 0..2 {
            let output = signal_binary(&format!("kill -{signal} $pid"));
            assert!(
                !output.status.success(),
                "{signal} probe {repetition} unexpectedly succeeded"
            );
            assert_terminal_boundaries(&output.stdout);
            assert_termios_restored(&output.stdout);
        }
    }
}

#[test]
fn suspend_restores_and_continue_reenters_before_termination() {
    for repetition in 0..2 {
        let output = signal_binary(
            "kill -TSTP $pid; i=0; while state=$(ps -o stat= -p $pid); do case $state in T*) break;; esac; i=$((i+1)); test $i -lt 500 || exit 93; sleep 0.01; done; test \"$(stty -g)\" = \"$before\" || exit 95; kill -CONT $pid; wait_raw; kill -TERM $pid",
        );
        assert!(
            !output.status.success(),
            "signal probe {repetition} unexpectedly passed"
        );
        assert_termios_restored(&output.stdout);
        assert!(
            output
                .stdout
                .windows(b"\x1b[?1049h".len())
                .filter(|window| *window == b"\x1b[?1049h")
                .count()
                >= 2,
            "alternate screen was not re-entered after SIGCONT"
        );
        assert!(
            output
                .stdout
                .windows(b"\x1b[?1049l".len())
                .filter(|window| *window == b"\x1b[?1049l")
                .count()
                >= 2,
            "alternate screen was not restored before suspend and termination"
        );
    }
}

#[test]
fn no_color_tty_remains_interactive_without_color_or_unicode() {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let command = format!(
        "before=$(stty -g) || exit 90; {binary}; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf '\\nASB_TERMIOS_RESTORED\\n'; exit $status"
    );
    let mut child = Command::new("/usr/bin/script")
        .args(["-q", "-e", "-c", &command, "/dev/null"])
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"q").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "NO_COLOR UI failed");
    assert_termios_restored(&output.stdout);
    assert!(
        output
            .stdout
            .windows(b"\x1b[?1049h".len())
            .any(|window| window == b"\x1b[?1049h")
    );
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(
        rendered.is_ascii(),
        "NO_COLOR output contained Unicode glyphs"
    );
    for color in ["\u{1b}[36m", "\u{1b}[90m", "\u{1b}[39m"] {
        assert!(!rendered.contains(color), "unexpected color SGR {color:?}");
    }
}

#[test]
fn huge_initial_pty_size_fails_before_ratatui_allocation() {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let command = format!(
        "stty cols 65535 rows 65535 || exit 89; before=$(stty -g) || exit 90; {binary}; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf '\\nASB_TERMIOS_RESTORED\\n'; test $status -eq 2"
    );
    let output = Command::new("/usr/bin/timeout")
        .args([
            "--signal=KILL",
            "10s",
            "/usr/bin/script",
            "-q",
            "-e",
            "-c",
            &command,
            "/dev/null",
        ])
        .env("TERM", "xterm-256color")
        .output()
        .unwrap();
    assert!(output.status.success(), "huge initial PTY probe failed");
    assert_termios_restored(&output.stdout);
    assert!(
        !output
            .stdout
            .windows(b"\x1b[?1049h".len())
            .any(|window| window == b"\x1b[?1049h"),
        "oversized initial terminal entered the alternate screen"
    );
}

#[test]
fn runtime_resize_storm_fails_closed_and_restores_termios() {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let command = format!(
        "stty cols 80 rows 24; before=$(stty -g) || exit 90; wait_raw() {{ i=0; while test \"$(stty -g)\" = \"$before\"; do i=$((i+1)); test $i -lt 500 || exit 94; sleep 0.01; done; }}; {binary} </dev/tty >/dev/tty 2>/dev/tty & pid=$!; wait_raw; i=0; while test $i -lt 32; do stty cols 65535 rows 65535; kill -WINCH $pid 2>/dev/null || break; i=$((i+1)); done; i=0; while kill -0 $pid 2>/dev/null; do i=$((i+1)); test $i -lt 500 || {{ kill -KILL $pid; exit 93; }}; sleep 0.01; done; wait $pid; status=$?; stty cols 80 rows 24; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf '\\nASB_TERMIOS_RESTORED\\n'; test $status -eq 2"
    );
    let output = Command::new("/usr/bin/timeout")
        .args([
            "--signal=KILL",
            "15s",
            "/usr/bin/script",
            "-q",
            "-e",
            "-c",
            &command,
            "/dev/null",
        ])
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env_remove("NO_COLOR")
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_TTY")
        .env_remove("TMUX")
        .env_remove("STY")
        .output()
        .unwrap();
    assert!(output.status.success(), "resize-storm PTY probe failed");
    assert_terminal_boundaries(&output.stdout);
    assert_termios_restored(&output.stdout);
}

fn wait_for_marker(path: &std::path::Path) -> bool {
    for _ in 0..500 {
        if path.is_file() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn bounded_command(program: &str, args: &[&str]) -> bool {
    Command::new("/usr/bin/timeout")
        .args(["--signal=KILL", "3s", program])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

enum MultiplexerSession {
    Tmux { socket: String, session: String },
    Screen { session: String },
}

impl MultiplexerSession {
    fn tmux_socket_path(socket: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/tmux-{}/{}",
            rustix::process::getuid().as_raw(),
            socket
        ))
    }

    fn stop(&self) {
        match self {
            Self::Tmux { socket, .. } => {
                let _ = bounded_command("/usr/bin/tmux", &["-L", socket, "kill-server"]);
                let path = Self::tmux_socket_path(socket);
                if fs::symlink_metadata(&path).is_ok_and(|metadata| {
                    metadata.file_type().is_socket()
                        && metadata.uid() == rustix::process::getuid().as_raw()
                }) {
                    let _ = fs::remove_file(path);
                }
            }
            Self::Screen { session } => {
                let _ = bounded_command("/usr/bin/screen", &["-S", session, "-X", "quit"]);
            }
        }
    }

    fn is_gone(&self) -> bool {
        match self {
            Self::Tmux { socket, session } => {
                !bounded_command(
                    "/usr/bin/tmux",
                    &["-L", socket, "has-session", "-t", session],
                ) && !Self::tmux_socket_path(socket).exists()
            }
            Self::Screen { session } => {
                !bounded_command("/usr/bin/screen", &["-S", session, "-Q", "select", "."])
            }
        }
    }
}

impl Drop for MultiplexerSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn wait_for_rendered_screen(mut capture: impl FnMut() -> Option<String>) -> bool {
    for _ in 0..100 {
        if capture().is_some_and(|text| text.contains("Connection")) {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn local_tmux_and_screen_sessions_quit_and_restore_termios() {
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let directory = PrivateDirectory::create();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let tmux_marker = directory.path().join("tmux-restored");
    let tmux_session = format!("asb-tui-test-{}-{nonce}", std::process::id());
    let tmux_socket = format!("asb-tui-socket-{}-{nonce}", std::process::id());
    let tmux_command = format!(
        "before=$(stty -g) || exit 90; {binary}; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf restored >{}; exit $status",
        tmux_marker.display()
    );
    assert!(bounded_command(
        "/usr/bin/tmux",
        &[
            "-L",
            &tmux_socket,
            "new-session",
            "-d",
            "-s",
            &tmux_session,
            "-x",
            "80",
            "-y",
            "24",
            &tmux_command,
        ],
    ));
    let tmux_guard = MultiplexerSession::Tmux {
        socket: tmux_socket.clone(),
        session: tmux_session.clone(),
    };
    assert!(
        wait_for_rendered_screen(|| {
            Command::new("/usr/bin/timeout")
                .args([
                    "--signal=KILL",
                    "3s",
                    "/usr/bin/tmux",
                    "-L",
                    &tmux_socket,
                    "capture-pane",
                    "-p",
                    "-t",
                    &tmux_session,
                ])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        }),
        "tmux UI did not reach its first rendered frame"
    );
    assert!(bounded_command(
        "/usr/bin/tmux",
        &["-L", &tmux_socket, "send-keys", "-t", &tmux_session, "q"],
    ));
    let tmux_restored = wait_for_marker(&tmux_marker);
    assert!(tmux_restored, "tmux session did not restore termios");
    tmux_guard.stop();
    assert!(tmux_guard.is_gone(), "tmux session leaked after cleanup");

    let screen_marker = directory.path().join("screen-restored");
    let screen_capture = directory.path().join("screen.capture");
    let screen_session = format!("asb-tui-test-{}-{nonce}", std::process::id());
    let screen_command = format!(
        "before=$(stty -g) || exit 90; {binary}; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf restored >{}; exit $status",
        screen_marker.display()
    );
    assert!(bounded_command(
        "/usr/bin/screen",
        &["-dmS", &screen_session, "sh", "-c", &screen_command],
    ));
    let screen_guard = MultiplexerSession::Screen {
        session: screen_session.clone(),
    };
    assert!(
        wait_for_rendered_screen(|| {
            let capture = screen_capture.to_string_lossy();
            if !bounded_command(
                "/usr/bin/screen",
                &["-S", &screen_session, "-p", "0", "-X", "hardcopy", &capture],
            ) {
                return None;
            }
            fs::read_to_string(&screen_capture).ok()
        }),
        "GNU screen UI did not reach its first rendered frame"
    );
    assert!(bounded_command(
        "/usr/bin/screen",
        &["-S", &screen_session, "-p", "0", "-X", "stuff", "q"],
    ));
    let screen_restored = wait_for_marker(&screen_marker);
    assert!(
        screen_restored,
        "GNU screen session did not restore termios"
    );
    screen_guard.stop();
    assert!(
        screen_guard.is_gone(),
        "GNU screen session leaked after cleanup"
    );
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
