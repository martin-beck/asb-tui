// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Crossterm input and rollback-safe terminal lifecycle primitives.

use crate::{
    app::{Action, AppError, AppState},
    control_transport::AuthenticatedBrokerSession,
    live_projection::ControlProjection,
    startup::ReadinessProvider,
    terminal::{RenderPolicy, frame_dimensions_are_safe},
    ui,
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
use std::{
    fmt, io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

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

/// Perform one authenticated, read-only control refresh and publish it to the
/// workspace. The transport layer validates every response; the projection is
/// cloned and committed atomically so a failed refresh leaves the prior UI
/// snapshot intact.
pub fn poll_authenticated_workspace(
    session: &mut AuthenticatedBrokerSession,
    projection: &mut ControlProjection,
    workspace: &mut ui::WorkspaceState,
) -> Result<(), RuntimeError> {
    session
        .poll_projection_with_context_for_runtime(projection, false)
        .map_err(|error| RuntimeError(io::Error::other(error)))?;
    workspace.apply_live_snapshot(projection.snapshot());
    Ok(())
}

/// Dispatch a renderer-neutral provider/recording action through the
/// authenticated control seam. The action model performs local fail-closed
/// gating; transport remains authoritative for every resulting state.
pub fn dispatch_control_action(
    action: crate::actions::UiAction,
    recording: &mut crate::recording_dispatch::RecordingDispatchState,
    session: &mut AuthenticatedBrokerSession,
    projection: &mut ControlProjection,
    idempotency_key: String,
) -> Result<crate::recording_dispatch::RecordingDispatchOutcome, RuntimeError> {
    if action == crate::actions::UiAction::SelectOfflineCassette {
        let catalog = recording.authenticated_catalog.as_ref().ok_or_else(|| {
            RuntimeError(io::Error::other(
                "authenticated cassette catalog unavailable",
            ))
        })?;
        recording.select_cassette(select_next_authenticated_cassette(
            catalog,
            recording.selected_cassette_sha256.as_deref(),
        )?);
        return Ok(crate::recording_dispatch::RecordingDispatchOutcome::CassetteSelected);
    }
    if action == crate::actions::UiAction::ReplaySelected {
        let catalog = recording.authenticated_catalog.as_ref().ok_or_else(|| {
            RuntimeError(io::Error::other(
                "authenticated cassette catalog unavailable",
            ))
        })?;
        let cassette_sha256 = recording
            .selected_cassette_sha256
            .as_deref()
            .ok_or_else(|| RuntimeError(io::Error::other("no cassette selected")))?;
        let entry = catalog
            .entries
            .iter()
            .find(|entry| entry.cassette_sha256 == cassette_sha256)
            .ok_or_else(|| RuntimeError(io::Error::other("selected cassette is not in catalog")))?;
        let mut campaign = prepare_authenticated_replay_campaign(catalog, cassette_sha256)?;
        run_authenticated_replay_selection(
            session,
            &mut campaign,
            catalog,
            &entry.workload_id,
            cassette_sha256,
            idempotency_key,
        )?;
        return Ok(crate::recording_dispatch::RecordingDispatchOutcome::ReplayDispatched);
    }
    let outcome = crate::recording_dispatch::dispatch_with_backend(
        action,
        recording,
        session,
        projection,
        idempotency_key,
    )
    .map_err(|_| RuntimeError(io::Error::other("control action failed")))?;
    if action == crate::actions::UiAction::ActivateOfflineDefault {
        let snapshot = projection.snapshot();
        let campaign = snapshot
            .recording_campaign_lifecycle
            .as_ref()
            .ok_or_else(|| RuntimeError(io::Error::other("recording campaign unavailable")))?;
        recording.authenticated_catalog = Some(fetch_authenticated_cassette_catalog(
            session,
            campaign.campaign_id.clone(),
            campaign.generation,
        )?);
    }
    Ok(outcome)
}

/// Execute the explicit operator retry action against one selected run.
/// Selection and eligibility remain runner-authoritative; this helper only
/// carries the typed identity and stable idempotency key.
pub fn dispatch_retry_run(
    session: &mut AuthenticatedBrokerSession,
    projection: &mut ControlProjection,
    run_id: crate::control_codec::RunId,
    idempotency_key: String,
) -> Result<(), RuntimeError> {
    session
        .repeat_run(projection, run_id, idempotency_key)
        .map_err(|error| RuntimeError(io::Error::other(error)))
}

/// Execute the explicit live/offline comparison action. ASB validates run
/// compatibility and returns the digest-bound analysis projection.
pub fn dispatch_compare_live_offline(
    session: &mut AuthenticatedBrokerSession,
    projection: &mut ControlProjection,
    run_ids: Vec<crate::control_codec::RunId>,
) -> Result<(), RuntimeError> {
    session
        .analyze_runs(projection, run_ids)
        .map_err(|error| RuntimeError(io::Error::other(error)))
}

fn select_next_authenticated_cassette(
    catalog: &crate::benchmark_route::AuthenticatedCassetteCatalog,
    selected: Option<&str>,
) -> Result<String, RuntimeError> {
    if catalog.entries.is_empty() {
        return Err(RuntimeError(io::Error::other(
            "authenticated cassette catalog is empty",
        )));
    }
    let next = selected
        .and_then(|selected| {
            catalog
                .entries
                .iter()
                .position(|entry| entry.cassette_sha256 == selected)
        })
        .map_or(0, |index| (index + 1) % catalog.entries.len());
    Ok(catalog.entries[next].cassette_sha256.clone())
}

fn prepare_authenticated_replay_campaign(
    catalog: &crate::benchmark_route::AuthenticatedCassetteCatalog,
    cassette_sha256: &str,
) -> Result<crate::benchmark_route::GuidedCampaign, RuntimeError> {
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.cassette_sha256 == cassette_sha256)
        .ok_or_else(|| RuntimeError(io::Error::other("selected cassette is not in catalog")))?;
    let guided_catalog = crate::benchmark_route::GuidedCatalog::new(
        vec![entry.workload_id.clone()],
        vec![entry.agent_id.clone()],
        vec![entry.scorer_revision.clone()],
    )
    .map_err(|error| {
        RuntimeError(io::Error::other(format!(
            "replay catalog rejected: {error:?}"
        )))
    })?;
    let mut campaign = crate::benchmark_route::GuidedCampaign::with_catalog(
        entry.workload_id.clone(),
        guided_catalog,
    )
    .map_err(|error| {
        RuntimeError(io::Error::other(format!(
            "replay campaign rejected: {error:?}"
        )))
    })?;
    campaign
        .add_agent(entry.agent_id.clone())
        .and_then(|_| campaign.add_measure(entry.scorer_revision.clone()))
        .and_then(|_| campaign.bind_provider_profile_sha256(entry.provider_profile_sha256.clone()))
        .and_then(|_| campaign.set_replay_mode(crate::benchmark_route::ReplayMode::OfflineReplay))
        .and_then(|_| campaign.review())
        .map_err(|error| {
            RuntimeError(io::Error::other(format!(
                "replay selection rejected: {error:?}"
            )))
        })?;
    Ok(campaign)
}

/// Fetch the runner-owned digest-only cassette catalog through the
/// authenticated control session.  Cassette bytes never enter the TUI.
pub fn fetch_authenticated_cassette_catalog(
    session: &mut AuthenticatedBrokerSession,
    campaign_id: String,
    generation: crate::control_codec::Revision,
) -> Result<crate::benchmark_route::AuthenticatedCassetteCatalog, RuntimeError> {
    let catalog = session
        .recording_cassette_catalog(campaign_id, generation)
        .map_err(|error| RuntimeError(io::Error::other(error)))?;
    catalog.try_into().map_err(|error| {
        RuntimeError(io::Error::other(format!(
            "cassette catalog rejected: {error:?}"
        )))
    })
}

/// Dispatch a guided replay intent through ASB's authenticated, provider-free
/// replay authority.  The transport rechecks campaign, generation, profile,
/// agent, workload, and cassette identity before returning.
pub fn dispatch_authenticated_replay(
    session: &mut AuthenticatedBrokerSession,
    intent: &crate::benchmark_route::ReplayIntent,
    idempotency_key: String,
) -> Result<crate::control_codec::RecordingReplayDispatch, RuntimeError> {
    session
        .dispatch_replay_intent(intent, idempotency_key)
        .map_err(|error| RuntimeError(io::Error::other(error)))
}

/// Complete the selection-driven replay route: bind the user's workload and
/// cassette choice to the authenticated catalog, then dispatch only the
/// resulting strict offline intent.  A catalog, generation, or identity
/// mismatch fails before any runner request is sent.
pub fn run_authenticated_replay_selection(
    session: &mut AuthenticatedBrokerSession,
    campaign: &mut crate::benchmark_route::GuidedCampaign,
    catalog: &crate::benchmark_route::AuthenticatedCassetteCatalog,
    workload_id: &str,
    cassette_sha256: &str,
    idempotency_key: String,
) -> Result<crate::control_codec::RecordingReplayDispatch, RuntimeError> {
    let intent = campaign
        .start_offline_replay_from_authenticated_catalog(catalog, workload_id, cassette_sha256)
        .map_err(|error| {
            RuntimeError(io::Error::other(format!(
                "replay selection rejected: {error:?}"
            )))
        })?;
    dispatch_authenticated_replay(session, &intent, idempotency_key)
}

/// Run the interactive loop after one authenticated control refresh. The
/// mutable session is borrowed by the entry seam for the full loop lifetime;
/// it is never replaced by a second connection and remains available to a
/// future periodic refresh scheduler.
pub fn run_interactive_with_control(
    state: &mut AppState,
    policy: RenderPolicy,
    session: &mut AuthenticatedBrokerSession,
) -> Result<(), RuntimeError> {
    run_interactive_with_control_context(state, policy, session, false)
}

pub fn run_interactive_with_control_context(
    state: &mut AppState,
    policy: RenderPolicy,
    session: &mut AuthenticatedBrokerSession,
    development_mode: bool,
) -> Result<(), RuntimeError> {
    let mut projection = ControlProjection::default();
    let mut workspace = ui::WorkspaceState::default();
    session
        .poll_projection_with_context_for_runtime(&mut projection, development_mode)
        .map_err(|error| RuntimeError(io::Error::other(error)))?;
    workspace.apply_live_snapshot(projection.snapshot());
    if development_mode {
        workspace.use_development_context();
    }
    run_interactive_loop(
        state,
        policy,
        workspace,
        Some(session),
        Some(&mut projection),
        development_mode,
    )
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
    run_interactive_loop(
        state,
        policy,
        ui::WorkspaceState::default(),
        None,
        None,
        false,
    )
}

/// Run the interactive frontend after one injected, normalized readiness
/// observation. The provider owns probing; this runtime only selects the
/// initial route and never persists or contacts ASB for readiness.
pub fn run_interactive_with_readiness<P: ReadinessProvider>(
    state: &mut AppState,
    policy: RenderPolicy,
    provider: &mut P,
) -> Result<(), RuntimeError> {
    let workspace = ui::WorkspaceState::for_readiness(provider.read());
    run_interactive_loop(state, policy, workspace, None, None, false)
}

fn run_interactive_loop(
    state: &mut AppState,
    policy: RenderPolicy,
    mut workspace: ui::WorkspaceState,
    mut control: Option<&mut AuthenticatedBrokerSession>,
    mut projection: Option<&mut ControlProjection>,
    development_mode: bool,
) -> Result<(), RuntimeError> {
    let mut signals = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP, SIGCONT])?;
    let mut session = TerminalSession::enter(policy)?;
    let backend = BoundedBackend(CrosstermBackend::new(io::stdout()));
    let mut terminal = Terminal::new(backend)?;
    let mut recording_state: Option<crate::recording_dispatch::RecordingDispatchState> = None;
    let mut active_adapter_id: Option<String> = None;
    let mut development_worker: Option<(
        Arc<AtomicBool>,
        Receiver<crate::development_lifecycle::Response>,
    )> = None;
    let mut next_live_refresh = Instant::now();
    while !state.should_quit() {
        if let Some((_, receiver)) = development_worker.as_ref()
            && let Ok(response) = receiver.try_recv()
        {
            workspace.development_handoff.apply_response(&response);
            development_worker = None;
        }
        handle_signals(&mut signals, &mut session, &mut terminal, policy)?;
        if let (Some(control), Some(projection)) =
            (control.as_deref_mut(), projection.as_deref_mut())
            && workspace.launch_state_mut().is_some()
            && Instant::now() >= next_live_refresh
        {
            // Refresh only while a reviewed launch exists. Every response is
            // projected through the negotiated transport and stale revisions
            // are rejected; a refresh never restarts a run.
            if control
                .poll_projection_with_context_for_runtime(projection, development_mode)
                .is_err()
            {
                if let Some(launch) = workspace.launch_state_mut() {
                    launch.disconnected();
                    launch.reconnect_started();
                }
                next_live_refresh = Instant::now() + Duration::from_millis(500);
                continue;
            }
            let snapshot = projection.snapshot();
            if let Some(launch) = workspace.launch_state_mut()
                && let Some(run) = snapshot.runs.first()
            {
                let _ = launch.observe_summary(run);
            }
            workspace.apply_live_snapshot(snapshot);
            next_live_refresh = Instant::now() + Duration::from_millis(500);
        }
        terminal.draw(|frame| ui::render(frame, &workspace, policy))?;
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Resize(columns, lines) if frame_dimensions_are_safe(columns, lines) => {
                    state.apply(Action::Resize { columns, lines })?;
                    // Keep presentation focus bounded while retaining the
                    // active route, search query, and help overlay.
                    workspace.apply_resize(columns, lines);
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let ui_action = workspace.handle_key(key);
                    if matches!(ui_action, ui::UiAction::Quit) {
                        state.apply(Action::Quit)?;
                    }
                    if let ui::UiAction::Control(control_action) = ui_action
                        && matches!(
                            control_action,
                            crate::actions::UiAction::MaterializeDevelopment
                                | crate::actions::UiAction::RetryDevelopment
                        )
                    {
                        if development_mode {
                            let operation = workspace.development_handoff.operation();
                            let cancelled = Arc::new(AtomicBool::new(false));
                            let worker_cancelled = Arc::clone(&cancelled);
                            let (sender, receiver) = mpsc::channel();
                            std::thread::spawn(move || {
                                let response = crate::development_lifecycle::execute_with_cancel(
                                    operation,
                                    &worker_cancelled,
                                );
                                let _ = sender.send(response);
                            });
                            development_worker = Some((cancelled, receiver));
                        } else {
                            workspace.development_handoff.phase =
                                crate::development_handoff::Phase::Failed;
                            workspace.development_handoff.code =
                                "development_profile_required".into();
                        }
                        continue;
                    }
                    if matches!(
                        ui_action,
                        ui::UiAction::Control(crate::actions::UiAction::CancelDevelopment)
                    ) {
                        if let Some((cancelled, _)) = development_worker.as_ref() {
                            cancelled.store(true, Ordering::Relaxed);
                        }
                        workspace.development_handoff.cancel();
                        continue;
                    }
                    if let ui::UiAction::Control(control_action) = ui_action
                        && let (Some(control), Some(projection)) =
                            (control.as_deref_mut(), projection.as_deref_mut())
                    {
                        if control_action == crate::actions::UiAction::StartRun {
                            let bundle = workspace.preflight_bundle().ok_or_else(|| {
                                RuntimeError(io::Error::other(
                                    "reviewed preflight is required before launch",
                                ))
                            })?;
                            control
                                .launch_materialized(projection, bundle, "asb-tui-launch".into())
                                .map_err(|error| RuntimeError(io::Error::other(error)))?;
                            let snapshot = projection.snapshot();
                            if let Some(launch) = workspace.launch_state_mut()
                                && let Some(run) = snapshot.runs.first()
                            {
                                let _ = launch.observe_summary(run);
                            }
                            workspace.apply_live_snapshot(snapshot);
                            continue;
                        }
                        if control_action == crate::actions::UiAction::CancelRun {
                            if let Some(launch) = workspace.launch_state_mut() {
                                let _ = launch.request_cancel();
                            }
                            let launch = workspace.launch_state().cloned().ok_or_else(|| {
                                RuntimeError(io::Error::other("active launch is unavailable"))
                            })?;
                            control
                                .cancel_active_run(projection, &launch, "asb-tui-cancel".into())
                                .map_err(|error| RuntimeError(io::Error::other(error)))?;
                            let snapshot = projection.snapshot();
                            if let Some(launch) = workspace.launch_state_mut()
                                && let Some(run) = snapshot.runs.first()
                            {
                                let _ = launch.observe_summary(run);
                            }
                            workspace.apply_live_snapshot(snapshot);
                            continue;
                        }
                        if control_action == crate::actions::UiAction::Reconnect {
                            if control
                                .poll_projection_with_context_for_runtime(
                                    projection,
                                    development_mode,
                                )
                                .is_err()
                            {
                                if let Some(launch) = workspace.launch_state_mut() {
                                    launch.disconnected();
                                    launch.reconnect_started();
                                }
                                continue;
                            }
                            let snapshot = projection.snapshot();
                            if let Some(launch) = workspace.launch_state_mut() {
                                launch.reconnect_complete(snapshot.latest_revision);
                            }
                            workspace.apply_live_snapshot(snapshot);
                            continue;
                        }
                        if matches!(
                            control_action,
                            crate::actions::UiAction::OpenPreflight
                                | crate::actions::UiAction::ApplyPreflight
                        ) {
                            // These are local, renderer-owned configuration
                            // actions. They must not enter the ASB recording
                            // dispatcher, which only accepts backend calls.
                            continue;
                        }
                        if recording_state.is_none()
                            && let Some(configuration) = workspace
                                .live
                                .as_ref()
                                .and_then(|live| live.configuration.as_ref())
                        {
                            recording_state =
                                crate::recording_dispatch::RecordingDispatchState::new(
                                    configuration.provider_id.clone().unwrap_or_default(),
                                    configuration.model_id.clone().unwrap_or_default(),
                                    configuration.agent_ids.clone(),
                                    crate::recording_campaign::WorkloadScope::All,
                                )
                                .ok();
                            if let (Some(adapter_id), Some(recording)) =
                                (active_adapter_id.as_deref(), recording_state.as_mut())
                            {
                                recording
                                    .bind_adapter_id(adapter_id)
                                    .map_err(io::Error::other)
                                    .map_err(RuntimeError)?;
                            }
                        }
                        if let Some(recording) = recording_state.as_mut() {
                            let (selected_run, comparison_runs) =
                                workspace.operator_run_selection();
                            recording.selected_run_id = selected_run;
                            recording.comparison_run_ids = comparison_runs;
                            dispatch_control_action(
                                control_action,
                                recording,
                                control,
                                projection,
                                format!("asb-tui-{}", control_action.id()),
                            )?;
                            workspace.apply_live_snapshot(projection.snapshot());
                        }
                    }
                    if let (Some(control), Some(projection), Some(values)) = (
                        control.as_deref_mut(),
                        projection.as_deref_mut(),
                        workspace.take_wizard_completion(),
                    ) {
                        active_adapter_id = workspace.take_wizard_adapter_completion();
                        if let (Some(adapter_id), Some(recording)) =
                            (active_adapter_id.as_deref(), recording_state.as_mut())
                        {
                            recording
                                .bind_adapter_id(adapter_id)
                                .map_err(io::Error::other)
                                .map_err(RuntimeError)?;
                        }
                        // A negotiated catalog is authoritative: bind the
                        // completed wizard to it before any configuration
                        // mutation. The disconnected development fixture keeps
                        // the bounded legacy parser because it has no wire
                        // catalog to validate against.
                        let snapshot = projection.snapshot();
                        let draft = match (
                            snapshot.agent_catalog.as_ref(),
                            snapshot.provider_catalog.as_ref(),
                        ) {
                            (Some(agents), Some(providers)) => {
                                ui::WorkspaceState::wizard_provider_setup_draft(
                                    &values, agents, providers,
                                )
                                .map_err(|error| {
                                    RuntimeError(io::Error::other(format!(
                                        "provider setup draft rejected: {error:?}"
                                    )))
                                })?
                            }
                            (None, None) => {
                                // Only a completely disconnected development
                                // fixture may use the legacy bounded parser.
                                let selection =
                                    ui::WorkspaceState::wizard_configuration_selection(&values)
                                        .map_err(|reason| RuntimeError(io::Error::other(reason)))?;
                                crate::provider_setup::ProviderSetupDraft::from_selection_for_development(selection)
                                    .map_err(|error| RuntimeError(io::Error::other(format!("provider setup draft rejected: {error:?}"))))?
                            }
                            _ => {
                                return Err(RuntimeError(io::Error::other(
                                    "provider setup catalogs are incomplete",
                                )));
                            }
                        };
                        let current_generation = snapshot
                            .provider_catalog
                            .as_ref()
                            .map_or(crate::control_codec::Revision(1), |catalog| {
                                catalog.generation
                            });
                        let selection = draft.selection().clone();
                        if let Some(receipt) =
                            ui::WorkspaceState::wizard_credential_helper_receipt(&values)
                                .map_err(|reason| RuntimeError(io::Error::other(reason)))?
                        {
                            control
                                .enroll_auth_receipt(
                                    projection,
                                    receipt,
                                    format!("asb-tui-auth-{}", selection.provider_id),
                                )
                                .map_err(|error| RuntimeError(io::Error::other(error)))?;
                            control
                                .auth_status(projection, selection.provider_id.clone())
                                .map_err(|error| RuntimeError(io::Error::other(error)))?;
                        }
                        workspace
                            .provider_setup_apply
                            .apply(draft, current_generation, |selection| {
                                control
                                    .apply_configuration(projection, selection.clone())
                                    .map_err(|error| error.to_string())
                            })
                            .map_err(|error| {
                                RuntimeError(io::Error::other(format!(
                                    "provider setup apply rejected: {error:?}"
                                )))
                            })?;
                        workspace.apply_live_snapshot(projection.snapshot());
                    }
                }
                _ => {}
            }
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

    #[test]
    fn replay_route_requires_explicit_catalog_selection_and_binds_all_identities() {
        let digest = "a".repeat(64);
        let catalog = crate::benchmark_route::AuthenticatedCassetteCatalog {
            runner_instance_id: "runner-1".into(),
            generation: crate::control_codec::Revision(7),
            campaign_id: "campaign-1".into(),
            entries: vec![crate::benchmark_route::AuthenticatedCassetteEntry {
                cassette_id: "cassette-1".into(),
                cassette_sha256: digest.clone(),
                provider_profile_sha256: "b".repeat(64),
                agent_id: "agent-1".into(),
                workload_id: "workload-1".into(),
                scorer_revision: "scorer-1".into(),
            }],
        };
        assert!(prepare_authenticated_replay_campaign(&catalog, &"c".repeat(64)).is_err());
        let mut campaign = prepare_authenticated_replay_campaign(&catalog, &digest).unwrap();
        let intent = campaign
            .start_offline_replay_from_authenticated_catalog(&catalog, "workload-1", &digest)
            .unwrap();
        assert_eq!(intent.campaign_id.as_deref(), Some("campaign-1"));
        assert_eq!(intent.generation, Some(crate::control_codec::Revision(7)));
        assert_eq!(intent.cassette_sha256.as_deref(), Some(digest.as_str()));
    }

    #[test]
    fn cassette_selector_cycles_only_authenticated_entries() {
        let entry = |id: &str, digest: &str| crate::benchmark_route::AuthenticatedCassetteEntry {
            cassette_id: id.into(),
            cassette_sha256: digest.into(),
            provider_profile_sha256: "b".repeat(64),
            agent_id: "agent-1".into(),
            workload_id: "workload-1".into(),
            scorer_revision: "scorer-1".into(),
        };
        let catalog = crate::benchmark_route::AuthenticatedCassetteCatalog {
            runner_instance_id: "runner-1".into(),
            generation: crate::control_codec::Revision(7),
            campaign_id: "campaign-1".into(),
            entries: vec![
                entry("cassette-1", &"a".repeat(64)),
                entry("cassette-2", &"c".repeat(64)),
            ],
        };
        assert_eq!(
            select_next_authenticated_cassette(&catalog, None).unwrap(),
            "a".repeat(64)
        );
        assert_eq!(
            select_next_authenticated_cassette(&catalog, Some(&"a".repeat(64))).unwrap(),
            "c".repeat(64)
        );
        assert_eq!(
            select_next_authenticated_cassette(&catalog, Some(&"c".repeat(64))).unwrap(),
            "a".repeat(64)
        );
        let empty = crate::benchmark_route::AuthenticatedCassetteCatalog {
            entries: Vec::new(),
            ..catalog
        };
        assert!(select_next_authenticated_cassette(&empty, None).is_err());
    }

    #[test]
    fn select_cassette_action_updates_dispatch_state_without_transport_io() {
        use std::os::unix::net::UnixStream;
        let (stream, _peer) = UnixStream::pair().unwrap();
        let mut session = AuthenticatedBrokerSession::test_session(stream).unwrap();
        let mut recording = crate::recording_dispatch::RecordingDispatchState::new(
            "provider".into(),
            "model".into(),
            vec!["agent-1".into()],
            crate::recording_campaign::WorkloadScope::All,
        )
        .unwrap();
        recording.authenticated_catalog =
            Some(crate::benchmark_route::AuthenticatedCassetteCatalog {
                runner_instance_id: "runner-1".into(),
                generation: crate::control_codec::Revision(7),
                campaign_id: "campaign-1".into(),
                entries: vec![crate::benchmark_route::AuthenticatedCassetteEntry {
                    cassette_id: "cassette-1".into(),
                    cassette_sha256: "a".repeat(64),
                    provider_profile_sha256: "b".repeat(64),
                    agent_id: "agent-1".into(),
                    workload_id: "workload-1".into(),
                    scorer_revision: "scorer-1".into(),
                }],
            });
        let mut projection = ControlProjection::default();
        let outcome = dispatch_control_action(
            crate::actions::UiAction::SelectOfflineCassette,
            &mut recording,
            &mut session,
            &mut projection,
            "select-1".into(),
        )
        .unwrap();
        assert_eq!(
            outcome,
            crate::recording_dispatch::RecordingDispatchOutcome::CassetteSelected
        );
        assert_eq!(
            recording.selected_cassette_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
    }

    #[test]
    fn replay_action_rejects_missing_or_stale_selection_before_transport() {
        use std::os::unix::net::UnixStream;
        let (stream, _peer) = UnixStream::pair().unwrap();
        let mut session = AuthenticatedBrokerSession::test_session(stream).unwrap();
        let mut projection = ControlProjection::default();
        let mut recording = crate::recording_dispatch::RecordingDispatchState::new(
            "provider".into(),
            "model".into(),
            vec!["agent-1".into()],
            crate::recording_campaign::WorkloadScope::All,
        )
        .unwrap();
        assert!(
            dispatch_control_action(
                crate::actions::UiAction::ReplaySelected,
                &mut recording,
                &mut session,
                &mut projection,
                "replay-1".into(),
            )
            .is_err()
        );
        recording.authenticated_catalog =
            Some(crate::benchmark_route::AuthenticatedCassetteCatalog {
                runner_instance_id: "runner-1".into(),
                generation: crate::control_codec::Revision(7),
                campaign_id: "campaign-1".into(),
                entries: Vec::new(),
            });
        recording.selected_cassette_sha256 = Some("a".repeat(64));
        assert!(
            dispatch_control_action(
                crate::actions::UiAction::ReplaySelected,
                &mut recording,
                &mut session,
                &mut projection,
                "replay-2".into(),
            )
            .is_err()
        );
    }

    #[test]
    fn replay_action_dispatches_authenticated_offline_result() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || {
            let mut header = [0_u8; 4];
            peer.read_exact(&mut header).unwrap();
            let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
            peer.read_exact(&mut body).unwrap();
            let request: crate::control_codec::ControlRequest =
                serde_json::from_slice(&body).unwrap();
            assert!(matches!(
                request.call,
                crate::control_codec::ControlCall::RecordingReplayDispatch(_)
            ));
            let response = crate::control_codec::ControlResponse::Success(
                crate::control_codec::SuccessResponse {
                    jsonrpc: crate::control_codec::JSONRPC_VERSION.into(),
                    id: request.id,
                    result: crate::control_codec::ControlSuccess::Operation(
                        crate::control_codec::BoundResult {
                            request_sha256: "c".repeat(64),
                            result: crate::control_codec::ControlResult::RecordingReplayDispatch(
                                crate::control_codec::RecordingReplayDispatch {
                                    runner_instance_id: "runner-1".into(),
                                    generation: crate::control_codec::Revision(7),
                                    campaign_id: "campaign-1".into(),
                                    provider_profile_sha256: "b".repeat(64),
                                    agent_id: "agent-1".into(),
                                    workload_id: "workload-1".into(),
                                    cassette_sha256: "a".repeat(64),
                                    offline_only: true,
                                },
                            ),
                        },
                    ),
                },
            );
            peer.write_all(&crate::control_codec::encode(&response, 64 * 1024).unwrap())
                .unwrap();
        });
        let mut session = AuthenticatedBrokerSession::test_session(stream).unwrap();
        let mut recording = crate::recording_dispatch::RecordingDispatchState::new(
            "provider".into(),
            "model".into(),
            vec!["agent-1".into()],
            crate::recording_campaign::WorkloadScope::All,
        )
        .unwrap();
        recording.authenticated_catalog =
            Some(crate::benchmark_route::AuthenticatedCassetteCatalog {
                runner_instance_id: "runner-1".into(),
                generation: crate::control_codec::Revision(7),
                campaign_id: "campaign-1".into(),
                entries: vec![crate::benchmark_route::AuthenticatedCassetteEntry {
                    cassette_id: "cassette-1".into(),
                    cassette_sha256: "a".repeat(64),
                    provider_profile_sha256: "b".repeat(64),
                    agent_id: "agent-1".into(),
                    workload_id: "workload-1".into(),
                    scorer_revision: "scorer-1".into(),
                }],
            });
        recording.selected_cassette_sha256 = Some("a".repeat(64));
        let mut projection = ControlProjection::default();
        let outcome = dispatch_control_action(
            crate::actions::UiAction::ReplaySelected,
            &mut recording,
            &mut session,
            &mut projection,
            "replay-3".into(),
        )
        .unwrap();
        assert_eq!(
            outcome,
            crate::recording_dispatch::RecordingDispatchOutcome::ReplayDispatched
        );
        server.join().unwrap();
    }
}
