// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{agent_catalog::parse_agent_catalog_response, agent_lifecycle::*};

fn catalog(generation: u64) -> asb_tui::agent_catalog::AgentCatalog {
    let mut value = serde_json::json!({
    "jsonrpc":"2.0", "id":7, "result":{"kind":"operation","value":{"request_sha256":"f".repeat(64),"result":{"kind":"agent_catalog","value":{
    "runner_instance_id":"runner-1", "generation":generation, "catalog_sha256":"a".repeat(64),
    "target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
    "agents":[{"agent_id":"local-echo","target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
      "package":{"package_id":"pkg","version":"1.0.0","sha256":"b".repeat(64),"signature_sha256":"c".repeat(64)},
      "provenance":{"source_revision":"d".repeat(40),"manifest_sha256":"e".repeat(64)},
      "capabilities":["benchmark"],"availability":{"status":"available"}}],"refreshed":false
    }}}}});
    let raw = value["result"]["value"]["result"]["value"].clone();
    let mut catalog: asb_tui::agent_catalog::AgentCatalog = serde_json::from_value(raw).unwrap();
    catalog.catalog_sha256 = catalog.computed_digest().unwrap();
    value["result"]["value"]["result"]["value"] = serde_json::to_value(catalog).unwrap();
    parse_agent_catalog_response(&value.to_string()).unwrap()
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
            agent_id: "local-echo".into(),
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
            agent_id: "local-echo".into(),
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
            agent_id: "local-echo".into(),
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

#[test]
fn lifecycle_state_rejects_invalid_progress_totals_and_reason_text() {
    let ready = ready();
    for total in [0, MAX_PROGRESS_UNITS + 1] {
        assert!(
            ready
                .clone()
                .apply(LifecycleEvent::InstallRequested {
                    agent_id: "local-echo".into(),
                    total,
                })
                .is_err()
        );
    }
    let installing = ready
        .apply(LifecycleEvent::InstallRequested {
            agent_id: "local-echo".into(),
            total: 10,
        })
        .unwrap();
    assert!(
        installing
            .clone()
            .apply(LifecycleEvent::Progress {
                generation: 1,
                completed: 11,
                total: 10,
            })
            .is_err()
    );
    assert!(
        installing
            .clone()
            .apply(LifecycleEvent::Progress {
                generation: 1,
                completed: 0,
                total: 9,
            })
            .is_err()
    );
    assert!(
        installing
            .apply(LifecycleEvent::InstallFailed {
                generation: 1,
                reason: "\n".into(),
            })
            .is_err()
    );
}

#[test]
fn lifecycle_events_cover_retry_refresh_and_invalid_transitions() {
    let installing = ready()
        .apply(LifecycleEvent::InstallRequested {
            agent_id: "local-echo".into(),
            total: 1,
        })
        .unwrap();
    let failed = installing
        .apply(LifecycleEvent::InstallFailed {
            generation: 1,
            reason: "temporary failure".into(),
        })
        .unwrap();
    let refreshing = failed.apply(LifecycleEvent::RetryRequested).unwrap();
    assert!(matches!(
        refreshing,
        LifecycleState::Refreshing { generation: 1 }
    ));
    assert!(
        LifecycleState::Disconnected
            .apply(LifecycleEvent::RetryRequested)
            .is_err()
    );
    assert!(
        LifecycleState::Disconnected
            .apply(LifecycleEvent::CancelRequested)
            .is_err()
    );
    assert!(
        LifecycleState::Disconnected
            .apply(LifecycleEvent::RemoveRequested)
            .is_err()
    );
}

#[test]
fn unavailable_agents_and_reconnect_paths_are_fail_closed() {
    let mut unavailable_value = serde_json::json!({
    "jsonrpc":"2.0", "id":7, "result":{"kind":"operation","value":{"request_sha256":"f".repeat(64),"result":{"kind":"agent_catalog","value":{
        "runner_instance_id":"runner-1", "generation":1, "catalog_sha256":"a".repeat(64),
        "target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},
        "agents":[{"agent_id":"local-echo","target":{"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"2.35"},"package":{"package_id":"pkg","version":"1.0","sha256":"b".repeat(64),"signature_sha256":"c".repeat(64)},"provenance":{"source_revision":"d".repeat(40),"manifest_sha256":"e".repeat(64)},"capabilities":["benchmark"],"availability":{"status":"unavailable","reason":"policy_denied"}}],"refreshed":false
    }}}}});
    let unavailable_raw = unavailable_value["result"]["value"]["result"]["value"].clone();
    let mut unavailable_catalog: asb_tui::agent_catalog::AgentCatalog =
        serde_json::from_value(unavailable_raw).unwrap();
    unavailable_catalog.catalog_sha256 = unavailable_catalog.computed_digest().unwrap();
    unavailable_value["result"]["value"]["result"]["value"] =
        serde_json::to_value(unavailable_catalog).unwrap();
    let unavailable = parse_agent_catalog_response(&unavailable_value.to_string()).unwrap();
    let ready = LifecycleState::disconnected()
        .apply(LifecycleEvent::RefreshRequested)
        .unwrap()
        .apply(LifecycleEvent::CatalogAccepted(unavailable))
        .unwrap();
    assert!(
        ready
            .clone()
            .apply(LifecycleEvent::InstallRequested {
                agent_id: "local-echo".into(),
                total: 1,
            })
            .is_err()
    );
    let reconnecting = ready.apply(LifecycleEvent::ReconnectRequested).unwrap();
    assert!(
        reconnecting
            .clone()
            .apply(LifecycleEvent::Reconnected(catalog(1)))
            .is_ok()
    );
}

#[test]
fn installed_state_can_refresh_or_reconnect_without_losing_generation() {
    let installed = ready()
        .apply(LifecycleEvent::InstallRequested {
            agent_id: "local-echo".into(),
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
    assert_eq!(
        installed
            .clone()
            .apply(LifecycleEvent::RefreshRequested)
            .unwrap(),
        LifecycleState::Refreshing { generation: 1 }
    );
    assert_eq!(
        installed.apply(LifecycleEvent::ReconnectRequested).unwrap(),
        LifecycleState::Reconnecting { generation: 1 }
    );
}
