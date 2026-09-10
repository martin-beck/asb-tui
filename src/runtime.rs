// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Crossterm input and rollback-safe terminal lifecycle primitives.

use crate::app::Action;
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::{fmt, io, time::Duration};

/// Terminal startup, input, or restoration error.
#[derive(Debug)]
pub struct RuntimeError(io::Error);

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("terminal operation failed")
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<io::Error> for RuntimeError {
    fn from(error: io::Error) -> Self {
        Self(error)
    }
}

trait LifecycleOps {
    fn enter(&mut self) -> io::Result<()>;
    fn restore(&mut self) -> io::Result<()>;
}

struct CrosstermLifecycle;

impl LifecycleOps for CrosstermLifecycle {
    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        if let Err(error) = execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        ) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        let screen_result = execute!(
            io::stdout(),
            Show,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let raw_result = disable_raw_mode();
        screen_result.and(raw_result)
    }
}

struct LifecycleGuard<O: LifecycleOps> {
    ops: O,
    active: bool,
}

impl<O: LifecycleOps> LifecycleGuard<O> {
    fn start(mut ops: O) -> io::Result<Self> {
        if let Err(error) = ops.enter() {
            let _ = ops.restore();
            return Err(error);
        }
        Ok(Self { ops, active: true })
    }

    fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        self.ops.restore()
    }
}

impl<O: LifecycleOps> Drop for LifecycleGuard<O> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Active raw/alternate-screen session. Dropping it always attempts restoration.
pub struct TerminalSession {
    guard: LifecycleGuard<CrosstermLifecycle>,
}

impl TerminalSession {
    /// Enter raw input and the alternate screen atomically.
    pub fn enter() -> Result<Self, RuntimeError> {
        Ok(Self {
            guard: LifecycleGuard::start(CrosstermLifecycle)?,
        })
    }

    /// Restore cursor, paste, screen, and raw-input state. Calling twice is harmless.
    pub fn restore(&mut self) -> Result<(), RuntimeError> {
        self.guard.restore()?;
        Ok(())
    }
}

/// Poll once for a bounded terminal action. Unknown input is ignored.
pub fn poll_action(timeout: Duration) -> Result<Option<Action>, RuntimeError> {
    if !event::poll(timeout)? {
        return Ok(None);
    }
    Ok(action_from_event(event::read()?))
}

fn action_from_event(event: Event) -> Option<Action> {
    match event {
        Event::Resize(columns, lines) if columns > 0 && lines > 0 => {
            Some(Action::Resize { columns, lines })
        }
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            let quit = key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL));
            quit.then_some(Action::Quit)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};
    use std::{cell::RefCell, rc::Rc};

    struct FakeOps {
        calls: Rc<RefCell<Vec<&'static str>>>,
        fail_enter: bool,
        fail_restore: bool,
    }

    impl LifecycleOps for FakeOps {
        fn enter(&mut self) -> io::Result<()> {
            self.calls.borrow_mut().push("enter");
            if self.fail_enter {
                Err(io::Error::other("enter"))
            } else {
                Ok(())
            }
        }

        fn restore(&mut self) -> io::Result<()> {
            self.calls.borrow_mut().push("restore");
            if self.fail_restore {
                Err(io::Error::other("restore"))
            } else {
                Ok(())
            }
        }
    }

    fn fake(calls: &Rc<RefCell<Vec<&'static str>>>) -> FakeOps {
        FakeOps {
            calls: Rc::clone(calls),
            fail_enter: false,
            fail_restore: false,
        }
    }

    #[test]
    fn lifecycle_restores_on_normal_exit_drop_and_failed_startup() {
        let normal = Rc::new(RefCell::new(Vec::new()));
        let mut guard = LifecycleGuard::start(fake(&normal)).unwrap();
        guard.restore().unwrap();
        guard.restore().unwrap();
        drop(guard);
        assert_eq!(*normal.borrow(), ["enter", "restore"]);

        let dropped = Rc::new(RefCell::new(Vec::new()));
        drop(LifecycleGuard::start(fake(&dropped)).unwrap());
        assert_eq!(*dropped.borrow(), ["enter", "restore"]);

        let failed = Rc::new(RefCell::new(Vec::new()));
        let mut ops = fake(&failed);
        ops.fail_enter = true;
        assert!(LifecycleGuard::start(ops).is_err());
        assert_eq!(*failed.borrow(), ["enter", "restore"]);
    }

    #[test]
    fn restoration_error_does_not_repeat_destructive_cleanup() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut ops = fake(&calls);
        ops.fail_restore = true;
        let mut guard = LifecycleGuard::start(ops).unwrap();
        assert!(guard.restore().is_err());
        drop(guard);
        assert_eq!(*calls.borrow(), ["enter", "restore"]);
    }

    #[test]
    fn input_mapping_accepts_only_press_quit_and_valid_resize() {
        let key = |code, modifiers, kind| {
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind,
                state: KeyEventState::NONE,
            })
        };
        assert_eq!(
            action_from_event(key(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Press,
            )),
            Some(Action::Quit)
        );
        assert_eq!(
            action_from_event(key(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            )),
            Some(Action::Quit)
        );
        assert_eq!(
            action_from_event(key(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            None
        );
        assert_eq!(
            action_from_event(Event::Resize(80, 24)),
            Some(Action::Resize {
                columns: 80,
                lines: 24,
            })
        );
        assert_eq!(action_from_event(Event::Resize(0, 24)), None);
    }

    #[test]
    fn runtime_error_is_content_free_and_preserves_source() {
        let error = RuntimeError::from(io::Error::other("private detail"));
        assert_eq!(error.to_string(), "terminal operation failed");
        assert!(std::error::Error::source(&error).is_some());
        assert!(!error.to_string().contains("private detail"));
    }
}
