// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{agent_catalog::parse_agent_catalog, agent_lifecycle::*};

fn catalog(generation: &str) -> asb_tui::agent_catalog::AgentCatalog {
    parse_agent_catalog(&serde_json::json!({
        "protocol":"asb-agent-catalog", "protocol_version":1,
        "runner_generation":generation, "catalog_digest":"a".repeat(64),
        "authentication":{"status":"verified","authorization":"agent-catalog-read"},
        "agents":[{"id":"local.echo","name":"Echo","version":"1.0.0",
          "targets":[{"operating_system":"linux","architecture":"x86_64"}],
          "package":{"format":"oci","digest":"b".repeat(64),"signature":"sig","provenance":"prov"},
          "capabilities":["benchmark"],"availability":{"state":"available","reason":null}}]
    }).to_string()).unwrap()
}

fn ready() -> LifecycleState {
    LifecycleState::disconnected()
        .apply(LifecycleEvent::RefreshRequested)
        .unwrap()
        .apply(LifecycleEvent::CatalogAccepted(catalog("g1")))
        .unwrap()
}

#[test]
fn install_requires_available_catalog_and_verified_completion() {
    let state = ready()
        .apply(LifecycleEvent::InstallRequested {
            agent_id: "local.echo".into(),
            total: 2,
        })
        .unwrap();
    let state = state
        .apply(LifecycleEvent::Progress {
            generation: "g1".into(),
            completed: 1,
            total: 2,
        })
        .unwrap();
    let state = state
        .apply(LifecycleEvent::Progress {
            generation: "g1".into(),
            completed: 2,
            total: 2,
        })
        .unwrap();
    assert!(matches!(
        state
            .apply(LifecycleEvent::InstallSucceeded {
                generation: "g1".into()
            })
            .unwrap(),
        LifecycleState::Installed { .. }
    ));
}

#[test]
fn cancellation_and_failure_are_generation_fenced() {
    let installing = ready()
        .apply(LifecycleEvent::InstallRequested {
            agent_id: "local.echo".into(),
            total: 1,
        })
        .unwrap();
    assert!(
        installing
            .clone()
            .apply(LifecycleEvent::InstallFailed {
                generation: "old".into(),
                reason: "network".into()
            })
            .is_err()
    );
    let cancelling = installing.apply(LifecycleEvent::CancelRequested).unwrap();
    assert!(
        cancelling
            .clone()
            .apply(LifecycleEvent::Cancelled {
                generation: "old".into()
            })
            .is_err()
    );
    assert_eq!(
        cancelling
            .apply(LifecycleEvent::Cancelled {
                generation: "g1".into()
            })
            .unwrap(),
        LifecycleState::Disconnected
    );
}

#[test]
fn success_remove_reconnect_and_stale_refresh_are_explicit() {
    let installed = ready()
        .apply(LifecycleEvent::InstallRequested {
            agent_id: "local.echo".into(),
            total: 1,
        })
        .unwrap()
        .apply(LifecycleEvent::Progress {
            generation: "g1".into(),
            completed: 1,
            total: 1,
        })
        .unwrap()
        .apply(LifecycleEvent::InstallSucceeded {
            generation: "g1".into(),
        })
        .unwrap();
    let removing = installed.apply(LifecycleEvent::RemoveRequested).unwrap();
    let reconnecting = removing
        .apply(LifecycleEvent::RemoveSucceeded {
            generation: "g1".into(),
        })
        .unwrap();
    assert!(matches!(reconnecting, LifecycleState::Reconnecting { .. }));
    let stale = reconnecting
        .apply(LifecycleEvent::Reconnected(catalog("g2")))
        .unwrap();
    assert_eq!(
        stale,
        LifecycleState::Stale {
            generation: "g2".into()
        }
    );
}

#[test]
fn invalid_events_do_not_mutate_state() {
    let state = ready();
    let error = state
        .clone()
        .apply(LifecycleEvent::InstallRequested {
            agent_id: "missing".into(),
            total: 1,
        })
        .unwrap_err();
    assert_eq!(error, "unknown agent id");
    assert_eq!(state, ready());
    assert!(
        LifecycleState::Disconnected
            .apply(LifecycleEvent::CatalogAccepted(catalog("g1")))
            .is_err()
    );
}

#[test]
fn refresh_keeps_generation_fence_and_marks_replayed_catalog_stale() {
    let refreshing = ready().apply(LifecycleEvent::RefreshRequested).unwrap();
    let stale = refreshing
        .apply(LifecycleEvent::CatalogAccepted(catalog("old")))
        .unwrap();
    assert_eq!(
        stale,
        LifecycleState::Stale {
            generation: "old".into()
        }
    );
}
