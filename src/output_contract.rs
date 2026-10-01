// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Stable output for guided, non-interactive command entry points.
//!
//! The TUI owns interaction, while this contract gives installers, scripts,
//! and first-time users a predictable way to select a route.  It intentionally
//! contains only public state: credentials, endpoints, paths, and raw provider
//! errors never enter an [`Output`].

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Setup,
    Benchmark,
    Recording,
    Replay,
    Comparison,
    Status,
    Doctor,
    Upgrade,
    Remove,
}

impl Route {
    pub const ALL: [Self; 9] = [
        Self::Setup,
        Self::Benchmark,
        Self::Recording,
        Self::Replay,
        Self::Comparison,
        Self::Status,
        Self::Doctor,
        Self::Upgrade,
        Self::Remove,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "setup" | "wizard" => Self::Setup,
            "benchmark" | "run" => Self::Benchmark,
            "record" | "recording" => Self::Recording,
            "replay" => Self::Replay,
            "compare" | "comparison" => Self::Comparison,
            "status" => Self::Status,
            "doctor" => Self::Doctor,
            "upgrade" => Self::Upgrade,
            "remove" | "uninstall" => Self::Remove,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Benchmark => "benchmark",
            Self::Recording => "recording",
            Self::Replay => "replay",
            Self::Comparison => "comparison",
            Self::Status => "status",
            Self::Doctor => "doctor",
            Self::Upgrade => "upgrade",
            Self::Remove => "remove",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Setup => "Choose a provider and configure defaults",
            Self::Benchmark => "Choose a benchmark workload and start a run",
            Self::Recording => "Choose a run and save a recording",
            Self::Replay => "Choose a recording and replay it offline",
            Self::Comparison => "Choose runs and compare their results",
            Self::Status => "Inspect connection and run status",
            Self::Doctor => "Check terminal and compatibility readiness",
            Self::Upgrade => "Choose a release and verify the upgrade",
            Self::Remove => "Remove the local installation safely",
        }
    }
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub struct Output {
    pub schema_version: u8,
    pub command: &'static str,
    pub status: &'static str,
    pub message: &'static str,
    pub next_action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub development_warning: Option<&'static str>,
}

impl Output {
    pub fn for_route(route: Route) -> Self {
        Self {
            schema_version: 1,
            command: route.name(),
            status: "unavailable",
            message: route.label(),
            next_action: "Start asb-tui without a command to open the guided screen",
            development_warning: Some(
                "development-only: the installed ASB channel is not verified; no credentials were used",
            ),
        }
    }

    pub fn human(&self) -> String {
        let mut output = format!(
            "{}\nStatus: {}\n{}\nNext: {}",
            self.command, self.status, self.message, self.next_action
        );
        if let Some(warning) = self.development_warning {
            output.push_str(&format!("\nWarning: {warning}"));
        }
        output.push('\n');
        output
    }
}

pub fn render(route: Route, json: bool) -> String {
    let output = Output::for_route(route);
    if json {
        serde_json::to_string(&output).expect("output contract is serializable") + "\n"
    } else {
        output.human()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_select_the_same_safe_route() {
        assert_eq!(Route::parse("wizard"), Route::parse("setup"));
        assert_eq!(Route::parse("run"), Route::parse("benchmark"));
        assert_eq!(Route::parse("uninstall"), Route::parse("remove"));
    }

    #[test]
    fn json_is_stable_and_credential_free() {
        let json = render(Route::Replay, true);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["command"], "replay");
        assert!(!json.contains("token"));
        assert!(!json.contains("password"));
        assert!(!json.contains("endpoint"));
    }

    #[test]
    fn all_routes_have_a_selectable_name() {
        for route in Route::ALL {
            assert_eq!(Route::parse(route.name()), Some(route));
        }
    }
}
