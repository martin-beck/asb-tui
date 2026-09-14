// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral visual language for the standalone frontend.
//!
//! This module deliberately contains presentation contracts only.  It does not
//! depend on a terminal backend or on Ratatui, so screens can be checked for
//! accessibility and responsive behaviour before a renderer is attached.

use std::fmt;

/// Supported visual policies. `NoColor` retains the same semantic information
/// through text and emphasis rather than terminal colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Theme {
    Dark,
    Light,
    HighContrast,
    NoColor,
}

/// A terminal-safe RGB colour used for contrast checks and renderer mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    fn relative_luminance(self) -> f64 {
        fn channel(value: u8) -> f64 {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.red) + 0.7152 * channel(self.green) + 0.0722 * channel(self.blue)
    }

    /// WCAG relative contrast ratio between two colours.
    pub fn contrast_ratio(self, other: Self) -> f64 {
        let (bright, dark) = {
            let left = self.relative_luminance();
            let right = other.relative_luminance();
            if left >= right {
                (left, right)
            } else {
                (right, left)
            }
        };
        (bright + 0.05) / (dark + 0.05)
    }
}

/// Semantic roles are intentionally finite so screens cannot invent
/// colour-only states without also choosing a text equivalent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticRole {
    Surface,
    Text,
    Muted,
    Accent,
    Focus,
    Success,
    Warning,
    Error,
    Disabled,
    Stale,
}

/// Token values consumed by a renderer, with redundant text guidance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    pub foreground: Rgb,
    pub background: Rgb,
    /// Whether the role must include a textual/status equivalent.
    pub requires_text_equivalent: bool,
    /// Whether the role is expected to be visibly focusable without colour.
    pub requires_non_color_emphasis: bool,
}

impl Token {
    const fn new(foreground: Rgb, background: Rgb) -> Self {
        Self {
            foreground,
            background,
            requires_text_equivalent: true,
            requires_non_color_emphasis: true,
        }
    }
}

/// Complete theme token set.  The renderer may map these values to its own
/// style type, but must preserve the role and redundancy requirements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemeTokens {
    pub theme: Theme,
    pub surface: Token,
    pub text: Token,
    pub muted: Token,
    pub accent: Token,
    pub focus: Token,
    pub success: Token,
    pub warning: Token,
    pub error: Token,
    pub disabled: Token,
    pub stale: Token,
}

impl Theme {
    pub const fn tokens(self) -> ThemeTokens {
        let (background, text, muted, accent, focus, success, warning, error, disabled, stale) =
            match self {
                Self::Dark | Self::NoColor => (
                    Rgb::new(18, 18, 18),
                    Rgb::new(245, 245, 245),
                    Rgb::new(190, 190, 190),
                    Rgb::new(120, 210, 255),
                    Rgb::new(255, 255, 255),
                    Rgb::new(120, 220, 150),
                    Rgb::new(255, 210, 90),
                    Rgb::new(255, 125, 125),
                    Rgb::new(150, 150, 150),
                    Rgb::new(255, 180, 90),
                ),
                Self::Light => (
                    Rgb::new(250, 250, 250),
                    Rgb::new(25, 25, 25),
                    Rgb::new(85, 85, 85),
                    Rgb::new(0, 75, 130),
                    Rgb::new(0, 0, 0),
                    Rgb::new(0, 105, 50),
                    Rgb::new(125, 75, 0),
                    Rgb::new(160, 0, 0),
                    Rgb::new(105, 105, 105),
                    Rgb::new(130, 65, 0),
                ),
                Self::HighContrast => (
                    Rgb::new(0, 0, 0),
                    Rgb::new(255, 255, 255),
                    Rgb::new(255, 255, 255),
                    Rgb::new(0, 255, 255),
                    Rgb::new(255, 255, 0),
                    Rgb::new(0, 255, 0),
                    Rgb::new(255, 255, 0),
                    Rgb::new(255, 80, 80),
                    Rgb::new(190, 190, 190),
                    Rgb::new(255, 160, 0),
                ),
            };
        ThemeTokens {
            theme: self,
            surface: Token::new(text, background),
            text: Token::new(text, background),
            muted: Token::new(muted, background),
            accent: Token::new(accent, background),
            focus: Token::new(focus, background),
            success: Token::new(success, background),
            warning: Token::new(warning, background),
            error: Token::new(error, background),
            disabled: Token::new(disabled, background),
            stale: Token::new(stale, background),
        }
    }
}

impl ThemeTokens {
    pub const fn token(self, role: SemanticRole) -> Token {
        match role {
            SemanticRole::Surface => self.surface,
            SemanticRole::Text => self.text,
            SemanticRole::Muted => self.muted,
            SemanticRole::Accent => self.accent,
            SemanticRole::Focus => self.focus,
            SemanticRole::Success => self.success,
            SemanticRole::Warning => self.warning,
            SemanticRole::Error => self.error,
            SemanticRole::Disabled => self.disabled,
            SemanticRole::Stale => self.stale,
        }
    }
    /// All normal text roles meet the 4.5:1 threshold.
    pub fn normal_text_contrast_ok(self) -> bool {
        [
            SemanticRole::Text,
            SemanticRole::Muted,
            SemanticRole::Accent,
            SemanticRole::Focus,
            SemanticRole::Success,
            SemanticRole::Warning,
            SemanticRole::Error,
            SemanticRole::Disabled,
            SemanticRole::Stale,
        ]
        .into_iter()
        .all(|role| {
            self.token(role)
                .foreground
                .contrast_ratio(self.surface.background)
                >= 4.5
        })
    }
}

/// Responsive layout class derived solely from terminal dimensions.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LayoutTier {
    Tiny,
    Compact,
    Standard,
    Wide,
}

impl LayoutTier {
    pub const fn from_dimensions(columns: u16, lines: u16) -> Self {
        if columns < 40 || lines < 10 {
            Self::Tiny
        } else if columns < 80 || lines < 24 {
            Self::Compact
        } else if columns < 120 {
            Self::Standard
        } else {
            Self::Wide
        }
    }
    pub const fn preserves_action_labels(self) -> bool {
        !matches!(self, Self::Tiny)
    }
}

/// Result of fitting a label into a bounded region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LabelFit {
    Full(String),
    Truncated { text: String, omitted: usize },
}

impl LabelFit {
    pub fn fit(label: &str, width: usize) -> Self {
        let chars: Vec<char> = label.chars().collect();
        if chars.len() <= width {
            return Self::Full(label.to_owned());
        }
        if width <= 1 {
            return Self::Truncated {
                text: "…".chars().take(width).collect(),
                omitted: chars.len(),
            };
        }
        Self::Truncated {
            text: chars[..width - 1].iter().collect::<String>() + "…",
            omitted: chars.len() - (width - 1),
        }
    }
}

/// Maximum steady redraw frequency required by the visual contract.
pub const MAX_STEADY_REDRAWS_PER_SECOND: u8 = 20;

impl fmt::Display for LayoutTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tiny => "tiny",
            Self::Compact => "compact",
            Self::Standard => "standard",
            Self::Wide => "wide",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_are_contrast_safe_and_redundant() {
        for theme in [
            Theme::Dark,
            Theme::Light,
            Theme::HighContrast,
            Theme::NoColor,
        ] {
            let tokens = theme.tokens();
            assert!(tokens.normal_text_contrast_ok(), "{theme:?}");
            for role in [
                SemanticRole::Success,
                SemanticRole::Warning,
                SemanticRole::Error,
                SemanticRole::Focus,
            ] {
                assert!(tokens.token(role).requires_text_equivalent);
                assert!(tokens.token(role).requires_non_color_emphasis);
            }
        }
    }

    #[test]
    fn tiers_are_deterministic_and_monotonic() {
        assert_eq!(LayoutTier::from_dimensions(1, 1), LayoutTier::Tiny);
        assert_eq!(LayoutTier::from_dimensions(40, 10), LayoutTier::Compact);
        assert_eq!(LayoutTier::from_dimensions(80, 24), LayoutTier::Standard);
        assert_eq!(LayoutTier::from_dimensions(160, 48), LayoutTier::Wide);
    }

    #[test]
    fn labels_truncate_by_unicode_scalars_with_visible_marker() {
        assert_eq!(
            LabelFit::fit("abcdef", 4),
            LabelFit::Truncated {
                text: "abc…".into(),
                omitted: 3
            }
        );
        assert_eq!(LabelFit::fit("温度", 2), LabelFit::Full("温度".into()));
        assert_eq!(
            LabelFit::fit("温度計", 2),
            LabelFit::Truncated {
                text: "温…".into(),
                omitted: 2
            }
        );
    }
}
