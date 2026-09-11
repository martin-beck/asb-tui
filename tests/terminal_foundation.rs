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
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    mem::MaybeUninit,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::ffi::OsStrExt,
    os::unix::fs::{FileTypeExt, MetadataExt},
    process::{Command, Stdio},
    sync::{Arc, Mutex, MutexGuard, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::PrivateDirectory;

const CHILD_MODE: &str = "ASB_TUI_TERMINAL_FOUNDATION_CHILD";
const MAX_MULTIPLEXER_OUTPUT: usize = 4_096;
const RENDER_WAIT_ATTEMPTS: usize = 100;
const TMUX_STARTUP_ATTEMPTS: usize = 100;
static LIVE_TMUX_FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn serialize_live_tmux_fixture() -> MutexGuard<'static, ()> {
    lock_unpoisoned(&LIVE_TMUX_FIXTURE_LOCK)
}

fn lock_unpoisoned(lock: &Mutex<()>) -> MutexGuard<'_, ()> {
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

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
    let mut command = Command::new("/usr/bin/timeout");
    command
        .env_remove("TMUX_TMPDIR")
        .args(["--signal=KILL", "3s", program])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.status().is_ok_and(|status| status.success())
}

fn bounded_output(program: &str, args: &[&str]) -> Option<(bool, Vec<u8>)> {
    let (success, output) = bounded_output_observation(program, args)?;
    Some((success, output?))
}

fn bounded_output_observation(program: &str, args: &[&str]) -> Option<(bool, Option<Vec<u8>>)> {
    let mut command = Command::new("/usr/bin/timeout");
    command
        .env_remove("TMUX_TMPDIR")
        .args(["--signal=KILL", "3s", program])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut output = Vec::new();
    child
        .stdout
        .take()?
        .take((MAX_MULTIPLEXER_OUTPUT + 1) as u64)
        .read_to_end(&mut output)
        .ok()?;
    let status = child.wait().ok()?;
    let output = (output.len() <= MAX_MULTIPLEXER_OUTPUT).then_some(output);
    Some((status.success(), output))
}

fn tmux_args<'a>(socket: &'a str, args: &'a [&'a str]) -> Vec<&'a str> {
    assert!(std::path::Path::new(socket).is_absolute());
    let mut command = vec!["-f", "/dev/null", "-S", socket];
    command.extend_from_slice(args);
    command
}

fn bounded_tmux(socket: &str, args: &[&str]) -> bool {
    bounded_command("/usr/bin/tmux", &tmux_args(socket, args))
}

fn bounded_tmux_output(socket: &str, args: &[&str]) -> Option<(bool, Vec<u8>)> {
    bounded_output("/usr/bin/tmux", &tmux_args(socket, args))
}

const TMUX_CREATION_FAILED: &str = "tmux_creation_failed";
const TMUX_MALFORMED_WINDOW_IDENTITY: &str = "tmux_malformed_window_identity";
const TMUX_STARTUP_NOT_READY: &str = "tmux_startup_not_ready";
const TMUX_SERVER_OR_SESSION_DISAPPEARED: &str = "tmux_server_or_session_disappeared";
const TMUX_OPTION_UNSUPPORTED_OR_UNKNOWN: &str = "tmux_option_unsupported_or_unknown";

fn tmux_new_session_args(session: &str, args: &[&str]) -> Vec<String> {
    let mut command = vec![
        "new-session".to_owned(),
        "-P".to_owned(),
        "-F".to_owned(),
        "#{window_id}".to_owned(),
        "-d".to_owned(),
        "-s".to_owned(),
        session.to_owned(),
    ];
    command.extend(args.iter().map(|argument| (*argument).to_owned()));
    command
}

fn tmux_new_session_output(
    socket: &str,
    session: &str,
    args: &[&str],
) -> Option<(bool, Option<Vec<u8>>)> {
    let owned = tmux_new_session_args(session, args);
    let command = owned.iter().map(String::as_str).collect::<Vec<_>>();
    bounded_output_observation("/usr/bin/tmux", &tmux_args(socket, &command))
}

fn parse_tmux_window_identity(output: &[u8]) -> Option<String> {
    parse_tmux_numeric_identity(output, '@')
}

fn parse_tmux_numeric_identity(output: &[u8], prefix: char) -> Option<String> {
    if output.len() > MAX_MULTIPLEXER_OUTPUT {
        return None;
    }
    let value = std::str::from_utf8(output).ok()?.strip_suffix('\n')?;
    if value.contains('\n') {
        return None;
    }
    let digits = value.strip_prefix(prefix)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let identity = digits.parse::<u32>().ok()?;
    (identity.to_string() == digits).then(|| format!("{prefix}{identity}"))
}

fn start_guarded_tmux_session(
    socket: &str,
    session: &str,
    args: &[&str],
) -> Result<MultiplexerSession, String> {
    let (created, output) =
        tmux_new_session_output(socket, session, args).ok_or(TMUX_CREATION_FAILED)?;
    if !created {
        return Err(TMUX_CREATION_FAILED.to_owned());
    }
    finish_guarded_tmux_session(socket, session, output)
}

fn finish_guarded_tmux_session(
    socket: &str,
    session: &str,
    output: Option<Vec<u8>>,
) -> Result<MultiplexerSession, String> {
    let mut guard = MultiplexerSession::Tmux {
        socket: socket.to_owned(),
        session: session.to_owned(),
        server: None,
        pane_process: None,
        startup: None,
    };
    let server =
        wait_for_server_observation(TMUX_STARTUP_ATTEMPTS, Duration::from_millis(10), || {
            tmux_server_observation_diagnostic(socket, session)
        })
        .map_err(|stage| {
            format!(
                "{TMUX_STARTUP_NOT_READY}:server_observation={}",
                stage.label()
            )
        })?;
    guard.set_server(server.clone());
    let startup = wait_for_stable_startup_observation(
        TMUX_STARTUP_ATTEMPTS,
        Duration::from_millis(10),
        || tmux_startup_observation_diagnostic(socket, session),
        |observed| observed.server == server,
        |observed| guard.set_pane_process(observed.pane_process.clone()),
    )
    .map_err(|stage| {
        format!(
            "{TMUX_STARTUP_NOT_READY}:startup_observation={}",
            stage.label()
        )
    })?;
    let window = output
        .as_deref()
        .and_then(parse_tmux_window_identity)
        .ok_or_else(|| TMUX_MALFORMED_WINDOW_IDENTITY.to_owned())?;
    if window != startup.window {
        return Err(format!(
            "{TMUX_STARTUP_NOT_READY}:startup_observation={}",
            TmuxStartupObservationStage::CreationWindowChanged.label()
        ));
    }
    guard.set_startup(startup);
    Ok(guard)
}

fn wait_for_server_observation(
    attempts: usize,
    delay: Duration,
    mut observe: impl FnMut() -> Result<TmuxServerObservation, TmuxServerObservationStage>,
) -> Result<TmuxServerObservation, TmuxServerObservationStage> {
    let mut last_stage = TmuxServerObservationStage::Unavailable;
    for attempt in 0..attempts {
        match observe() {
            Ok(observation) => return Ok(observation),
            Err(stage) => last_stage = stage,
        }
        if attempt + 1 < attempts {
            thread::sleep(delay);
        }
    }
    Err(last_stage)
}

fn wait_for_available_observation<T>(
    attempts: usize,
    delay: Duration,
    mut observe: impl FnMut() -> Option<T>,
) -> Option<T> {
    for attempt in 0..attempts {
        if let Some(current) = observe() {
            return Some(current);
        }
        if attempt + 1 < attempts {
            thread::sleep(delay);
        }
    }
    None
}

fn wait_for_stable_startup_observation<T: Clone + Eq>(
    attempts: usize,
    delay: Duration,
    mut observe: impl FnMut() -> Result<T, TmuxStartupObservationStage>,
    matches_retained: impl Fn(&T) -> bool,
    mut retain: impl FnMut(&T),
) -> Result<T, TmuxStartupObservationStage> {
    let mut retained = None;
    let mut previous = None;
    let mut last_stage = TmuxStartupObservationStage::Unavailable;
    for attempt in 0..attempts {
        match observe() {
            Ok(observed) if !matches_retained(&observed) => {
                return Err(TmuxStartupObservationStage::RetainedServerChanged);
            }
            Ok(observed) if retained.as_ref().is_some_and(|value| value != &observed) => {
                return Err(TmuxStartupObservationStage::StartupIdentityChanged);
            }
            Ok(observed) if previous.as_ref() == Some(&observed) => return Ok(observed),
            Ok(observed) => {
                if retained.is_none() {
                    retain(&observed);
                    retained = Some(observed.clone());
                }
                previous = Some(observed);
            }
            Err(stage) => {
                previous = None;
                last_stage = stage;
            }
        }
        if attempt + 1 < attempts {
            thread::sleep(delay);
        }
    }
    Err(last_stage)
}

fn wait_for_stable_matching_observation<T: Clone + Eq>(
    attempts: usize,
    delay: Duration,
    mut observe: impl FnMut() -> Option<T>,
    mut matches_retained_authority: impl FnMut(&T) -> bool,
    mut retain: impl FnMut(&T),
) -> Option<T> {
    let mut retained = None;
    let mut previous = None;
    for attempt in 0..attempts {
        match observe() {
            Some(current) if !matches_retained_authority(&current) => return None,
            Some(current) if retained.as_ref().is_some_and(|value| value != &current) => {
                return None;
            }
            Some(current) if previous.as_ref() == Some(&current) => return Some(current),
            Some(current) => {
                if retained.is_none() {
                    retain(&current);
                    retained = Some(current.clone());
                }
                previous = Some(current);
            }
            None => previous = None,
        }
        if attempt + 1 < attempts {
            thread::sleep(delay);
        }
    }
    None
}

fn tmux_remain_on_exit_args(window: &str) -> Vec<String> {
    vec![
        "set-window-option".to_owned(),
        "-t".to_owned(),
        window.to_owned(),
        "remain-on-exit".to_owned(),
        "on".to_owned(),
    ]
}

fn set_tmux_remain_on_exit_with(
    session: &str,
    window: &str,
    mut run: impl FnMut(&[&str]) -> bool,
) -> Result<(), &'static str> {
    let session_check = ["has-session", "-t", session];
    if !run(&session_check) {
        return Err(TMUX_SERVER_OR_SESSION_DISAPPEARED);
    }
    let owned = tmux_remain_on_exit_args(window);
    let args = owned.iter().map(String::as_str).collect::<Vec<_>>();
    if run(&args) {
        return Ok(());
    }
    if !run(&session_check) {
        return Err(TMUX_SERVER_OR_SESSION_DISAPPEARED);
    }
    Err(TMUX_OPTION_UNSUPPORTED_OR_UNKNOWN)
}

fn set_tmux_remain_on_exit_authenticated_with(
    expected: &TmuxStartupObservation,
    mut observe: impl FnMut() -> Option<TmuxStartupObservation>,
    mut run: impl FnMut(&[&str]) -> bool,
) -> Result<(), &'static str> {
    if observe().as_ref() != Some(expected) {
        return Err(TMUX_SERVER_OR_SESSION_DISAPPEARED);
    }
    let owned = tmux_remain_on_exit_args(&expected.window);
    let args = owned.iter().map(String::as_str).collect::<Vec<_>>();
    if run(&args) {
        return Ok(());
    }
    if observe().as_ref() != Some(expected) {
        return Err(TMUX_SERVER_OR_SESSION_DISAPPEARED);
    }
    Err(TMUX_OPTION_UNSUPPORTED_OR_UNKNOWN)
}

fn set_tmux_remain_on_exit_for_guard_with(
    guard: &MultiplexerSession,
    mut observe: impl FnMut() -> Option<TmuxStartupObservation>,
    mut run: impl FnMut(&[&str]) -> bool,
) -> Result<(), &'static str> {
    let expected = guard
        .tmux_startup()
        .ok_or(TMUX_SERVER_OR_SESSION_DISAPPEARED)?;
    set_tmux_remain_on_exit_authenticated_with(expected, &mut observe, &mut run)
}

fn set_tmux_remain_on_exit(
    socket: &str,
    session: &str,
    guard: &MultiplexerSession,
) -> Result<(), &'static str> {
    set_tmux_remain_on_exit_for_guard_with(
        guard,
        || tmux_startup_observation(socket, session),
        |args| bounded_tmux(socket, args),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessGeneration {
    pid: u32,
    start_time: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TmuxServerObservation {
    process: ProcessGeneration,
    socket_device: u64,
    socket_inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TmuxServerObservationStage {
    Unavailable,
    SocketMetadataUnavailable,
    SocketMetadataInvalid,
    SocketPathInvalid,
    SocketCreationUnavailable,
    SocketConnectRejected,
    SocketPollUnavailable,
    SocketPollTimeout,
    SocketPollInvalid,
    SocketPollNoCompletion,
    SocketErrorUnavailable,
    SocketErrorMalformed,
    SocketErrorNonzero,
    PeerCredentialsUnavailable,
    PeerCredentialsInvalid,
    ProcessGenerationUnavailable,
    RepeatedPeerIdentityChanged,
    RepeatedSocketIdentityChanged,
    ProcessGenerationChanged,
}

impl TmuxServerObservationStage {
    const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::SocketMetadataUnavailable => "socket_metadata_unavailable",
            Self::SocketMetadataInvalid => "socket_metadata_invalid",
            Self::SocketPathInvalid => "socket_path_invalid",
            Self::SocketCreationUnavailable => "socket_creation_unavailable",
            Self::SocketConnectRejected => "socket_connect_rejected",
            Self::SocketPollUnavailable => "socket_poll_unavailable",
            Self::SocketPollTimeout => "socket_poll_timeout",
            Self::SocketPollInvalid => "socket_poll_invalid",
            Self::SocketPollNoCompletion => "socket_poll_no_completion",
            Self::SocketErrorUnavailable => "socket_error_unavailable",
            Self::SocketErrorMalformed => "socket_error_malformed",
            Self::SocketErrorNonzero => "socket_error_nonzero",
            Self::PeerCredentialsUnavailable => "peer_credentials_unavailable",
            Self::PeerCredentialsInvalid => "peer_credentials_invalid",
            Self::ProcessGenerationUnavailable => "process_generation_unavailable",
            Self::RepeatedPeerIdentityChanged => "repeated_peer_identity_changed",
            Self::RepeatedSocketIdentityChanged => "repeated_socket_identity_changed",
            Self::ProcessGenerationChanged => "process_generation_changed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TmuxStartupObservationStage {
    Unavailable,
    ServerBeforeUnavailable,
    SessionBeforeUnavailable,
    WindowBeforeUnavailable,
    PaneIdentityUnavailable,
    PaneIdentityMalformed,
    TtyIdentityUnavailable,
    PaneGenerationUnavailable,
    ProcessListBeforeUnavailable,
    ProcessTupleInvalid,
    ForegroundGenerationUnavailable,
    ProcessListAfterUnavailable,
    ProcessTupleAfterInvalid,
    ProcessTupleChanged,
    PaneIdentityChanged,
    TtyIdentityChanged,
    PaneGenerationChanged,
    ForegroundGenerationChanged,
    SessionAfterUnavailable,
    WindowAfterUnavailable,
    ServerAfterUnavailable,
    ServerChanged,
    SessionChanged,
    WindowChanged,
    StartupIdentityChanged,
    RetainedServerChanged,
    CreationWindowChanged,
}

impl TmuxStartupObservationStage {
    const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::ServerBeforeUnavailable => "server_before_unavailable",
            Self::SessionBeforeUnavailable => "session_before_unavailable",
            Self::WindowBeforeUnavailable => "window_before_unavailable",
            Self::PaneIdentityUnavailable => "pane_identity_unavailable",
            Self::PaneIdentityMalformed => "pane_identity_malformed",
            Self::TtyIdentityUnavailable => "tty_identity_unavailable",
            Self::PaneGenerationUnavailable => "pane_generation_unavailable",
            Self::ProcessListBeforeUnavailable => "process_list_before_unavailable",
            Self::ProcessTupleInvalid => "process_tuple_invalid",
            Self::ForegroundGenerationUnavailable => "foreground_generation_unavailable",
            Self::ProcessListAfterUnavailable => "process_list_after_unavailable",
            Self::ProcessTupleAfterInvalid => "process_tuple_after_invalid",
            Self::ProcessTupleChanged => "process_tuple_changed",
            Self::PaneIdentityChanged => "pane_identity_changed",
            Self::TtyIdentityChanged => "tty_identity_changed",
            Self::PaneGenerationChanged => "pane_generation_changed",
            Self::ForegroundGenerationChanged => "foreground_generation_changed",
            Self::SessionAfterUnavailable => "session_after_unavailable",
            Self::WindowAfterUnavailable => "window_after_unavailable",
            Self::ServerAfterUnavailable => "server_after_unavailable",
            Self::ServerChanged => "server_changed",
            Self::SessionChanged => "session_changed",
            Self::WindowChanged => "window_changed",
            Self::StartupIdentityChanged => "startup_identity_changed",
            Self::RetainedServerChanged => "retained_server_changed",
            Self::CreationWindowChanged => "creation_window_changed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TmuxStartupObservation {
    server: TmuxServerObservation,
    pane_process: PaneProcessObservation,
    session_identity: String,
    window: String,
}

enum MultiplexerSession {
    Tmux {
        socket: String,
        session: String,
        server: Option<TmuxServerObservation>,
        pane_process: Option<PaneProcessObservation>,
        startup: Option<Box<TmuxStartupObservation>>,
    },
    Screen {
        session: String,
    },
}

impl MultiplexerSession {
    fn set_server(&mut self, validated: TmuxServerObservation) {
        match self {
            Self::Tmux { server, .. } => {
                assert!(server.is_none());
                *server = Some(validated);
            }
            Self::Screen { .. } => panic!("screen sessions have no tmux server"),
        }
    }

    fn set_pane_process(&mut self, validated: PaneProcessObservation) {
        match self {
            Self::Tmux { pane_process, .. } => {
                assert!(pane_process.is_none());
                *pane_process = Some(validated);
            }
            Self::Screen { .. } => panic!("screen sessions have no tmux pane process group"),
        }
    }

    fn set_startup(&mut self, validated: TmuxStartupObservation) {
        match self {
            Self::Tmux {
                server,
                pane_process,
                startup,
                ..
            } => {
                assert_eq!(server.as_ref(), Some(&validated.server));
                assert_eq!(pane_process.as_ref(), Some(&validated.pane_process));
                assert!(startup.is_none());
                *startup = Some(Box::new(validated));
            }
            Self::Screen { .. } => panic!("screen sessions have no tmux startup authority"),
        }
    }

    fn tmux_startup(&self) -> Option<&TmuxStartupObservation> {
        match self {
            Self::Tmux { startup, .. } => startup.as_deref(),
            Self::Screen { .. } => None,
        }
    }

    fn tmux_socket_path(socket: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(socket)
    }

    fn process_group_is_gone(process_group: u32) -> bool {
        let group = format!("-{process_group}");
        !bounded_command("/bin/kill", &["-0", "--", &group])
    }

    fn path_is_absent(path: &std::path::Path) -> bool {
        matches!(
            fs::symlink_metadata(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        )
    }

    fn stop_process_group_with(
        observed: &PaneProcessObservation,
        mut observe: impl FnMut() -> Option<PaneProcessObservation>,
        mut signal: impl FnMut(&str, u32),
        mut is_gone: impl FnMut(u32) -> bool,
        mut wait: impl FnMut(),
    ) -> bool {
        let process_group = observed.foreground_group;
        if is_gone(process_group) {
            return true;
        }
        if observe().as_ref() != Some(observed) {
            return false;
        }
        signal("-TERM", process_group);
        for _ in 0..100 {
            if is_gone(process_group) {
                return true;
            }
            wait();
        }
        if observe().as_ref() != Some(observed) {
            return false;
        }
        signal("-KILL", process_group);
        for _ in 0..100 {
            if is_gone(process_group) {
                return true;
            }
            wait();
        }
        false
    }

    fn stop_process_group(socket: &str, session: &str, observed: &PaneProcessObservation) -> bool {
        Self::stop_process_group_with(
            observed,
            || tmux_pane_process_observation(socket, session),
            |signal, process_group| {
                let group = format!("-{process_group}");
                let _ = bounded_command("/bin/kill", &[signal, "--", &group]);
            },
            Self::process_group_is_gone,
            || thread::sleep(Duration::from_millis(10)),
        )
    }

    fn stop_tmux_server_with(
        mut authenticate: impl FnMut() -> bool,
        mut kill_server: impl FnMut() -> bool,
        mut server_gone: impl FnMut() -> bool,
        mut wait: impl FnMut(),
    ) -> bool {
        if server_gone() {
            return true;
        }
        if !authenticate() {
            return false;
        }
        if !kill_server() {
            return false;
        }
        for _ in 0..100 {
            if server_gone() {
                return true;
            }
            wait();
        }
        false
    }

    fn stop(&self) -> bool {
        match self {
            Self::Tmux {
                socket,
                session,
                server,
                pane_process,
                ..
            } => {
                let group_stopped = pane_process
                    .as_ref()
                    .is_some_and(|observed| Self::stop_process_group(socket, session, observed));
                let server_clean = server.as_ref().is_some_and(|expected| {
                    Self::stop_tmux_server_with(
                        || tmux_server_matches(socket, session, expected),
                        || bounded_tmux(socket, &["kill-server"]),
                        || !process_generation_matches(&expected.process),
                        || thread::sleep(Duration::from_millis(10)),
                    )
                });
                let group_clean = group_stopped
                    || pane_process.as_ref().is_some_and(|observed| {
                        Self::process_group_is_gone(observed.foreground_group)
                    });
                group_clean && server_clean
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
                server,
                pane_process,
                ..
            } => {
                !bounded_tmux(socket, &["has-session", "-t", session])
                    && server
                        .as_ref()
                        .is_some_and(|expected| tmux_socket_is_stale(socket, expected))
                    && pane_process.as_ref().is_some_and(|observed| {
                        Self::process_group_is_gone(observed.foreground_group)
                    })
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneProcessObservation {
    pane_pid: u32,
    pane_start_time: u64,
    pane_tty: String,
    tty_device: u64,
    tty_inode: u64,
    tty_rdevice: u64,
    foreground_group: u32,
    foreground_start_time: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneProcessTuple {
    pane_pid: u32,
    pane_tty: String,
    foreground_group: u32,
}

fn parse_pane_process_observation(
    pane: &[u8],
    processes: &[u8],
    current_group: u32,
    current_uid: u32,
) -> Option<PaneProcessTuple> {
    let pane = std::str::from_utf8(pane).ok()?.strip_suffix('\n')?;
    let (pane_pid, pane_tty) = pane.split_once('\t')?;
    let pane_pid = pane_pid.parse::<u32>().ok().filter(|pid| *pid > 1)?;
    let tty_number = pane_tty.strip_prefix("/dev/pts/")?;
    if tty_number.is_empty() || !tty_number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let expected_tty = pane_tty.strip_prefix("/dev/")?;
    #[derive(Clone, Copy)]
    struct ProcessRow {
        pid: u32,
        pgid: u32,
        tpgid: u32,
        uid: u32,
        euid: u32,
    }
    let mut rows = Vec::new();
    for line in std::str::from_utf8(processes).ok()?.lines() {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 6 {
            return None;
        }
        let pid = fields[0].parse::<u32>().ok().filter(|pid| *pid > 1)?;
        let pgid = fields[1].parse::<u32>().ok().filter(|pgid| *pgid > 1)?;
        let tpgid = fields[2].parse::<u32>().ok().filter(|tpgid| *tpgid > 1)?;
        if fields[3] != expected_tty {
            return None;
        }
        let uid = fields[4].parse::<u32>().ok()?;
        let euid = fields[5].parse::<u32>().ok()?;
        rows.push(ProcessRow {
            pid,
            pgid,
            tpgid,
            uid,
            euid,
        });
    }
    if rows.iter().enumerate().any(|(index, row)| {
        rows.iter()
            .skip(index + 1)
            .any(|candidate| candidate.pid == row.pid)
    }) {
        return None;
    }
    let mut pane_rows = rows.iter().filter(|row| row.pid == pane_pid);
    let pane_row = *pane_rows.next()?;
    if pane_rows.next().is_some() || pane_row.uid != current_uid || pane_row.euid != current_uid {
        return None;
    }
    let foreground_group = pane_row.tpgid;
    let foreground_leaders = rows
        .iter()
        .filter(|row| row.pid == foreground_group && row.pgid == foreground_group)
        .collect::<Vec<_>>();
    if foreground_group == current_group
        || rows.iter().any(|row| row.tpgid != foreground_group)
        || foreground_leaders.len() != 1
        || foreground_leaders[0].uid != current_uid
        || foreground_leaders[0].euid != current_uid
        || rows.iter().any(|row| {
            row.pgid == foreground_group && (row.uid != current_uid || row.euid != current_uid)
        })
    {
        return None;
    }
    Some(PaneProcessTuple {
        pane_pid,
        pane_tty: pane_tty.into(),
        foreground_group,
    })
}

fn process_start_time(process: u32) -> Option<u64> {
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{process}/stat"))
        .ok()?
        .take((MAX_MULTIPLEXER_OUTPUT + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_MULTIPLEXER_OUTPUT {
        return None;
    }
    let stat = std::str::from_utf8(&bytes).ok()?;
    let fields = stat.get(stat.rfind(") ")?.checked_add(2)?..)?;
    let mut fields = fields.split_ascii_whitespace();
    if fields.next()? == "Z" {
        return None;
    }
    fields
        .nth(18)?
        .parse::<u64>()
        .ok()
        .filter(|start| *start > 0)
}

fn process_generation_matches(process: &ProcessGeneration) -> bool {
    process_start_time(process.pid) == Some(process.start_time)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TtyIdentity {
    device: u64,
    inode: u64,
    rdevice: u64,
}

fn tty_identity(path: &str) -> Option<TtyIdentity> {
    tty_identity_for_uid(path, rustix::process::getuid().as_raw())
}

fn tty_identity_for_uid(path: &str, current_uid: u32) -> Option<TtyIdentity> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_char_device() || metadata.uid() != current_uid {
        return None;
    }
    Some(TtyIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        rdevice: metadata.rdev(),
    })
}

fn tmux_pane_identity(socket: &str, session: &str) -> Option<Vec<u8>> {
    bounded_tmux_output(
        socket,
        &[
            "display-message",
            "-p",
            "-t",
            session,
            "#{pane_pid}\t#{pane_tty}",
        ],
    )
    .filter(|(success, _)| *success)
    .map(|(_, output)| output)
}

fn tmux_server_observation(socket: &str, session: &str) -> Option<TmuxServerObservation> {
    tmux_server_observation_diagnostic(socket, session).ok()
}

fn tmux_server_observation_diagnostic(
    socket: &str,
    _session: &str,
) -> Result<TmuxServerObservation, TmuxServerObservationStage> {
    let path = MultiplexerSession::tmux_socket_path(socket);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| TmuxServerObservationStage::SocketMetadataUnavailable)?;
    if !metadata.file_type().is_socket() || metadata.uid() != rustix::process::getuid().as_raw() {
        return Err(TmuxServerObservationStage::SocketMetadataInvalid);
    }
    let process = tmux_socket_peer_process(&path)?;
    let observation = TmuxServerObservation {
        process,
        socket_device: metadata.dev(),
        socket_inode: metadata.ino(),
    };
    let after = fs::symlink_metadata(path)
        .map_err(|_| TmuxServerObservationStage::RepeatedSocketIdentityChanged)?;
    if tmux_socket_peer_process(&MultiplexerSession::tmux_socket_path(socket))?
        != observation.process
    {
        return Err(TmuxServerObservationStage::RepeatedPeerIdentityChanged);
    }
    if !after.file_type().is_socket()
        || after.uid() != rustix::process::getuid().as_raw()
        || after.dev() != observation.socket_device
        || after.ino() != observation.socket_inode
    {
        return Err(TmuxServerObservationStage::RepeatedSocketIdentityChanged);
    }
    if !process_generation_matches(&observation.process) {
        return Err(TmuxServerObservationStage::ProcessGenerationChanged);
    }
    Ok(observation)
}

fn tmux_socket_peer_process(
    path: &std::path::Path,
) -> Result<ProcessGeneration, TmuxServerObservationStage> {
    let path_bytes = path.as_os_str().as_bytes();
    let mut address = unsafe {
        // SAFETY: Zero is a valid initialization for sockaddr_un before its family and path are set.
        MaybeUninit::<libc::sockaddr_un>::zeroed().assume_init()
    };
    if path_bytes.is_empty()
        || path_bytes.contains(&0)
        || path_bytes.len() >= address.sun_path.len()
    {
        return Err(TmuxServerObservationStage::SocketPathInvalid);
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (destination, source) in address.sun_path.iter_mut().zip(path_bytes) {
        *destination = *source as libc::c_char;
    }
    let address_len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path_bytes.len() + 1)
        .try_into()
        .map_err(|_| TmuxServerObservationStage::SocketPathInvalid)?;
    let raw_fd = unsafe {
        // SAFETY: The constants describe a Linux Unix stream socket and no borrowed pointer is used.
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw_fd < 0 {
        return Err(TmuxServerObservationStage::SocketCreationUnavailable);
    }
    let socket = unsafe {
        // SAFETY: raw_fd was just returned uniquely by socket and is transferred exactly once.
        OwnedFd::from_raw_fd(raw_fd)
    };
    let connected = unsafe {
        // SAFETY: address is initialized above and address_len covers its family and pathname.
        libc::connect(
            socket.as_raw_fd(),
            (&raw const address).cast::<libc::sockaddr>(),
            address_len,
        )
    };
    if connected != 0 {
        let connect_error = io::Error::last_os_error().raw_os_error();
        classify_unix_connect_attempt(connected, connect_error)?;
        let mut descriptor = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        let ready = unsafe {
            // SAFETY: descriptor points to one initialized pollfd for the duration of this call.
            libc::poll(&raw mut descriptor, 1, 100)
        };
        let mut socket_error = 0;
        let mut socket_error_len = std::mem::size_of_val(&socket_error) as libc::socklen_t;
        let status = unsafe {
            // SAFETY: socket_error and its exact initialized length are valid getsockopt outputs.
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&raw mut socket_error).cast(),
                &raw mut socket_error_len,
            )
        };
        classify_unix_connect_completion(
            ready,
            descriptor.revents,
            status,
            socket_error_len,
            socket_error,
        )?;
    }
    let mut credentials = MaybeUninit::<libc::ucred>::uninit();
    let mut credentials_len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let status = unsafe {
        // SAFETY: credentials has the exact ucred size advertised to getsockopt and is read only on success.
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &raw mut credentials_len,
        )
    };
    if status != 0 || credentials_len as usize != std::mem::size_of::<libc::ucred>() {
        return Err(TmuxServerObservationStage::PeerCredentialsUnavailable);
    }
    let credentials = unsafe {
        // SAFETY: A successful getsockopt above initialized all bytes of the exact ucred output.
        credentials.assume_init()
    };
    let pid = u32::try_from(credentials.pid)
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or(TmuxServerObservationStage::PeerCredentialsInvalid)?;
    if credentials.uid != rustix::process::getuid().as_raw() {
        return Err(TmuxServerObservationStage::PeerCredentialsInvalid);
    }
    let start_time =
        process_start_time(pid).ok_or(TmuxServerObservationStage::ProcessGenerationUnavailable)?;
    Ok(ProcessGeneration { pid, start_time })
}

fn classify_unix_connect_attempt(
    connected: i32,
    error: Option<i32>,
) -> Result<bool, TmuxServerObservationStage> {
    if connected == 0 {
        return Ok(false);
    }
    matches!(error, Some(code) if code == libc::EINPROGRESS || code == libc::EAGAIN)
        .then_some(true)
        .ok_or(TmuxServerObservationStage::SocketConnectRejected)
}

fn classify_unix_connect_completion(
    ready: i32,
    revents: libc::c_short,
    getsockopt_status: i32,
    socket_error_len: libc::socklen_t,
    socket_error: i32,
) -> Result<(), TmuxServerObservationStage> {
    let completion_events = libc::POLLOUT | libc::POLLERR | libc::POLLHUP;
    if ready < 0 {
        return Err(TmuxServerObservationStage::SocketPollUnavailable);
    }
    if ready == 0 {
        return Err(TmuxServerObservationStage::SocketPollTimeout);
    }
    if ready != 1 || revents & libc::POLLNVAL != 0 {
        return Err(TmuxServerObservationStage::SocketPollInvalid);
    }
    if revents & completion_events == 0 {
        return Err(TmuxServerObservationStage::SocketPollNoCompletion);
    }
    if getsockopt_status != 0 {
        return Err(TmuxServerObservationStage::SocketErrorUnavailable);
    }
    if socket_error_len as usize != std::mem::size_of::<libc::c_int>() {
        return Err(TmuxServerObservationStage::SocketErrorMalformed);
    }
    if socket_error != 0 {
        return Err(TmuxServerObservationStage::SocketErrorNonzero);
    }
    Ok(())
}

fn tmux_server_matches(socket: &str, session: &str, expected: &TmuxServerObservation) -> bool {
    let path = MultiplexerSession::tmux_socket_path(socket);
    let socket_matches = || {
        fs::symlink_metadata(&path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.uid() == rustix::process::getuid().as_raw()
                && metadata.dev() == expected.socket_device
                && metadata.ino() == expected.socket_inode
        })
    };
    if !socket_matches() {
        return false;
    }
    let current = tmux_server_observation(socket, session);
    current.as_ref() == Some(expected)
        && socket_matches()
        && process_generation_matches(&expected.process)
}

fn tmux_current_window_identity(socket: &str, session: &str) -> Option<String> {
    bounded_tmux_output(
        socket,
        &["display-message", "-p", "-t", session, "#{window_id}"],
    )
    .filter(|(success, _)| *success)
    .and_then(|(_, output)| parse_tmux_window_identity(&output))
}

fn tmux_current_session_identity(socket: &str, session: &str) -> Option<String> {
    bounded_tmux_output(
        socket,
        &["display-message", "-p", "-t", session, "#{session_id}"],
    )
    .filter(|(success, _)| *success)
    .and_then(|(_, output)| parse_tmux_numeric_identity(&output, '$'))
}

// tmux 3.4 intentionally retains a stale socket inode after graceful server exit.
// The session guard never unlinks this pathname: it only proves the retained
// inode is the authenticated server's now-unconnectable socket. The enclosing
// disposable scratch root owns eventual whole-root cleanup.
fn tmux_socket_is_stale(socket: &str, expected: &TmuxServerObservation) -> bool {
    if process_generation_matches(&expected.process) {
        return false;
    }
    let path = MultiplexerSession::tmux_socket_path(socket);
    if MultiplexerSession::path_is_absent(&path) {
        return true;
    }
    fs::symlink_metadata(&path).is_ok_and(|metadata| {
        metadata.file_type().is_socket()
            && metadata.uid() == rustix::process::getuid().as_raw()
            && metadata.dev() == expected.socket_device
            && metadata.ino() == expected.socket_inode
            && std::os::unix::net::UnixStream::connect(&path).is_err()
    })
}

fn tmux_pane_process_observation(socket: &str, session: &str) -> Option<PaneProcessObservation> {
    tmux_pane_process_observation_diagnostic(socket, session).ok()
}

fn tmux_pane_process_observation_diagnostic(
    socket: &str,
    session: &str,
) -> Result<PaneProcessObservation, TmuxStartupObservationStage> {
    let pane = tmux_pane_identity(socket, session)
        .ok_or(TmuxStartupObservationStage::PaneIdentityUnavailable)?;
    let pane_text = std::str::from_utf8(&pane)
        .ok()
        .and_then(|value| value.strip_suffix('\n'))
        .ok_or(TmuxStartupObservationStage::PaneIdentityMalformed)?;
    let (pane_pid, pane_tty) = pane_text
        .split_once('\t')
        .ok_or(TmuxStartupObservationStage::PaneIdentityMalformed)?;
    let pane_pid = pane_pid
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or(TmuxStartupObservationStage::PaneIdentityMalformed)?;
    let tty_number = pane_tty
        .strip_prefix("/dev/pts/")
        .ok_or(TmuxStartupObservationStage::PaneIdentityMalformed)?;
    if tty_number.is_empty() || !tty_number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TmuxStartupObservationStage::PaneIdentityMalformed);
    }
    let tty_before =
        tty_identity(pane_tty).ok_or(TmuxStartupObservationStage::TtyIdentityUnavailable)?;
    let pane_start_before = process_start_time(pane_pid)
        .ok_or(TmuxStartupObservationStage::PaneGenerationUnavailable)?;
    let ps_args = [
        "--no-headers",
        "-o",
        "pid=,pgid=,tpgid=,tty=,uid=,euid=",
        "-t",
        pane_tty
            .strip_prefix("/dev/")
            .ok_or(TmuxStartupObservationStage::PaneIdentityMalformed)?,
    ];
    let processes_before = bounded_output("/usr/bin/ps", &ps_args)
        .filter(|(success, _)| *success)
        .map(|(_, output)| output)
        .ok_or(TmuxStartupObservationStage::ProcessListBeforeUnavailable)?;
    let current_group = u32::try_from(rustix::process::getpgrp().as_raw_pid())
        .map_err(|_| TmuxStartupObservationStage::ProcessTupleInvalid)?;
    let current_uid = rustix::process::getuid().as_raw();
    let tuple =
        parse_pane_process_observation(&pane, &processes_before, current_group, current_uid)
            .ok_or(TmuxStartupObservationStage::ProcessTupleInvalid)?;
    let foreground_start_before = process_start_time(tuple.foreground_group)
        .ok_or(TmuxStartupObservationStage::ForegroundGenerationUnavailable)?;
    let processes_after = bounded_output("/usr/bin/ps", &ps_args)
        .filter(|(success, _)| *success)
        .map(|(_, output)| output)
        .ok_or(TmuxStartupObservationStage::ProcessListAfterUnavailable)?;
    let tuple_after =
        parse_pane_process_observation(&pane, &processes_after, current_group, current_uid)
            .ok_or(TmuxStartupObservationStage::ProcessTupleAfterInvalid)?;
    if tuple_after != tuple {
        return Err(TmuxStartupObservationStage::ProcessTupleChanged);
    }
    if tmux_pane_identity(socket, session).as_ref() != Some(&pane) {
        return Err(TmuxStartupObservationStage::PaneIdentityChanged);
    }
    if tty_identity(pane_tty) != Some(tty_before) {
        return Err(TmuxStartupObservationStage::TtyIdentityChanged);
    }
    if process_start_time(pane_pid) != Some(pane_start_before) {
        return Err(TmuxStartupObservationStage::PaneGenerationChanged);
    }
    if process_start_time(tuple.foreground_group) != Some(foreground_start_before) {
        return Err(TmuxStartupObservationStage::ForegroundGenerationChanged);
    }
    Ok(PaneProcessObservation {
        pane_pid: tuple.pane_pid,
        pane_start_time: pane_start_before,
        pane_tty: tuple.pane_tty,
        tty_device: tty_before.device,
        tty_inode: tty_before.inode,
        tty_rdevice: tty_before.rdevice,
        foreground_group: tuple.foreground_group,
        foreground_start_time: foreground_start_before,
    })
}

fn tmux_startup_observation(socket: &str, session: &str) -> Option<TmuxStartupObservation> {
    tmux_startup_observation_diagnostic(socket, session).ok()
}

fn tmux_startup_observation_diagnostic(
    socket: &str,
    session: &str,
) -> Result<TmuxStartupObservation, TmuxStartupObservationStage> {
    let server_before = tmux_server_observation(socket, session)
        .ok_or(TmuxStartupObservationStage::ServerBeforeUnavailable)?;
    let session_before = tmux_current_session_identity(socket, session)
        .ok_or(TmuxStartupObservationStage::SessionBeforeUnavailable)?;
    let window_before = tmux_current_window_identity(socket, session)
        .ok_or(TmuxStartupObservationStage::WindowBeforeUnavailable)?;
    let pane_process = tmux_pane_process_observation_diagnostic(socket, session)?;
    let session_after = tmux_current_session_identity(socket, session)
        .ok_or(TmuxStartupObservationStage::SessionAfterUnavailable)?;
    let window_after = tmux_current_window_identity(socket, session)
        .ok_or(TmuxStartupObservationStage::WindowAfterUnavailable)?;
    let server_after = tmux_server_observation(socket, session)
        .ok_or(TmuxStartupObservationStage::ServerAfterUnavailable)?;
    if server_before != server_after {
        return Err(TmuxStartupObservationStage::ServerChanged);
    }
    if session_before != session_after {
        return Err(TmuxStartupObservationStage::SessionChanged);
    }
    if window_before != window_after {
        return Err(TmuxStartupObservationStage::WindowChanged);
    }
    Ok(TmuxStartupObservation {
        server: server_before,
        pane_process,
        session_identity: session_before,
        window: window_before,
    })
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
fn live_tmux_fixture_lock_is_exclusive() {
    let lock = Arc::new(Mutex::new(()));
    let first = lock_unpoisoned(&lock);
    let (attempting_tx, attempting_rx) = mpsc::sync_channel(0);
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let contender_lock = Arc::clone(&lock);
    let contender = thread::spawn(move || {
        attempting_tx.send(()).unwrap();
        let _second = lock_unpoisoned(&contender_lock);
        entered_tx.send(()).unwrap();
    });

    attempting_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("contending fixture did not reach the acquisition boundary");
    assert!(
        entered_rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "a second live tmux fixture entered before the first released exclusion"
    );
    drop(first);
    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("contending fixture did not enter after exclusion was released");
    contender.join().unwrap();
}

#[test]
fn local_tmux_and_screen_sessions_quit_and_restore_termios() {
    let _fixture_lock = serialize_live_tmux_fixture();
    let binary = env!("CARGO_BIN_EXE_asb-tui");
    let directory = PrivateDirectory::create();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let tmux_marker = directory.path().join("tmux-restored");
    let tmux_session = format!("asb-tui-test-{}-{nonce}", std::process::id());
    let tmux_socket = directory
        .path()
        .join(format!("asb-tui-socket-{}-{nonce}", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let tmux_command = format!(
        "before=$(stty -g) || exit 90; {binary}; status=$?; after=$(stty -g) || exit 91; test \"$before\" = \"$after\" || exit 92; printf restored >{}; exit $status",
        tmux_marker.display()
    );
    let tmux_guard = start_guarded_tmux_session(
        &tmux_socket,
        &tmux_session,
        &["-x", "80", "-y", "24", &tmux_command],
    )
    .unwrap_or_else(|failure| panic!("{failure}"));
    set_tmux_remain_on_exit(&tmux_socket, &tmux_session, &tmux_guard)
        .unwrap_or_else(|failure| panic!("{failure}"));
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
    let _fixture_lock = serialize_live_tmux_fixture();
    let scratch = PrivateDirectory::create();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session = format!("asb-tui-cleanup-{}-{nonce}", std::process::id());
    let socket = scratch
        .path()
        .join(format!(
            "asb-tui-cleanup-socket-{}-{nonce}",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let mut guard = start_guarded_tmux_session(
        &socket,
        &session,
        &["trap '' HUP TERM; while :; do sleep 1; done"],
    )
    .unwrap_or_else(|failure| panic!("{failure}"));
    let pane_process = match &guard {
        MultiplexerSession::Tmux {
            pane_process: Some(pane_process),
            ..
        } => pane_process.clone(),
        _ => unreachable!("guarded tmux session has pane authority"),
    };
    let process_group = pane_process.foreground_group;
    set_tmux_remain_on_exit(&socket, &session, &guard)
        .unwrap_or_else(|failure| panic!("{failure}"));
    let unrelated_group = u32::try_from(rustix::process::getpgrp().as_raw_pid()).unwrap();
    assert_ne!(unrelated_group, process_group);
    let mut changed_pid = pane_process.clone();
    changed_pid.pane_pid = changed_pid.pane_pid.checked_add(1).unwrap();
    let mut changed_tty = pane_process.clone();
    changed_tty.pane_tty = "/dev/pts/999999".into();
    let mut changed_group = pane_process.clone();
    changed_group.foreground_group = unrelated_group;
    for transitioned in [&changed_pid, &changed_tty, &changed_group] {
        assert!(
            !MultiplexerSession::stop_process_group(&socket, &session, transitioned),
            "transitioned pane tuple must not authorize group signalling"
        );
    }
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            guard.set_pane_process(changed_pid);
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
fn pane_foreground_group_parser_accepts_interposed_pane_leader_and_rejects_widening() {
    let pane = b"200\t/dev/pts/9\n";
    let trusted_topology = b" 200 200 300 pts/9 1000 1000\n 300 300 300 pts/9 1000 1000\n";
    assert_eq!(
        parse_pane_process_observation(pane, trusted_topology, 900, 1000),
        Some(PaneProcessTuple {
            pane_pid: 200,
            pane_tty: "/dev/pts/9".into(),
            foreground_group: 300,
        })
    );
    for rejected in [
        b" 200 200 300 pts/9 1000 1000\n 300 300 301 pts/9 1000 1000\n".as_slice(),
        b" 200 200 300 pts/9 1000 1000\n 300 300 300 pts/9 1001 1001\n".as_slice(),
        b" 200 200 300 pts/9 1000 1000\n 300 300 300 pts/9 1000 1001\n".as_slice(),
        b" 200 200 300 pts/9 1000 1000\n 200 300 300 pts/9 1000 1000\n 300 300 300 pts/9 1000 1000\n".as_slice(),
        b" 200 200 300 pts/9 1000 1000\n 300 300 300 pts/9 1000 1000\n 300 300 300 pts/9 1000 1000\n".as_slice(),
        b" 200 200 300 pts/9 1000 1000\n 400 400 400 pts/9 1000 1000\n".as_slice(),
        b" 200 200 300 pts/8 1000 1000\n 300 300 300 pts/8 1000 1000\n".as_slice(),
        b" 200 200 300 pts/9 1000 1000\n".as_slice(),
        b" 200 200 -1 pts/9 1000 1000\n".as_slice(),
        b" 200 200 300 pts/9 1000\n".as_slice(),
        b"\xff 200 200 300 pts/9 1000 1000\n".as_slice(),
    ] {
        assert_eq!(
            parse_pane_process_observation(pane, rejected, 900, 1000),
            None
        );
    }
    assert_eq!(
        parse_pane_process_observation(pane, trusted_topology, 300, 1000),
        None,
        "the test process group must never become a cleanup target"
    );
    for malformed_pane in [
        b"".as_slice(),
        b"1\t/dev/pts/9\n".as_slice(),
        b"200\tpts/9\n".as_slice(),
        b"200\t/dev/tty9\n".as_slice(),
        b"200\t/dev/pts/x\n".as_slice(),
        b"200\t/dev/pts/9\n201\t/dev/pts/9\n".as_slice(),
        b"\xff200\t/dev/pts/9\n".as_slice(),
    ] {
        assert_eq!(
            parse_pane_process_observation(malformed_pane, trusted_topology, 900, 1000),
            None
        );
    }
}

#[test]
fn process_group_signals_require_stable_generations_and_reobserve_before_kill() {
    let observed = PaneProcessObservation {
        pane_pid: 200,
        pane_start_time: 10,
        pane_tty: "/dev/pts/9".into(),
        tty_device: 1,
        tty_inode: 2,
        tty_rdevice: 3,
        foreground_group: 300,
        foreground_start_time: 20,
    };
    let pre_term_signals = std::cell::RefCell::new(Vec::new());
    let mut pre_term_mismatch = observed.clone();
    pre_term_mismatch.pane_start_time += 1;
    assert!(!MultiplexerSession::stop_process_group_with(
        &observed,
        || Some(pre_term_mismatch.clone()),
        |signal, group| {
            pre_term_signals
                .borrow_mut()
                .push((signal.to_owned(), group));
        },
        |_| false,
        || {},
    ));
    assert!(
        pre_term_signals.into_inner().is_empty(),
        "pre-TERM generation mismatch must issue zero signals"
    );
    for changed in [
        PaneProcessObservation {
            pane_start_time: 11,
            ..observed.clone()
        },
        PaneProcessObservation {
            tty_inode: 4,
            ..observed.clone()
        },
        PaneProcessObservation {
            foreground_start_time: 21,
            ..observed.clone()
        },
    ] {
        let observations = std::cell::Cell::new(0);
        let signals = std::cell::RefCell::new(Vec::new());
        assert!(
            !MultiplexerSession::stop_process_group_with(
                &observed,
                || {
                    let count = observations.get();
                    observations.set(count + 1);
                    (count == 0)
                        .then(|| observed.clone())
                        .or_else(|| Some(changed.clone()))
                },
                |signal, group| signals.borrow_mut().push((signal.to_owned(), group)),
                |_| false,
                || {},
            ),
            "generation or TTY reuse must suppress KILL"
        );
        assert_eq!(
            signals.into_inner(),
            [("-TERM".to_owned(), observed.foreground_group)]
        );
    }

    let observations = std::cell::Cell::new(0);
    let gone_checks = std::cell::Cell::new(0);
    let signals = std::cell::RefCell::new(Vec::new());
    assert!(MultiplexerSession::stop_process_group_with(
        &observed,
        || {
            observations.set(observations.get() + 1);
            Some(observed.clone())
        },
        |signal, group| signals.borrow_mut().push((signal.to_owned(), group)),
        |_| {
            let count = gone_checks.get();
            gone_checks.set(count + 1);
            count >= 101
        },
        || {},
    ));
    assert_eq!(observations.get(), 2, "TERM and KILL each need authority");
    assert_eq!(
        signals.into_inner(),
        [
            ("-TERM".to_owned(), observed.foreground_group),
            ("-KILL".to_owned(), observed.foreground_group),
        ]
    );
}

#[test]
fn server_cleanup_authenticates_generation_and_socket_before_kill() {
    let expected = TmuxServerObservation {
        process: ProcessGeneration {
            pid: 400,
            start_time: 30,
        },
        socket_device: 40,
        socket_inode: 50,
    };
    for changed in [
        TmuxServerObservation {
            process: ProcessGeneration {
                start_time: 31,
                ..expected.process.clone()
            },
            ..expected.clone()
        },
        TmuxServerObservation {
            socket_inode: 51,
            ..expected.clone()
        },
    ] {
        let kills = std::cell::Cell::new(0);
        assert!(!MultiplexerSession::stop_tmux_server_with(
            || changed == expected,
            || {
                kills.set(kills.get() + 1);
                true
            },
            || false,
            || {},
        ));
        assert_eq!(kills.get(), 0, "identity mismatch must issue zero kills");
    }

    let kills = std::cell::Cell::new(0);
    assert!(MultiplexerSession::stop_tmux_server_with(
        || true,
        || {
            kills.set(kills.get() + 1);
            true
        },
        || kills.get() == 1,
        || {},
    ));
    assert_eq!(
        kills.get(),
        1,
        "authenticated server gets one graceful kill"
    );
}

#[test]
fn tty_identity_rejects_symlinks_non_devices_and_wrong_owners() {
    use std::os::unix::fs::symlink;

    let directory = PrivateDirectory::create();
    let regular = directory.path().join("regular");
    fs::write(&regular, b"not a terminal").unwrap();
    let link = directory.path().join("link");
    symlink("/dev/null", &link).unwrap();
    let dangling = directory.path().join("dangling");
    symlink(directory.path().join("missing"), &dangling).unwrap();
    assert_eq!(tty_identity(regular.to_str().unwrap()), None);
    assert_eq!(tty_identity(link.to_str().unwrap()), None);
    assert!(
        !MultiplexerSession::path_is_absent(&dangling),
        "dangling symlink must not be treated as an absent owned socket"
    );
    let replacement_before = fs::symlink_metadata(&dangling).unwrap();
    let kills = std::cell::Cell::new(0);
    assert!(!MultiplexerSession::stop_tmux_server_with(
        || false,
        || {
            kills.set(kills.get() + 1);
            true
        },
        || false,
        || {},
    ));
    let replacement_after = fs::symlink_metadata(&dangling).unwrap();
    assert_eq!(kills.get(), 0);
    assert!(replacement_after.file_type().is_symlink());
    assert_eq!(replacement_after.dev(), replacement_before.dev());
    assert_eq!(replacement_after.ino(), replacement_before.ino());
    let wrong_uid = rustix::process::getuid().as_raw().wrapping_add(1);
    assert_eq!(tty_identity_for_uid("/dev/null", wrong_uid), None);
}

#[test]
fn hostile_tmux_tmpdir_cannot_redirect_owned_server_or_cleanup() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("hostile-tmux-tmpdir") {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let session = format!("asb-tui-tmpdir-{}-{nonce}", std::process::id());
        let scratch = PrivateDirectory::create();
        let socket = scratch
            .path()
            .join(format!(
                "asb-tui-tmpdir-socket-{}-{nonce}",
                std::process::id()
            ))
            .to_string_lossy()
            .into_owned();
        let guard = start_guarded_tmux_session(&socket, &session, &["sleep 30"])
            .unwrap_or_else(|failure| panic!("{failure}"));
        set_tmux_remain_on_exit(&socket, &session, &guard)
            .unwrap_or_else(|failure| panic!("{failure}"));
        assert!(
            MultiplexerSession::tmux_socket_path(&socket)
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_socket()),
            "clean tmux server did not use the fixed owner-private /tmp root"
        );
        assert!(guard.stop());
        assert!(guard.is_gone());
        return;
    }

    // The parent retains the process-local exclusion while the exact child
    // exercises the hostile environment. The child must not reacquire it.
    let _fixture_lock = serialize_live_tmux_fixture();
    let hostile = PrivateDirectory::create();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hostile_tmux_tmpdir_cannot_redirect_owned_server_or_cleanup",
        ])
        .env(CHILD_MODE, "hostile-tmux-tmpdir")
        .env("TMUX_TMPDIR", hostile.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "hostile TMUX_TMPDIR child failed without exposing its value"
    );
    assert_eq!(
        fs::read_dir(hostile.path()).unwrap().count(),
        0,
        "tmux created state under the hostile ambient root"
    );
}

#[test]
fn malformed_created_window_identity_drops_the_fully_authorized_guard() {
    let _fixture_lock = serialize_live_tmux_fixture();
    let scratch = PrivateDirectory::create();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session = format!("asb-tui-window-identity-{}-{nonce}", std::process::id());
    let socket = scratch
        .path()
        .join(format!(
            "asb-tui-window-identity-socket-{}-{nonce}",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let (created, _) = tmux_new_session_output(
        &socket,
        &session,
        &["trap '' HUP TERM; while :; do sleep 1; done"],
    )
    .expect(TMUX_CREATION_FAILED);
    assert!(created, "{TMUX_CREATION_FAILED}");

    // The guarded finisher must be the first operation after creation. The
    // dedicated HUP-resistant fixture proves exact group cleanup in detail.
    let result = finish_guarded_tmux_session(&socket, &session, Some(b"@1\n@2\n".to_vec())).err();
    assert_eq!(result.as_deref(), Some(TMUX_MALFORMED_WINDOW_IDENTITY));
    assert!(!bounded_tmux(&socket, &["has-session", "-t", &session]));
}

#[test]
fn tmux_guard_without_process_authority_still_cleans_its_exact_server() {
    let _fixture_lock = serialize_live_tmux_fixture();
    let scratch = PrivateDirectory::create();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session = format!("asb-tui-no-authority-{}-{nonce}", std::process::id());
    let socket = scratch
        .path()
        .join(format!(
            "asb-tui-no-authority-socket-{}-{nonce}",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let guarded_socket = socket.clone();
    let (created, _output) =
        tmux_new_session_output(&socket, &session, &["sleep 30"]).expect(TMUX_CREATION_FAILED);
    assert!(created, "{TMUX_CREATION_FAILED}");
    let mut guard = MultiplexerSession::Tmux {
        socket: guarded_socket.clone(),
        session: session.clone(),
        server: None,
        pane_process: None,
        startup: None,
    };
    let server =
        wait_for_available_observation(TMUX_STARTUP_ATTEMPTS, Duration::from_millis(10), || {
            tmux_server_observation(&guarded_socket, &session)
        })
        .expect(TMUX_STARTUP_NOT_READY);
    guard.set_server(server.clone());
    let observed_group =
        wait_for_available_observation(TMUX_STARTUP_ATTEMPTS, Duration::from_millis(10), || {
            tmux_pane_process_observation(&guarded_socket, &session)
        })
        .expect("cleanup probe must observe but not retain pane authority")
        .foreground_group;
    let option_calls = std::cell::Cell::new(0);
    assert_eq!(
        set_tmux_remain_on_exit_for_guard_with(
            &guard,
            || panic!("guard without pane authority must not observe startup"),
            |_| {
                option_calls.set(option_calls.get() + 1);
                true
            },
        ),
        Err(TMUX_SERVER_OR_SESSION_DISAPPEARED)
    );
    assert_eq!(option_calls.get(), 0);
    let acquisition = std::panic::catch_unwind(move || {
        let _guard = guard;
        tmux_pane_process_observation(&guarded_socket, "missing-session")
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
        tmux_socket_is_stale(&socket, &server),
        "exact tmux server remained live or its stale socket became connectable"
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
        tmux_args("/tmp/socket", &["capture-pane", "-p"]),
        ["-f", "/dev/null", "-S", "/tmp/socket", "capture-pane", "-p"]
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
fn tmux_window_identity_parser_is_closed_and_bounded() {
    for (input, expected) in [
        (b"@0\n".as_slice(), Some("@0")),
        (b"@42\n".as_slice(), Some("@42")),
        (b"@4294967295\n".as_slice(), Some("@4294967295")),
        (b"".as_slice(), None),
        (b"@\n".as_slice(), None),
        (b"@1".as_slice(), None),
        (b"@1\n@2\n".as_slice(), None),
        (b" @1\n".as_slice(), None),
        (b"@1 \n".as_slice(), None),
        (b"@+1\n".as_slice(), None),
        (b"@-1\n".as_slice(), None),
        (b"@01\n".as_slice(), None),
        (b"1\n".as_slice(), None),
        (b"@4294967296\n".as_slice(), None),
        ([0xff, b'\n'].as_slice(), None),
    ] {
        assert_eq!(
            parse_tmux_window_identity(input).as_deref(),
            expected,
            "unexpected parser result for {input:?}"
        );
    }
    assert_eq!(
        parse_tmux_window_identity(&vec![b'1'; MAX_MULTIPLEXER_OUTPUT + 1]),
        None
    );
    assert_eq!(
        parse_tmux_numeric_identity(b"$42\n", '$').as_deref(),
        Some("$42")
    );
    for malformed in [b"$01\n".as_slice(), b"$1\n$2\n", b"@1\n", b"$1"] {
        assert_eq!(parse_tmux_numeric_identity(malformed, '$'), None);
    }
}

fn synthetic_startup_observation(seed: u32, window: &str) -> TmuxStartupObservation {
    TmuxStartupObservation {
        server: TmuxServerObservation {
            process: ProcessGeneration {
                pid: seed + 10,
                start_time: u64::from(seed) + 20,
            },
            socket_device: u64::from(seed) + 30,
            socket_inode: u64::from(seed) + 40,
        },
        pane_process: PaneProcessObservation {
            pane_pid: seed + 50,
            pane_start_time: u64::from(seed) + 60,
            pane_tty: format!("/dev/pts/{}", seed + 70),
            tty_device: u64::from(seed) + 80,
            tty_inode: u64::from(seed) + 90,
            tty_rdevice: u64::from(seed) + 100,
            foreground_group: seed + 110,
            foreground_start_time: u64::from(seed) + 120,
        },
        session_identity: format!("${seed}"),
        window: window.to_owned(),
    }
}

#[test]
fn tmux_server_observation_diagnostics_are_closed_bounded_and_sequence_exact() {
    let stages = [
        TmuxServerObservationStage::Unavailable,
        TmuxServerObservationStage::SocketMetadataUnavailable,
        TmuxServerObservationStage::SocketMetadataInvalid,
        TmuxServerObservationStage::SocketPathInvalid,
        TmuxServerObservationStage::SocketCreationUnavailable,
        TmuxServerObservationStage::SocketConnectRejected,
        TmuxServerObservationStage::SocketPollUnavailable,
        TmuxServerObservationStage::SocketPollTimeout,
        TmuxServerObservationStage::SocketPollInvalid,
        TmuxServerObservationStage::SocketPollNoCompletion,
        TmuxServerObservationStage::SocketErrorUnavailable,
        TmuxServerObservationStage::SocketErrorMalformed,
        TmuxServerObservationStage::SocketErrorNonzero,
        TmuxServerObservationStage::PeerCredentialsUnavailable,
        TmuxServerObservationStage::PeerCredentialsInvalid,
        TmuxServerObservationStage::ProcessGenerationUnavailable,
        TmuxServerObservationStage::RepeatedPeerIdentityChanged,
        TmuxServerObservationStage::RepeatedSocketIdentityChanged,
        TmuxServerObservationStage::ProcessGenerationChanged,
    ];
    let mut labels = std::collections::BTreeSet::new();
    for stage in stages {
        let label = stage.label();
        assert!(label.len() <= 40);
        assert!(
            label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        );
        assert!(labels.insert(label), "diagnostic labels must be unique");
    }

    let expected = synthetic_startup_observation(300, "@9").server;
    let mut eventually_available = [
        Err(TmuxServerObservationStage::SocketMetadataUnavailable),
        Err(TmuxServerObservationStage::SocketConnectRejected),
        Ok(expected.clone()),
    ]
    .into_iter();
    assert_eq!(
        wait_for_server_observation(3, Duration::ZERO, || {
            eventually_available.next().unwrap()
        }),
        Ok(expected)
    );

    let mut exhausted = [
        Err(TmuxServerObservationStage::SocketMetadataUnavailable),
        Err(TmuxServerObservationStage::PeerCredentialsUnavailable),
    ]
    .into_iter();
    assert_eq!(
        wait_for_server_observation(2, Duration::ZERO, || exhausted.next().unwrap()),
        Err(TmuxServerObservationStage::PeerCredentialsUnavailable),
        "exhaustion must report only the final closed observation stage"
    );
}

#[test]
fn unix_socket_peer_credentials_bind_the_exact_live_process_generation() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("peer-credentials-server") {
        let socket = std::env::var_os("ASB_TUI_PEER_SOCKET").unwrap();
        let marker = std::env::var_os("ASB_TUI_PEER_READY").unwrap();
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        fs::write(marker, b"ready").unwrap();
        listener.accept().map(|_| ()).unwrap();
        return;
    }
    let scratch = PrivateDirectory::create();
    let socket = scratch.path().join("peer-credentials.sock");
    let ready = scratch.path().join("peer-ready");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unix_socket_peer_credentials_bind_the_exact_live_process_generation",
        ])
        .env(CHILD_MODE, "peer-credentials-server")
        .env("ASB_TUI_PEER_SOCKET", &socket)
        .env("ASB_TUI_PEER_READY", &ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(wait_for_marker(&ready), "peer fixture did not become ready");
    let pid = child.id();
    assert_eq!(
        tmux_socket_peer_process(&socket),
        Ok(ProcessGeneration {
            pid,
            start_time: process_start_time(pid).unwrap(),
        })
    );
    assert!(child.wait().unwrap().success());

    let absent = scratch.path().join("absent.sock");
    assert_eq!(
        tmux_socket_peer_process(&absent),
        Err(TmuxServerObservationStage::SocketConnectRejected)
    );

    let nul_path = std::path::Path::new(std::ffi::OsStr::from_bytes(b"invalid\0socket"));
    assert_eq!(
        tmux_socket_peer_process(nul_path),
        Err(TmuxServerObservationStage::SocketPathInvalid)
    );
    let address = unsafe {
        // SAFETY: Zero initializes sockaddr_un sufficiently to inspect its fixed array length.
        MaybeUninit::<libc::sockaddr_un>::zeroed().assume_init()
    };
    let overlong = vec![b'a'; address.sun_path.len()];
    assert_eq!(
        tmux_socket_peer_process(std::path::Path::new(std::ffi::OsStr::from_bytes(&overlong))),
        Err(TmuxServerObservationStage::SocketPathInvalid)
    );
}

#[test]
fn unix_socket_nonblocking_completion_uses_exact_so_error() {
    assert_eq!(classify_unix_connect_attempt(0, None), Ok(false));
    assert_eq!(
        classify_unix_connect_attempt(-1, Some(libc::EINPROGRESS)),
        Ok(true)
    );
    assert_eq!(
        classify_unix_connect_attempt(-1, Some(libc::EAGAIN)),
        Ok(true)
    );
    for error in [None, Some(libc::ECONNREFUSED), Some(libc::EINTR)] {
        assert_eq!(
            classify_unix_connect_attempt(-1, error),
            Err(TmuxServerObservationStage::SocketConnectRejected)
        );
    }

    let exact_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    assert_eq!(
        classify_unix_connect_completion(1, libc::POLLOUT, 0, exact_len, 0),
        Ok(())
    );
    assert_eq!(
        classify_unix_connect_completion(1, libc::POLLOUT | libc::POLLHUP, 0, exact_len, 0),
        Ok(())
    );
    assert_eq!(
        classify_unix_connect_completion(1, libc::POLLERR, 0, exact_len, 0),
        Ok(())
    );

    for (ready, revents, status, length, error, expected) in [
        (
            0,
            0,
            0,
            exact_len,
            0,
            TmuxServerObservationStage::SocketPollTimeout,
        ),
        (
            -1,
            0,
            0,
            exact_len,
            0,
            TmuxServerObservationStage::SocketPollUnavailable,
        ),
        (
            2,
            libc::POLLOUT,
            0,
            exact_len,
            0,
            TmuxServerObservationStage::SocketPollInvalid,
        ),
        (
            1,
            libc::POLLIN,
            0,
            exact_len,
            0,
            TmuxServerObservationStage::SocketPollNoCompletion,
        ),
        (
            1,
            libc::POLLOUT | libc::POLLNVAL,
            0,
            exact_len,
            0,
            TmuxServerObservationStage::SocketPollInvalid,
        ),
        (
            1,
            libc::POLLOUT,
            -1,
            exact_len,
            0,
            TmuxServerObservationStage::SocketErrorUnavailable,
        ),
        (
            1,
            libc::POLLOUT,
            0,
            exact_len - 1,
            0,
            TmuxServerObservationStage::SocketErrorMalformed,
        ),
        (
            1,
            libc::POLLOUT,
            0,
            exact_len,
            libc::ECONNREFUSED,
            TmuxServerObservationStage::SocketErrorNonzero,
        ),
    ] {
        assert_eq!(
            classify_unix_connect_completion(ready, revents, status, length, error),
            Err(expected)
        );
    }
}

#[test]
fn tmux_startup_requires_consecutive_complete_equal_observations() {
    let first = synthetic_startup_observation(100, "@7");
    let second = synthetic_startup_observation(200, "@8");
    let mut eventually_stable = [
        None,
        Some(first.clone()),
        None,
        Some(first.clone()),
        Some(first.clone()),
    ]
    .into_iter();
    let retained = std::cell::RefCell::new(Vec::new());
    assert_eq!(
        wait_for_stable_matching_observation(
            5,
            Duration::ZERO,
            || eventually_stable.next().flatten(),
            |_| true,
            |observed| retained.borrow_mut().push(observed.clone()),
        ),
        Some(first.clone())
    );
    let retained = retained.into_inner();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0], first);

    let mut alternating = [
        Some(first.clone()),
        Some(second.clone()),
        Some(first.clone()),
        Some(second),
    ]
    .into_iter();
    assert_eq!(
        wait_for_stable_matching_observation(
            4,
            Duration::ZERO,
            || alternating.next().flatten(),
            |_| true,
            |_| {},
        ),
        None
    );
    assert_eq!(
        wait_for_stable_matching_observation(
            1,
            Duration::ZERO,
            || Some(first.clone()),
            |_| true,
            |_| {},
        ),
        None,
        "elapsed time or one sample must not establish readiness"
    );
    assert_eq!(
        wait_for_stable_matching_observation::<TmuxStartupObservation>(
            3,
            Duration::ZERO,
            || None,
            |_| true,
            |_| {},
        ),
        None
    );
    let mut becomes_available = [None, None, Some(first.clone())].into_iter();
    assert_eq!(
        wait_for_available_observation(3, Duration::ZERO, || {
            becomes_available.next().flatten()
        }),
        Some(first.clone())
    );
    assert_eq!(
        wait_for_available_observation::<TmuxStartupObservation>(2, Duration::ZERO, || None,),
        None
    );

    let retained_server = first.server.clone();
    assert_eq!(
        wait_for_stable_matching_observation(
            2,
            Duration::ZERO,
            || Some(first.clone()),
            |observed| observed.server != retained_server,
            |_| panic!("mismatched authority must not be retained"),
        ),
        None
    );
}

#[test]
fn tmux_startup_observation_diagnostics_are_closed_and_sequence_exact() {
    let mut labels = BTreeSet::new();
    for stage in [
        TmuxStartupObservationStage::Unavailable,
        TmuxStartupObservationStage::ServerBeforeUnavailable,
        TmuxStartupObservationStage::SessionBeforeUnavailable,
        TmuxStartupObservationStage::WindowBeforeUnavailable,
        TmuxStartupObservationStage::PaneIdentityUnavailable,
        TmuxStartupObservationStage::PaneIdentityMalformed,
        TmuxStartupObservationStage::TtyIdentityUnavailable,
        TmuxStartupObservationStage::PaneGenerationUnavailable,
        TmuxStartupObservationStage::ProcessListBeforeUnavailable,
        TmuxStartupObservationStage::ProcessTupleInvalid,
        TmuxStartupObservationStage::ForegroundGenerationUnavailable,
        TmuxStartupObservationStage::ProcessListAfterUnavailable,
        TmuxStartupObservationStage::ProcessTupleAfterInvalid,
        TmuxStartupObservationStage::ProcessTupleChanged,
        TmuxStartupObservationStage::PaneIdentityChanged,
        TmuxStartupObservationStage::TtyIdentityChanged,
        TmuxStartupObservationStage::PaneGenerationChanged,
        TmuxStartupObservationStage::ForegroundGenerationChanged,
        TmuxStartupObservationStage::SessionAfterUnavailable,
        TmuxStartupObservationStage::WindowAfterUnavailable,
        TmuxStartupObservationStage::ServerAfterUnavailable,
        TmuxStartupObservationStage::ServerChanged,
        TmuxStartupObservationStage::SessionChanged,
        TmuxStartupObservationStage::WindowChanged,
        TmuxStartupObservationStage::StartupIdentityChanged,
        TmuxStartupObservationStage::RetainedServerChanged,
        TmuxStartupObservationStage::CreationWindowChanged,
    ] {
        let label = stage.label();
        assert!(label.len() <= 40);
        assert!(
            label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        );
        assert!(labels.insert(label));
    }

    let first = synthetic_startup_observation(100, "@7");
    let mut sequence = [
        Err(TmuxStartupObservationStage::SessionBeforeUnavailable),
        Ok(first.clone()),
        Ok(first.clone()),
    ]
    .into_iter();
    let retained = std::cell::Cell::new(0);
    assert_eq!(
        wait_for_stable_startup_observation(
            3,
            Duration::ZERO,
            || sequence.next().unwrap(),
            |_| true,
            |_| retained.set(retained.get() + 1),
        ),
        Ok(first)
    );
    assert_eq!(retained.get(), 1);

    let retained_after_error = std::cell::Cell::new(0);
    let mut fails_after_first = [
        Ok(synthetic_startup_observation(150, "@7")),
        Err(TmuxStartupObservationStage::ProcessListAfterUnavailable),
    ]
    .into_iter();
    assert_eq!(
        wait_for_stable_startup_observation(
            2,
            Duration::ZERO,
            || fails_after_first.next().unwrap(),
            |_| true,
            |_| retained_after_error.set(retained_after_error.get() + 1),
        ),
        Err(TmuxStartupObservationStage::ProcessListAfterUnavailable)
    );
    assert_eq!(
        retained_after_error.get(),
        1,
        "the first authenticated process authority must be retained for cleanup"
    );

    let retained_before_drift = std::cell::RefCell::new(Vec::new());
    let mut drifts = [
        Ok(synthetic_startup_observation(175, "@7")),
        Ok(synthetic_startup_observation(176, "@7")),
        Ok(synthetic_startup_observation(175, "@7")),
    ]
    .into_iter();
    assert_eq!(
        wait_for_stable_startup_observation(
            3,
            Duration::ZERO,
            || drifts.next().unwrap(),
            |_| true,
            |observed| retained_before_drift.borrow_mut().push(observed.clone()),
        ),
        Err(TmuxStartupObservationStage::StartupIdentityChanged)
    );
    assert_eq!(
        retained_before_drift.into_inner(),
        [synthetic_startup_observation(175, "@7")],
        "identity drift must not replace retained cleanup authority"
    );

    let mut mismatch = [Ok(synthetic_startup_observation(200, "@8"))].into_iter();
    assert_eq!(
        wait_for_stable_startup_observation(
            1,
            Duration::ZERO,
            || mismatch.next().unwrap(),
            |_| false,
            |_| panic!("mismatched startup must not be retained"),
        ),
        Err(TmuxStartupObservationStage::RetainedServerChanged)
    );
}

#[test]
fn retained_startup_transition_runs_no_window_option() {
    let expected = synthetic_startup_observation(100, "@7");
    let mut changed_session = expected.clone();
    changed_session.session_identity = "$101".into();
    let mut changed_pane = expected.clone();
    changed_pane.pane_process.pane_start_time += 1;
    for observed in [
        None,
        Some(synthetic_startup_observation(101, "@7")),
        Some(synthetic_startup_observation(100, "@8")),
        Some(changed_session),
        Some(changed_pane),
    ] {
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            set_tmux_remain_on_exit_authenticated_with(
                &expected,
                || observed.clone(),
                |_| {
                    calls.set(calls.get() + 1);
                    true
                },
            ),
            Err(TMUX_SERVER_OR_SESSION_DISAPPEARED)
        );
        assert_eq!(calls.get(), 0, "transition must run zero option commands");
    }

    let commands = std::cell::RefCell::new(Vec::<Vec<String>>::new());
    assert_eq!(
        set_tmux_remain_on_exit_authenticated_with(
            &expected,
            || Some(expected.clone()),
            |args| {
                commands
                    .borrow_mut()
                    .push(args.iter().map(|value| (*value).to_owned()).collect());
                true
            },
        ),
        Ok(())
    );
    assert_eq!(
        commands.into_inner(),
        [vec![
            "set-window-option",
            "-t",
            "@7",
            "remain-on-exit",
            "on"
        ]]
    );

    let observations = std::cell::RefCell::new([Some(expected.clone()), None].into_iter());
    let option_calls = std::cell::Cell::new(0);
    assert_eq!(
        set_tmux_remain_on_exit_authenticated_with(
            &expected,
            || observations.borrow_mut().next().flatten(),
            |_| {
                option_calls.set(option_calls.get() + 1);
                false
            },
        ),
        Err(TMUX_SERVER_OR_SESSION_DISAPPEARED)
    );
    assert_eq!(option_calls.get(), 1);
}

#[test]
fn tmux_creation_and_window_option_argv_are_exact_and_fail_closed() {
    assert_eq!(
        tmux_new_session_args("portable-session", &["sleep 30"]),
        [
            "new-session",
            "-P",
            "-F",
            "#{window_id}",
            "-d",
            "-s",
            "portable-session",
            "sleep 30",
        ]
    );

    let observed = std::cell::RefCell::new(Vec::<Vec<String>>::new());
    assert_eq!(
        set_tmux_remain_on_exit_with("portable-session", "@42", |args| {
            observed
                .borrow_mut()
                .push(args.iter().map(|value| (*value).to_owned()).collect());
            true
        }),
        Ok(())
    );
    assert_eq!(
        observed.into_inner(),
        vec![
            vec!["has-session", "-t", "portable-session"],
            vec!["set-window-option", "-t", "@42", "remain-on-exit", "on"],
        ]
    );

    let private_session = "private-session-name";
    let private_window = "@98765";
    let disappeared =
        set_tmux_remain_on_exit_with(private_session, private_window, |_| false).unwrap_err();
    let mut calls = 0;
    let unsupported = set_tmux_remain_on_exit_with(private_session, private_window, |_| {
        calls += 1;
        calls != 2
    })
    .unwrap_err();
    let mut calls = 0;
    let raced = set_tmux_remain_on_exit_with(private_session, private_window, |_| {
        calls += 1;
        calls == 1
    })
    .unwrap_err();
    assert_eq!(disappeared, TMUX_SERVER_OR_SESSION_DISAPPEARED);
    assert_eq!(unsupported, TMUX_OPTION_UNSUPPORTED_OR_UNKNOWN);
    assert_eq!(raced, TMUX_SERVER_OR_SESSION_DISAPPEARED);
    for failure in [
        TMUX_CREATION_FAILED,
        TMUX_MALFORMED_WINDOW_IDENTITY,
        TMUX_STARTUP_NOT_READY,
        disappeared,
        unsupported,
        raced,
    ] {
        assert!(!failure.contains(private_session));
        assert!(!failure.contains(private_window));
        assert!(!failure.contains('/') && failure.is_ascii());
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
