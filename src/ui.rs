// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! The interactive workspace: keyboard-first navigation and Ratatui projection.
//!
//! This module owns only presentation state. Benchmark execution and credentials remain in
//! the external ASB control plane.

use crate::terminal::{CapabilityTier, RenderPolicy};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Landing,
    Measures,
    Configuration,
    Reports,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    None,
    Quit,
    Resize(u16, u16),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceState {
    pub screen: Screen,
    pub help: bool,
    pub search: String,
    pub measure_cursor: usize,
    pub measures: Vec<MeasureRow>,
    pub config_cursor: usize,
    pub report_cursor: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeasureRow {
    pub group: &'static str,
    pub name: &'static str,
    pub selected: bool,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            screen: Screen::Landing,
            help: false,
            search: String::new(),
            measure_cursor: 0,
            measures: vec![
                MeasureRow {
                    group: "Quality",
                    name: "Correctness",
                    selected: true,
                },
                MeasureRow {
                    group: "Quality",
                    name: "Consistency",
                    selected: true,
                },
                MeasureRow {
                    group: "Efficiency",
                    name: "Latency",
                    selected: true,
                },
                MeasureRow {
                    group: "Efficiency",
                    name: "Token usage",
                    selected: false,
                },
                MeasureRow {
                    group: "Safety",
                    name: "Policy adherence",
                    selected: false,
                },
            ],
            config_cursor: 0,
            report_cursor: 0,
        }
    }
}

impl WorkspaceState {
    pub fn handle_key(&mut self, key: KeyEvent) -> UiAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return UiAction::Quit;
        }
        if self.help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h')
            ) {
                self.help = false;
            }
            return UiAction::None;
        }
        match key.code {
            KeyCode::Char('q') => UiAction::Quit,
            KeyCode::Char('?') | KeyCode::Char('h') => {
                self.help = true;
                UiAction::None
            }
            KeyCode::Char('1') => {
                self.screen = Screen::Landing;
                UiAction::None
            }
            KeyCode::Char('2') => {
                self.screen = Screen::Measures;
                UiAction::None
            }
            KeyCode::Char('3') => {
                self.screen = Screen::Configuration;
                UiAction::None
            }
            KeyCode::Char('4') => {
                self.screen = Screen::Reports;
                UiAction::None
            }
            KeyCode::Tab | KeyCode::Right => {
                self.screen = next_screen(self.screen);
                UiAction::None
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.screen = previous_screen(self.screen);
                UiAction::None
            }
            KeyCode::Up => {
                self.move_cursor(-1);
                UiAction::None
            }
            KeyCode::Down => {
                self.move_cursor(1);
                UiAction::None
            }
            KeyCode::Char(' ') if self.screen == Screen::Measures => {
                if let Some(row) = self
                    .visible_indices()
                    .get(self.measure_cursor)
                    .and_then(|i| self.measures.get_mut(*i))
                {
                    row.selected = !row.selected;
                }
                UiAction::None
            }
            KeyCode::Char('g') if self.screen == Screen::Measures => {
                self.toggle_visible_group();
                UiAction::None
            }
            KeyCode::Char('/') if self.screen == Screen::Measures => UiAction::None,
            KeyCode::Char(c) if self.screen == Screen::Measures && !c.is_control() => {
                self.search.push(c);
                self.measure_cursor = 0;
                UiAction::None
            }
            KeyCode::Backspace if self.screen == Screen::Measures => {
                self.search.pop();
                self.measure_cursor = 0;
                UiAction::None
            }
            _ => UiAction::None,
        }
    }

    fn move_cursor(&mut self, delta: i8) {
        let max = match self.screen {
            Screen::Measures => self.visible_indices().len(),
            Screen::Configuration => 4,
            Screen::Reports => 3,
            _ => 1,
        };
        if max == 0 {
            self.measure_cursor = 0;
            return;
        }
        let cursor = match self.screen {
            Screen::Measures => &mut self.measure_cursor,
            Screen::Configuration => &mut self.config_cursor,
            Screen::Reports => &mut self.report_cursor,
            _ => return,
        };
        if delta < 0 {
            *cursor = cursor.saturating_sub(1);
        } else {
            *cursor = (*cursor + 1).min(max - 1);
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self.search.to_ascii_lowercase();
        self.measures
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                query.is_empty()
                    || row.name.to_ascii_lowercase().contains(&query)
                    || row.group.to_ascii_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn toggle_visible_group(&mut self) {
        let indices = self.visible_indices();
        let Some(&current) = indices.get(self.measure_cursor) else {
            return;
        };
        let group = self.measures[current].group;
        let members: Vec<usize> = indices
            .into_iter()
            .filter(|index| self.measures[*index].group == group)
            .collect();
        let select = members.iter().any(|index| !self.measures[*index].selected);
        for index in members {
            self.measures[index].selected = select;
        }
    }
}

fn next_screen(screen: Screen) -> Screen {
    match screen {
        Screen::Landing => Screen::Measures,
        Screen::Measures => Screen::Configuration,
        Screen::Configuration => Screen::Reports,
        Screen::Reports | Screen::Help => Screen::Landing,
    }
}
fn previous_screen(screen: Screen) -> Screen {
    match screen {
        Screen::Landing => Screen::Reports,
        Screen::Measures => Screen::Landing,
        Screen::Configuration => Screen::Measures,
        Screen::Reports | Screen::Help => Screen::Configuration,
    }
}

pub fn render(frame: &mut Frame<'_>, state: &WorkspaceState, policy: RenderPolicy) {
    let area = frame.area();
    if area.width < 38 || area.height < 8 {
        render_compact(frame, area, policy);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(area);
    let titles = ["1 Home", "2 Measures", "3 Configure", "4 Reports"];
    let selected = match state.screen {
        Screen::Landing => 0,
        Screen::Measures => 1,
        Screen::Configuration => 2,
        Screen::Reports => 3,
        Screen::Help => 0,
    };
    frame.render_widget(
        Tabs::new(titles)
            .select(selected)
            .block(
                Block::default()
                    .title(" ASB - Agent Systems Benchmark ")
                    .borders(Borders::ALL),
            )
            .highlight_style(accent(policy)),
        chunks[0],
    );
    match state.screen {
        Screen::Landing => landing(frame, chunks[1], policy),
        Screen::Measures => measures(frame, chunks[1], state, policy),
        Screen::Configuration => configuration(frame, chunks[1], state, policy),
        Screen::Reports => reports(frame, chunks[1], state, policy),
        Screen::Help => landing(frame, chunks[1], policy),
    }
    footer(frame, chunks[2], state, policy);
    if state.help {
        help_overlay(frame, area, policy);
    }
}

fn landing(frame: &mut Frame<'_>, area: Rect, policy: RenderPolicy) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new("Welcome to ASB")
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center),
        inner[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Review benchmark plans, select measures, and compare recent runs."),
            Line::from("Connection: waiting for ASB control plane"),
            Line::from("The runner remains external; this workspace is a safe control surface."),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(panel(" Ready ", policy)),
        inner[1],
    );
    frame.render_widget(
        Paragraph::new("Press 2 to choose measures  |  3 to configure  |  4 to inspect reports")
            .alignment(Alignment::Center)
            .style(muted(policy)),
        inner[2],
    );
}

fn measures(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(area);
    let search = if state.search.is_empty() {
        "Search measures (type to filter)"
    } else {
        &state.search
    };
    frame.render_widget(
        Paragraph::new(search)
            .block(panel(" / Search ", policy))
            .style(if state.search.is_empty() {
                muted(policy)
            } else {
                accent(policy)
            }),
        cols[0],
    );
    let list_area = Rect {
        x: cols[0].x,
        y: cols[0].y.saturating_add(2),
        width: cols[0].width,
        height: cols[0].height.saturating_sub(2),
    };
    let items: Vec<ListItem> = state
        .visible_indices()
        .iter()
        .map(|i| {
            let row = &state.measures[*i];
            ListItem::new(Line::from(vec![
                Span::styled(if row.selected { "[x] " } else { "[ ] " }, accent(policy)),
                Span::styled(format!("{} / {}", row.group, row.name), Style::default()),
            ]))
        })
        .collect();
    let mut ls = ListState::default();
    ls.select((state.measure_cursor < items.len()).then_some(state.measure_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Measures - Space item, g group ", policy))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        list_area,
        &mut ls,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Grouped selection"),
            Line::from(""),
            Line::from(format!(
                "{} selected",
                state.measures.iter().filter(|m| m.selected).count()
            )),
            Line::from(""),
            Line::from("Up/Down navigate   Space item   g group"),
            Line::from("Type to search"),
        ])
        .block(panel(" Selection ", policy))
        .wrap(Wrap { trim: true }),
        cols[1],
    );
}

fn configuration(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let entries = [
        "Provider: default",
        "Model: negotiated",
        "Run mode: reproducible",
        "Save changes: Ctrl-S",
    ];
    let items: Vec<ListItem> = entries.iter().map(|x| ListItem::new(*x)).collect();
    let mut ls = ListState::default();
    ls.select(Some(state.config_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Configuration - Enter edits ", policy))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut ls,
    );
}

fn reports(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let entries = [
        "Most recent | today | complete",
        "Previous run | yesterday | complete",
        "Compare selected runs | Enter opens",
    ];
    let items: Vec<ListItem> = entries.iter().map(|x| ListItem::new(*x)).collect();
    let mut ls = ListState::default();
    ls.select(Some(state.report_cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(panel(" Recent runs - Reports ", policy))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut ls,
    );
}

fn footer(frame: &mut Frame<'_>, area: Rect, state: &WorkspaceState, policy: RenderPolicy) {
    let help = if state.help {
        "Esc close help"
    } else {
        "? help"
    };
    frame.render_widget(
        Paragraph::new(format!("{}    Tab/Left/Right navigate    q quit", help))
            .alignment(Alignment::Center)
            .style(muted(policy)),
        area,
    );
}
fn help_overlay(frame: &mut Frame<'_>, area: Rect, policy: RenderPolicy) {
    let popup = centered(area, 70, 65);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("Keyboard help", accent(policy))),
            Line::from("1-4  switch workspace"),
            Line::from("Tab / arrows  navigate"),
            Line::from("Up/Down  move selection"),
            Line::from("Space  toggle measure"),
            Line::from("Type / Backspace  search measures"),
            Line::from("q / Ctrl-C  quit"),
            Line::from("Esc  close this window"),
        ])
        .block(panel(" Help ", policy))
        .wrap(Wrap { trim: true }),
        popup,
    );
}
fn render_compact(frame: &mut Frame<'_>, area: Rect, policy: RenderPolicy) {
    frame.render_widget(
        Paragraph::new("ASB | 1 Home 2 Measures 3 Config 4 Reports\n? help | q quit")
            .style(accent(policy)),
        area,
    );
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}
fn panel(title: &'static str, policy: RenderPolicy) -> Block<'static> {
    Block::default().title(title).borders(if policy.unicode {
        Borders::ALL
    } else {
        Borders::NONE
    })
}
fn accent(policy: RenderPolicy) -> Style {
    if matches!(policy.tier, CapabilityTier::Plain) {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }
}
fn muted(policy: RenderPolicy) -> Style {
    if matches!(policy.tier, CapabilityTier::Plain) {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;
    use ratatui::{Terminal, backend::TestBackend};
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
    }
    fn policy() -> RenderPolicy {
        RenderPolicy {
            tier: CapabilityTier::TrueColor,
            unicode: true,
            mouse: false,
            focus: false,
            bracketed_paste: false,
            synchronized_output: false,
            alternate_screen: true,
        }
    }
    #[test]
    fn navigation_and_selection_are_keyboard_first() {
        let mut s = WorkspaceState::default();
        assert_eq!(s.handle_key(key(KeyCode::Char('2'))), UiAction::None);
        assert_eq!(s.screen, Screen::Measures);
        s.handle_key(key(KeyCode::Down));
        s.handle_key(key(KeyCode::Char(' ')));
        assert!(!s.measures[1].selected);
        s.handle_key(key(KeyCode::Char('g')));
        assert!(s.measures[0].selected && s.measures[1].selected);
        s.handle_key(key(KeyCode::Char('c')));
        assert_eq!(s.search, "c");
        s.handle_key(key(KeyCode::Char('?')));
        assert!(s.help);
        s.handle_key(key(KeyCode::Esc));
        assert!(!s.help);
    }
    #[test]
    fn test_backend_snapshot_contains_contextual_controls() {
        let s = WorkspaceState::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| render(f, &s, policy())).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Welcome to ASB"));
        assert!(text.contains("? help"));
    }
}
