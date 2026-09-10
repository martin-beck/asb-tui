// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic terminal capability evidence and conservative render policy.

use serde::{Deserialize, Serialize};
use std::{
    env, fmt,
    io::{IsTerminal, stdin, stdout},
};

/// Maximum terminal dimension accepted from an environment hint.
pub const MAX_TERMINAL_DIMENSION: u16 = 16_384;

/// Responsive layout selected from the current authoritative dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutClass {
    /// A one-column, keyboard-first view for very small terminals.
    Compact,
    /// A reduced table view that preserves labels and focus.
    Standard,
    /// Full panels and numeric detail are available.
    Wide,
}

/// Current dimensions and responsive layout decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsiveLayout {
    /// Authoritative width used for this frame.
    pub columns: u16,
    /// Authoritative height used for this frame.
    pub lines: u16,
    /// Layout class for this frame only.
    pub class: LayoutClass,
}

impl ResponsiveLayout {
    /// Select a safe layout; missing dimensions use the compact fallback.
    pub fn from_dimensions(columns: Option<u16>, lines: Option<u16>) -> Self {
        let columns = columns.unwrap_or(1).max(1);
        let lines = lines.unwrap_or(1).max(1);
        let class = if columns < 40 || lines < 8 {
            LayoutClass::Compact
        } else if columns < 100 || lines < 20 {
            LayoutClass::Standard
        } else {
            LayoutClass::Wide
        };
        Self {
            columns,
            lines,
            class,
        }
    }
}

/// Evidence supplied by the channel and terminal, kept separate from policy.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalEvidence {
    /// Whether stdin and stdout are attached to a TTY.
    pub tty: bool,
    /// Bounded TERM value, if present.
    pub term: Option<String>,
    /// Bounded terminal-program hint, if present.
    pub term_program: Option<String>,
    /// Bounded color-terminal hint, if present.
    pub color_term: Option<String>,
    /// Whether the channel advertises an SSH session.
    pub ssh: bool,
    /// Whether a tmux multiplexer is present.
    pub tmux: bool,
    /// Whether a screen multiplexer is present.
    pub screen: bool,
    /// Whether color output was explicitly disabled.
    pub no_color: bool,
    /// Width hint, when supplied by the channel.
    pub columns: Option<u16>,
    /// Height hint, when supplied by the channel.
    pub lines: Option<u16>,
}

impl TerminalEvidence {
    /// Capture only bounded process-environment hints and TTY state.
    pub fn from_environment() -> Self {
        Self {
            tty: stdin().is_terminal() && stdout().is_terminal(),
            term: bounded_env("TERM"),
            term_program: bounded_env("TERM_PROGRAM"),
            color_term: bounded_env("COLORTERM"),
            ssh: env::var_os("SSH_CONNECTION").is_some() || env::var_os("SSH_TTY").is_some(),
            tmux: env::var_os("TMUX").is_some(),
            screen: env::var_os("STY").is_some(),
            no_color: env::var_os("NO_COLOR").is_some(),
            columns: bounded_dimension("COLUMNS"),
            lines: bounded_dimension("LINES"),
        }
    }

    /// Validate evidence before using it in a public diagnostic or policy decision.
    pub fn validate(&self) -> Result<(), TerminalError> {
        for value in [&self.term, &self.term_program, &self.color_term]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err(TerminalError::InvalidEvidence);
            }
        }
        for dimension in [self.columns, self.lines].into_iter().flatten() {
            if dimension == 0 || dimension > MAX_TERMINAL_DIMENSION {
                return Err(TerminalError::InvalidEvidence);
            }
        }
        Ok(())
    }
}

fn bounded_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| {
        !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
    })
}

fn bounded_dimension(name: &str) -> Option<u16> {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value > 0 && *value <= MAX_TERMINAL_DIMENSION)
}

/// Conservative rendering capability tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTier {
    /// Plain ASCII and no color, suitable for pipes and unknown channels.
    Plain,
    /// ANSI 8/16-color output with conservative Unicode.
    BasicColor,
    /// 256-color output with Unicode when width behavior is known.
    IndexedColor,
    /// Truecolor and enhanced input after explicit evidence.
    TrueColor,
}

/// Feature gates selected from terminal evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPolicy {
    /// Selected color/Unicode capability tier.
    pub tier: CapabilityTier,
    /// Whether Unicode glyphs may be emitted.
    pub unicode: bool,
    /// Whether optional mouse input may be enabled.
    pub mouse: bool,
    /// Whether focus reporting may be enabled.
    pub focus: bool,
    /// Whether bracketed paste may be enabled.
    pub bracketed_paste: bool,
    /// Whether synchronized output is safe to request.
    pub synchronized_output: bool,
    /// Whether alternate-screen mode is allowed.
    pub alternate_screen: bool,
}

impl RenderPolicy {
    /// Select a fail-closed policy from validated evidence.
    pub fn from_evidence(evidence: &TerminalEvidence) -> Result<Self, TerminalError> {
        evidence.validate()?;
        let term = evidence.term.as_deref().unwrap_or("");
        let color = evidence.color_term.as_deref().unwrap_or("");
        let indexed = term.contains("256color") || color.eq_ignore_ascii_case("256");
        let truecolor = color.eq_ignore_ascii_case("truecolor")
            || color.eq_ignore_ascii_case("24bit")
            || term.contains("direct");
        let unknown_terminal = term.is_empty() || term.eq_ignore_ascii_case("dumb");
        let tier = if !evidence.tty || evidence.no_color || unknown_terminal {
            CapabilityTier::Plain
        } else if truecolor {
            CapabilityTier::TrueColor
        } else if indexed {
            CapabilityTier::IndexedColor
        } else {
            CapabilityTier::BasicColor
        };
        let enhanced = matches!(
            tier,
            CapabilityTier::IndexedColor | CapabilityTier::TrueColor
        ) && evidence.tty
            && !evidence.tmux
            && !evidence.screen;
        Ok(Self {
            tier,
            unicode: !matches!(tier, CapabilityTier::Plain),
            mouse: enhanced,
            focus: enhanced,
            bracketed_paste: enhanced,
            synchronized_output: matches!(tier, CapabilityTier::TrueColor) && !evidence.ssh,
            alternate_screen: evidence.tty,
        })
    }

    /// Compact, stable diagnostic that does not reproduce environment values.
    pub fn doctor_line(&self, evidence: &TerminalEvidence) -> String {
        format!(
            "tier={} tty={} channel={} size={}x{} unicode={} mouse={} focus={} no_color={}",
            tier_name(self.tier),
            evidence.tty,
            channel(evidence),
            evidence
                .columns
                .map_or_else(|| "?".into(), |v| v.to_string()),
            evidence.lines.map_or_else(|| "?".into(), |v| v.to_string()),
            self.unicode,
            self.mouse,
            self.focus,
            evidence.no_color
        )
    }
}

/// Produce the privacy-safe terminal capability diagnostic.
pub fn doctor(evidence: TerminalEvidence) -> Result<String, TerminalError> {
    let policy = RenderPolicy::from_evidence(&evidence)?;
    Ok(policy.doctor_line(&evidence))
}

fn channel(evidence: &TerminalEvidence) -> &str {
    if evidence.tmux {
        "tmux"
    } else if evidence.screen {
        "screen"
    } else if evidence.ssh {
        "ssh"
    } else if evidence.tty {
        "local-tty"
    } else {
        "non-tty"
    }
}

fn tier_name(tier: CapabilityTier) -> &'static str {
    match tier {
        CapabilityTier::Plain => "plain",
        CapabilityTier::BasicColor => "basic_color",
        CapabilityTier::IndexedColor => "indexed_color",
        CapabilityTier::TrueColor => "true_color",
    }
}

/// Terminal evidence or policy was malformed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalError {
    /// An untrusted hint exceeded the public bound or contained control data.
    InvalidEvidence,
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid terminal capability evidence")
    }
}

impl std::error::Error for TerminalError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> TerminalEvidence {
        TerminalEvidence {
            tty: true,
            term: Some("xterm-256color".into()),
            term_program: Some("WezTerm".into()),
            color_term: Some("truecolor".into()),
            ssh: false,
            tmux: false,
            screen: false,
            no_color: false,
            columns: Some(120),
            lines: Some(40),
        }
    }

    #[test]
    fn policy_requires_evidence_and_enables_only_supported_features() {
        let policy = RenderPolicy::from_evidence(&evidence()).unwrap();
        assert_eq!(policy.tier, CapabilityTier::TrueColor);
        assert!(policy.unicode && policy.mouse && policy.focus);
        assert!(policy.synchronized_output);
        assert!(
            policy
                .doctor_line(&evidence())
                .contains("channel=local-tty")
        );
    }

    #[test]
    fn multiplexers_and_ssh_disable_risky_enhancements() {
        let mut value = evidence();
        value.tmux = true;
        value.ssh = true;
        let policy = RenderPolicy::from_evidence(&value).unwrap();
        assert!(!policy.mouse && !policy.focus && !policy.synchronized_output);
    }

    #[test]
    fn pipes_and_no_color_are_plain_and_bounded() {
        let mut value = evidence();
        value.tty = false;
        value.no_color = true;
        value.columns = Some(0);
        assert_eq!(
            RenderPolicy::from_evidence(&value),
            Err(TerminalError::InvalidEvidence)
        );
        value.columns = None;
        let policy = RenderPolicy::from_evidence(&value).unwrap();
        assert_eq!(policy.tier, CapabilityTier::Plain);
        assert!(!policy.unicode && !policy.alternate_screen);

        let mut unknown = evidence();
        unknown.term = Some("dumb".into());
        unknown.color_term = None;
        let policy = RenderPolicy::from_evidence(&unknown).unwrap();
        assert_eq!(policy.tier, CapabilityTier::Plain);
        assert!(!policy.unicode);
    }

    #[test]
    fn control_values_and_oversized_dimensions_fail_closed() {
        let mut value = evidence();
        value.term = Some("xterm\n".into());
        assert_eq!(value.validate(), Err(TerminalError::InvalidEvidence));
        value.term = Some("xterm".into());
        value.lines = Some(MAX_TERMINAL_DIMENSION + 1);
        assert_eq!(value.validate(), Err(TerminalError::InvalidEvidence));
    }

    #[test]
    fn responsive_layout_never_trusts_missing_or_tiny_dimensions() {
        assert_eq!(
            ResponsiveLayout::from_dimensions(Some(1), Some(1)).class,
            LayoutClass::Compact
        );
        assert_eq!(
            ResponsiveLayout::from_dimensions(Some(80), Some(24)).class,
            LayoutClass::Standard
        );
        assert_eq!(
            ResponsiveLayout::from_dimensions(Some(160), Some(50)).class,
            LayoutClass::Wide
        );
        assert_eq!(
            ResponsiveLayout::from_dimensions(None, None).class,
            LayoutClass::Compact
        );
    }

    #[test]
    fn doctor_output_contains_no_environment_values() {
        let mut value = evidence();
        value.term_program = Some("private-terminal-name".into());
        let output = doctor(value).unwrap();
        assert!(!output.contains("private-terminal-name"));
        assert!(output.contains("tier=true_color"));
    }

    #[test]
    fn channel_and_color_tiers_cover_only_closed_labels() {
        let mut value = evidence();
        value.color_term = None;
        assert_eq!(
            RenderPolicy::from_evidence(&value).unwrap().tier,
            CapabilityTier::IndexedColor
        );

        value.term = Some("xterm".into());
        assert_eq!(
            RenderPolicy::from_evidence(&value).unwrap().tier,
            CapabilityTier::BasicColor
        );

        value.ssh = true;
        let policy = RenderPolicy::from_evidence(&value).unwrap();
        assert!(!policy.synchronized_output);
        assert!(policy.doctor_line(&value).contains("channel=ssh"));

        value.screen = true;
        assert!(
            RenderPolicy::from_evidence(&value)
                .unwrap()
                .doctor_line(&value)
                .contains("channel=screen")
        );

        value.tmux = true;
        assert!(
            RenderPolicy::from_evidence(&value)
                .unwrap()
                .doctor_line(&value)
                .contains("channel=tmux")
        );
    }

    #[test]
    fn invalid_empty_and_control_values_have_stable_error() {
        let mut value = evidence();
        value.term_program = Some(String::new());
        assert_eq!(value.validate(), Err(TerminalError::InvalidEvidence));
        assert_eq!(
            TerminalError::InvalidEvidence.to_string(),
            "invalid terminal capability evidence"
        );
    }
}
