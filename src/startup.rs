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
}
