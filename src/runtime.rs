// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Crossterm input and rollback-safe terminal lifecycle primitives.

use crate::{
    app::{Action, AppError, AppState},
    renderer,
    terminal::{RenderPolicy, frame_dimensions_are_safe},
};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
};
use signal_hook::{
    consts::signal::{SIGCONT, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP},
    iterator::Signals,
};
use std::{fmt, io, time::Duration};

struct BoundedBackend<B>(B);

fn bounded_size(size: Size) -> io::Result<Size> {
    if size.width == 0 || size.height == 0 {
        return Ok(Size::new(1, 1));
    }
    frame_dimensions_are_safe(size.width, size.height)
        .then_some(size)
        .ok_or_else(|| io::Error::other("terminal frame exceeds allocation policy"))
}

impl<B: Backend<Error = io::Error>> Backend for BoundedBackend<B> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.0.draw(content)
    }

    fn append_lines(&mut self, count: u16) -> Result<(), Self::Error> {
        self.0.append_lines(count)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.0.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.0.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.0.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.0.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        bounded_size(self.0.size()?)
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        let window = self.0.window_size()?;
        bounded_size(window.columns_rows)?;
        Ok(window)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush()
    }
}

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

impl From<AppError> for RuntimeError {
    fn from(error: AppError) -> Self {
        Self(io::Error::other(error))
    }
}

trait LifecycleOps {
    fn enter(&mut self) -> io::Result<()>;
    fn restore(&mut self) -> io::Result<()>;
}

trait TerminalEffects {
    fn enable_raw(&mut self) -> io::Result<()>;
    fn enter_screen(&mut self) -> io::Result<()>;
    fn hide_cursor(&mut self) -> io::Result<()>;
    fn enable_paste(&mut self) -> io::Result<()>;
    fn disable_paste(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    fn leave_screen(&mut self) -> io::Result<()>;
    fn disable_raw(&mut self) -> io::Result<()>;
}

struct SystemEffects;

impl TerminalEffects for SystemEffects {
    fn enable_raw(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn enter_screen(&mut self) -> io::Result<()> {
        execute!(io::stdout(), EnterAlternateScreen)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Hide)
    }

    fn enable_paste(&mut self) -> io::Result<()> {
        execute!(io::stdout(), EnableBracketedPaste)
    }

    fn disable_paste(&mut self) -> io::Result<()> {
        execute!(io::stdout(), DisableBracketedPaste)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Show)
    }

    fn leave_screen(&mut self) -> io::Result<()> {
        execute!(io::stdout(), LeaveAlternateScreen)
    }

    fn disable_raw(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }
}

struct CrosstermLifecycle<E: TerminalEffects = SystemEffects> {
    effects: E,
    alternate_screen: bool,
    bracketed_paste: bool,
    raw_active: bool,
    screen_active: bool,
    cursor_hidden: bool,
    paste_active: bool,
}

fn restore_effect(
    active: &mut bool,
    operation: impl FnOnce() -> io::Result<()>,
    first_error: &mut Option<io::Error>,
) {
    if !*active {
        return;
    }
    match operation() {
        Ok(()) => *active = false,
        Err(error) if first_error.is_none() => *first_error = Some(error),
        Err(_) => {}
    }
}

impl<E: TerminalEffects> LifecycleOps for CrosstermLifecycle<E> {
    fn enter(&mut self) -> io::Result<()> {
        self.effects.enable_raw()?;
        self.raw_active = true;
        if self.alternate_screen {
            self.effects.enter_screen()?;
            self.screen_active = true;
            self.effects.hide_cursor()?;
            self.cursor_hidden = true;
        }
        if self.bracketed_paste {
            self.effects.enable_paste()?;
            self.paste_active = true;
        }
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        let mut first_error = None;
        restore_effect(
            &mut self.paste_active,
            || self.effects.disable_paste(),
            &mut first_error,
        );
        restore_effect(
            &mut self.cursor_hidden,
            || self.effects.show_cursor(),
            &mut first_error,
        );
        restore_effect(
            &mut self.screen_active,
            || self.effects.leave_screen(),
            &mut first_error,
        );
        restore_effect(
            &mut self.raw_active,
            || self.effects.disable_raw(),
            &mut first_error,
        );
        first_error.map_or(Ok(()), Err)
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
        self.ops.restore()?;
        self.active = false;
        Ok(())
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
    /// Enter only the raw-input features approved by the render policy.
    pub fn enter(policy: RenderPolicy) -> Result<Self, RuntimeError> {
        if !policy.alternate_screen {
            return Err(RuntimeError(io::Error::other(
                "interactive terminal policy required",
            )));
        }
        Ok(Self {
            guard: LifecycleGuard::start(CrosstermLifecycle {
                effects: SystemEffects,
                alternate_screen: policy.alternate_screen,
                bracketed_paste: policy.bracketed_paste,
                raw_active: false,
                screen_active: false,
                cursor_hidden: false,
                paste_active: false,
            })?,
        })
    }

    /// Restore cursor, paste, screen, and raw-input state. Calling twice is harmless.
    pub fn restore(&mut self) -> Result<(), RuntimeError> {
        self.guard.restore()?;
        Ok(())
    }
}

/// Run the single-writer interactive draw loop until the operator quits.
pub fn run_interactive(state: &mut AppState, policy: RenderPolicy) -> Result<(), RuntimeError> {
    let mut signals = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP, SIGCONT])?;
    let mut session = TerminalSession::enter(policy)?;
    let backend = BoundedBackend(CrosstermBackend::new(io::stdout()));
    let mut terminal = Terminal::new(backend)?;
    while !state.should_quit() {
        handle_signals(&mut signals, &mut session, &mut terminal, policy)?;
        terminal.draw(|frame| renderer::render(frame, state, policy))?;
        if let Some(action) = poll_action(Duration::from_millis(50))? {
            state.apply(action)?;
        }
        handle_signals(&mut signals, &mut session, &mut terminal, policy)?;
    }
    drop(terminal);
    session.restore()
}

fn handle_signals(
    signals: &mut Signals,
    session: &mut TerminalSession,
    terminal: &mut Terminal<BoundedBackend<CrosstermBackend<io::Stdout>>>,
    policy: RenderPolicy,
) -> Result<(), RuntimeError> {
    for signal in signals.pending() {
        match signal {
            SIGHUP | SIGINT | SIGQUIT | SIGTERM => {
                session.restore()?;
                signal_hook::low_level::emulate_default_handler(signal)?;
                return Err(RuntimeError(io::Error::other(
                    "termination signal returned",
                )));
            }
            SIGTSTP => {
                session.restore()?;
                signal_hook::low_level::emulate_default_handler(SIGTSTP)?;
                *session = TerminalSession::enter(policy)?;
                terminal.clear()?;
            }
            SIGCONT => {}
            _ => return Err(RuntimeError(io::Error::other("unknown signal"))),
        }
    }
    Ok(())
}

/// Poll once for a bounded terminal action. Unknown input is ignored.
pub fn poll_action(timeout: Duration) -> Result<Option<Action>, RuntimeError> {
    let ready = match event::poll(timeout) {
        Ok(ready) => ready,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !ready {
        return Ok(None);
    }
    Ok(action_from_event(event::read()?))
}

fn action_from_event(event: Event) -> Option<Action> {
    match event {
        Event::Resize(columns, lines) if frame_dimensions_are_safe(columns, lines) => {
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
    fn restoration_error_is_retried_on_drop() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut ops = fake(&calls);
        ops.fail_restore = true;
        let mut guard = LifecycleGuard::start(ops).unwrap();
        assert!(guard.restore().is_err());
        drop(guard);
        assert_eq!(*calls.borrow(), ["enter", "restore", "restore"]);
    }

    struct FakeTerminalEffects {
        calls: Rc<RefCell<Vec<&'static str>>>,
        fail_on: &'static str,
        fail_once: bool,
    }

    impl FakeTerminalEffects {
        fn effect(&mut self, name: &'static str) -> io::Result<()> {
            self.calls.borrow_mut().push(name);
            if name == self.fail_on && self.fail_once {
                self.fail_once = false;
                Err(io::Error::other(name))
            } else {
                Ok(())
            }
        }
    }

    impl TerminalEffects for FakeTerminalEffects {
        fn enable_raw(&mut self) -> io::Result<()> {
            self.effect("enable_raw")
        }

        fn enter_screen(&mut self) -> io::Result<()> {
            self.effect("enter_screen")
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.effect("hide_cursor")
        }

        fn enable_paste(&mut self) -> io::Result<()> {
            self.effect("enable_paste")
        }

        fn disable_paste(&mut self) -> io::Result<()> {
            self.effect("disable_paste")
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.effect("show_cursor")
        }

        fn leave_screen(&mut self) -> io::Result<()> {
            self.effect("leave_screen")
        }

        fn disable_raw(&mut self) -> io::Result<()> {
            self.effect("disable_raw")
        }
    }

    fn terminal_ops(
        calls: &Rc<RefCell<Vec<&'static str>>>,
        fail_on: &'static str,
    ) -> CrosstermLifecycle<FakeTerminalEffects> {
        CrosstermLifecycle {
            effects: FakeTerminalEffects {
                calls: Rc::clone(calls),
                fail_on,
                fail_once: true,
            },
            alternate_screen: true,
            bracketed_paste: true,
            raw_active: false,
            screen_active: false,
            cursor_hidden: false,
            paste_active: false,
        }
    }

    #[test]
    fn partial_terminal_startup_restores_every_acquired_effect() {
        for (failure, expected) in [
            ("enable_raw", vec!["enable_raw"]),
            (
                "enter_screen",
                vec!["enable_raw", "enter_screen", "disable_raw"],
            ),
            (
                "hide_cursor",
                vec![
                    "enable_raw",
                    "enter_screen",
                    "hide_cursor",
                    "leave_screen",
                    "disable_raw",
                ],
            ),
            (
                "enable_paste",
                vec![
                    "enable_raw",
                    "enter_screen",
                    "hide_cursor",
                    "enable_paste",
                    "show_cursor",
                    "leave_screen",
                    "disable_raw",
                ],
            ),
        ] {
            let calls = Rc::new(RefCell::new(Vec::new()));
            assert!(LifecycleGuard::start(terminal_ops(&calls, failure)).is_err());
            assert_eq!(*calls.borrow(), expected, "failure at {failure}");
        }
    }

    #[test]
    fn every_failed_restore_effect_is_retried_once_and_then_deactivated() {
        for failure in [
            "disable_paste",
            "show_cursor",
            "leave_screen",
            "disable_raw",
        ] {
            let calls = Rc::new(RefCell::new(Vec::new()));
            let mut guard = LifecycleGuard::start(terminal_ops(&calls, failure)).unwrap();
            assert!(guard.restore().is_err(), "failure at {failure}");
            assert_eq!(
                &calls.borrow()[..8],
                [
                    "enable_raw",
                    "enter_screen",
                    "hide_cursor",
                    "enable_paste",
                    "disable_paste",
                    "show_cursor",
                    "leave_screen",
                    "disable_raw",
                ],
                "later cleanup effects must run after {failure}"
            );
            guard.restore().unwrap();
            guard.restore().unwrap();
            drop(guard);
            assert_eq!(
                &calls.borrow()[8..],
                [failure],
                "only the failed effect must be retried"
            );
        }
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
    fn backend_size_policy_precedes_frame_allocation() {
        assert_eq!(bounded_size(Size::new(0, 0)).unwrap(), Size::new(1, 1));
        assert_eq!(bounded_size(Size::new(80, 24)).unwrap(), Size::new(80, 24));
        assert!(bounded_size(Size::new(u16::MAX, u16::MAX)).is_err());
        assert!(bounded_size(Size::new(4_096, 4_096)).is_err());
    }

    #[test]
    fn runtime_error_is_content_free_and_preserves_source() {
        let error = RuntimeError::from(io::Error::other("private detail"));
        assert_eq!(error.to_string(), "terminal operation failed");
        assert!(std::error::Error::source(&error).is_some());
        assert!(!error.to_string().contains("private detail"));
    }
}
