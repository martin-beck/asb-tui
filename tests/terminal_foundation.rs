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
    io::{self, Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt},
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::PrivateDirectory;

const CHILD_MODE: &str = "ASB_TUI_TERMINAL_FOUNDATION_CHILD";
const MAX_MULTIPLEXER_OUTPUT: usize = 4_096;
const RENDER_WAIT_ATTEMPTS: usize = 100;

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

fn bounded_output(program: &str, args: &[&str]) -> Option<(bool, Vec<u8>)> {
    let mut child = Command::new("/usr/bin/timeout")
        .args(["--signal=KILL", "3s", program])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut output = Vec::new();
    child
        .stdout
        .take()?
        .take((MAX_MULTIPLEXER_OUTPUT + 1) as u64)
        .read_to_end(&mut output)
        .ok()?;
    let status = child.wait().ok()?;
    (output.len() <= MAX_MULTIPLEXER_OUTPUT).then_some((status.success(), output))
}

fn tmux_args<'a>(socket: &'a str, args: &'a [&'a str]) -> Vec<&'a str> {
    let mut command = vec!["-f", "/dev/null", "-L", socket];
    command.extend_from_slice(args);
    command
}

fn bounded_tmux(socket: &str, args: &[&str]) -> bool {
    bounded_command("/usr/bin/tmux", &tmux_args(socket, args))
}

fn bounded_tmux_output(socket: &str, args: &[&str]) -> Option<(bool, Vec<u8>)> {
    bounded_output("/usr/bin/tmux", &tmux_args(socket, args))
}

enum MultiplexerSession {
    Tmux {
        socket: String,
        session: String,
        process_group: Option<u32>,
    },
    Screen {
        session: String,
    },
}

impl MultiplexerSession {
    fn set_process_group(&mut self, validated_process_group: u32) {
        match self {
            Self::Tmux { process_group, .. } => {
                assert!(process_group.is_none());
                *process_group = Some(validated_process_group);
            }
            Self::Screen { .. } => panic!("screen sessions have no tmux pane process group"),
        }
    }

    fn tmux_socket_path(socket: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/tmux-{}/{}",
            rustix::process::getuid().as_raw(),
            socket
        ))
    }

    fn process_group_is_gone(process_group: u32) -> bool {
        !bounded_command("/bin/kill", &["-0", "--", &format!("-{process_group}")])
    }

    fn stop_process_group(socket: &str, session: &str, process_group: u32) -> bool {
        if Self::process_group_is_gone(process_group) {
            return true;
        }
        if tmux_pane_process_group(socket, session) != Some(process_group) {
            return false;
        }
        let group = format!("-{process_group}");
        let _ = bounded_command("/bin/kill", &["-TERM", "--", &group]);
        for _ in 0..100 {
            if Self::process_group_is_gone(process_group) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = bounded_command("/bin/kill", &["-KILL", "--", &group]);
        for _ in 0..100 {
            if Self::process_group_is_gone(process_group) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn stop(&self) -> bool {
        match self {
            Self::Tmux {
                socket,
                session,
                process_group,
                ..
            } => {
                let group_clean = process_group
                    .is_some_and(|group| Self::stop_process_group(socket, session, group));
                let _ = bounded_tmux(socket, &["kill-server"]);
                let path = Self::tmux_socket_path(socket);
                if fs::symlink_metadata(&path).is_ok_and(|metadata| {
                    metadata.file_type().is_socket()
                        && metadata.uid() == rustix::process::getuid().as_raw()
                }) {
                    let _ = fs::remove_file(path);
                }
                group_clean
            }
            Self::Screen { session } => {
                let _ = bounded_command("/usr/bin/screen", &["-S", session, "-X", "quit"]);
                true
            }
        }
    }

    fn is_gone(&self) -> bool {
        match self {
            Self::Tmux {
                socket,
                session,
                process_group,
            } => {
                !bounded_tmux(socket, &["has-session", "-t", session])
                    && !Self::tmux_socket_path(socket).exists()
                    && process_group.is_some_and(Self::process_group_is_gone)
            }
            Self::Screen { session } => {
                !bounded_command("/usr/bin/screen", &["-S", session, "-Q", "select", "."])
            }
        }
    }
}

impl Drop for MultiplexerSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PaneSummary {
    Unavailable,
    Oversized,
    NonUtf8 { bytes: usize },
    Recognized { bytes: usize, labels: String },
    Unrecognized { bytes: usize },
}

fn pane_summary(capture: Option<Vec<u8>>) -> PaneSummary {
    let Some(capture) = capture else {
        return PaneSummary::Unavailable;
    };
    if capture.len() > MAX_MULTIPLEXER_OUTPUT {
        return PaneSummary::Oversized;
    }
    let Ok(text) = std::str::from_utf8(&capture) else {
        return PaneSummary::NonUtf8 {
            bytes: capture.len(),
        };
    };
    let labels = [
        "Agent Systems Benchmark",
        "Connection",
        "Last event",
        "runner ownership remains external",
    ]
    .into_iter()
    .filter(|label| text.contains(label))
    .collect::<Vec<_>>()
    .join("|");
    if labels.is_empty() {
        PaneSummary::Unrecognized {
            bytes: capture.len(),
        }
    } else {
        PaneSummary::Recognized {
            bytes: capture.len(),
            labels,
        }
    }
}

fn wait_for_rendered_screen(
    attempts: usize,
    delay: Duration,
    mut capture: impl FnMut() -> Option<Vec<u8>>,
) -> Result<(), PaneSummary> {
    let mut last = PaneSummary::Unavailable;
    for _ in 0..attempts {
        let summary = pane_summary(capture());
        if frame_is_recognized(&summary) {
            return Ok(());
        }
        last = summary;
        thread::sleep(delay);
    }
    Err(last)
}

fn frame_is_recognized(summary: &PaneSummary) -> bool {
    matches!(
        summary,
        PaneSummary::Recognized { labels, .. }
            if labels.split('|').any(|label| label == "Connection")
    )
}

fn wait_for_tmux_rendered_screen(
    attempts: usize,
    delay: Duration,
    mut capture: impl FnMut() -> Option<(bool, Vec<u8>, bool)>,
) -> Result<(), PaneSummary> {
    let mut last = PaneSummary::Unavailable;
    for _ in 0..attempts {
        let observed = capture();
        let coherent_alternate = observed
            .as_ref()
            .is_some_and(|(before, _, after)| *before && *after);
        let summary = pane_summary(observed.map(|(_, pane, _)| pane));
        if coherent_alternate && frame_is_recognized(&summary) {
            return Ok(());
        }
        last = summary;
        thread::sleep(delay);
    }
    Err(last)
}

fn tmux_alternate_on(socket: &str, session: &str) -> Option<bool> {
    let output = bounded_tmux_output(
        socket,
        &["display-message", "-p", "-t", session, "#{alternate_on}"],
    )
    .filter(|(success, _)| *success)
    .map(|(_, output)| output)?;
    match output.as_slice() {
        b"0\n" => Some(false),
        b"1\n" => Some(true),
        _ => None,
    }
}

fn tmux_pane_process_group(socket: &str, session: &str) -> Option<u32> {
    let pane_pid = bounded_tmux_output(
        socket,
        &["display-message", "-p", "-t", session, "#{pane_pid}"],
    )
    .filter(|(success, _)| *success)
    .and_then(|(_, output)| String::from_utf8(output).ok())?
    .trim()
    .parse::<u32>()
    .ok()
    .filter(|pid| *pid > 1)?;
    let pid = pane_pid.to_string();
    let process_group = bounded_output("/usr/bin/ps", &["-o", "pgid=", "-p", &pid])
        .filter(|(success, _)| *success)
        .and_then(|(_, output)| String::from_utf8(output).ok())?
        .trim()
        .parse::<u32>()
        .ok()?;
    (pane_pid == process_group).then_some(process_group)
}

fn tmux_visible_alternate_capture(socket: &str, session: &str) -> Option<(bool, Vec<u8>, bool)> {
    let before = tmux_alternate_on(socket, session)?;
    let pane = bounded_tmux_output(socket, &["capture-pane", "-p", "-t", session])
        .filter(|(success, _)| *success)
        .map(|(_, output)| output)?;
    let after = tmux_alternate_on(socket, session)?;
    Some((before, pane, after))
}

fn diagnostic_value(value: Option<&str>, kind: &str) -> String {
    let Some(value) = value else {
        return "unavailable".into();
    };
    let valid = match kind {
        "boolean" => matches!(value, "0" | "1"),
        "status" => {
            value.is_empty()
                || (value.len() <= 10 && value.bytes().all(|byte| byte.is_ascii_digit()))
        }
        "command" => {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
        }
        _ => false,
    };
    if valid {
        if value.is_empty() {
            "none".into()
        } else {
            value.into()
        }
    } else {
        "malformed".into()
    }
}

fn tmux_diagnostics(socket: &str, session: &str, pane: &PaneSummary) -> String {
    let format = concat!(
        "alternate_on=#{alternate_on}\n",
        "pane_dead=#{pane_dead}\n",
        "pane_dead_status=#{pane_dead_status}\n",
        "pane_current_command=#{pane_current_command}"
    );
    let output = bounded_tmux_output(socket, &["display-message", "-p", "-t", session, format])
        .filter(|(success, _)| *success)
        .and_then(|(_, output)| String::from_utf8(output).ok());
    let mut alternate_on = None;
    let mut pane_dead = None;
    let mut pane_dead_status = None;
    let mut pane_current_command = None;
    if let Some(output) = output.as_deref() {
        for line in output.lines() {
            if let Some(value) = line.strip_prefix("alternate_on=") {
                alternate_on = Some(value);
            } else if let Some(value) = line.strip_prefix("pane_dead=") {
                pane_dead = Some(value);
            } else if let Some(value) = line.strip_prefix("pane_dead_status=") {
                pane_dead_status = Some(value);
            } else if let Some(value) = line.strip_prefix("pane_current_command=") {
                pane_current_command = Some(value);
            }
        }
    }
    format!(
        "pane={pane:?} alternate_on={} pane_dead={} pane_dead_status={} pane_current_command={}",
        diagnostic_value(alternate_on, "boolean"),
        diagnostic_value(pane_dead, "boolean"),
        diagnostic_value(pane_dead_status, "status"),
        diagnostic_value(pane_current_command, "command"),
    )
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
    assert!(bounded_tmux(
        &tmux_socket,
        &[
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
    let mut tmux_guard = MultiplexerSession::Tmux {
        socket: tmux_socket.clone(),
        session: tmux_session.clone(),
        process_group: None,
    };
    let tmux_process_group = tmux_pane_process_group(&tmux_socket, &tmux_session)
        .expect("tmux pane must own its process group");
    tmux_guard.set_process_group(tmux_process_group);
    let readiness =
        wait_for_tmux_rendered_screen(RENDER_WAIT_ATTEMPTS, Duration::from_millis(25), || {
            tmux_visible_alternate_capture(&tmux_socket, &tmux_session)
        });
    if let Err(pane) = readiness {
        let diagnostic = tmux_diagnostics(&tmux_socket, &tmux_session, &pane);
        assert!(
            tmux_guard.stop(),
            "tmux cleanup ownership could not be revalidated"
        );
        assert!(
            tmux_guard.is_gone(),
            "tmux cleanup failed after readiness timeout"
        );
        panic!("tmux UI did not reach its first rendered frame: {diagnostic}");
    }
    assert!(bounded_tmux(
        &tmux_socket,
        &["send-keys", "-t", &tmux_session, "q"],
    ));
    let tmux_restored = wait_for_marker(&tmux_marker);
    assert!(tmux_restored, "tmux session did not restore termios");
    assert!(
        tmux_guard.stop(),
        "tmux cleanup ownership could not be revalidated"
    );
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
        wait_for_rendered_screen(RENDER_WAIT_ATTEMPTS, Duration::from_millis(25), || {
            let capture = screen_capture.to_string_lossy();
            if !bounded_command(
                "/usr/bin/screen",
                &["-S", &screen_session, "-p", "0", "-X", "hardcopy", &capture],
            ) {
                return None;
            }
            fs::File::open(&screen_capture).ok().and_then(|file| {
                let mut output = Vec::new();
                file.take((MAX_MULTIPLEXER_OUTPUT + 1) as u64)
                    .read_to_end(&mut output)
                    .ok()?;
                (output.len() <= MAX_MULTIPLEXER_OUTPUT).then_some(output)
            })
        })
        .is_ok(),
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
    assert!(screen_guard.stop(), "screen cleanup failed");
    assert!(
        screen_guard.is_gone(),
        "GNU screen session leaked after cleanup"
    );
}

#[test]
fn tmux_guard_reaps_a_hup_resistant_owned_pane_group_after_readiness_failure() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session = format!("asb-tui-cleanup-{}-{nonce}", std::process::id());
    let socket = format!("asb-tui-cleanup-socket-{}-{nonce}", std::process::id());
    assert!(bounded_tmux(
        &socket,
        &[
            "new-session",
            "-d",
            "-s",
            &session,
            "trap '' HUP; while :; do sleep 1; done",
        ],
    ));
    let mut guard = MultiplexerSession::Tmux {
        socket: socket.clone(),
        session: session.clone(),
        process_group: None,
    };
    let process_group =
        tmux_pane_process_group(&socket, &session).expect("tmux pane must own its process group");
    guard.set_process_group(process_group);
    let unrelated_group = u32::try_from(rustix::process::getpgrp().as_raw_pid()).unwrap();
    assert_ne!(unrelated_group, process_group);
    assert!(
        !MultiplexerSession::stop_process_group(&socket, &session, unrelated_group),
        "mismatched live process group must not be signalled"
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            guard.set_process_group(unrelated_group);
        }))
        .is_err(),
        "duplicate process-group assignment unexpectedly succeeded"
    );
    assert!(
        !MultiplexerSession::process_group_is_gone(process_group),
        "owned pane group changed during mismatch rejection"
    );
    assert_eq!(
        wait_for_tmux_rendered_screen(1, Duration::ZERO, || None),
        Err(PaneSummary::Unavailable)
    );
    assert!(
        guard.stop(),
        "tmux cleanup ownership could not be revalidated"
    );
    assert!(
        guard.is_gone(),
        "tmux pane process group or server leaked after failed readiness"
    );
}

#[test]
fn tmux_guard_without_process_authority_still_cleans_its_exact_server() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session = format!("asb-tui-no-authority-{}-{nonce}", std::process::id());
    let socket = format!("asb-tui-no-authority-socket-{}-{nonce}", std::process::id());
    assert!(bounded_tmux(
        &socket,
        &["new-session", "-d", "-s", &session, "sleep 30"],
    ));
    let observed_group =
        tmux_pane_process_group(&socket, &session).expect("cleanup probe must own a pane group");
    let guarded_socket = socket.clone();
    let acquisition = std::panic::catch_unwind(move || {
        let _guard = MultiplexerSession::Tmux {
            socket: guarded_socket.clone(),
            session,
            process_group: None,
        };
        tmux_pane_process_group(&guarded_socket, "missing-session")
            .expect("intentional process-group acquisition failure");
    });
    assert!(
        acquisition.is_err(),
        "invalid pane unexpectedly supplied process-group authority"
    );
    assert!(
        !bounded_tmux(&socket, &["has-session"]),
        "exact tmux server survived no-authority cleanup"
    );
    assert!(
        !MultiplexerSession::tmux_socket_path(&socket).exists(),
        "exact tmux socket survived no-authority cleanup"
    );
    assert!(
        MultiplexerSession::process_group_is_gone(observed_group),
        "no-authority cleanup left the ordinary pane group alive"
    );
}

#[test]
fn rendered_screen_wait_rejects_pre_alternate_malformed_and_oversized_content() {
    let mut captures = [
        None,
        Some((false, b"Connection: disconnected".to_vec(), false)),
        Some((true, b"Connection: disconnected".to_vec(), false)),
        Some((true, vec![0xff], true)),
        Some((true, b"Connection: disconnected".to_vec(), true)),
    ]
    .into_iter();
    assert_eq!(
        wait_for_tmux_rendered_screen(5, Duration::ZERO, || captures.next().flatten()),
        Ok(())
    );
    assert_eq!(
        wait_for_tmux_rendered_screen(1, Duration::ZERO, || {
            Some((true, b"Connection: disconnected".to_vec(), false))
        }),
        Err(PaneSummary::Recognized {
            bytes: 24,
            labels: "Connection".into()
        })
    );
    assert_eq!(
        wait_for_tmux_rendered_screen(2, Duration::ZERO, || None),
        Err(PaneSummary::Unavailable)
    );
    assert_eq!(
        wait_for_tmux_rendered_screen(1, Duration::ZERO, || { Some((true, vec![0xff], true)) }),
        Err(PaneSummary::NonUtf8 { bytes: 1 })
    );
    assert_eq!(
        wait_for_tmux_rendered_screen(1, Duration::ZERO, || {
            Some((true, vec![b'x'; MAX_MULTIPLEXER_OUTPUT + 1], true))
        }),
        Err(PaneSummary::Oversized)
    );
    let mut malformed = vec![0xff];
    malformed.extend_from_slice(b"Connection");
    assert_eq!(
        wait_for_tmux_rendered_screen(1, Duration::ZERO, || {
            Some((true, malformed.clone(), true))
        }),
        Err(PaneSummary::NonUtf8 {
            bytes: malformed.len()
        })
    );
}

#[test]
fn tmux_diagnostic_fields_reject_malformed_or_sensitive_values() {
    assert_eq!(
        tmux_args("socket", &["capture-pane", "-p"]),
        ["-f", "/dev/null", "-L", "socket", "capture-pane", "-p"]
    );
    assert_eq!(diagnostic_value(Some("1"), "boolean"), "1");
    assert_eq!(diagnostic_value(Some("2"), "boolean"), "malformed");
    assert_eq!(diagnostic_value(Some("126"), "status"), "126");
    assert_eq!(
        diagnostic_value(Some("/private/program"), "command"),
        "malformed"
    );
    assert_eq!(
        diagnostic_value(Some("program with args"), "command"),
        "malformed"
    );
    assert_eq!(diagnostic_value(None, "command"), "unavailable");
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
