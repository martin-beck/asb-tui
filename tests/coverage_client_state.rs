// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Coverage-only tests for reconnect and authenticated client state.

use asb_tui::control_client::{
    AdoptedChannel, ConnectionState, ControlLimits, ControlVersion, CursorToken, MeasurementReason,
    Negotiated, PeerCredentials, ReconnectCursor, RequestClass, RequestId, RequestLedger,
    RunnerIdentity, V1_0, V1_3, V1_5, ValidationCategory, ValidationIssue, validate_issue,
};

fn identity() -> RunnerIdentity {
    RunnerIdentity {
        uid: 1,
        pid: 2,
        service_generation: 3,
        runner_instance_id: "runner".into(),
    }
}

fn negotiated() -> Negotiated {
    Negotiated {
        version: V1_5,
        limits: ControlLimits::default(),
        runner: identity(),
        oldest_revision: 2,
        latest_revision: 5,
    }
}

#[test]
fn reconnect_state_preserves_cursor_and_abandons_mutations() {
    let channel = AdoptedChannel::new(
        3,
        PeerCredentials {
            uid: 1,
            pid: 2,
            service_generation: 3,
        },
    )
    .unwrap();
    let mut state = ConnectionState::adopt(channel, &identity()).unwrap();
    state = state
        .establish(
            AdoptedChannel::new(
                3,
                PeerCredentials {
                    uid: 1,
                    pid: 2,
                    service_generation: 3,
                },
            )
            .unwrap(),
            negotiated(),
            &[V1_0, V1_5].into_iter().collect(),
            ControlLimits::default(),
        )
        .unwrap();
    state.advance_cursor(3).unwrap();
    assert_eq!(state.cursor().revision(), Some(3));
    assert_eq!(state.disconnect().cursor().revision(), Some(3));
    assert_eq!(
        state.advance_cursor(3),
        Err(asb_tui::control_client::ClientError::NonMonotonicCursor)
    );

    let mut ledger = RequestLedger::default();
    let ledger_limits = ControlLimits {
        max_in_flight: 2,
        ..ControlLimits::default()
    };
    let read = RequestId::new("read").unwrap();
    let mutation = RequestId::new("mutation").unwrap();
    ledger
        .begin(read.clone(), RequestClass::ReadOnly, ledger_limits)
        .unwrap();
    ledger
        .begin(mutation.clone(), RequestClass::Mutation, ledger_limits)
        .unwrap();
    let plan = ledger.reconnect_plan();
    assert_eq!(plan.read_only, vec![read]);
    assert_eq!(plan.abandoned_mutations, vec![mutation]);
}

#[test]
fn cursor_and_issue_boundaries_reject_ambiguous_values() {
    let mut cursor = ReconnectCursor::default();
    let session = negotiated();
    assert_eq!(cursor.resume_after(&session).unwrap(), None);
    assert_eq!(
        cursor.advance(1, &session),
        Err(asb_tui::control_client::ClientError::InvalidCursor)
    );
    cursor.advance(2, &session).unwrap();
    assert_eq!(cursor.resume_after(&session).unwrap(), Some(2));
    assert_eq!(
        CursorToken::parse("01"),
        Err(asb_tui::control_client::ClientError::InvalidCursorToken)
    );

    let issue = ValidationIssue {
        category: ValidationCategory::MeasurementIssue,
        reason: Some(MeasurementReason::UnknownId),
        measurement_id: None,
    };
    assert!(validate_issue(V1_3, &issue).is_ok());
    assert!(validate_issue(V1_0, &issue).is_err());
    let unsupported = ControlVersion { major: 1, minor: 9 };
    assert!(validate_issue(unsupported, &issue).is_err());
}
