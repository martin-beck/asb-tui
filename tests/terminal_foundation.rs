// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Real pseudo-terminal checks for the standalone Crossterm lifecycle.
#![cfg(unix)]

use asb_tui::runtime::TerminalSession;
use std::process::Command;

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

#[test]
fn pseudo_terminal_normal_exit_restores_screen_cursor_paste_and_raw_mode() {
    if std::env::var(CHILD_MODE).as_deref() == Ok("normal") {
        let mut session = TerminalSession::enter().unwrap();
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
        let _session = TerminalSession::enter().unwrap();
        panic!("intentional terminal restoration probe");
    }
    let output = under_pty("pseudo_terminal_panic_unwind_restores_terminal", "panic");
    assert!(
        !output.status.success(),
        "nested panic probe unexpectedly passed"
    );
    assert_terminal_boundaries(&output.stdout);
}
