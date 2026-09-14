// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic renderer-neutral shell and terminal fallback checks.

use asb_tui::{
    Capabilities,
    app::AppState,
    shell::{
        ApplicationShell, ConnectionState, DisabledReason, Route, RouteAvailability, ShellAction,
    },
    terminal::{CapabilityTier, RenderPolicy},
};

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
fn shell_resize_and_plain_fallback_are_deterministic() {
    let mut shell = ApplicationShell::new(80, 24).unwrap();
    shell.set_capabilities(all_capabilities());
    shell
        .apply(ShellAction::Navigate(Route::MeasurementSelection))
        .unwrap();
    shell
        .apply(ShellAction::Resize {
            columns: 12,
            lines: 2,
        })
        .unwrap();

    assert_eq!(shell.connection(), ConnectionState::Disconnected);
    assert_eq!(shell.state().frame_model().layout, "compact");
    assert_eq!(
        shell.state().plain_text(),
        concat!(
            "Agent Systems Benchmark\n",
            "connection: disconnected\n",
            "last event: none\n",
            "runner ownership remains external\n"
        )
    );
    let plain = RenderPolicy {
        tier: CapabilityTier::Plain,
        unicode: false,
        mouse: false,
        focus: false,
        bracketed_paste: false,
        synchronized_output: false,
        alternate_screen: false,
    };
    assert!(!plain.unicode && !plain.alternate_screen);
}

#[test]
fn shell_route_and_connection_projection_fail_closed() {
    let mut shell = ApplicationShell::new(80, 24).unwrap();
    for route in [Route::Landing, Route::Configuration, Route::Help] {
        assert_eq!(route.availability(None), RouteAvailability::Available);
        shell.apply(ShellAction::Navigate(route)).unwrap();
        assert_eq!(shell.route(), route);
    }
    assert_eq!(
        Route::Reports.availability(None),
        RouteAvailability::Disabled(DisabledReason::NotNegotiated)
    );
    shell.apply(ShellAction::Reconnect).unwrap();
    assert_eq!(shell.connection(), ConnectionState::Reconnecting);
    assert_eq!(shell.state().frame_model().connection, "disconnected");
    shell.apply(ShellAction::Disconnected).unwrap();
    assert_eq!(shell.connection(), ConnectionState::Disconnected);

    // Tiny/unknown terminal dimensions use the bounded 1x1 layout fallback
    // instead of allowing a renderer allocation or an inferred screen size.
    assert_eq!(AppState::new(1, 1).unwrap().frame_model().layout, "compact");
}
