// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic single-writer frontend state and renderer-neutral frame model.

use crate::terminal::{LayoutClass, ResponsiveLayout, frame_dimensions_are_safe};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Privacy-reviewed control event kinds accepted by the frontend projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlEventKind {
    /// The external runner became ready.
    RunnerReady,
    /// A plan was durably created.
    PlanCreated,
    /// A run started.
    RunStarted,
    /// A run projection changed.
    RunUpdated,
    /// A run completed successfully.
    RunCompleted,
    /// A run failed.
    RunFailed,
    /// A run was cancelled.
    RunCancelled,
    /// The runner requires explicit reconciliation.
    ReconciliationRequired,
}

/// Minimal external control projection. It contains no benchmark payload or credential data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEvent {
    /// Monotonic durable event revision.
    pub revision: u64,
    /// Privacy-reviewed event kind.
    pub kind: ControlEventKind,
}

/// Input accepted by the single state writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    /// The terminal was resized to these authoritative dimensions.
    Resize {
        /// Current terminal width.
        columns: u16,
        /// Current terminal height.
        lines: u16,
    },
    /// A privacy-reviewed event arrived from the negotiated control connection.
    Control(ControlEvent),
    /// The connection negotiated an authoritative event baseline.
    Connected {
        /// Latest durable runner revision observed during negotiation.
        baseline: u64,
    },
    /// The control connection closed; runner ownership is unaffected.
    Disconnected,
    /// The operator requested frontend shutdown.
    Quit,
}

/// Fail-closed application-state update error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppError {
    /// A resize carried invalid terminal dimensions.
    InvalidDimensions,
    /// An event was duplicated, stale, skipped, or overflowed a durable revision.
    InvalidEventOrder,
    /// A control event arrived without an active negotiated connection.
    ControlUnavailable,
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDimensions => "invalid terminal dimensions",
            Self::InvalidEventOrder => "control event revision is not contiguous",
            Self::ControlUnavailable => "control connection is unavailable",
        })
    }
}

impl std::error::Error for AppError {}

/// Complete renderer-owned state. Runner and benchmark state remain external.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppState {
    layout: ResponsiveLayout,
    connected: bool,
    quit: bool,
    last_revision: Option<u64>,
    last_event: Option<ControlEventKind>,
}

impl AppState {
    /// Create a disconnected frontend projection for the current terminal size.
    pub fn new(columns: u16, lines: u16) -> Result<Self, AppError> {
        if !frame_dimensions_are_safe(columns, lines) {
            return Err(AppError::InvalidDimensions);
        }
        Ok(Self {
            layout: ResponsiveLayout::from_dimensions(Some(columns), Some(lines)),
            connected: false,
            quit: false,
            last_revision: None,
            last_event: None,
        })
    }

    /// Apply one action atomically. Invalid actions leave state unchanged.
    pub fn apply(&mut self, action: Action) -> Result<(), AppError> {
        match action {
            Action::Resize { columns, lines } => {
                if !frame_dimensions_are_safe(columns, lines) {
                    return Err(AppError::InvalidDimensions);
                }
                self.layout = ResponsiveLayout::from_dimensions(Some(columns), Some(lines));
            }
            Action::Control(event) => {
                if !self.connected {
                    return Err(AppError::ControlUnavailable);
                }
                let expected = self
                    .last_revision
                    .map_or(Some(1), |last| last.checked_add(1));
                if expected != Some(event.revision) {
                    return Err(AppError::InvalidEventOrder);
                }
                self.last_revision = Some(event.revision);
                self.last_event = Some(event.kind);
            }
            Action::Connected { baseline } => {
                if self.last_revision.is_some_and(|known| baseline < known) {
                    return Err(AppError::InvalidEventOrder);
                }
                if self.last_revision.is_some_and(|known| baseline > known) {
                    self.last_event = None;
                }
                self.last_revision = Some(baseline);
                self.connected = true;
            }
            Action::Disconnected => self.connected = false,
            Action::Quit => self.quit = true,
        }
        Ok(())
    }

    /// Whether the frontend event loop should restore the terminal and stop.
    pub const fn should_quit(&self) -> bool {
        self.quit
    }

    /// Create the immutable, renderer-neutral content for one frame.
    pub fn frame_model(&self) -> FrameModel {
        FrameModel {
            title: "Agent Systems Benchmark",
            connection: if self.connected {
                "connected"
            } else {
                "disconnected"
            },
            last_event: self.last_event.map_or("none", event_name),
            layout: match self.layout.class {
                LayoutClass::Compact => "compact",
                LayoutClass::Standard => "standard",
                LayoutClass::Wide => "wide",
            },
            ownership: "runner ownership remains external",
        }
    }

    /// Stable plain-text equivalent for pipes and unknown terminals.
    pub fn plain_text(&self) -> String {
        let frame = self.frame_model();
        format!(
            "{}\nconnection: {}\nlast event: {}\n{}\n",
            frame.title, frame.connection, frame.last_event, frame.ownership
        )
    }
}

/// Immutable content projected for a renderer. No renderer may mutate application state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameModel {
    /// Product title.
    pub title: &'static str,
    /// Closed connection label.
    pub connection: &'static str,
    /// Closed event-kind label.
    pub last_event: &'static str,
    /// Closed responsive-layout label.
    pub layout: &'static str,
    /// Explicit runner ownership boundary.
    pub ownership: &'static str,
}

fn event_name(kind: ControlEventKind) -> &'static str {
    match kind {
        ControlEventKind::RunnerReady => "runner_ready",
        ControlEventKind::PlanCreated => "plan_created",
        ControlEventKind::RunStarted => "run_started",
        ControlEventKind::RunUpdated => "run_updated",
        ControlEventKind::RunCompleted => "run_completed",
        ControlEventKind::RunFailed => "run_failed",
        ControlEventKind::RunCancelled => "run_cancelled",
        ControlEventKind::ReconciliationRequired => "reconciliation_required",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(revision: u64, kind: ControlEventKind) -> ControlEvent {
        ControlEvent { revision, kind }
    }

    #[test]
    fn actions_are_deterministic_and_invalid_events_are_atomic() {
        let mut state = AppState::new(120, 40).unwrap();
        state.apply(Action::Connected { baseline: 0 }).unwrap();
        state
            .apply(Action::Control(event(1, ControlEventKind::RunnerReady)))
            .unwrap();
        let before = state.clone();
        assert_eq!(
            state.apply(Action::Control(event(3, ControlEventKind::RunStarted))),
            Err(AppError::InvalidEventOrder)
        );
        assert_eq!(state, before);
        assert_eq!(
            state.apply(Action::Resize {
                columns: 4_096,
                lines: 4_096,
            }),
            Err(AppError::InvalidDimensions)
        );
        assert_eq!(state, before);
        assert_eq!(state.frame_model().last_event, "runner_ready");
        assert!(
            state
                .plain_text()
                .contains("runner ownership remains external")
        );
    }

    #[test]
    fn frame_model_is_stable_bounded_and_renderer_neutral() {
        let state = AppState::new(48, 12).unwrap();
        assert_eq!(
            state.frame_model(),
            FrameModel {
                title: "Agent Systems Benchmark",
                connection: "disconnected",
                last_event: "none",
                layout: "standard",
                ownership: "runner ownership remains external",
            }
        );
        assert!(!state.plain_text().contains('/'));
    }

    #[test]
    fn resize_and_quit_are_explicit() {
        let mut state = AppState::new(120, 40).unwrap();
        state
            .apply(Action::Resize {
                columns: 20,
                lines: 6,
            })
            .unwrap();
        assert_eq!(state.frame_model().layout, "compact");
        let before = state.clone();
        assert_eq!(
            state.apply(Action::Resize {
                columns: 0,
                lines: 6,
            }),
            Err(AppError::InvalidDimensions)
        );
        assert_eq!(state, before);
        state.apply(Action::Quit).unwrap();
        assert!(state.should_quit());
    }

    #[test]
    fn maximum_revision_fails_closed_without_wrapping_or_mutation() {
        let mut state = AppState::new(80, 24).unwrap();
        state
            .apply(Action::Connected {
                baseline: u64::MAX - 1,
            })
            .unwrap();
        state
            .apply(Action::Control(event(
                u64::MAX,
                ControlEventKind::RunnerReady,
            )))
            .unwrap();
        let before = state.clone();
        assert_eq!(
            state.apply(Action::Control(event(0, ControlEventKind::RunStarted))),
            Err(AppError::InvalidEventOrder)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn first_event_requires_one_or_the_negotiated_baseline_successor() {
        let mut standalone = AppState::new(80, 24).unwrap();
        let untouched = standalone.clone();
        assert_eq!(
            standalone.apply(Action::Control(event(1, ControlEventKind::RunnerReady))),
            Err(AppError::ControlUnavailable)
        );
        assert_eq!(standalone, untouched);

        let mut negotiated = AppState::new(80, 24).unwrap();
        negotiated
            .apply(Action::Connected { baseline: 90 })
            .unwrap();
        let baseline = negotiated.clone();
        assert_eq!(
            negotiated.apply(Action::Control(event(90, ControlEventKind::RunnerReady))),
            Err(AppError::InvalidEventOrder)
        );
        assert_eq!(negotiated, baseline);
        negotiated
            .apply(Action::Control(event(91, ControlEventKind::RunnerReady)))
            .unwrap();
    }

    #[test]
    fn disconnected_events_and_advanced_baselines_are_atomic_and_not_stale() {
        let mut state = AppState::new(80, 24).unwrap();
        state.apply(Action::Connected { baseline: 0 }).unwrap();
        state
            .apply(Action::Control(event(1, ControlEventKind::RunnerReady)))
            .unwrap();
        state.apply(Action::Disconnected).unwrap();
        let disconnected = state.clone();
        assert_eq!(
            state.apply(Action::Control(event(2, ControlEventKind::RunStarted))),
            Err(AppError::ControlUnavailable)
        );
        assert_eq!(state, disconnected);

        state.apply(Action::Connected { baseline: 4 }).unwrap();
        assert_eq!(state.frame_model().last_event, "none");
        assert_eq!(state.frame_model().connection, "connected");
        state
            .apply(Action::Control(event(5, ControlEventKind::RunUpdated)))
            .unwrap();
        let current = state.clone();
        state.apply(Action::Connected { baseline: 5 }).unwrap();
        assert_eq!(
            state, current,
            "equal baseline preserves current projection"
        );
    }

    #[test]
    fn every_event_label_and_connection_transition_is_closed() {
        let kinds = [
            (ControlEventKind::RunnerReady, "runner_ready"),
            (ControlEventKind::PlanCreated, "plan_created"),
            (ControlEventKind::RunStarted, "run_started"),
            (ControlEventKind::RunUpdated, "run_updated"),
            (ControlEventKind::RunCompleted, "run_completed"),
            (ControlEventKind::RunFailed, "run_failed"),
            (ControlEventKind::RunCancelled, "run_cancelled"),
            (
                ControlEventKind::ReconciliationRequired,
                "reconciliation_required",
            ),
        ];
        let mut state = AppState::new(160, 50).unwrap();
        state.apply(Action::Connected { baseline: 0 }).unwrap();
        assert_eq!(state.frame_model().connection, "connected");
        for (index, (kind, label)) in kinds.into_iter().enumerate() {
            state
                .apply(Action::Control(event(index as u64 + 1, kind)))
                .unwrap();
            assert_eq!(state.frame_model().last_event, label);
        }
        let before = state.clone();
        assert_eq!(
            state.apply(Action::Connected { baseline: 7 }),
            Err(AppError::InvalidEventOrder)
        );
        assert_eq!(state, before);
        state.apply(Action::Disconnected).unwrap();
        assert_eq!(state.frame_model().connection, "disconnected");
        assert_eq!(state.frame_model().layout, "wide");
    }

    #[test]
    fn zero_dimensions_are_rejected_at_construction() {
        assert_eq!(AppState::new(0, 24), Err(AppError::InvalidDimensions));
        assert_eq!(AppState::new(80, 0), Err(AppError::InvalidDimensions));
        assert_eq!(
            AppState::new(4_096, 4_096),
            Err(AppError::InvalidDimensions)
        );
        assert_eq!(
            AppError::InvalidDimensions.to_string(),
            "invalid terminal dimensions"
        );
        assert_eq!(
            AppError::InvalidEventOrder.to_string(),
            "control event revision is not contiguous"
        );
        assert_eq!(
            AppError::ControlUnavailable.to_string(),
            "control connection is unavailable"
        );
    }
}
