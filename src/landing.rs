// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-independent landing-screen projection.
//!
//! This module deliberately owns state and decisions, not terminal widgets.
//! A renderer may turn [`LandingProjection`] into a screen, plain text, or a
//! test snapshot without giving this module authority over any destination.

use crate::{
    Capabilities,
    shell::{Route, RouteAvailability},
};
use std::cmp::Reverse;

/// Maximum number of summaries retained by the landing screen.
pub const MAX_RECENT_ACTIVITY: usize = 5;
/// Maximum length of each public summary field, measured in Unicode scalars.
pub const MAX_SUMMARY_FIELD_SCALARS: usize = 160;
/// Minimum interval between accepted authoritative activity refreshes.
pub const MIN_REFRESH_INTERVAL_MS: u64 = 1_000;
const MAX_INPUT_ACTIVITY: usize = 1_024;

/// Closed connection states projected from the authenticated control client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionStatus {
    Unavailable,
    Disconnected,
    Negotiating,
    Connected,
}

/// Whether the source of a landing projection is trusted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustStatus {
    Unknown,
    Untrusted,
    Trusted,
}

/// Freshness of the authoritative state snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    Unknown,
    Stale,
    Fresh,
}

/// Public status retained for one recent-run summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Draft,
}

/// One privacy-reviewed activity item received from authoritative state.
///
/// Only these bounded public fields may reach the landing renderer. Endpoint
/// details, benchmark payloads, prompts, paths, and credentials are not part
/// of this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityRecord {
    pub run_id: String,
    pub label: String,
    pub status: ActivityStatus,
    pub observed_at: u64,
    pub cursor: u64,
}

/// A bounded activity summary safe for rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivitySummary {
    pub run_id: String,
    pub label: String,
    pub status: ActivityStatus,
    pub observed_at: u64,
}

/// Active-run state used only to choose the next safe landing action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    None,
    Active,
    ResumableDraft,
}

/// Stable primary action selected by authoritative state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrimaryAction {
    InstallOrConnect,
    Reconnect,
    Monitor,
    ContinueReview,
    Configure,
}

/// A destination shortcut and its closed capability gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Destination {
    pub route: Route,
    pub availability: RouteAvailability,
}

/// Inputs supplied by the control-client projection; no renderer state is
/// accepted here, so stale frontend caches cannot masquerade as authority.
pub struct LandingInput {
    pub connection: ConnectionStatus,
    pub trust: TrustStatus,
    pub freshness: Freshness,
    pub run: RunState,
    pub activity: Vec<ActivityRecord>,
    pub capabilities: Option<Capabilities>,
}

/// Immutable content for a landing renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LandingProjection {
    pub connection: ConnectionStatus,
    pub trust: TrustStatus,
    pub freshness: Freshness,
    pub primary_action: PrimaryAction,
    pub recent_activity: Vec<ActivitySummary>,
    pub destinations: Vec<Destination>,
}

/// Errors that prevent an authoritative landing projection from being used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LandingError {
    TooManyActivityRecords,
    EmptyRunId,
    InvalidRunId,
    InvalidPublicText,
    DuplicateRunId,
    InvalidCursor,
    ContradictoryOrder,
}

/// Build a deterministic landing projection from an authoritative snapshot.
pub fn project(input: LandingInput) -> Result<LandingProjection, LandingError> {
    if input.activity.len() > MAX_INPUT_ACTIVITY {
        return Err(LandingError::TooManyActivityRecords);
    }

    let mut records = input.activity;
    let mut seen = std::collections::BTreeSet::new();
    let mut summaries = Vec::with_capacity(records.len().min(MAX_RECENT_ACTIVITY));
    for record in &records {
        validate_run_id(&record.run_id)?;
        if record.cursor == 0 {
            return Err(LandingError::InvalidCursor);
        }
        if !seen.insert(record.run_id.as_str()) {
            return Err(LandingError::DuplicateRunId);
        }
        summaries.push(ActivitySummary {
            run_id: record.run_id.clone(),
            label: bounded_public_text(&record.label)?,
            status: record.status,
            observed_at: record.observed_at,
        });
    }

    // A durable cursor is monotonic with authoritative event time. A source
    // claiming the opposite cannot be safely presented as "most recent".
    for left in &records {
        for right in &records {
            if left.observed_at > right.observed_at && left.cursor < right.cursor {
                return Err(LandingError::ContradictoryOrder);
            }
        }
    }

    // Time and cursor are authoritative; run ID is only the deterministic
    // tie-break. Reverse gives newest-first ordering without local list order.
    records
        .sort_by_key(|record| Reverse((record.observed_at, record.cursor, record.run_id.clone())));
    let order: std::collections::BTreeMap<&str, usize> = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.run_id.as_str(), index))
        .collect();
    summaries.sort_by_key(|summary| order.get(summary.run_id.as_str()).copied());
    summaries.truncate(MAX_RECENT_ACTIVITY);

    let primary_action = choose_primary(input.connection, input.trust, input.freshness, input.run);
    let capabilities = input.capabilities.as_ref();
    let destinations = [
        Route::Landing,
        Route::Configuration,
        Route::MeasurementSelection,
        Route::RunControl,
        Route::RecentRuns,
        Route::Reports,
        Route::Help,
    ]
    .into_iter()
    .map(|route| Destination {
        route,
        availability: route.availability(capabilities),
    })
    .collect();

    Ok(LandingProjection {
        connection: input.connection,
        trust: input.trust,
        freshness: input.freshness,
        primary_action,
        recent_activity: summaries,
        destinations,
    })
}

fn choose_primary(
    connection: ConnectionStatus,
    trust: TrustStatus,
    freshness: Freshness,
    run: RunState,
) -> PrimaryAction {
    if matches!(connection, ConnectionStatus::Unavailable)
        || matches!(trust, TrustStatus::Unknown | TrustStatus::Untrusted)
    {
        return PrimaryAction::InstallOrConnect;
    }
    if matches!(connection, ConnectionStatus::Disconnected)
        || matches!(freshness, Freshness::Unknown | Freshness::Stale)
    {
        return PrimaryAction::Reconnect;
    }
    match run {
        RunState::None => PrimaryAction::Configure,
        RunState::Active => PrimaryAction::Monitor,
        RunState::ResumableDraft => PrimaryAction::ContinueReview,
    }
}

fn validate_run_id(value: &str) -> Result<(), LandingError> {
    if value.is_empty() {
        return Err(LandingError::EmptyRunId);
    }
    if value.chars().count() > MAX_SUMMARY_FIELD_SCALARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(LandingError::InvalidRunId);
    }
    Ok(())
}

fn bounded_public_text(value: &str) -> Result<String, LandingError> {
    if value.chars().any(char::is_control) {
        return Err(LandingError::InvalidPublicText);
    }
    Ok(value.chars().take(MAX_SUMMARY_FIELD_SCALARS).collect())
}

/// Monotonic gate preventing refresh storms and backwards authoritative time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActivityRefreshGate {
    last_refresh_ms: Option<u64>,
    last_response_id: Option<u64>,
}

impl ActivityRefreshGate {
    /// Whether a refresh may be requested at this monotonic timestamp.
    pub fn can_refresh(&self, now_ms: u64) -> bool {
        self.last_refresh_ms
            .is_none_or(|last| now_ms >= last.saturating_add(MIN_REFRESH_INTERVAL_MS))
    }

    /// Record an accepted refresh, rejecting clock rollback and rate abuse.
    pub fn accept(&mut self, now_ms: u64) -> bool {
        if !self.can_refresh(now_ms) {
            return false;
        }
        self.last_refresh_ms = Some(now_ms);
        true
    }

    /// Accept only a response newer than the last applied request response.
    /// Older responses are superseded even if they arrive after a refresh.
    pub fn accept_response(&mut self, response_id: u64) -> bool {
        if self
            .last_response_id
            .is_some_and(|last| response_id <= last)
        {
            return false;
        }
        self.last_response_id = Some(response_id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::DisabledReason;

    fn record(id: &str, at: u64, cursor: u64) -> ActivityRecord {
        ActivityRecord {
            run_id: id.into(),
            label: format!("run {id}"),
            status: ActivityStatus::Succeeded,
            observed_at: at,
            cursor,
        }
    }

    fn input() -> LandingInput {
        LandingInput {
            connection: ConnectionStatus::Connected,
            trust: TrustStatus::Trusted,
            freshness: Freshness::Fresh,
            run: RunState::None,
            activity: Vec::new(),
            capabilities: None,
        }
    }

    #[test]
    fn primary_action_table_is_authoritative_and_fail_closed() {
        let cases = [
            (
                ConnectionStatus::Unavailable,
                TrustStatus::Trusted,
                Freshness::Fresh,
                RunState::Active,
                PrimaryAction::InstallOrConnect,
            ),
            (
                ConnectionStatus::Connected,
                TrustStatus::Untrusted,
                Freshness::Fresh,
                RunState::Active,
                PrimaryAction::InstallOrConnect,
            ),
            (
                ConnectionStatus::Disconnected,
                TrustStatus::Trusted,
                Freshness::Fresh,
                RunState::None,
                PrimaryAction::Reconnect,
            ),
            (
                ConnectionStatus::Negotiating,
                TrustStatus::Trusted,
                Freshness::Stale,
                RunState::None,
                PrimaryAction::Reconnect,
            ),
            (
                ConnectionStatus::Connected,
                TrustStatus::Trusted,
                Freshness::Fresh,
                RunState::Active,
                PrimaryAction::Monitor,
            ),
            (
                ConnectionStatus::Connected,
                TrustStatus::Trusted,
                Freshness::Fresh,
                RunState::ResumableDraft,
                PrimaryAction::ContinueReview,
            ),
            (
                ConnectionStatus::Connected,
                TrustStatus::Trusted,
                Freshness::Fresh,
                RunState::None,
                PrimaryAction::Configure,
            ),
        ];
        for (connection, trust, freshness, run, expected) in cases {
            let mut value = input();
            value.connection = connection;
            value.trust = trust;
            value.freshness = freshness;
            value.run = run;
            assert_eq!(project(value).unwrap().primary_action, expected);
        }
    }

    #[test]
    fn recent_activity_is_authoritative_deterministic_bounded_and_private() {
        let mut value = input();
        value.activity = (0..7)
            .map(|index| record(&format!("r{index}"), 10, index + 1))
            .collect();
        value.activity.reverse();
        let projection = project(value).unwrap();
        assert_eq!(projection.recent_activity.len(), MAX_RECENT_ACTIVITY);
        assert_eq!(projection.recent_activity[0].run_id, "r6");
        assert_eq!(projection.recent_activity[4].run_id, "r2");
        assert!(
            projection
                .recent_activity
                .iter()
                .all(|summary| summary.label.chars().count() <= MAX_SUMMARY_FIELD_SCALARS)
        );
    }

    #[test]
    fn equal_time_and_cursor_use_run_id_tie_break() {
        let mut value = input();
        value.activity = vec![record("z", 2, 4), record("a", 2, 4)];
        let projection = project(value).unwrap();
        assert_eq!(
            projection
                .recent_activity
                .iter()
                .map(|summary| summary.run_id.as_str())
                .collect::<Vec<_>>(),
            ["z", "a"]
        );
    }

    #[test]
    fn invalid_or_contradictory_input_fails_closed() {
        let mut value = input();
        value.activity = vec![record("same", 2, 1), record("same", 3, 2)];
        assert_eq!(project(value), Err(LandingError::DuplicateRunId));
        let mut value = input();
        value.activity = vec![record("r", 2, 0)];
        assert_eq!(project(value), Err(LandingError::InvalidCursor));
        let mut value = input();
        value.activity = vec![record("new", 3, 1), record("old", 2, 2)];
        assert_eq!(project(value), Err(LandingError::ContradictoryOrder));
        let mut value = input();
        value.activity = vec![ActivityRecord {
            label: "bad\ntext".into(),
            ..record("r", 2, 1)
        }];
        assert_eq!(project(value), Err(LandingError::InvalidPublicText));
    }

    #[test]
    fn route_registry_is_visible_but_unavailable_routes_are_disabled() {
        let projection = project(input()).unwrap();
        let reports = projection
            .destinations
            .iter()
            .find(|destination| destination.route == Route::Reports)
            .unwrap();
        assert_eq!(
            reports.availability,
            RouteAvailability::Disabled(DisabledReason::NotNegotiated)
        );
    }

    #[test]
    fn refresh_gate_enforces_one_second_and_monotonicity() {
        let mut gate = ActivityRefreshGate::default();
        assert!(gate.accept(10));
        assert!(!gate.accept(1_009));
        assert!(gate.accept(1_010));
        assert!(!gate.accept(1_000));
        assert!(gate.accept_response(2));
        assert!(!gate.accept_response(1));
        assert!(!gate.accept_response(2));
    }
}
