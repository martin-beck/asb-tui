// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::configuration::*;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_dir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "asb-tui-config-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}

#[test]
fn defaults_are_valid_and_draft_cancel_preserves_state() {
    let defaults = Configuration::default();
    let mut draft = ConfigurationDraft::new(defaults.clone()).unwrap();
    draft.focus(Some("frontend.contrast"));
    draft
        .edit(|configuration| configuration.frontend.contrast = true)
        .unwrap();
    assert!(draft.is_dirty());
    draft.reset_focused().unwrap();
    assert!(!draft.is_dirty());
    draft.set_query("refresh").unwrap();
    assert_eq!(draft.visible_settings().len(), 1);
    draft.cancel();
    assert_eq!(draft.current(), &defaults);
}

#[test]
fn apply_commits_draft_and_reset_all_requires_confirmation() {
    let mut draft = ConfigurationDraft::new(Configuration::default()).unwrap();
    assert!(draft.reset_all(false).is_err());
    assert!(draft.reset_all(true).is_ok());
    let applied = draft.apply().unwrap();
    assert_eq!(applied, Configuration::default());
    assert!(!draft.is_dirty());
}

#[test]
fn import_rejects_unknown_secret_oversize_and_control_text() {
    assert!(matches!(
        Configuration::from_json(
            r#"{"schema_version":1,"frontend":{"theme":"dark","contrast":false,"motion":"full","refresh_interval_ms":1000,"show_key_hints":true,"landing":"home"},"benchmark":{"agents":[],"workloads":[],"measures":[],"repetitions":1,"resource_preset":"balanced"},"api_token":"x"}"#
        ),
        Err(ConfigError::SecretInput)
    ));
    assert!(matches!(
        Configuration::from_json(&"x".repeat(MAX_FILE_BYTES + 1)),
        Err(ConfigError::TooLarge)
    ));
    let mut value = Configuration::default();
    value.benchmark.resource_preset = "bad\nvalue".into();
    assert!(value.validate().is_err());
    let nested = "[".repeat(MAX_NESTING_DEPTH + 1);
    assert!(matches!(
        Configuration::from_json(&nested),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn unicode_scalar_bounds_are_not_byte_bounds() {
    let mut draft = ConfigurationDraft::new(Configuration::default()).unwrap();
    let query = "界".repeat(MAX_QUERY_SCALARS);
    draft.set_query(&query).unwrap();
    assert!(draft.set_query(&format!("{query}x")).is_err());
}

#[test]
fn reset_section_is_not_limited_by_search_results() {
    let mut draft = ConfigurationDraft::new(Configuration::default()).unwrap();
    draft.set_query("theme").unwrap();
    draft.reset_section("Navigation").unwrap();
    assert_eq!(draft.current().frontend.landing, LandingDestination::Home);
}

#[test]
fn store_round_trip_is_atomic_and_symlinks_are_refused() {
    let dir = temp_dir();
    let path = dir.join("config.json");
    let store = ConfigurationStore::new(&path);
    store.save(&Configuration::default()).unwrap();
    assert_eq!(store.load().unwrap(), Configuration::default());
    let link = dir.join("link.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&path, &link).unwrap();
    #[cfg(unix)]
    assert_eq!(
        ConfigurationStore::new(&link).load(),
        Err(ConfigError::SymlinkRefused)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn capability_gate_is_fail_closed() {
    let descriptor = SETTING_DESCRIPTORS
        .iter()
        .find(|setting| setting.id == "benchmark")
        .unwrap();
    assert!(!capability_availability(descriptor, false, true, true).available);
    assert!(
        capability_availability(descriptor, false, true, true)
            .reason
            .is_some()
    );
}
