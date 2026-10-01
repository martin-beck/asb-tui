// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded separation of the inherited broker fd from the interactive PTY.

use std::io::IsTerminal;
use std::{os::unix::fs::FileTypeExt, path::Path};

/// Replace stdin with the authenticated process's controlling terminal.
/// fd 0 may contain broker frames before this function runs, so no caller
/// supplied path or broker descriptor is accepted as a terminal substitute.
pub fn redirect_stdin_to_controlling_terminal() -> std::io::Result<()> {
    let terminal = std::fs::File::open("/dev/tty")?;
    let metadata = terminal.metadata()?;
    if !terminal.is_terminal() || !metadata.file_type().is_char_device() {
        return Err(std::io::Error::other(
            "controlling terminal is not a character device",
        ));
    }
    redirect_open_terminal(terminal)
}

/// Development-only injection seam for a qualification PTY. Production code
/// must use [`redirect_stdin_to_controlling_terminal`].
pub fn redirect_stdin_to_terminal_path(path: &Path) -> std::io::Result<()> {
    let path_text = path
        .to_str()
        .ok_or_else(|| std::io::Error::other("terminal path is not valid UTF-8"))?;
    let Some(pty_name) = path_text.strip_prefix("/dev/pts/") else {
        return Err(std::io::Error::other("terminal path is outside /dev/pts"));
    };
    if pty_name.is_empty()
        || pty_name.len() > 16
        || !pty_name.bytes().all(|byte| byte.is_ascii_digit())
        || path_text.len() > 64
    {
        return Err(std::io::Error::other("terminal path is malformed"));
    }
    let metadata = std::fs::symlink_metadata(path)?;
    use std::os::unix::fs::MetadataExt;
    if !metadata.file_type().is_char_device()
        || metadata.uid() != rustix::process::getuid().as_raw()
    {
        return Err(std::io::Error::other("terminal owner mismatch"));
    }
    let terminal = std::fs::File::open(path)?;
    let opened_metadata = terminal.metadata()?;
    if !terminal.is_terminal() || !opened_metadata.file_type().is_char_device() {
        return Err(std::io::Error::other(
            "controlling terminal is not a character device",
        ));
    }
    redirect_open_terminal(terminal)
}

fn redirect_open_terminal(terminal: std::fs::File) -> std::io::Result<()> {
    rustix::stdio::dup2_stdin(&terminal).map_err(std::io::Error::other)?;
    if !std::io::stdin().is_terminal() {
        return Err(std::io::Error::other("stdin is not a terminal"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_controlling_terminal_fails_closed() {
        if !std::io::stdin().is_terminal() {
            assert!(redirect_stdin_to_controlling_terminal().is_err());
        }
    }

    #[test]
    fn non_terminal_injection_fails_closed() {
        assert!(redirect_stdin_to_terminal_path(Path::new("/dev/null")).is_err());
        assert!(redirect_stdin_to_terminal_path(Path::new("relative-tty")).is_err());
        assert!(redirect_stdin_to_terminal_path(Path::new("/dev/pts/999999")).is_err());
    }
}
