// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Pure Ratatui projection of immutable application state.

use crate::{
    app::AppState,
    landing::{
        ActivityStatus, ConnectionStatus, Destination, Freshness, LandingProjection, PrimaryAction,
        TrustStatus,
    },
    shell::{DisabledReason, Route, RouteAvailability},
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
    .block(panel_block("", policy));
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

/// Render the standalone application's state-aware home screen.
///
/// The projection is deliberately supplied by the control-client boundary;
/// this function only formats bounded public fields and never queries ASB or
/// dispatches a destination.  Callers can therefore test the landing screen
/// without a runner, socket, or benchmark payload.
pub fn render_landing(frame: &mut Frame<'_>, projection: &LandingProjection, policy: RenderPolicy) {
    let area = frame.area();
    if area.width < 28 || area.height < 8 {
        render_landing_tiny(frame, area, projection, policy);
        return;
    }

    let regions = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled("ASB", accent(policy)),
        Span::raw("  Agent Systems Benchmark"),
    ]))
    .alignment(Alignment::Center)
    .block(panel_block(" Home ", policy));
    frame.render_widget(title, regions[0]);
    frame.render_widget(primary_panel(projection, policy), regions[1]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
        .split(regions[2]);
    frame.render_widget(recent_panel(projection, policy), body[0]);
    frame.render_widget(destination_panel(&projection.destinations, policy), body[1]);

    frame.render_widget(
        Paragraph::new(navigation_hint(policy))
            .alignment(Alignment::Center)
            .style(muted(policy)),
        regions[3],
    );
}

/// Stable plain-text landing equivalent for pipes and unknown terminals.
pub fn landing_plain_text(projection: &LandingProjection) -> String {
    let mut output = format!(
        "Agent Systems Benchmark\nconnection: {} | trust: {} | freshness: {}\nnext: {}\nrecent runs:\n",
        connection_name(projection.connection),
        trust_name(projection.trust),
        freshness_name(projection.freshness),
        primary_name(projection.primary_action),
    );
    if projection.recent_activity.is_empty() {
        output.push_str("  none\n");
    } else {
        for item in &projection.recent_activity {
            output.push_str(&format!(
                "  {} | {} | {}\n",
                item.run_id,
                item.label,
                activity_name(item.status)
            ));
        }
    }
    output.push_str("shortcuts: up/down navigate | Enter select | ? help | q quit\n");
    output
}

fn render_landing_tiny(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &LandingProjection,
    policy: RenderPolicy,
) {
    let primary = primary_name(projection.primary_action);
    let text = if area.height > 2 {
        format!("ASB\nnext: {primary}\n? help | q quit")
    } else if area.height > 1 {
        format!("ASB\n{primary}")
    } else {
        "ASB | ? help | q quit".to_owned()
    };
    frame.render_widget(Paragraph::new(text).style(accent(policy)), area);
}

fn primary_panel(projection: &LandingProjection, policy: RenderPolicy) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Next action: ", accent(policy)),
            Span::raw(primary_name(projection.primary_action)),
        ]),
        Line::from(format!(
            "Connection: {}  Trust: {}  Freshness: {}",
            connection_name(projection.connection),
            trust_name(projection.trust),
            freshness_name(projection.freshness)
        )),
    ])
    .wrap(Wrap { trim: true })
    .block(panel_block(" Ready when you are ", policy))
}

fn recent_panel(projection: &LandingProjection, policy: RenderPolicy) -> Paragraph<'static> {
    let mut lines = vec![Line::from(Span::styled("Latest activity", accent(policy)))];
    if projection.recent_activity.is_empty() {
        lines.push(Line::from("No runs yet — configure a benchmark to begin."));
    } else {
        lines.extend(projection.recent_activity.iter().map(|item| {
            Line::from(format!(
                "{}  {}  [{}]",
                item.run_id,
                item.label,
                activity_name(item.status)
            ))
        }));
    }
    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(panel_block(" Recent runs ", policy))
}

fn destination_panel(destinations: &[Destination], policy: RenderPolicy) -> Paragraph<'static> {
    let mut lines = vec![Line::from(Span::styled("Workspaces", accent(policy)))];
    for destination in destinations {
        let marker = match destination.availability {
            RouteAvailability::Available => "•",
            RouteAvailability::Disabled(_) => "×",
        };
        lines.push(Line::from(format!(
            "{marker} {}{}",
            route_name(destination.route),
            disabled_suffix(destination.availability)
        )));
    }
    Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(panel_block(" Navigate ", policy))
}

fn disabled_suffix(availability: RouteAvailability) -> String {
    match availability {
        RouteAvailability::Available => String::new(),
        RouteAvailability::Disabled(reason) => format!(" ({})", disabled_name(reason)),
    }
}

fn route_name(route: Route) -> &'static str {
    match route {
        Route::Landing => "Home",
        Route::Configuration => "Configure",
        Route::MeasurementSelection => "Measures",
        Route::RunControl => "Run control",
        Route::RecentRuns => "Recent runs",
        Route::Reports => "Reports",
        Route::Help => "Help",
    }
}

fn disabled_name(reason: DisabledReason) -> &'static str {
    match reason {
        DisabledReason::NotNegotiated => "not connected",
        DisabledReason::Analysis => "analysis unavailable",
        DisabledReason::Cancel => "cancel unavailable",
        DisabledReason::Events => "events unavailable",
        DisabledReason::History => "history unavailable",
        DisabledReason::Launch => "launch unavailable",
        DisabledReason::Planning => "planning unavailable",
    }
}

fn primary_name(action: PrimaryAction) -> &'static str {
    match action {
        PrimaryAction::InstallOrConnect => "Install or connect",
        PrimaryAction::Reconnect => "Reconnect",
        PrimaryAction::Monitor => "Monitor active run",
        PrimaryAction::ContinueReview => "Continue draft review",
        PrimaryAction::Configure => "Configure a benchmark",
    }
}

fn connection_name(value: ConnectionStatus) -> &'static str {
    match value {
        ConnectionStatus::Unavailable => "unavailable",
        ConnectionStatus::Disconnected => "disconnected",
        ConnectionStatus::Negotiating => "connecting",
        ConnectionStatus::Connected => "connected",
    }
}

fn trust_name(value: TrustStatus) -> &'static str {
    match value {
        TrustStatus::Unknown => "unknown",
        TrustStatus::Untrusted => "untrusted",
        TrustStatus::Trusted => "trusted",
    }
}

fn freshness_name(value: Freshness) -> &'static str {
    match value {
        Freshness::Unknown => "unknown",
        Freshness::Stale => "stale",
        Freshness::Fresh => "fresh",
    }
}

fn activity_name(value: ActivityStatus) -> &'static str {
    match value {
        ActivityStatus::Running => "running",
        ActivityStatus::Succeeded => "succeeded",
        ActivityStatus::Failed => "failed",
        ActivityStatus::Cancelled => "cancelled",
        ActivityStatus::Draft => "draft",
    }
}

fn navigation_hint(policy: RenderPolicy) -> &'static str {
    if policy.unicode {
        "↑/↓ navigate  Enter select  ? help  q quit"
    } else {
        "up/down navigate  Enter select  ? help  q quit"
    }
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
        .block(panel_block(" Status ", policy))
}

fn boundary_panel(policy: RenderPolicy) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from("Benchmark execution and credentials stay in ASB."),
        Line::from("This frontend displays negotiated control events only."),
        Line::from("Closing the UI never owns or cancels a runner."),
    ])
    .style(muted(policy))
    .wrap(Wrap { trim: true })
    .block(panel_block(" Ownership ", policy))
}

fn panel_block(title: &'static str, policy: RenderPolicy) -> Block<'static> {
    let block = Block::default().title(title);
    if policy.unicode {
        block.borders(Borders::ALL)
    } else {
        block
    }
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
