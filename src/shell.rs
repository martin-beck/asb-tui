// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-independent application shell and route extension seam.
//!
//! The shell owns only frontend navigation and the projection of the external
//! runner connection.  Screens are deliberately not implemented here: the
//! downstream UX ARs provide their renderers and screen-specific state.

use crate::Capabilities;
use crate::app::{Action, AppError, AppState, ControlEvent};

/// Stable application destinations.  These names are a contract for screen
/// extensions, not an indication that a screen is implemented by this module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Landing,
    Configuration,
    MeasurementSelection,
    RunControl,
    RecentRuns,
    Reports,
    Help,
}

impl Route {
    /// Return the deterministic capability gate for this destination.
    #[must_use]
    pub fn availability(self, capabilities: Option<&Capabilities>) -> RouteAvailability {
        if matches!(self, Self::Landing | Self::Configuration | Self::Help) {
            return RouteAvailability::Available;
        }
        let Some(capabilities) = capabilities else {
            return RouteAvailability::Disabled(DisabledReason::NotNegotiated);
        };
        let required = match self {
            Self::Landing | Self::Configuration | Self::Help => None,
            Self::MeasurementSelection => {
                (!capabilities.planning).then_some(DisabledReason::Planning)
            }
            Self::RunControl => {
                if !capabilities.launch {
                    Some(DisabledReason::Launch)
                } else if !capabilities.cancel {
                    Some(DisabledReason::Cancel)
                } else if !capabilities.events {
                    Some(DisabledReason::Events)
                } else {
                    None
                }
            }
            Self::RecentRuns => (!capabilities.history).then_some(DisabledReason::History),
            Self::Reports => {
                if !capabilities.analysis {
                    Some(DisabledReason::Analysis)
                } else if !capabilities.history {
                    Some(DisabledReason::History)
                } else {
                    None
                }
            }
        };
        required.map_or(RouteAvailability::Available, RouteAvailability::Disabled)
    }

    /// Whether a negotiated runner can support this route's backend journey.
    /// Visual availability remains the responsibility of the screen layer.
    #[must_use]
    pub fn available(self, capabilities: &Capabilities) -> bool {
        matches!(
            self.availability(Some(capabilities)),
            RouteAvailability::Available
        )
    }
}

/// Stable reason a route is unavailable until a later negotiation or capability set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisabledReason {
    NotNegotiated,
    Analysis,
    Cancel,
    Events,
    History,
    Launch,
    Planning,
}

/// Closed route gate consumed by renderers and downstream screen registries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteAvailability {
    Available,
    Disabled(DisabledReason),
}

/// Connection state exposed to renderers without exposing endpoint details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Incompatible,
}

/// Inputs understood by the shell.  Screen-specific inputs remain downstream
/// extension events and are not interpreted by this foundation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellAction {
    Quit,
    Navigate(Route),
    Resize { columns: u16, lines: u16 },
    Connected { baseline: u64 },
    Control(ControlEvent),
    Disconnected,
    Reconnect,
}

/// Single-writer application shell.  It provides stable routing and lifecycle
/// seams while retaining runner ownership outside the frontend process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationShell {
    state: AppState,
    route: Route,
    connection: ConnectionState,
    capabilities: Option<Capabilities>,
}

impl ApplicationShell {
    /// Create a shell with a disconnected landing route.
    pub fn new(columns: u16, lines: u16) -> Result<Self, AppError> {
        Ok(Self {
            state: AppState::new(columns, lines)?,
            route: Route::Landing,
            connection: ConnectionState::Disconnected,
            capabilities: None,
        })
    }

    /// Apply one shell action atomically. Unsupported navigation does not
    /// alter the current route.
    pub fn apply(&mut self, action: ShellAction) -> Result<(), AppError> {
        match action {
            ShellAction::Quit => self.state.apply(Action::Quit),
            ShellAction::Resize { columns, lines } => {
                self.state.apply(Action::Resize { columns, lines })
            }
            ShellAction::Control(event) => self.state.apply(Action::Control(event)),
            ShellAction::Connected { baseline } => {
                self.state.apply(Action::Connected { baseline })?;
                self.connection = ConnectionState::Connected;
                Ok(())
            }
            ShellAction::Disconnected => {
                self.state.apply(Action::Disconnected)?;
                self.connection = ConnectionState::Disconnected;
                Ok(())
            }
            ShellAction::Reconnect => {
                self.state.apply(Action::Disconnected)?;
                self.connection = ConnectionState::Reconnecting;
                Ok(())
            }
            ShellAction::Navigate(route) => {
                if matches!(
                    route.availability(self.capabilities.as_ref()),
                    RouteAvailability::Available
                ) {
                    self.route = route;
                }
                Ok(())
            }
        }
    }

    /// Record a successfully negotiated capability set and return to the
    /// landing route if the current destination is no longer supported.
    pub fn set_capabilities(&mut self, capabilities: Capabilities) {
        if !self.route.available(&capabilities) {
            self.route = Route::Landing;
        }
        self.capabilities = Some(capabilities);
    }

    #[must_use]
    pub const fn route(&self) -> Route {
        self.route
    }

    #[must_use]
    pub const fn connection(&self) -> ConnectionState {
        self.connection
    }

    #[must_use]
    pub const fn state(&self) -> &AppState {
        &self.state
    }

    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.state.should_quit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_capabilities() -> Capabilities {
        Capabilities {
            analysis: true,
            artifacts: true,
            cancel: true,
            events: true,
            history: true,
            launch: true,
            planning: true,
            repeat: true,
        }
    }

    #[test]
    fn shell_starts_disconnected_on_landing_and_routes_after_negotiation() {
        let mut shell = ApplicationShell::new(80, 24).unwrap();
        assert_eq!(shell.route(), Route::Landing);
        assert_eq!(shell.connection(), ConnectionState::Disconnected);
        shell.set_capabilities(all_capabilities());
        shell.apply(ShellAction::Navigate(Route::Reports)).unwrap();
        assert_eq!(shell.route(), Route::Reports);
        shell.apply(ShellAction::Connected { baseline: 0 }).unwrap();
        assert_eq!(shell.connection(), ConnectionState::Connected);
    }

    #[test]
    fn unavailable_routes_are_not_selected_and_disconnect_is_reconnectable() {
        let mut shell = ApplicationShell::new(80, 24).unwrap();
        for route in [Route::Landing, Route::Configuration, Route::Help] {
            assert_eq!(route.availability(None), RouteAvailability::Available);
            shell.apply(ShellAction::Navigate(route)).unwrap();
            assert_eq!(shell.route(), route);
        }
        shell.apply(ShellAction::Navigate(Route::Landing)).unwrap();
        assert_eq!(
            Route::Reports.availability(None),
            RouteAvailability::Disabled(DisabledReason::NotNegotiated)
        );
        shell.set_capabilities(Capabilities {
            analysis: false,
            artifacts: false,
            cancel: false,
            events: false,
            history: false,
            launch: false,
            planning: false,
            repeat: false,
        });
        shell.apply(ShellAction::Navigate(Route::Reports)).unwrap();
        assert_eq!(shell.route(), Route::Landing);
        shell.apply(ShellAction::Reconnect).unwrap();
        assert_eq!(shell.connection(), ConnectionState::Reconnecting);
        assert_eq!(
            shell.apply(ShellAction::Control(ControlEvent {
                revision: 1,
                kind: crate::app::ControlEventKind::RunStarted,
            })),
            Err(AppError::ControlUnavailable)
        );
        shell.apply(ShellAction::Disconnected).unwrap();
        assert_eq!(shell.connection(), ConnectionState::Disconnected);
    }

    #[test]
    fn route_gates_report_first_missing_capability_deterministically() {
        let capabilities = Capabilities {
            analysis: false,
            artifacts: true,
            cancel: false,
            events: false,
            history: false,
            launch: false,
            planning: false,
            repeat: false,
        };
        assert_eq!(
            Route::MeasurementSelection.availability(Some(&capabilities)),
            RouteAvailability::Disabled(DisabledReason::Planning)
        );
        assert_eq!(
            Route::RunControl.availability(Some(&capabilities)),
            RouteAvailability::Disabled(DisabledReason::Launch)
        );
        assert_eq!(
            Route::Reports.availability(Some(&capabilities)),
            RouteAvailability::Disabled(DisabledReason::Analysis)
        );
    }
}
