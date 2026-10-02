// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Standalone command adapter for the future `asb tui` entry point.
//!
//! The ASB command owns the outer router.  This adapter deliberately lives in
//! the standalone package: it gives the router a small, typed development
//! seam while keeping all lifecycle verification in [`crate::development_router`].
//! A development marker is mandatory for lifecycle operations so a fixture can
//! never be mistaken for production support.

use crate::{
    channel_selection::{ChannelSelection, ReleaseChannel},
    delegated::LifecycleResponse,
    development_router,
};
use serde_json::Value;
use std::io::Read;

const MAX_ARGUMENTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiOperation {
    Install,
    Upgrade,
    Status,
    Launch,
    Remove,
}

impl TuiOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Status => "status",
            Self::Launch => "launch",
            Self::Remove => "remove",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiCommand {
    LaunchUi,
    Lifecycle {
        operation: TuiOperation,
        development: bool,
        channel_dev: bool,
        channel: ReleaseChannel,
        selection: ChannelSelection,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    Usage,
    DevelopmentMarkerRequired,
    ProductionMarkerUnsupported,
}

impl ParseError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usage => "usage_invalid",
            Self::DevelopmentMarkerRequired => "development_marker_required",
            Self::ProductionMarkerUnsupported => "production_profile_unsupported",
        }
    }
}

/// Parse `asb-tui tui` arguments.  The standalone command is also the target
/// of the parent ASB router; no unbounded flags or caller-selected trust data
/// are accepted here.
pub fn parse(arguments: &[String]) -> Result<TuiCommand, ParseError> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(ParseError::Usage);
    }
    let Some(operation) = arguments.first().map(String::as_str) else {
        return Ok(TuiCommand::LaunchUi);
    };
    let operation = match operation {
        "install" => TuiOperation::Install,
        "upgrade" => TuiOperation::Upgrade,
        "status" => TuiOperation::Status,
        "launch" => TuiOperation::Launch,
        "remove" => TuiOperation::Remove,
        _ => return Err(ParseError::Usage),
    };
    let mut development = false;
    let mut channel_dev = false;
    let mut requested_channel = None;
    let mut format_json = false;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--development" if !development => development = true,
            "--channel" if !channel_dev => {
                let Some(value) = arguments.get(index + 1).map(String::as_str) else {
                    return Err(ParseError::Usage);
                };
                let channel = match value {
                    "dev" => ReleaseChannel::Dev,
                    "stable" => ReleaseChannel::Stable,
                    "nightly" => ReleaseChannel::Nightly,
                    "experimental" => ReleaseChannel::Experimental,
                    _ => return Err(ParseError::Usage),
                };
                requested_channel = Some(channel);
                channel_dev = channel == ReleaseChannel::Dev;
                development = true;
                index += 1;
            }
            "--production" => return Err(ParseError::ProductionMarkerUnsupported),
            "--json" if !format_json => {
                format_json = true;
            }
            "--format"
                if !format_json && arguments.get(index + 1).map(String::as_str) == Some("json") =>
            {
                format_json = true;
                index += 1;
            }
            _ => return Err(ParseError::Usage),
        }
        index += 1;
    }
    if !development {
        return Err(if !development {
            ParseError::DevelopmentMarkerRequired
        } else {
            ParseError::Usage
        });
    }
    let selection = ChannelSelection::for_request(ReleaseChannel::Dev, requested_channel);
    Ok(TuiCommand::Lifecycle {
        operation,
        development,
        channel_dev,
        channel: selection.active,
        selection,
    })
}

/// Dispatch a lifecycle request received from the parent router.
///
/// The request must be a development envelope and its operation must match the
/// selected command.  The existing closed JSON parser then performs all path,
/// manifest, signature, artifact and lifecycle checks.
pub fn execute_lifecycle(
    operation: TuiOperation,
    channel: ReleaseChannel,
    mut input: impl Read,
) -> LifecycleResponse {
    let mut bytes = Vec::new();
    if input
        .by_ref()
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.is_empty()
        || bytes.len() > 128 * 1024
    {
        return LifecycleResponse::result(false, "router_request_size_invalid")
            .with_channel(channel.as_str());
    }
    let Ok(envelope) = serde_json::from_slice::<Value>(&bytes) else {
        return LifecycleResponse::result(false, "router_request_invalid")
            .with_channel(channel.as_str());
    };
    if envelope.get("profile").and_then(Value::as_str) != Some("development") {
        return LifecycleResponse::result(false, "router_profile_unsupported")
            .with_channel(channel.as_str());
    }
    let Some(request) = envelope.get("request") else {
        return LifecycleResponse::result(false, "router_request_invalid")
            .with_channel(channel.as_str());
    };
    if request.get("operation").and_then(Value::as_str) != Some(operation.as_str()) {
        return LifecycleResponse::result(false, "router_operation_mismatch")
            .with_channel(channel.as_str());
    }
    development_router::execute_input(bytes.as_slice())
        .lifecycle
        .with_channel(channel.as_str())
}

pub fn usage() -> &'static str {
    "usage: asb-tui tui | asb-tui tui <install|upgrade|status|launch|remove> --channel dev [--json|--format json] | ... --development [--json|--format json]"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn bare_tui_is_the_interactive_frontend() {
        assert_eq!(parse(&[]), Ok(TuiCommand::LaunchUi));
    }

    #[test]
    fn lifecycle_defaults_to_human_and_accepts_explicit_json() {
        assert_eq!(
            parse(&args(&["status", "--format", "json"])),
            Err(ParseError::DevelopmentMarkerRequired)
        );
        assert_eq!(
            parse(&args(&["status", "--development"])),
            Ok(TuiCommand::Lifecycle {
                operation: TuiOperation::Status,
                development: true,
                channel_dev: false,
                channel: ReleaseChannel::Dev,
                selection: ChannelSelection::fresh(),
            })
        );
        assert_eq!(
            parse(&args(&["status", "--development", "--json"])),
            Ok(TuiCommand::Lifecycle {
                operation: TuiOperation::Status,
                development: true,
                channel_dev: false,
                channel: ReleaseChannel::Dev,
                selection: ChannelSelection::fresh(),
            })
        );
    }

    #[test]
    fn production_marker_is_never_downgraded_to_development() {
        assert_eq!(
            parse(&args(&["status", "--production", "--format", "json"])),
            Err(ParseError::ProductionMarkerUnsupported)
        );
    }

    #[test]
    fn dev_channel_is_an_explicit_development_profile() {
        assert_eq!(
            parse(&args(&["install", "--channel", "dev", "--format", "json"])),
            Ok(TuiCommand::Lifecycle {
                operation: TuiOperation::Install,
                development: true,
                channel_dev: true,
                channel: ReleaseChannel::Dev,
                selection: ChannelSelection::for_request(
                    ReleaseChannel::Dev,
                    Some(ReleaseChannel::Dev),
                ),
            })
        );
        assert_eq!(
            parse(&args(&[
                "install",
                "--channel",
                "stable",
                "--format",
                "json"
            ])),
            Ok(TuiCommand::Lifecycle {
                operation: TuiOperation::Install,
                development: true,
                channel_dev: false,
                channel: ReleaseChannel::Dev,
                selection: ChannelSelection::for_request(
                    ReleaseChannel::Dev,
                    Some(ReleaseChannel::Stable),
                ),
            })
        );
    }

    #[test]
    fn operation_mismatch_is_rejected_before_lifecycle_dispatch() {
        let response = execute_lifecycle(
            TuiOperation::Status,
            ReleaseChannel::Dev,
            br#"{"router_version":1,"profile":"development","request":{"operation":"remove","schema_version":1,"install_root":"/tmp/asb-tui"}}"#.as_slice(),
        );
        assert_eq!(response.code, "router_operation_mismatch");
    }
}
