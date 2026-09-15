// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral configuration center state for AR-1034.
//!
//! This module owns bounded, non-secret frontend preferences and benchmark-plan
//! drafts.  It intentionally has no terminal, widget, renderer, credential, or
//! runner-mutation authority.  A renderer can project [`ConfigurationDraft`]
//! and [`SettingDescriptor`] into any screen without changing the persistence
//! contract.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_SETTINGS: usize = 256;
pub const MAX_STRING_SCALARS: usize = 4_096;
pub const MAX_QUERY_SCALARS: usize = 256;
pub const MAX_LIST_ITEMS: usize = 256;
pub const MAX_NESTING_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    Dark,
    Light,
    HighContrast,
    NoColor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreference {
    Full,
    Reduced,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LandingDestination {
    Home,
    Runs,
    Measures,
    Reports,
    Configuration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontendPreferences {
    pub theme: ThemePreference,
    pub contrast: bool,
    pub motion: MotionPreference,
    pub refresh_interval_ms: u64,
    pub show_key_hints: bool,
    pub landing: LandingDestination,
}

impl Default for FrontendPreferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::Dark,
            contrast: false,
            motion: MotionPreference::Full,
            refresh_interval_ms: 1_000,
            show_key_hints: true,
            landing: LandingDestination::Home,
        }
    }
}

/// A runner-independent plan draft. Measure IDs are opaque canonical IDs; this
/// type does not dispatch provider or benchmark configuration mutations.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkDefaults {
    pub agents: Vec<String>,
    pub workloads: Vec<String>,
    pub measures: Vec<String>,
    pub repetitions: u32,
    pub resource_preset: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema_version: u32,
    pub frontend: FrontendPreferences,
    pub benchmark: BenchmarkDefaults,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            frontend: FrontendPreferences::default(),
            benchmark: BenchmarkDefaults {
                repetitions: 1,
                resource_preset: "balanced".into(),
                ..Default::default()
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingCapability {
    Always,
    History,
    Planning,
    Events,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityAvailability {
    pub available: bool,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub help: &'static str,
    pub capability: SettingCapability,
    pub restart_required: bool,
}

pub const SETTING_DESCRIPTORS: &[SettingDescriptor] = &[
    SettingDescriptor {
        id: "frontend.theme",
        label: "Theme",
        category: "Appearance",
        help: "Choose the visual theme",
        capability: SettingCapability::Always,
        restart_required: false,
    },
    SettingDescriptor {
        id: "frontend.contrast",
        label: "High contrast",
        category: "Appearance",
        help: "Increase non-colour emphasis",
        capability: SettingCapability::Always,
        restart_required: false,
    },
    SettingDescriptor {
        id: "frontend.motion",
        label: "Motion",
        category: "Appearance",
        help: "Limit animation and motion",
        capability: SettingCapability::Always,
        restart_required: false,
    },
    SettingDescriptor {
        id: "frontend.refresh_interval_ms",
        label: "Refresh cadence",
        category: "Behaviour",
        help: "How often authoritative status is refreshed",
        capability: SettingCapability::Events,
        restart_required: false,
    },
    SettingDescriptor {
        id: "frontend.show_key_hints",
        label: "Key hints",
        category: "Navigation",
        help: "Show context-sensitive keyboard hints",
        capability: SettingCapability::Always,
        restart_required: false,
    },
    SettingDescriptor {
        id: "frontend.landing",
        label: "Landing destination",
        category: "Navigation",
        help: "Choose the first destination",
        capability: SettingCapability::Always,
        restart_required: false,
    },
    SettingDescriptor {
        id: "benchmark",
        label: "Benchmark defaults",
        category: "Benchmark",
        help: "Prepare a reusable benchmark plan",
        capability: SettingCapability::Planning,
        restart_required: false,
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    TooLarge,
    TooManySettings,
    UnsupportedSchema,
    Invalid(String),
    SecretInput,
    Io(String),
    NotRegularFile,
    SymlinkRefused,
}

impl From<io::Error> for ConfigError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind().to_string())
    }
}

fn bounded_string(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.chars().count() > MAX_STRING_SCALARS {
        return Err(ConfigError::Invalid(format!("{field} is too long")));
    }
    if value.chars().any(char::is_control) {
        return Err(ConfigError::Invalid(format!(
            "{field} contains control text"
        )));
    }
    Ok(())
}

fn validate_list(field: &str, values: &[String]) -> Result<(), ConfigError> {
    if values.len() > MAX_LIST_ITEMS {
        return Err(ConfigError::Invalid(format!("{field} has too many items")));
    }
    for value in values {
        bounded_string(field, value)?;
    }
    Ok(())
}

impl Configuration {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchema);
        }
        if !(250..=86_400_000).contains(&self.frontend.refresh_interval_ms) {
            return Err(ConfigError::Invalid(
                "refresh_interval_ms is outside bounds".into(),
            ));
        }
        bounded_string("resource_preset", &self.benchmark.resource_preset)?;
        validate_list("agents", &self.benchmark.agents)?;
        validate_list("workloads", &self.benchmark.workloads)?;
        validate_list("measures", &self.benchmark.measures)?;
        if self.benchmark.repetitions == 0 || self.benchmark.repetitions > 10_000 {
            return Err(ConfigError::Invalid("repetitions is outside bounds".into()));
        }
        let count = self.benchmark.agents.len()
            + self.benchmark.workloads.len()
            + self.benchmark.measures.len()
            + 6;
        if count > MAX_SETTINGS {
            return Err(ConfigError::TooManySettings);
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, ConfigError> {
        self.validate()?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        if text.len() > MAX_FILE_BYTES {
            return Err(ConfigError::TooLarge);
        }
        Ok(text)
    }

    pub fn from_json(input: &str) -> Result<Self, ConfigError> {
        if input.len() > MAX_FILE_BYTES {
            return Err(ConfigError::TooLarge);
        }
        reject_excessive_nesting(input)?;
        reject_secret_shapes(input)?;
        let value: Value =
            serde_json::from_str(input).map_err(|error| ConfigError::Invalid(error.to_string()))?;
        let config: Self = serde_json::from_value(value)
            .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }
}

fn reject_secret_shapes(input: &str) -> Result<(), ConfigError> {
    let value: Value =
        serde_json::from_str(input).map_err(|error| ConfigError::Invalid(error.to_string()))?;
    fn walk(value: &Value) -> Result<(), ConfigError> {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let lower = key.to_ascii_lowercase();
                    if [
                        "secret",
                        "password",
                        "token",
                        "credential",
                        "api_key",
                        "private_key",
                    ]
                    .iter()
                    .any(|part| lower.contains(part))
                    {
                        return Err(ConfigError::SecretInput);
                    }
                    walk(child)?;
                }
            }
            Value::Array(values) => {
                for child in values {
                    walk(child)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(&value)
}

fn reject_excessive_nesting(input: &str) -> Result<(), ConfigError> {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in input.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return Err(ConfigError::Invalid(
                        "configuration nesting is too deep".into(),
                    ));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationDraft {
    original: Configuration,
    current: Configuration,
    query: String,
    focused: Option<&'static str>,
}

impl ConfigurationDraft {
    pub fn new(configuration: Configuration) -> Result<Self, ConfigError> {
        configuration.validate()?;
        Ok(Self {
            original: configuration.clone(),
            current: configuration,
            query: String::new(),
            focused: None,
        })
    }
    pub fn current(&self) -> &Configuration {
        &self.current
    }
    /// Apply a bounded draft edit transactionally. Invalid edits leave the
    /// previous draft untouched, including its dirty state.
    pub fn edit<F>(&mut self, edit: F) -> Result<(), ConfigError>
    where
        F: FnOnce(&mut Configuration),
    {
        let mut candidate = self.current.clone();
        edit(&mut candidate);
        candidate.validate()?;
        self.current = candidate;
        Ok(())
    }
    pub fn is_dirty(&self) -> bool {
        self.current != self.original
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn set_query(&mut self, query: &str) -> Result<(), ConfigError> {
        if query.chars().count() > MAX_QUERY_SCALARS || query.chars().any(char::is_control) {
            return Err(ConfigError::Invalid(
                "invalid configuration search query".into(),
            ));
        }
        self.query = query.to_string();
        Ok(())
    }
    pub fn focus(&mut self, id: Option<&'static str>) {
        self.focused = id;
    }
    pub fn focused(&self) -> Option<&'static str> {
        self.focused
    }
    pub fn cancel(&mut self) {
        self.current = self.original.clone();
    }
    pub fn reset_all(&mut self, confirmed: bool) -> Result<(), ConfigError> {
        if !confirmed {
            return Err(ConfigError::Invalid(
                "reset-all requires confirmation".into(),
            ));
        }
        self.current = Configuration::default();
        Ok(())
    }
    pub fn reset_section(&mut self, category: &str) -> Result<(), ConfigError> {
        match category {
            "Appearance" => self.current.frontend = FrontendPreferences::default(),
            "Navigation" => {
                self.current.frontend.show_key_hints = true;
                self.current.frontend.landing = LandingDestination::Home;
            }
            "Behaviour" => self.current.frontend.refresh_interval_ms = 1_000,
            "Benchmark" => self.current.benchmark = Configuration::default().benchmark,
            _ => {
                return Err(ConfigError::Invalid(
                    "unknown configuration category".into(),
                ));
            }
        }
        Ok(())
    }
    pub fn reset_focused(&mut self) -> Result<(), ConfigError> {
        match self.focused {
            Some("frontend.theme") => {
                self.current.frontend.theme = FrontendPreferences::default().theme
            }
            Some("frontend.contrast") => self.current.frontend.contrast = false,
            Some("frontend.motion") => {
                self.current.frontend.motion = FrontendPreferences::default().motion
            }
            Some("frontend.refresh_interval_ms") => {
                self.current.frontend.refresh_interval_ms = 1_000
            }
            Some("frontend.show_key_hints") => self.current.frontend.show_key_hints = true,
            Some("frontend.landing") => self.current.frontend.landing = LandingDestination::Home,
            Some("benchmark") => self.current.benchmark = Configuration::default().benchmark,
            _ => return Err(ConfigError::Invalid("no resettable setting focused".into())),
        }
        Ok(())
    }
    pub fn apply(&mut self) -> Result<Configuration, ConfigError> {
        self.current.validate()?;
        self.original = self.current.clone();
        Ok(self.current.clone())
    }
    pub fn visible_settings(&self) -> Vec<&'static SettingDescriptor> {
        let q = self.query.to_ascii_lowercase();
        SETTING_DESCRIPTORS
            .iter()
            .filter(|setting| {
                q.is_empty()
                    || [setting.id, setting.label, setting.category, setting.help]
                        .iter()
                        .any(|text| text.to_ascii_lowercase().contains(&q))
            })
            .collect()
    }
}

pub fn capability_availability(
    setting: &SettingDescriptor,
    planning: bool,
    history: bool,
    events: bool,
) -> CapabilityAvailability {
    let available = match setting.capability {
        SettingCapability::Always => true,
        SettingCapability::Planning => planning,
        SettingCapability::History => history,
        SettingCapability::Events => events,
    };
    CapabilityAvailability {
        available,
        reason: (!available).then_some("required ASB capability is unavailable"),
    }
}

pub struct ConfigurationStore {
    path: PathBuf,
}
impl ConfigurationStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn load(&self) -> Result<Configuration, ConfigError> {
        let metadata = fs::symlink_metadata(&self.path).map_err(ConfigError::from)?;
        if metadata.file_type().is_symlink() {
            return Err(ConfigError::SymlinkRefused);
        }
        if !metadata.is_file() {
            return Err(ConfigError::NotRegularFile);
        }
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(ConfigError::TooLarge);
        }
        Configuration::from_json(&fs::read_to_string(&self.path)?)
    }
    pub fn load_or_default(&self) -> Result<Configuration, ConfigError> {
        match self.load() {
            Ok(config) => Ok(config),
            Err(ConfigError::Io(kind))
                if kind == io::ErrorKind::NotFound.to_string()
                    && self
                        .path
                        .parent()
                        .and_then(|parent| fs::symlink_metadata(parent).ok())
                        .is_some_and(|metadata| metadata.is_dir()) =>
            {
                // A missing file in an existing directory is the first-run
                // case.  Missing parents and all other I/O failures must not
                // silently replace a user's configuration with defaults.
                Ok(Configuration::default())
            }
            Err(error) => Err(error),
        }
    }
    pub fn save(&self, config: &Configuration) -> Result<(), ConfigError> {
        let text = config.to_json()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ConfigError::Invalid("configuration path has no parent".into()))?;
        let metadata = fs::metadata(parent)?;
        if !metadata.is_dir() {
            return Err(ConfigError::NotRegularFile);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(ConfigError::Invalid(
                    "configuration directory is not private".into(),
                ));
            }
        }
        if let Ok(existing) = fs::symlink_metadata(&self.path) {
            if existing.file_type().is_symlink() {
                return Err(ConfigError::SymlinkRefused);
            }
            if !existing.is_file() {
                return Err(ConfigError::NotRegularFile);
            }
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ConfigError::Io("clock".into()))?
            .as_nanos();
        let temp = parent.join(format!(".asb-tui-config-{nonce}.tmp"));
        let result = (|| {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)?;
            // Persist the directory entry as well as the file contents so an
            // interrupted rename cannot report success while losing the new
            // configuration after a crash.
            #[cfg(unix)]
            fs::File::open(parent)?.sync_all()?;
            Ok::<(), io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(ConfigError::from)
    }
}

pub fn decode_bounded<T: DeserializeOwned>(input: &str) -> Result<T, ConfigError> {
    if input.len() > MAX_FILE_BYTES {
        return Err(ConfigError::TooLarge);
    }
    serde_json::from_str(input).map_err(|error| ConfigError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("asb-tui-configuration-{nonce}"));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    #[test]
    fn configuration_round_trip_and_validation_are_bounded() {
        let config = Configuration::default();
        let text = config.to_json().unwrap();
        assert_eq!(Configuration::from_json(&text).unwrap(), config);
        assert_eq!(decode_bounded::<Configuration>(&text).unwrap(), config);
        let mut invalid = config.clone();
        invalid.schema_version += 1;
        assert_eq!(invalid.validate(), Err(ConfigError::UnsupportedSchema));
        invalid = config.clone();
        invalid.frontend.refresh_interval_ms = 1;
        assert!(matches!(invalid.validate(), Err(ConfigError::Invalid(_))));
        invalid = config.clone();
        invalid.benchmark.repetitions = 0;
        assert!(matches!(invalid.validate(), Err(ConfigError::Invalid(_))));
        assert!(matches!(
            Configuration::from_json("{\"token\":\"x\"}"),
            Err(ConfigError::SecretInput)
        ));
        assert!(matches!(
            Configuration::from_json(&"[".repeat(MAX_NESTING_DEPTH + 1)),
            Err(ConfigError::Invalid(_))
        ));
        assert!(matches!(
            decode_bounded::<Configuration>(&"x".repeat(MAX_FILE_BYTES + 1)),
            Err(ConfigError::TooLarge)
        ));
    }

    #[test]
    fn draft_edits_reset_and_capability_availability_are_explicit() {
        let mut draft = ConfigurationDraft::new(Configuration::default()).unwrap();
        draft
            .edit(|config| config.frontend.show_key_hints = false)
            .unwrap();
        assert!(draft.is_dirty());
        draft.set_query("theme").unwrap();
        assert_eq!(draft.visible_settings().len(), 1);
        assert!(draft.set_query("\u{7f}").is_err());
        draft.focus(Some("frontend.show_key_hints"));
        draft.reset_focused().unwrap();
        draft.reset_section("Appearance").unwrap();
        draft.reset_section("Navigation").unwrap();
        draft.reset_section("Behaviour").unwrap();
        draft.reset_section("Benchmark").unwrap();
        assert!(draft.reset_section("unknown").is_err());
        assert!(draft.reset_all(false).is_err());
        draft.reset_all(true).unwrap();
        draft.cancel();
        draft.apply().unwrap();
        let history = SettingDescriptor {
            capability: SettingCapability::History,
            ..SETTING_DESCRIPTORS[0]
        };
        let events = SettingDescriptor {
            capability: SettingCapability::Events,
            ..SETTING_DESCRIPTORS[0]
        };
        for (setting, available) in [
            (SETTING_DESCRIPTORS[0], true),
            (history, false),
            (events, false),
        ] {
            assert_eq!(
                capability_availability(&setting, false, false, false).available,
                available
            );
        }
        assert!(capability_availability(&history, false, true, false).available);
        assert!(capability_availability(&events, false, false, true).available);
        for descriptor in SETTING_DESCRIPTORS {
            draft.focus(Some(descriptor.id));
            draft.reset_focused().unwrap();
        }
        draft.focus(None);
        assert!(draft.reset_focused().is_err());
    }

    #[test]
    fn store_uses_private_atomic_files_and_default_fallback() {
        let directory = temp_dir();
        let path = directory.join("config.json");
        let store = ConfigurationStore::new(&path);
        assert_eq!(store.path(), path.as_path());
        assert_eq!(store.load_or_default().unwrap(), Configuration::default());
        store.save(&Configuration::default()).unwrap();
        assert_eq!(store.load().unwrap(), Configuration::default());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let bad = directory.join("directory");
        fs::create_dir(&bad).unwrap();
        assert!(matches!(
            ConfigurationStore::new(&bad).load(),
            Err(ConfigError::NotRegularFile)
        ));
        let malformed = directory.join("malformed.json");
        fs::write(&malformed, "{}\n").unwrap();
        assert!(matches!(
            ConfigurationStore::new(&malformed).load(),
            Err(ConfigError::Invalid(_))
        ));
        let missing_parent = directory.join("missing/config.json");
        assert!(matches!(
            ConfigurationStore::new(&missing_parent).save(&Configuration::default()),
            Err(ConfigError::Io(_))
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = directory.join("link.json");
            symlink(&path, &link).unwrap();
            assert!(matches!(
                ConfigurationStore::new(&link).load(),
                Err(ConfigError::SymlinkRefused)
            ));
            assert!(matches!(
                ConfigurationStore::new(&link).save(&Configuration::default()),
                Err(ConfigError::SymlinkRefused)
            ));
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
