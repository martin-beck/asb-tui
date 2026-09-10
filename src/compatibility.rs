// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic, privacy-safe standalone compatibility evaluation.

use serde::{Deserialize, Serialize};

pub const COORDINATOR_COMMIT: &str = "510817b93feb80dde13e5a6c61d657954fae2346";
pub const QUALITY_COMMIT: &str = "8a9f056b7fc7926b9465a0f7a09225d4da1c572a";
pub const COORDINATOR_VERSION: &str = "v0.3.5";
pub const QUALITY_VERSION: &str = "v0.23.0";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatingSystem {
    Linux,
    Macos,
    Windows,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    Ubuntu2404,
    Debian12,
    Other,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    X86_64,
    Aarch64,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColorLevel {
    None,
    Ansi16,
    Ansi256,
    Truecolor,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalChannel {
    Tty,
    Pipe,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Multiplexer {
    None,
    Tmux,
    Screen,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlatformProbe {
    pub os: OperatingSystem,
    pub distribution: Distribution,
    pub architecture: Architecture,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AsbProbe {
    pub version: String,
    pub protocol_version: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DependencyProbe {
    pub coordinator_version: String,
    pub coordinator_commit: String,
    pub quality_version: String,
    pub quality_commit: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TerminalProbe {
    pub columns: u16,
    pub rows: u16,
    pub color: ColorLevel,
    pub unicode: bool,
    pub resize_events: bool,
    pub channel: TerminalChannel,
    pub ssh: bool,
    pub multiplexer: Multiplexer,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProbe {
    pub config_writable: bool,
    pub cache_writable: bool,
    pub atomic_rename: bool,
    pub executable_files: bool,
    pub git_available: bool,
    pub ssh_keygen_available: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityProbe {
    pub schema_version: u64,
    pub platform: PlatformProbe,
    pub asb: AsbProbe,
    pub dependencies: DependencyProbe,
    pub terminal: TerminalProbe,
    pub runtime: RuntimeProbe,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CompatibilityReport {
    pub schema_version: u64,
    pub classification: &'static str,
    pub bundle: Option<&'static str>,
    pub platform: PlatformProbe,
    pub asb_version: String,
    pub protocol_version: u64,
    pub terminal: TerminalReport,
    pub reasons: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TerminalReport {
    pub columns: u16,
    pub rows: u16,
    pub color: ColorLevel,
    pub unicode: bool,
    pub resize_events: bool,
    pub channel: TerminalChannel,
    pub ssh: bool,
    pub multiplexer: Multiplexer,
}

/// Parse a closed probe document. Unknown members and wrong JSON types fail closed.
pub fn parse_probe(input: &str) -> Result<CompatibilityProbe, String> {
    if input.len() > 65_536 {
        return Err("compatibility probe exceeds size limit".into());
    }
    let probe: CompatibilityProbe =
        serde_json::from_str(input).map_err(|_| "invalid compatibility probe".to_owned())?;
    if probe.schema_version != 1 {
        return Err("unsupported compatibility probe version".into());
    }
    if !is_safe_version(&probe.asb.version) {
        return Err("invalid ASB version".into());
    }
    Ok(probe)
}

/// Evaluate only closed, normalized facts and return fixed diagnostic reason codes.
pub fn evaluate(probe: CompatibilityProbe) -> CompatibilityReport {
    let mut reasons = Vec::new();
    if probe.platform.os != OperatingSystem::Linux {
        reasons.push("unsupported_os");
    }
    if !matches!(
        probe.platform.distribution,
        Distribution::Ubuntu2404 | Distribution::Debian12
    ) {
        reasons.push("unsupported_distribution");
    }
    if probe.platform.os == OperatingSystem::Linux
        && probe.platform.distribution == Distribution::NotApplicable
    {
        reasons.push("inconsistent_platform");
    }
    if probe.platform.os != OperatingSystem::Linux
        && probe.platform.distribution != Distribution::NotApplicable
    {
        reasons.push("inconsistent_platform");
    }
    if probe.platform.architecture == Architecture::Other {
        reasons.push("unsupported_architecture");
    }
    if probe.asb.protocol_version != 1 {
        reasons.push("unsupported_protocol");
    }
    if probe.dependencies.coordinator_version != COORDINATOR_VERSION
        || probe.dependencies.coordinator_commit != COORDINATOR_COMMIT
        || probe.dependencies.quality_version != QUALITY_VERSION
        || probe.dependencies.quality_commit != QUALITY_COMMIT
    {
        reasons.push("tooling_identity_mismatch");
    }
    if probe.terminal.columns < 80 || probe.terminal.rows < 24 {
        reasons.push("terminal_too_small");
    }
    if probe.terminal.channel != TerminalChannel::Tty {
        reasons.push("non_interactive_channel");
    }
    if probe.terminal.color == ColorLevel::None || !probe.terminal.unicode {
        reasons.push("terminal_features_unavailable");
    }
    if !probe.terminal.resize_events {
        reasons.push("resize_events_unavailable");
    }
    if !probe.runtime.config_writable || !probe.runtime.cache_writable {
        reasons.push("storage_unavailable");
    }
    if !probe.runtime.atomic_rename || !probe.runtime.executable_files {
        reasons.push("filesystem_incompatible");
    }
    if !probe.runtime.git_available || !probe.runtime.ssh_keygen_available {
        reasons.push("release_verifier_unavailable");
    }
    reasons.sort_unstable();
    reasons.dedup();

    let bundle = if reasons.is_empty() {
        match probe.platform.architecture {
            Architecture::X86_64 => Some("asb-tui-v1-linux-x86_64"),
            Architecture::Aarch64 => Some("asb-tui-v1-linux-aarch64"),
            Architecture::Other => None,
        }
    } else {
        None
    };
    CompatibilityReport {
        schema_version: 1,
        classification: if bundle.is_some() {
            "compatible"
        } else {
            "unsupported"
        },
        bundle,
        platform: probe.platform,
        asb_version: probe.asb.version,
        protocol_version: probe.asb.protocol_version,
        terminal: TerminalReport {
            columns: probe.terminal.columns,
            rows: probe.terminal.rows,
            color: probe.terminal.color,
            unicode: probe.terminal.unicode,
            resize_events: probe.terminal.resize_events,
            channel: probe.terminal.channel,
            ssh: probe.terminal.ssh,
            multiplexer: probe.terminal.multiplexer,
        },
        reasons,
    }
}

fn is_safe_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 || !value.is_ascii() {
        return false;
    }
    let (core, suffix) = value
        .split_once('-')
        .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
    let mut components = core.split('.');
    let valid_number =
        |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    valid_number(components.next().unwrap_or_default())
        && valid_number(components.next().unwrap_or_default())
        && valid_number(components.next().unwrap_or_default())
        && components.next().is_none()
        && suffix.is_none_or(|suffix| !suffix.is_empty())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}
