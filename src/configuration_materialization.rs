// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic, credential-free configuration materialization for the TUI.
//!
//! This is a renderer-neutral handoff boundary. It validates the reviewed
//! provider and benchmark selections, emits a canonical versioned document,
//! and persists that document atomically without ever accepting raw secrets.

use crate::{
    control_codec::{ConfigurationSelection, ProviderAuthMethod, Revision},
    provider_setup::ProviderSetupDraft,
    selection::{BenchmarkCatalog, CampaignSelection},
    sha256::digest_hex,
};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 256;
const MAX_LIST: usize = 256;
const MAX_FILE_BYTES: usize = 256 * 1024;
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaterializationError {
    Invalid(&'static str),
    StaleProviderCatalog,
    StaleBenchmarkCatalog,
    RawSecret,
    Io(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationInput {
    pub provider: ConfigurationSelection,
    pub provider_catalog_generation: Revision,
    pub provider_catalog_digest: String,
    pub benchmark: CampaignSelection,
    pub asb_protocol: String,
    pub asb_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedConfiguration {
    pub schema_version: u32,
    pub asb_protocol: String,
    pub asb_version: String,
    pub provider: MaterializedProvider,
    pub benchmark: MaterializedBenchmark,
    pub configuration_digest_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedProvider {
    pub agent_ids: Vec<String>,
    pub provider_id: String,
    pub model_id: String,
    pub auth_method: ProviderAuthMethod,
    pub credential_reference_sha256: Option<String>,
    pub catalog_generation: Revision,
    pub catalog_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedBenchmark {
    pub generation: Revision,
    pub catalog_digest: String,
    pub pool_id: String,
    pub group_ids: Vec<String>,
    pub benchmark_ids: Vec<String>,
    pub measure_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedBundle {
    pub document: MaterializedConfiguration,
    pub canonical_json: String,
    pub digest_sha256: String,
}

/// Canonical, digest-bound handoff consumed by the launch/statistics route.
/// Downstream code must validate this binding before issuing control I/O.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchBinding {
    pub materialization_digest_sha256: String,
    pub provider_catalog_generation: Revision,
    pub provider_catalog_digest: String,
    pub benchmark_catalog_generation: Revision,
    pub benchmark_catalog_digest: String,
    pub agent_ids: Vec<String>,
    pub provider_id: String,
    pub model_id: String,
    pub pool_id: String,
    pub group_ids: Vec<String>,
    pub benchmark_ids: Vec<String>,
    pub measure_ids: Vec<String>,
    pub development_only: bool,
}

/// Renderer-friendly, secret-free preflight projection. It is derived from a
/// validated bundle and is safe to show before the user applies it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreflightSummary {
    pub digest_sha256: String,
    pub provider_id: String,
    pub model_id: String,
    pub agent_count: usize,
    pub pool_id: String,
    pub measure_count: usize,
    pub development_only: bool,
}

impl MaterializedBundle {
    /// Revalidate the serialized handoff at the control boundary.  A caller
    /// must never be able to replace the reviewed document with `{}` (or with
    /// a document whose digest was computed over different bytes).
    pub fn validate_integrity(&self) -> Result<(), MaterializationError> {
        if bundle_is_self_consistent(self)? {
            Ok(())
        } else {
            Err(MaterializationError::Invalid("bundle integrity"))
        }
    }

    #[must_use]
    pub fn launch_binding(&self) -> LaunchBinding {
        LaunchBinding {
            materialization_digest_sha256: self.digest_sha256.clone(),
            provider_catalog_generation: self.document.provider.catalog_generation,
            provider_catalog_digest: self.document.provider.catalog_digest.clone(),
            benchmark_catalog_generation: self.document.benchmark.generation,
            benchmark_catalog_digest: self.document.benchmark.catalog_digest.clone(),
            agent_ids: self.document.provider.agent_ids.clone(),
            provider_id: self.document.provider.provider_id.clone(),
            model_id: self.document.provider.model_id.clone(),
            pool_id: self.document.benchmark.pool_id.clone(),
            group_ids: self.document.benchmark.group_ids.clone(),
            benchmark_ids: self.document.benchmark.benchmark_ids.clone(),
            measure_ids: self.document.benchmark.measure_ids.clone(),
            development_only: self.document.provider.catalog_digest == "0".repeat(64),
        }
    }

    #[must_use]
    pub fn preflight_summary(&self) -> PreflightSummary {
        PreflightSummary {
            digest_sha256: self.digest_sha256.clone(),
            provider_id: self.document.provider.provider_id.clone(),
            model_id: self.document.provider.model_id.clone(),
            agent_count: self.document.provider.agent_ids.len(),
            pool_id: self.document.benchmark.pool_id.clone(),
            measure_count: self.document.benchmark.measure_ids.len(),
            development_only: self.document.provider.catalog_digest == "0".repeat(64),
        }
    }
}

impl MaterializedBundle {
    pub fn build(
        input: MaterializationInput,
        provider_draft: &ProviderSetupDraft,
        benchmark_catalog: &BenchmarkCatalog,
        expected_provider_generation: Revision,
        expected_benchmark_generation: Revision,
    ) -> Result<Self, MaterializationError> {
        if provider_draft.catalog_generation() != expected_provider_generation
            || input.provider_catalog_generation != expected_provider_generation
        {
            return Err(MaterializationError::StaleProviderCatalog);
        }
        if input.benchmark.generation != expected_benchmark_generation {
            return Err(MaterializationError::StaleBenchmarkCatalog);
        }
        benchmark_catalog
            .validate_campaign(&input.benchmark)
            .map_err(|_| MaterializationError::StaleBenchmarkCatalog)?;
        validate_text(&input.asb_protocol, MAX_TEXT_BYTES, "asb_protocol")?;
        validate_text(&input.asb_version, MAX_TEXT_BYTES, "asb_version")?;
        validate_digest(&input.provider_catalog_digest)?;
        validate_digest(&input.benchmark.catalog_digest)?;
        validate_selection(&input.provider)?;
        if provider_draft.selection() != &input.provider {
            return Err(MaterializationError::Invalid("provider review changed"));
        }
        if provider_draft.catalog_digest() != input.provider_catalog_digest {
            return Err(MaterializationError::StaleProviderCatalog);
        }
        let credential = input.provider.credential_reference_sha256.clone();
        if matches!(
            input.provider.auth_method,
            ProviderAuthMethod::CredentialReference
        ) != credential.is_some()
        {
            return Err(MaterializationError::Invalid("credential reference"));
        }
        if let Some(ref digest) = credential {
            validate_digest(digest)?;
        }
        let provider = MaterializedProvider {
            agent_ids: input.provider.agent_ids.clone(),
            provider_id: input.provider.provider_id.clone(),
            model_id: input.provider.model_id.clone(),
            auth_method: input.provider.auth_method,
            credential_reference_sha256: credential,
            catalog_generation: input.provider_catalog_generation,
            catalog_digest: input.provider_catalog_digest,
        };
        let benchmark = MaterializedBenchmark {
            generation: input.benchmark.generation,
            catalog_digest: input.benchmark.catalog_digest,
            pool_id: input.benchmark.pool_id,
            group_ids: input.benchmark.group_ids,
            benchmark_ids: input.benchmark.benchmark_ids,
            measure_ids: input.benchmark.measure_ids,
        };
        if benchmark.measure_ids.is_empty() {
            return Err(MaterializationError::Invalid("empty measures"));
        }
        let unsigned = MaterializedConfiguration {
            schema_version: SCHEMA_VERSION,
            asb_protocol: input.asb_protocol,
            asb_version: input.asb_version,
            provider,
            benchmark,
            configuration_digest_sha256: String::new(),
        };
        let canonical = serde_json::to_string(&unsigned)
            .map_err(|_| MaterializationError::Invalid("serialize"))?;
        let digest = digest_hex(format!("asb-tui.materialized.v1\0{canonical}").as_bytes());
        let mut document = unsigned;
        document.configuration_digest_sha256 = digest.clone();
        let canonical_json = serde_json::to_string(&document)
            .map_err(|_| MaterializationError::Invalid("serialize"))?;
        Ok(Self {
            document,
            canonical_json,
            digest_sha256: digest,
        })
    }
}

// Kept separate from the builder so tests can prove persistence failures do
// not replace the previously active bundle.
pub struct MaterializedBundleStore {
    root: PathBuf,
}
impl MaterializedBundleStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn load(&self) -> Result<Option<MaterializedBundle>, MaterializationError> {
        let path = self.root.join("bundle.json");
        let stage = self.root.join(".bundle.stage");
        if self.root.exists()
            && fs::symlink_metadata(&self.root)
                .map_err(io_error)?
                .file_type()
                .is_symlink()
        {
            return Err(MaterializationError::Invalid("symlink root"));
        }
        let (read_path, recover_stage) = if path.exists() {
            (path.clone(), false)
        } else if stage.exists() {
            (stage.clone(), true)
        } else {
            return Ok(None);
        };
        if fs::symlink_metadata(&read_path)
            .map_err(io_error)?
            .file_type()
            .is_symlink()
        {
            return Err(MaterializationError::Invalid("symlink bundle"));
        }
        let bytes = fs::read(&read_path).map_err(io_error)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(MaterializationError::Invalid("bundle size"));
        }
        let document: MaterializedConfiguration = serde_json::from_slice(&bytes)
            .map_err(|_| MaterializationError::Invalid("bundle json"))?;
        let canonical_json = serde_json::to_string(&document)
            .map_err(|_| MaterializationError::Invalid("serialize"))?;
        let digest = document.configuration_digest_sha256.clone();
        let mut unsigned = document.clone();
        unsigned.configuration_digest_sha256.clear();
        let canonical_unsigned = serde_json::to_string(&unsigned)
            .map_err(|_| MaterializationError::Invalid("serialize"))?;
        let expected =
            digest_hex(format!("asb-tui.materialized.v1\0{canonical_unsigned}").as_bytes());
        if digest != expected {
            return Err(MaterializationError::Invalid("bundle digest"));
        }
        let bundle = MaterializedBundle {
            document,
            canonical_json,
            digest_sha256: digest,
        };
        if recover_stage {
            fs::rename(&stage, &path).map_err(io_error)?;
            make_private_file(&path)?;
            fs::File::open(&self.root)
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
        }
        Ok(Some(bundle))
    }
    pub fn apply(&self, bundle: &MaterializedBundle) -> Result<(), MaterializationError> {
        if !bundle_is_self_consistent(bundle)? {
            return Err(MaterializationError::Invalid("bundle digest"));
        }
        if self.root.exists()
            && fs::symlink_metadata(&self.root)
                .map_err(io_error)?
                .file_type()
                .is_symlink()
        {
            return Err(MaterializationError::Invalid("symlink root"));
        }
        fs::create_dir_all(&self.root).map_err(io_error)?;
        make_private_directory(&self.root)?;
        let stage = self.root.join(".bundle.stage");
        let target = self.root.join("bundle.json");
        let bytes = bundle.canonical_json.as_bytes();
        if bytes.len() > MAX_FILE_BYTES {
            return Err(MaterializationError::Invalid("bundle size"));
        }
        if stage.exists()
            && fs::symlink_metadata(&stage)
                .map_err(io_error)?
                .file_type()
                .is_symlink()
        {
            return Err(MaterializationError::Invalid("symlink stage"));
        }
        if target.exists()
            && fs::symlink_metadata(&target)
                .map_err(io_error)?
                .file_type()
                .is_symlink()
        {
            return Err(MaterializationError::Invalid("symlink bundle"));
        }
        if stage.exists() && !target.exists() {
            // A prior process may have completed the staged write but not the
            // rename. Validate and recover it before replacing anything.
            let _ = self.load()?;
        }
        let _ = fs::remove_file(&stage);
        fs::write(&stage, bytes).map_err(io_error)?;
        make_private_file(&stage)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .open(&stage)
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::rename(&stage, &target).map_err(io_error)?;
        make_private_file(&target)?;
        let directory = fs::File::open(&self.root).map_err(io_error)?;
        directory.sync_all().map_err(io_error)?;
        Ok(())
    }
}

fn bundle_is_self_consistent(bundle: &MaterializedBundle) -> Result<bool, MaterializationError> {
    let mut unsigned = bundle.document.clone();
    unsigned.configuration_digest_sha256.clear();
    let canonical =
        serde_json::to_string(&unsigned).map_err(|_| MaterializationError::Invalid("serialize"))?;
    let expected = digest_hex(format!("asb-tui.materialized.v1\0{canonical}").as_bytes());
    Ok(bundle.digest_sha256 == expected
        && bundle.document.configuration_digest_sha256 == expected
        && bundle.canonical_json
            == serde_json::to_string(&bundle.document)
                .map_err(|_| MaterializationError::Invalid("serialize"))?)
}

#[cfg(unix)]
fn make_private_directory(path: &PathBuf) -> Result<(), MaterializationError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_private_directory(_path: &PathBuf) -> Result<(), MaterializationError> {
    Ok(())
}

#[cfg(unix)]
fn make_private_file(path: &PathBuf) -> Result<(), MaterializationError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_error)
}

#[cfg(not(unix))]
fn make_private_file(_path: &PathBuf) -> Result<(), MaterializationError> {
    Ok(())
}
fn io_error(error: io::Error) -> MaterializationError {
    MaterializationError::Io(error.kind().to_string())
}
fn validate_text(
    value: &str,
    max: usize,
    _field: &'static str,
) -> Result<(), MaterializationError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(MaterializationError::Invalid("text"));
    }
    Ok(())
}
fn validate_digest(value: &str) -> Result<(), MaterializationError> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(MaterializationError::Invalid("digest"));
    }
    Ok(())
}
fn validate_selection(selection: &ConfigurationSelection) -> Result<(), MaterializationError> {
    if selection.agent_ids.is_empty() || selection.agent_ids.len() > MAX_LIST {
        return Err(MaterializationError::Invalid("agents"));
    }
    for value in selection
        .agent_ids
        .iter()
        .chain([&selection.provider_id, &selection.model_id])
    {
        validate_text(value, MAX_ID_BYTES, "id")?;
    }
    if selection
        .credential_reference_sha256
        .as_deref()
        .is_some_and(|v| v.len() != 64)
    {
        return Err(MaterializationError::RawSecret);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{control_codec::ProviderAuthMethod, selection::CampaignSelection};

    fn input() -> (MaterializationInput, ProviderSetupDraft) {
        let selection = ConfigurationSelection {
            agent_ids: vec!["codex".into()],
            provider_id: "openrouter".into(),
            model_id: "free-model".into(),
            auth_method: ProviderAuthMethod::None,
            credential_reference_sha256: None,
        };
        let draft = ProviderSetupDraft::from_selection_for_development(selection.clone()).unwrap();
        let benchmark = CampaignSelection {
            generation: Revision(3),
            catalog_digest: "b".repeat(64),
            pool_id: "development".into(),
            group_ids: vec!["quality".into()],
            benchmark_ids: vec!["quality".into()],
            measure_ids: vec!["quality.correctness".into()],
        };
        (
            MaterializationInput {
                provider: selection,
                provider_catalog_generation: Revision(1),
                provider_catalog_digest: "0".repeat(64),
                benchmark,
                asb_protocol: "asb-control".into(),
                asb_version: "1.0.0".into(),
            },
            draft,
        )
    }

    #[test]
    fn identical_reviewed_input_has_stable_digest_and_canonical_bytes() {
        let (value, draft) = input();
        let catalog = benchmark_catalog();
        let one =
            MaterializedBundle::build(value.clone(), &draft, &catalog, Revision(1), Revision(3))
                .unwrap();
        let two =
            MaterializedBundle::build(value, &draft, &catalog, Revision(1), Revision(3)).unwrap();
        assert_eq!(one, two);
        assert_eq!(one.digest_sha256.len(), 64);
        assert!(!one.canonical_json.contains("api_key"));
    }

    #[test]
    fn launch_binding_is_complete_and_digest_bound() {
        let (value, draft) = input();
        let bundle = MaterializedBundle::build(
            value,
            &draft,
            &benchmark_catalog(),
            Revision(1),
            Revision(3),
        )
        .unwrap();
        let binding = bundle.launch_binding();
        assert_eq!(binding.materialization_digest_sha256, bundle.digest_sha256);
        assert_eq!(binding.provider_catalog_generation, Revision(1));
        assert_eq!(binding.benchmark_catalog_generation, Revision(3));
        assert_eq!(binding.agent_ids, vec!["codex"]);
        assert_eq!(binding.measure_ids, vec!["quality.correctness"]);
        assert!(binding.development_only);
        let json = serde_json::to_string(&binding).unwrap();
        assert!(json.contains("materialization_digest_sha256"));
        assert!(!json.contains("api_key"));
    }

    #[test]
    fn stale_and_raw_secret_inputs_fail_closed() {
        let (mut value, draft) = input();
        let catalog = benchmark_catalog();
        value.benchmark.generation = Revision(4);
        assert_eq!(
            MaterializedBundle::build(value.clone(), &draft, &catalog, Revision(1), Revision(3)),
            Err(MaterializationError::StaleBenchmarkCatalog)
        );
        value.benchmark.generation = Revision(3);
        value.provider.auth_method = ProviderAuthMethod::CredentialReference;
        value.provider.credential_reference_sha256 = Some("not-a-digest".into());
        assert!(
            MaterializedBundle::build(value, &draft, &catalog, Revision(1), Revision(3)).is_err()
        );
    }

    #[test]
    fn reviewed_provider_and_catalog_membership_are_bound() {
        let (mut value, draft) = input();
        let catalog = benchmark_catalog();
        value.provider.model_id = "different-model".into();
        assert_eq!(
            MaterializedBundle::build(value.clone(), &draft, &catalog, Revision(1), Revision(3)),
            Err(MaterializationError::Invalid("provider review changed"))
        );
        value.provider = draft.selection().clone();
        value.benchmark.measure_ids = vec!["unknown.measure".into()];
        assert_eq!(
            MaterializedBundle::build(value, &draft, &catalog, Revision(1), Revision(3)),
            Err(MaterializationError::StaleBenchmarkCatalog)
        );
    }

    #[test]
    fn store_apply_is_idempotent_and_rejects_tampering() {
        let (value, draft) = input();
        let catalog = benchmark_catalog();
        let bundle =
            MaterializedBundle::build(value, &draft, &catalog, Revision(1), Revision(3)).unwrap();
        let root =
            std::env::temp_dir().join(format!("asb-tui-materialized-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = MaterializedBundleStore::new(&root);
        store.apply(&bundle).unwrap();
        store.apply(&bundle).unwrap();
        assert_eq!(store.load().unwrap(), Some(bundle.clone()));
        let mut invalid = bundle.clone();
        invalid.digest_sha256 = "0".repeat(64);
        assert!(store.apply(&invalid).is_err());
        assert_eq!(store.load().unwrap(), Some(bundle.clone()));
        std::fs::write(root.join("bundle.json"), b"{}\n").unwrap();
        assert!(store.load().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn interrupted_stage_is_recovered_without_losing_bundle() {
        let (value, draft) = input();
        let catalog = benchmark_catalog();
        let bundle =
            MaterializedBundle::build(value, &draft, &catalog, Revision(1), Revision(3)).unwrap();
        let root = std::env::temp_dir().join(format!(
            "asb-tui-materialized-recovery-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".bundle.stage"), &bundle.canonical_json).unwrap();
        let store = MaterializedBundleStore::new(&root);
        assert_eq!(store.load().unwrap(), Some(bundle.clone()));
        assert!(root.join("bundle.json").exists());
        assert!(!root.join(".bundle.stage").exists());
        #[cfg(unix)]
        assert_eq!(
            std::os::unix::fs::MetadataExt::mode(
                &std::fs::metadata(root.join("bundle.json")).unwrap()
            ) & 0o777,
            0o600
        );
        std::fs::write(root.join(".bundle.stage"), b"corrupt").unwrap();
        store.apply(&bundle).unwrap();
        assert_eq!(store.load().unwrap(), Some(bundle));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn empty_store_and_invalid_paths_fail_closed() {
        let root =
            std::env::temp_dir().join(format!("asb-tui-materialized-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = MaterializedBundleStore::new(&root);
        assert_eq!(store.load().unwrap(), None);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".bundle.stage"), b"not-json").unwrap();
        assert!(store.load().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_input_is_rejected_before_materialization() {
        let (mut value, draft) = input();
        let catalog = benchmark_catalog();
        value.asb_protocol.clear();
        assert!(
            MaterializedBundle::build(value.clone(), &draft, &catalog, Revision(1), Revision(3))
                .is_err()
        );
        let (mut value, draft) = input();
        value.provider_catalog_digest = "short".into();
        assert!(
            MaterializedBundle::build(value.clone(), &draft, &catalog, Revision(1), Revision(3))
                .is_err()
        );
        let (mut value, draft) = input();
        value.benchmark.measure_ids.clear();
        assert!(
            MaterializedBundle::build(value, &draft, &catalog, Revision(1), Revision(3)).is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_store_targets_are_rejected() {
        let (value, draft) = input();
        let bundle = MaterializedBundle::build(
            value,
            &draft,
            &benchmark_catalog(),
            Revision(1),
            Revision(3),
        )
        .unwrap();
        let root =
            std::env::temp_dir().join(format!("asb-tui-materialized-link-{}", std::process::id()));
        let target = root.with_extension("target");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&root);
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &root).unwrap();
        assert!(MaterializedBundleStore::new(&root).apply(&bundle).is_err());
        let _ = std::fs::remove_file(&root);
        let _ = std::fs::remove_dir_all(target);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_bundle_and_stage_are_rejected() {
        let (value, draft) = input();
        let bundle = MaterializedBundle::build(
            value,
            &draft,
            &benchmark_catalog(),
            Revision(1),
            Revision(3),
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "asb-tui-materialized-target-link-{}",
            std::process::id()
        ));
        let elsewhere = root.with_extension("elsewhere");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&elsewhere);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("file"), b"safe").unwrap();
        std::os::unix::fs::symlink(elsewhere.join("file"), root.join("bundle.json")).unwrap();
        let store = MaterializedBundleStore::new(&root);
        assert!(store.apply(&bundle).is_err());
        std::fs::remove_file(root.join("bundle.json")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("file"), root.join(".bundle.stage")).unwrap();
        assert!(store.apply(&bundle).is_err());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(elsewhere);
    }

    #[test]
    fn stale_provider_generation_and_digest_are_rejected() {
        let (mut value, draft) = input();
        let catalog = benchmark_catalog();
        value.provider_catalog_generation = Revision(2);
        assert_eq!(
            MaterializedBundle::build(value.clone(), &draft, &catalog, Revision(1), Revision(3)),
            Err(MaterializationError::StaleProviderCatalog)
        );
        let (mut value, draft) = input();
        value.provider_catalog_digest = "1".repeat(64);
        assert_eq!(
            MaterializedBundle::build(value, &draft, &catalog, Revision(1), Revision(3)),
            Err(MaterializationError::StaleProviderCatalog)
        );
    }

    fn benchmark_catalog() -> BenchmarkCatalog {
        let measure = crate::selection::BenchmarkMeasure::new(
            "quality.correctness",
            "Correctness",
            "score",
            true,
        )
        .unwrap();
        let benchmark =
            crate::selection::BenchmarkDefinition::new("quality", "Quality", vec![measure])
                .unwrap();
        let group =
            crate::selection::BenchmarkGroup::new("quality", "Quality", vec![benchmark]).unwrap();
        let pool = crate::selection::BenchmarkPool::new("development", "Development", vec![group])
            .unwrap();
        BenchmarkCatalog::new(Revision(3), "b".repeat(64), vec![pool]).unwrap()
    }
}
