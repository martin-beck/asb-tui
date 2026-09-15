// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral startup readiness classification.
//!
//! This module only classifies already-probed startup facts. It does not read
//! configuration, contact a broker, or invent a protocol handshake.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    Configured,
    Unconfigured,
    Incomplete,
    Unavailable,
    Malformed,
    Stale,
    Unauthorized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupRoute {
    Landing,
    Configuration,
    Retry,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupInput {
    pub configuration_present: bool,
    pub configuration_complete: bool,
    pub endpoint_available: bool,
    pub configuration_malformed: bool,
    pub configuration_stale: bool,
    pub authorized: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupDecision {
    readiness: Readiness,
    route: StartupRoute,
}

/// The result of observing one authoritative readiness snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupObservation {
    decision: StartupDecision,
    auto_open_wizard: bool,
}

impl StartupObservation {
    #[must_use]
    pub const fn decision(self) -> StartupDecision {
        self.decision
    }

    /// Whether this observation should open the first-run wizard.
    ///
    /// This is distinct from the route in [`StartupDecision`]: repeated
    /// observations of an unconfigured runner remain a configuration route,
    /// but must not reopen a wizard that the user already dismissed or
    /// completed during this controller lifetime.
    #[must_use]
    pub const fn auto_open_wizard(self) -> bool {
        self.auto_open_wizard
    }
}

/// Stateful, renderer-neutral startup observation controller.
///
/// The controller deliberately consumes only the already-normalized facts in
/// [`StartupInput`]. It does not contact ASB, persist credentials, or perform
/// navigation. A fresh controller represents a fresh process; persisted ASB
/// configuration remains the authority for the next process' observation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StartupController {
    wizard_opened: bool,
}

impl StartupController {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            wizard_opened: false,
        }
    }

    /// Classify a readiness observation and consume the one-shot wizard gate.
    #[must_use]
    pub fn observe(&mut self, input: StartupInput) -> StartupObservation {
        let decision = classify(input);
        let auto_open_wizard = decision.auto_opens_wizard() && !self.wizard_opened;
        if auto_open_wizard {
            self.wizard_opened = true;
        }
        StartupObservation {
            decision,
            auto_open_wizard,
        }
    }

    /// Whether this controller has already issued its automatic wizard route.
    #[must_use]
    pub const fn wizard_opened(self) -> bool {
        self.wizard_opened
    }
}

impl StartupDecision {
    #[must_use]
    pub const fn readiness(self) -> Readiness {
        self.readiness
    }

    #[must_use]
    pub const fn route(self) -> StartupRoute {
        self.route
    }

    /// Manual reconfigure is always a local route decision. Applying changes
    /// remains the responsibility of the configuration/control layers.
    #[must_use]
    pub const fn manual_reconfigure(self) -> StartupRoute {
        StartupRoute::Configuration
    }

    /// Whether startup should enter the first-run wizard automatically.
    ///
    /// Only an authoritative response that explicitly says configuration is
    /// absent or incomplete may open the wizard.  Every other readiness state
    /// remains recoverable without presenting an editor for unverified data.
    #[must_use]
    pub const fn auto_opens_wizard(self) -> bool {
        matches!(
            self.readiness,
            Readiness::Unconfigured | Readiness::Incomplete
        )
    }

    /// Stable, renderer-neutral explanation for the startup route.
    ///
    /// These messages are intentionally short and actionable.  They contain
    /// no endpoint, credential, or provider content and can therefore be
    /// rendered by any frontend without leaking sensitive state.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self.readiness {
            Readiness::Configured => "ASB is configured; opening the landing screen.",
            Readiness::Unconfigured => {
                "ASB has no usable configuration; open the setup wizard to continue."
            }
            Readiness::Incomplete => {
                "ASB configuration is incomplete; open the setup wizard to finish it."
            }
            Readiness::Unavailable => {
                "ASB readiness is unavailable; retry the connection before configuring."
            }
            Readiness::Malformed => {
                "ASB returned malformed readiness data; review the connection and retry."
            }
            Readiness::Stale => {
                "ASB readiness is stale; refresh the connection before configuring."
            }
            Readiness::Unauthorized => {
                "ASB rejected authorization; resolve access and retry the connection."
            }
        }
    }
}

#[must_use]
pub const fn classify(input: StartupInput) -> StartupDecision {
    let readiness = if input.configuration_malformed {
        Readiness::Malformed
    } else if !input.endpoint_available {
        Readiness::Unavailable
    } else if !input.configuration_present {
        Readiness::Unconfigured
    } else if !input.configuration_complete {
        Readiness::Incomplete
    } else if input.configuration_stale {
        Readiness::Stale
    } else if !input.authorized {
        Readiness::Unauthorized
    } else {
        Readiness::Configured
    };
    let route = match readiness {
        Readiness::Configured => StartupRoute::Landing,
        Readiness::Unconfigured | Readiness::Incomplete => StartupRoute::Configuration,
        // A stale generation must be re-observed/retried; opening the wizard
        // would risk presenting an obsolete configuration as editable state.
        Readiness::Unavailable | Readiness::Stale => StartupRoute::Retry,
        Readiness::Malformed | Readiness::Unauthorized => StartupRoute::Error,
    };
    StartupDecision { readiness, route }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: StartupInput = StartupInput {
        configuration_present: true,
        configuration_complete: true,
        endpoint_available: true,
        configuration_malformed: false,
        configuration_stale: false,
        authorized: true,
    };

    #[test]
    fn classifies_all_readiness_states_and_routes() {
        let cases = [
            (READY, Readiness::Configured, StartupRoute::Landing),
            (
                StartupInput {
                    configuration_present: false,
                    ..READY
                },
                Readiness::Unconfigured,
                StartupRoute::Configuration,
            ),
            (
                StartupInput {
                    configuration_complete: false,
                    ..READY
                },
                Readiness::Incomplete,
                StartupRoute::Configuration,
            ),
            (
                StartupInput {
                    endpoint_available: false,
                    ..READY
                },
                Readiness::Unavailable,
                StartupRoute::Retry,
            ),
            (
                StartupInput {
                    configuration_malformed: true,
                    ..READY
                },
                Readiness::Malformed,
                StartupRoute::Error,
            ),
            (
                StartupInput {
                    configuration_stale: true,
                    ..READY
                },
                Readiness::Stale,
                StartupRoute::Retry,
            ),
            (
                StartupInput {
                    authorized: false,
                    ..READY
                },
                Readiness::Unauthorized,
                StartupRoute::Error,
            ),
        ];
        for (input, readiness, route) in cases {
            let decision = classify(input);
            assert_eq!((decision.readiness(), decision.route()), (readiness, route));
        }
    }

    #[test]
    fn manual_reconfigure_is_available_without_side_effects() {
        assert_eq!(
            classify(READY).manual_reconfigure(),
            StartupRoute::Configuration
        );
        assert_eq!(
            classify(StartupInput {
                configuration_malformed: true,
                ..READY
            })
            .manual_reconfigure(),
            StartupRoute::Configuration
        );
    }

    #[test]
    fn only_explicitly_unconfigured_or_incomplete_readiness_opens_wizard() {
        let cases = [
            (READY, false),
            (
                StartupInput {
                    configuration_present: false,
                    ..READY
                },
                true,
            ),
            (
                StartupInput {
                    configuration_complete: false,
                    ..READY
                },
                true,
            ),
            (
                StartupInput {
                    endpoint_available: false,
                    ..READY
                },
                false,
            ),
            (
                StartupInput {
                    configuration_malformed: true,
                    ..READY
                },
                false,
            ),
            (
                StartupInput {
                    configuration_stale: true,
                    ..READY
                },
                false,
            ),
            (
                StartupInput {
                    authorized: false,
                    ..READY
                },
                false,
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(classify(input).auto_opens_wizard(), expected);
        }
    }

    #[test]
    fn startup_decision_is_deterministic_and_idempotent() {
        let inputs = [
            READY,
            StartupInput {
                configuration_present: false,
                ..READY
            },
            StartupInput {
                configuration_complete: false,
                ..READY
            },
            StartupInput {
                endpoint_available: false,
                ..READY
            },
            StartupInput {
                configuration_malformed: true,
                ..READY
            },
            StartupInput {
                configuration_stale: true,
                ..READY
            },
            StartupInput {
                authorized: false,
                ..READY
            },
        ];

        for input in inputs {
            let first = classify(input);
            let second = classify(input);
            assert_eq!(first, second);
            assert!(!first.explanation().is_empty());
            assert!(!first.explanation().contains("credential"));
        }
    }

    #[test]
    fn unsafe_readiness_never_claims_configured_or_opens_wizard() {
        let unsafe_inputs = [
            StartupInput {
                endpoint_available: false,
                ..READY
            },
            StartupInput {
                configuration_malformed: true,
                ..READY
            },
            StartupInput {
                configuration_stale: true,
                ..READY
            },
            StartupInput {
                authorized: false,
                ..READY
            },
        ];

        for input in unsafe_inputs {
            let decision = classify(input);
            assert_ne!(decision.readiness(), Readiness::Configured);
            assert!(!decision.auto_opens_wizard());
            assert_ne!(decision.route(), StartupRoute::Configuration);
        }
    }

    #[test]
    fn controller_opens_unconfigured_wizard_once_and_preserves_route_decision() {
        let mut controller = StartupController::new();
        let input = StartupInput {
            configuration_present: false,
            ..READY
        };

        let first = controller.observe(input);
        assert!(first.auto_open_wizard());
        assert_eq!(first.decision().route(), StartupRoute::Configuration);
        assert!(controller.wizard_opened());

        let repeated = controller.observe(input);
        assert!(!repeated.auto_open_wizard());
        assert_eq!(repeated.decision(), first.decision());
    }

    #[test]
    fn controller_never_auto_opens_unsafe_or_configured_readiness() {
        let mut controller = StartupController::new();
        for input in [
            READY,
            StartupInput {
                endpoint_available: false,
                ..READY
            },
            StartupInput {
                configuration_malformed: true,
                ..READY
            },
            StartupInput {
                configuration_stale: true,
                ..READY
            },
            StartupInput {
                authorized: false,
                ..READY
            },
        ] {
            assert!(!controller.observe(input).auto_open_wizard());
        }
        assert!(!controller.wizard_opened());
    }

    #[test]
    fn controller_recovers_from_unavailable_before_one_shot_unconfigured_route() {
        let mut controller = StartupController::new();
        assert!(
            !controller
                .observe(StartupInput {
                    endpoint_available: false,
                    ..READY
                })
                .auto_open_wizard()
        );
        assert!(
            controller
                .observe(StartupInput {
                    configuration_complete: false,
                    ..READY
                })
                .auto_open_wizard()
        );
        assert!(
            !controller
                .observe(StartupInput {
                    configuration_complete: false,
                    ..READY
                })
                .auto_open_wizard()
        );
    }
}
