// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! AR-1595's selection-driven record/seal/replay/compare acceptance.
//!
//! This is deliberately an in-process development fixture: it proves the
//! frontend route and its bounded diagnostics without contacting a provider.

use asb_tui::{
    benchmark_route::{GuidedCampaign, GuidedCatalog, ReplayCatalog, ReplayChoice, ReplayMode},
    output_contract::{Route, render},
    recording_campaign::{
        CampaignObservation, CampaignPhase, Coverage, RecordingAction, RecordingCampaignModel,
        WorkloadScope,
    },
};

#[test]
fn selected_record_seal_replay_compare_route_is_explicit_and_renderer_neutral() {
    let catalog = GuidedCatalog::new(
        vec!["workload-a".into(), "workload-b".into()],
        vec!["agent-a".into(), "agent-b".into()],
        vec!["latency".into()],
    )
    .unwrap();
    let mut live = GuidedCampaign::with_catalog("workload-a", catalog.clone()).unwrap();
    live.add_agent("agent-a").unwrap();
    live.add_measure("latency").unwrap();
    live.set_replay_mode(ReplayMode::Record).unwrap();
    live.review().unwrap();
    assert_eq!(live.start().unwrap().agents, ["agent-a"]);
    live.finish(true).unwrap();

    let planned = CampaignObservation {
        generation: 1,
        campaign_id: Some("campaign-a".into()),
        phase: CampaignPhase::Planned,
        coverage: Coverage {
            requested: 1,
            complete: 0,
        },
        offline_ready: false,
    };
    let mut model =
        RecordingCampaignModel::new(WorkloadScope::Selected(vec!["workload-a".into()]), planned)
            .unwrap();
    model.arm_capture_confirmation().unwrap();
    assert_eq!(
        model
            .request(RecordingAction::ConfirmCapture)
            .unwrap()
            .action,
        RecordingAction::ConfirmCapture
    );
    model
        .apply(CampaignObservation {
            generation: 2,
            campaign_id: Some("campaign-a".into()),
            phase: CampaignPhase::Complete,
            coverage: Coverage {
                requested: 1,
                complete: 1,
            },
            offline_ready: true,
        })
        .unwrap();
    assert_eq!(
        model
            .request(RecordingAction::ActivateOfflineDefault)
            .unwrap()
            .action,
        RecordingAction::ActivateOfflineDefault
    );

    let replay = GuidedCampaign::with_catalog("workload-a", catalog).unwrap();
    let replay_catalog = ReplayCatalog::new(
        "b".repeat(64),
        "agent-a",
        vec![ReplayChoice {
            cassette_id: "cassette-a".into(),
            cassette_sha256: "a".repeat(64),
        }],
    )
    .unwrap();
    let mut replay = replay;
    replay.add_agent("agent-a").unwrap();
    replay.add_measure("latency").unwrap();
    replay.set_replay_mode(ReplayMode::OfflineReplay).unwrap();
    replay.bind_provider_profile_sha256("b".repeat(64)).unwrap();
    replay.review().unwrap();
    let intent = replay
        .start_offline_replay_from_catalog(&replay_catalog, &"a".repeat(64))
        .unwrap();
    assert!(intent.offline_only);
    assert!(render(Route::Recording, false).contains("recording"));
    assert!(render(Route::Replay, true).contains("\"command\":\"replay\""));
}

#[test]
fn replay_and_capture_fail_closed_for_missing_selection_or_incomplete_coverage() {
    let catalog = GuidedCatalog::new(
        vec!["workload".into()],
        vec!["agent".into()],
        vec!["measure".into()],
    )
    .unwrap();
    let mut replay = GuidedCampaign::with_catalog("workload", catalog).unwrap();
    replay.set_replay_mode(ReplayMode::OfflineReplay).unwrap();
    assert!(replay.review().is_err());

    let mut model = RecordingCampaignModel::new(
        WorkloadScope::Selected(vec!["workload".into()]),
        CampaignObservation {
            generation: 1,
            campaign_id: Some("campaign".into()),
            phase: CampaignPhase::Recording,
            coverage: Coverage {
                requested: 2,
                complete: 1,
            },
            offline_ready: false,
        },
    )
    .unwrap();
    assert!(
        model
            .request(RecordingAction::ActivateOfflineDefault)
            .is_err()
    );
}
