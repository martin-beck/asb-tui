// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{agent_catalog::parse_agent_catalog_response, agent_lifecycle::*};

fn catalog(generation: u64) -> asb_tui::agent_catalog::AgentCatalog {
    parse_agent_catalog_response(&serde_json::json!({
        "jsonrpc":"2.0", "id":7, "result":{"kind":"operation","value":{"request_sha256":"f".repeat(64),"result":{"kind":"agent_catalog","value":{
        "runner_instance_id":"runner-1", "generation":generation, "catalog_sha256":"a".repeat(64),
        "target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
        "agents":[{"agent_id":"local.echo","target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
          "package":{"package_id":"pkg","version":"1.0.0","sha256":"b".repeat(64),"signature_sha256":"c".repeat(64)},
          "provenance":{"source_revision":"d".repeat(40),"manifest_sha256":"e".repeat(64)},
          "capabilities":["benchmark"],"availability":{"status":"available"}}],"refreshed":false
        }}}}}).to_string()).unwrap()
}

fn ready() -> LifecycleState {
    LifecycleState::disconnected()
        .apply(LifecycleEvent::RefreshRequested)
        .unwrap()
        .apply(LifecycleEvent::CatalogAccepted(catalog(1)))
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
            generation: 1,
            completed: 1,
            total: 2,
        })
        .unwrap();
    let state = state
        .apply(LifecycleEvent::Progress {
            generation: 1,
            completed: 2,
            total: 2,
        })
        .unwrap();
    assert!(matches!(
        state
            .apply(LifecycleEvent::InstallSucceeded { generation: 1 })
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
                generation: 99,
                reason: "network".into()
            })
            .is_err()
    );
    let cancelling = installing.apply(LifecycleEvent::CancelRequested).unwrap();
    assert!(
        cancelling
            .clone()
            .apply(LifecycleEvent::Cancelled { generation: 99 })
            .is_err()
    );
    assert_eq!(
        cancelling
            .apply(LifecycleEvent::Cancelled { generation: 1 })
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
            generation: 1,
            completed: 1,
            total: 1,
        })
        .unwrap()
        .apply(LifecycleEvent::InstallSucceeded { generation: 1 })
        .unwrap();
    let removing = installed.apply(LifecycleEvent::RemoveRequested).unwrap();
    let reconnecting = removing
        .apply(LifecycleEvent::RemoveSucceeded { generation: 1 })
        .unwrap();
    assert!(matches!(reconnecting, LifecycleState::Reconnecting { .. }));
    let stale = reconnecting
        .apply(LifecycleEvent::Reconnected(catalog(2)))
        .unwrap();
    assert_eq!(stale, LifecycleState::Stale { generation: 2 });
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
            .apply(LifecycleEvent::CatalogAccepted(catalog(1)))
            .is_err()
    );
}

#[test]
fn refresh_keeps_generation_fence_and_marks_replayed_catalog_stale() {
    let refreshing = ready().apply(LifecycleEvent::RefreshRequested).unwrap();
    let stale = refreshing
        .apply(LifecycleEvent::CatalogAccepted(catalog(99)))
        .unwrap();
    assert_eq!(stale, LifecycleState::Stale { generation: 99 });
}
