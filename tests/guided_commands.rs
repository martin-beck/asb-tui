use std::process::Command;

const ROUTES: &[&str] = &[
    "setup",
    "benchmark",
    "recording",
    "replay",
    "comparison",
    "status",
    "doctor",
    "upgrade",
    "remove",
];

#[test]
fn every_guided_route_has_human_and_json_output() {
    for route in ROUTES {
        let human = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
            .arg(route)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(human.status.code(), Some(3), "{route} human exit");
        let text = String::from_utf8(human.stdout).unwrap();
        assert!(text.contains("Status: unavailable"), "{route} human status");
        assert!(text.contains("Next:"), "{route} human next action");
        assert!(!text.contains("password"));

        let json = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
            .args([route, "--json"])
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(json.status.code(), Some(3), "{route} json exit");
        let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["command"], *route);
        assert!(value["next_action"].is_string());
        assert!(!String::from_utf8(json.stdout).unwrap().contains("token"));
    }
}

#[test]
fn malformed_guided_options_are_rejected_without_payload() {
    let output = Command::new(env!("CARGO_BIN_EXE_asb-tui"))
        .args(["replay", "--recording", "private-secret"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("private-secret")
    );
}
