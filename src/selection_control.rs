// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral, visibly checkable selection rows.
//!
//! The control deliberately carries only presentation facts.  Catalogs retain
//! authority over what may be selected; this module makes that authority
//! readable in every wizard and benchmark selection surface.

/// Whether a selection surface admits one choice or a bounded set of choices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    Single,
    Multiple,
}

/// One bounded row projected by a catalog-backed selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionRow<'a> {
    pub label: &'a str,
    pub selected: bool,
    pub focused: bool,
    pub enabled: bool,
    /// A short public explanation for an unavailable row.
    pub disabled_reason: Option<&'a str>,
}

impl<'a> SelectionRow<'a> {
    #[must_use]
    pub const fn enabled(label: &'a str, selected: bool, focused: bool) -> Self {
        Self {
            label,
            selected,
            focused,
            enabled: true,
            disabled_reason: None,
        }
    }

    #[must_use]
    pub const fn disabled(label: &'a str, focused: bool, reason: &'a str) -> Self {
        Self {
            label,
            selected: false,
            focused,
            enabled: false,
            disabled_reason: Some(reason),
        }
    }
}

/// Render a row without relying on colour: focus, check state, and disabled
/// state remain distinct in the resulting text.
#[must_use]
pub fn row_text(row: &SelectionRow<'_>) -> String {
    let cursor = if row.focused { ">" } else { " " };
    let check = if row.selected { "[x]" } else { "[ ]" };
    match (row.enabled, row.disabled_reason) {
        (true, _) => format!("{cursor} {check} {}", row.label),
        (false, Some(reason)) => format!("{cursor} {check} {} — unavailable: {reason}", row.label),
        (false, None) => format!("{cursor} {check} {} — unavailable", row.label),
    }
}

/// Stable summary prose, kept separate from the description/details pane.
#[must_use]
pub fn summary_text(title: &str, selected: &[&str], mode: SelectionMode) -> Vec<String> {
    let noun = match mode {
        SelectionMode::Single => "choice",
        SelectionMode::Multiple => "choices",
    };
    if selected.is_empty() {
        return vec![format!("{title}: none selected"), format!("0 {noun}")];
    }
    vec![
        format!("{title}: {} selected", selected.len()),
        selected.join(", "),
    ]
}

/// The documented actions are intentionally finite and context-neutral.
#[must_use]
pub const fn action_hint(mode: SelectionMode) -> &'static str {
    match mode {
        SelectionMode::Single => "Enter selects the focused available choice.",
        SelectionMode::Multiple => "Space toggles; a selects all or clears all visible choices.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_text_keeps_focus_selection_and_disabled_meaning_separate() {
        assert_eq!(
            row_text(&SelectionRow::enabled("Agent A", true, false)),
            "  [x] Agent A"
        );
        assert_eq!(
            row_text(&SelectionRow::disabled("Agent B", true, "not installed")),
            "> [ ] Agent B — unavailable: not installed"
        );
    }

    #[test]
    fn summaries_explain_empty_single_and_multi_selection() {
        assert_eq!(
            summary_text("Agents", &[], SelectionMode::Multiple),
            ["Agents: none selected", "0 choices"]
        );
        assert_eq!(
            summary_text("Provider", &["OpenRouter"], SelectionMode::Single),
            ["Provider: 1 selected", "OpenRouter"]
        );
    }

    #[test]
    fn every_boolean_row_state_has_a_distinct_textual_contract() {
        for selected in [false, true] {
            for focused in [false, true] {
                for enabled in [false, true] {
                    let row = SelectionRow {
                        label: "choice",
                        selected,
                        focused,
                        enabled,
                        disabled_reason: (!enabled).then_some("blocked"),
                    };
                    let text = row_text(&row);
                    assert!(text.starts_with(if focused { ">" } else { " " }));
                    assert!(text.contains(if selected { "[x]" } else { "[ ]" }));
                    assert_eq!(text.contains("unavailable"), !enabled);
                }
            }
        }
    }
}
