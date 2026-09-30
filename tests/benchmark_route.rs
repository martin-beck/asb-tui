use asb_tui::benchmark_route::{CampaignError, CampaignStage, GuidedCampaign, ReplayMode};

#[test]
fn guided_route_selects_then_launches_and_comparisons_are_after_completion() {
    let mut route = GuidedCampaign::new("quality").unwrap();
    route.add_agent("agent-a").unwrap();
    route.add_agent("agent-b").unwrap();
    route.add_measure("quality.correctness").unwrap();
    route.review().unwrap();
    let intent = route.start().unwrap();
    assert_eq!(intent.agents, ["agent-a", "agent-b"]);
    assert_eq!(route.stage, CampaignStage::Running);
    route.finish(true).unwrap();
    assert_eq!(route.stage, CampaignStage::Complete);
}

#[test]
fn offline_replay_is_explicit_and_has_no_live_fallback() {
    let mut route = GuidedCampaign::new("quality").unwrap();
    route.add_agent("agent-a").unwrap();
    route.add_measure("quality.correctness").unwrap();
    route.set_replay_mode(ReplayMode::OfflineReplay).unwrap();
    route.review().unwrap();
    assert_eq!(route.start(), Err(CampaignError::OfflineRecordingRequired));
    let intent = route.start_offline_replay("cassette-1").unwrap();
    assert!(intent.offline_only);
    assert_eq!(intent.recording_id, "cassette-1");
}
