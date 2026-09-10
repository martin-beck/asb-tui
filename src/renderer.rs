// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Pure Ratatui projection of immutable application state.

use crate::{
    app::AppState,
    terminal::{CapabilityTier, RenderPolicy},
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render one immutable application snapshot into the supplied frame.
pub fn render(frame: &mut Frame<'_>, state: &AppState, policy: RenderPolicy) {
    let area = frame.area();
    if area.width < 20 || area.height < 4 {
        render_tiny(frame, area, policy);
        return;
    }

    let model = state.frame_model();
    let regions = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    let title = Paragraph::new(Line::from(vec![
        Span::styled("ASB", accent(policy)),
        Span::raw("  Agent Systems Benchmark"),
    ]))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, regions[0]);

    if model.layout == "wide" && regions[1].width >= 72 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(regions[1]);
        frame.render_widget(status_panel(state, policy), columns[0]);
        frame.render_widget(boundary_panel(policy), columns[1]);
    } else {
        frame.render_widget(status_panel(state, policy), regions[1]);
    }

    frame.render_widget(
        Paragraph::new("q / Ctrl-C quit | runner continues independently")
            .alignment(Alignment::Center)
            .style(muted(policy)),
        regions[2],
    );
}

fn render_tiny(frame: &mut Frame<'_>, area: Rect, policy: RenderPolicy) {
    let text = if area.height > 1 {
        "ASB\nq quit"
    } else {
        "ASB | q quit"
    };
    frame.render_widget(Paragraph::new(text).style(accent(policy)), area);
}

fn status_panel(state: &AppState, policy: RenderPolicy) -> Paragraph<'static> {
    let model = state.frame_model();
    let lines = vec![
        labelled("Connection", model.connection, policy),
        labelled("Last event", model.last_event, policy),
        labelled("Layout", model.layout, policy),
    ];
    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::default().title(" Status ").borders(Borders::ALL))
}

fn boundary_panel(policy: RenderPolicy) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from("Benchmark execution and credentials stay in ASB."),
        Line::from("This frontend displays negotiated control events only."),
        Line::from("Closing the UI never owns or cancels a runner."),
    ])
    .style(muted(policy))
    .wrap(Wrap { trim: true })
    .block(Block::default().title(" Ownership ").borders(Borders::ALL))
}

fn labelled(label: &'static str, value: &'static str, policy: RenderPolicy) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), accent(policy)),
        Span::raw(value),
    ])
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
