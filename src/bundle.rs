// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed verification and bounded retrieval of immutable extension bundles.

use crate::compatibility::{
    Architecture, COORDINATOR_COMMIT, COORDINATOR_VERSION, QUALITY_COMMIT, QUALITY_VERSION,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_MANIFEST_BYTES: usize = 128 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_ATTEMPTS: usize = 3;
const SIGNER_FINGERPRINT: &str = "SHA256:a36V6yPvRZyxnQ2113tiA/MlHt7mPfJEXAGByBXVkuE";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Executable,
    Source,
    LicenseReport,
    Sbom,
    Provenance,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub name: String,
    pub kind: ArtifactKind,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundleCompatibility {
    pub bundle: String,
    pub architecture: Architecture,
    pub asb_version: String,
    pub protocol_version: u64,
    pub coordinator_version: String,
    pub coordinator_commit: String,
    pub quality_version: String,
    pub quality_commit: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    schema_version: u64,
    release: String,
    source_commit: String,
    source_tree: String,
    issued_unix: u64,
    expires_unix: u64,
    compatibility: BundleCompatibility,
    components: Vec<BundleComponent>,
    artifacts: Vec<Artifact>,
}

impl BundleManifest {
    pub fn release(&self) -> &str {
        &self.release
    }

    pub fn artifacts(&self) -> &[Artifact] {
        &self.artifacts
    }

    pub(crate) fn source_identity(&self) -> (&str, &str) {
        (&self.source_commit, &self.source_tree)
    }
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> Result<String, &'static str> {
    sha256(bytes)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundleComponent {
    pub name: String,
    pub version: String,
    pub commit: String,
    pub tree: String,
    pub artifact_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpectedCompatibility<'a> {
    pub bundle: &'a str,
    pub architecture: Architecture,
    pub asb_version: &'a str,
    pub protocol_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactIoError;

/// A range-capable source. Implementations must return at most `limit` bytes.
pub trait ArtifactSource {
    fn read(
        &mut self,
        immutable_url: &str,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<u8>, ArtifactIoError>;
}

/// A verified cache. Unverified bytes are never inserted.
pub trait VerifiedCache {
    fn get(&self, sha256: &str, maximum_bytes: u64) -> Option<Vec<u8>>;
    fn insert(&mut self, sha256: &str, bytes: &[u8]) -> Result<(), ArtifactIoError>;
    fn get_partial(&self, _sha256: &str, _maximum_bytes: u64) -> Option<Vec<u8>> {
        None
    }
    fn store_partial(&mut self, _sha256: &str, _bytes: &[u8]) -> Result<(), ArtifactIoError> {
        Ok(())
    }
    fn clear_partial(&mut self, _sha256: &str) -> Result<(), ArtifactIoError> {
        Ok(())
    }
}

/// HTTPS range source restricted to already-validated immutable GitHub release URLs.
#[derive(Default)]
pub struct CurlSource;

impl ArtifactSource for CurlSource {
    fn read(
        &mut self,
        immutable_url: &str,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<u8>, ArtifactIoError> {
        if !immutable_release_url_shape(immutable_url) || limit == 0 || limit > CHUNK_BYTES {
            return Err(ArtifactIoError);
        }
        let end = offset
            .checked_add(u64::try_from(limit).map_err(|_| ArtifactIoError)?)
            .and_then(|value| value.checked_sub(1))
            .ok_or(ArtifactIoError)?;
        let mut child = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-redirs",
                "3",
                "--connect-timeout",
                "10",
                "--max-time",
                "30",
                "--range",
                &format!("{offset}-{end}"),
                "--max-filesize",
                &limit.to_string(),
                immutable_url,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ArtifactIoError)?;
        let mut bytes = Vec::with_capacity(limit);
        child
            .stdout
            .take()
            .ok_or(ArtifactIoError)?
            .take(u64::try_from(limit).map_err(|_| ArtifactIoError)? + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ArtifactIoError)?;
        let status = child.wait().map_err(|_| ArtifactIoError)?;
        if status.success() && !bytes.is_empty() && bytes.len() <= limit {
            Ok(bytes)
        } else {
            Err(ArtifactIoError)
        }
    }
}

/// Owner-private, content-addressed cache that never overwrites an existing entry.
pub struct FilesystemCache {
    directory: File,
    retained_path: PathBuf,
}

impl FilesystemCache {
    pub fn open(root: &Path) -> Result<Self, ArtifactIoError> {
        let before = fs::symlink_metadata(root).map_err(|_| ArtifactIoError)?;
        let directory = File::open(root).map_err(|_| ArtifactIoError)?;
        let metadata = directory.metadata().map_err(|_| ArtifactIoError)?;
        if !metadata.is_dir()
            || before.file_type().is_symlink()
            || (before.dev(), before.ino()) != (metadata.dev(), metadata.ino())
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(ArtifactIoError);
        }
        let retained_path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        Ok(Self {
            directory,
            retained_path,
        })
    }

    fn entry(&self, digest: &str) -> Option<PathBuf> {
        is_hex(digest, 64).then(|| self.retained_path.join(digest))
    }

    fn partial(&self, digest: &str) -> Option<PathBuf> {
        is_hex(digest, 64).then(|| self.retained_path.join(format!(".partial-{digest}")))
    }

    fn validate_directory(&self) -> Result<(), ArtifactIoError> {
        let metadata = self.directory.metadata().map_err(|_| ArtifactIoError)?;
        if metadata.is_dir()
            && metadata.uid() == rustix::process::getuid().as_raw()
            && metadata.mode() & 0o077 == 0
        {
            Ok(())
        } else {
            Err(ArtifactIoError)
        }
    }
}

impl VerifiedCache for FilesystemCache {
    fn get(&self, digest: &str, maximum_bytes: u64) -> Option<Vec<u8>> {
        self.validate_directory().ok()?;
        let path = self.entry(digest)?;
        let before = fs::symlink_metadata(&path).ok()?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || before.uid() != rustix::process::getuid().as_raw()
            || before.mode() & 0o077 != 0
            || before.len() > maximum_bytes
        {
            return None;
        }
        let mut file = File::open(&path).ok()?;
        let opened = file.metadata().ok()?;
        if (opened.dev(), opened.ino()) != (before.dev(), before.ino()) {
            return None;
        }
        let mut bytes = Vec::with_capacity(usize::try_from(opened.len()).ok()?);
        Read::by_ref(&mut file)
            .take(maximum_bytes.checked_add(1)?)
            .read_to_end(&mut bytes)
            .ok()?;
        let after = file.metadata().ok()?;
        ((after.dev(), after.ino(), after.len())
            == (opened.dev(), opened.ino(), bytes.len() as u64)
            && bytes.len() as u64 <= maximum_bytes)
            .then_some(bytes)
    }

    fn insert(&mut self, digest: &str, bytes: &[u8]) -> Result<(), ArtifactIoError> {
        self.validate_directory()?;
        let target = self.entry(digest).ok_or(ArtifactIoError)?;
        if target.exists() {
            return Ok(());
        }
        let staging = self
            .retained_path
            .join(format!(".{digest}.{}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staging)
            .map_err(|_| ArtifactIoError)?;
        let result = (|| {
            file.write_all(bytes).map_err(|_| ArtifactIoError)?;
            file.sync_all().map_err(|_| ArtifactIoError)?;
            fs::hard_link(&staging, &target)
                .or_else(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })
                .map_err(|_| ArtifactIoError)?;
            self.directory.sync_all().map_err(|_| ArtifactIoError)
        })();
        drop(file);
        let removed = fs::remove_file(&staging);
        if result.is_err() || removed.is_err() {
            return Err(ArtifactIoError);
        }
        Ok(())
    }

    fn get_partial(&self, digest: &str, maximum_bytes: u64) -> Option<Vec<u8>> {
        self.validate_directory().ok()?;
        let path = self.partial(digest)?;
        let before = fs::symlink_metadata(&path).ok()?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || before.uid() != rustix::process::getuid().as_raw()
            || before.mode() & 0o077 != 0
            || before.len() >= maximum_bytes
        {
            return None;
        }
        let mut file = File::open(path).ok()?;
        let opened = file.metadata().ok()?;
        if (opened.dev(), opened.ino()) != (before.dev(), before.ino()) {
            return None;
        }
        let mut bytes = Vec::with_capacity(usize::try_from(opened.len()).ok()?);
        Read::by_ref(&mut file)
            .take(maximum_bytes)
            .read_to_end(&mut bytes)
            .ok()?;
        let after = file.metadata().ok()?;
        ((after.dev(), after.ino(), after.len())
            == (opened.dev(), opened.ino(), bytes.len() as u64)
            && !bytes.is_empty()
            && (bytes.len() as u64) < maximum_bytes)
            .then_some(bytes)
    }

    fn store_partial(&mut self, digest: &str, bytes: &[u8]) -> Result<(), ArtifactIoError> {
        self.validate_directory()?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_ARTIFACT_BYTES {
            return Err(ArtifactIoError);
        }
        let target = self.partial(digest).ok_or(ArtifactIoError)?;
        let staging = self
            .retained_path
            .join(format!(".partial-{digest}.{}.tmp", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staging)
            .map_err(|_| ArtifactIoError)?;
        let result = (|| {
            file.write_all(bytes).map_err(|_| ArtifactIoError)?;
            file.sync_all().map_err(|_| ArtifactIoError)?;
            fs::rename(&staging, &target).map_err(|_| ArtifactIoError)?;
            self.directory.sync_all().map_err(|_| ArtifactIoError)
        })();
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result
    }

    fn clear_partial(&mut self, digest: &str) -> Result<(), ArtifactIoError> {
        self.validate_directory()?;
        let path = self.partial(digest).ok_or(ArtifactIoError)?;
        match fs::remove_file(path) {
            Ok(()) => self.directory.sync_all().map_err(|_| ArtifactIoError),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(ArtifactIoError),
        }
    }
}

/// Authenticate, parse, and validate a manifest in the required order.
pub fn verify_bundle_manifest(
    input: &[u8],
    signature: &Path,
    allowed_signers: &Path,
    now_unix: u64,
    expected: ExpectedCompatibility<'_>,
) -> Result<BundleManifest, &'static str> {
    if input.is_empty() || input.len() > MAX_MANIFEST_BYTES {
        return Err("invalid_manifest_size");
    }
    verify_manifest_signature(input, signature, allowed_signers)?;
    parse_and_validate_manifest(input, now_unix, expected)
}

pub fn parse_and_validate_manifest(
    input: &[u8],
    now_unix: u64,
    expected: ExpectedCompatibility<'_>,
) -> Result<BundleManifest, &'static str> {
    if input.is_empty() || input.len() > MAX_MANIFEST_BYTES {
        return Err("invalid_manifest_size");
    }
    let manifest: BundleManifest = serde_json::from_slice(input).map_err(|_| "invalid_manifest")?;
    if manifest.schema_version != 1
        || !is_version(&manifest.release)
        || !is_hex(&manifest.source_commit, 40)
        || !is_hex(&manifest.source_tree, 40)
        || manifest.issued_unix > now_unix
        || manifest.expires_unix <= now_unix
        || manifest.expires_unix.saturating_sub(manifest.issued_unix) > 366 * 24 * 60 * 60
    {
        return Err("invalid_manifest_policy");
    }
    validate_compatibility(&manifest.compatibility, expected)?;
    validate_artifacts(&manifest.artifacts, &manifest.release)?;
    validate_components(&manifest)?;
    Ok(manifest)
}

fn validate_components(manifest: &BundleManifest) -> Result<(), &'static str> {
    if manifest.components.len() != 3 {
        return Err("incomplete_component_set");
    }
    let executable_digest = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "asb-tui")
        .map(|artifact| artifact.sha256.as_str())
        .ok_or("incomplete_artifact_set")?;
    let expected = BTreeMap::from([
        (
            "asb-tui",
            (
                manifest.release.as_str(),
                manifest.source_commit.as_str(),
                manifest.source_tree.as_str(),
                executable_digest,
            ),
        ),
        (
            "agent-workflow-coordinator",
            (
                COORDINATOR_VERSION,
                COORDINATOR_COMMIT,
                "41d08ed42333cb47b07c2c401a9167b56c7cfb81",
                "a51e58ed71dd93979acc55560fc8208db7131d8e6d0a6b7b805fa383fef25b34",
            ),
        ),
        (
            "agent-workflow-quality",
            (
                QUALITY_VERSION,
                QUALITY_COMMIT,
                "ca77db478f0737142690d37683d826710cd953b0",
                "9d480cabe955a5faf88f6f1c8dfd2bfe63045445173c86f93d200a00c97fff89",
            ),
        ),
    ]);
    let mut observed = BTreeMap::new();
    for component in &manifest.components {
        if !is_hex(&component.commit, 40)
            || !is_hex(&component.tree, 40)
            || !is_hex(&component.artifact_sha256, 64)
            || observed
                .insert(
                    component.name.as_str(),
                    (
                        component.version.as_str(),
                        component.commit.as_str(),
                        component.tree.as_str(),
                        component.artifact_sha256.as_str(),
                    ),
                )
                .is_some()
        {
            return Err("invalid_component");
        }
    }
    if observed == expected {
        Ok(())
    } else {
        Err("component_identity_mismatch")
    }
}

fn validate_compatibility(
    value: &BundleCompatibility,
    expected: ExpectedCompatibility<'_>,
) -> Result<(), &'static str> {
    if value.bundle != expected.bundle
        || value.architecture != expected.architecture
        || value.asb_version != expected.asb_version
        || value.protocol_version != expected.protocol_version
        || value.coordinator_version != COORDINATOR_VERSION
        || value.coordinator_commit != COORDINATOR_COMMIT
        || value.quality_version != QUALITY_VERSION
        || value.quality_commit != QUALITY_COMMIT
    {
        return Err("incompatible_bundle");
    }
    Ok(())
}

fn validate_artifacts(artifacts: &[Artifact], release: &str) -> Result<(), &'static str> {
    if artifacts.len() != 5 {
        return Err("incomplete_artifact_set");
    }
    let expected = BTreeSet::from([
        ("asb-tui", ArtifactKind::Executable),
        ("source", ArtifactKind::Source),
        ("licenses", ArtifactKind::LicenseReport),
        ("sbom", ArtifactKind::Sbom),
        ("provenance", ArtifactKind::Provenance),
    ]);
    let mut observed = BTreeSet::new();
    let mut total = 0_u64;
    for artifact in artifacts {
        if artifact.name.is_empty()
            || artifact.name.len() > 32
            || !artifact
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            || artifact.size == 0
            || artifact.size > MAX_ARTIFACT_BYTES
            || !is_hex(&artifact.sha256, 64)
            || !immutable_release_url(&artifact.url, release)
            || !observed.insert((artifact.name.as_str(), artifact.kind.clone()))
        {
            return Err("invalid_artifact");
        }
        total = total
            .checked_add(artifact.size)
            .ok_or("artifact_quota_exceeded")?;
    }
    if total > MAX_BUNDLE_BYTES {
        Err("artifact_quota_exceeded")
    } else if observed != expected {
        Err("incomplete_artifact_set")
    } else {
        Ok(())
    }
}

fn immutable_release_url(url: &str, release: &str) -> bool {
    let prefix = format!("https://github.com/martin-beck/asb-tui/releases/download/{release}/");
    url.starts_with(&prefix)
        && url.len() > prefix.len()
        && !url[prefix.len()..].contains('/')
        && !url.contains(['?', '#', '%'])
}

/// Verify a detached manifest signature against the repository trust anchor.
pub fn verify_manifest_signature(
    manifest: &[u8],
    signature: &Path,
    allowed_signers: &Path,
) -> Result<(), &'static str> {
    let fingerprint = Command::new("ssh-keygen")
        .args(["-lf"])
        .arg(allowed_signers)
        .output()
        .map_err(|_| "signature_verifier_unavailable")?;
    let fingerprints: Vec<String> = String::from_utf8(fingerprint.stdout)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_owned))
        .collect();
    if !fingerprint.status.success() || fingerprints != [SIGNER_FINGERPRINT] {
        return Err("invalid_trust_anchor");
    }
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(allowed_signers)
        .args(["-I", "martin.beck2@gmx.de", "-n", "asb-tui-bundle-v1", "-s"])
        .arg(signature)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "signature_verifier_unavailable")?;
    child
        .stdin
        .take()
        .ok_or("signature_verifier_unavailable")?
        .write_all(manifest)
        .map_err(|_| "signature_verification_failed")?;
    if child
        .wait()
        .map_err(|_| "signature_verification_failed")?
        .success()
    {
        Ok(())
    } else {
        Err("signature_verification_failed")
    }
}

/// Return verified bytes, reusing only a digest-valid cache entry and resuming bounded reads.
pub fn obtain_artifact(
    artifact: &Artifact,
    source: &mut impl ArtifactSource,
    cache: &mut impl VerifiedCache,
) -> Result<Vec<u8>, &'static str> {
    if artifact.size == 0
        || artifact.size > MAX_ARTIFACT_BYTES
        || !is_hex(&artifact.sha256, 64)
        || !immutable_release_url_shape(&artifact.url)
    {
        return Err("invalid_artifact");
    }
    if let Some(bytes) = cache.get(&artifact.sha256, artifact.size)
        && bytes.len() as u64 == artifact.size
        && sha256(&bytes)? == artifact.sha256
    {
        return Ok(bytes);
    }
    let capacity = usize::try_from(artifact.size).map_err(|_| "artifact_too_large")?;
    let mut bytes = cache
        .get_partial(&artifact.sha256, artifact.size)
        .unwrap_or_else(|| Vec::with_capacity(capacity));
    if bytes.len() >= capacity {
        bytes.clear();
    }
    let mut failures = 0;
    while bytes.len() < capacity {
        let remaining = capacity - bytes.len();
        let request = remaining.min(CHUNK_BYTES);
        match source.read(&artifact.url, bytes.len() as u64, request) {
            Ok(chunk) if chunk.len() == request => {
                bytes.extend_from_slice(&chunk);
                cache
                    .store_partial(&artifact.sha256, &bytes)
                    .map_err(|_| "partial_cache_failed")?;
                failures = 0;
            }
            Ok(_) | Err(_) => {
                failures += 1;
                if failures >= MAX_ATTEMPTS {
                    return Err("artifact_transfer_failed");
                }
            }
        }
    }
    if sha256(&bytes)? != artifact.sha256 {
        cache
            .clear_partial(&artifact.sha256)
            .map_err(|_| "partial_cache_failed")?;
        return Err("artifact_digest_mismatch");
    }
    cache
        .insert(&artifact.sha256, &bytes)
        .map_err(|_| "verified_cache_failed")?;
    cache
        .clear_partial(&artifact.sha256)
        .map_err(|_| "partial_cache_failed")?;
    Ok(bytes)
}

/// Download every digest-verified artifact, then enforce license, SBOM, and provenance policy.
pub fn obtain_verified_bundle(
    manifest: &BundleManifest,
    source: &mut impl ArtifactSource,
    cache: &mut impl VerifiedCache,
) -> Result<BTreeMap<String, Vec<u8>>, &'static str> {
    let mut verified = BTreeMap::new();
    for artifact in &manifest.artifacts {
        let bytes = obtain_artifact(artifact, source, cache)?;
        verified.insert(artifact.name.clone(), bytes);
    }
    validate_bundle_documents(manifest, &verified)?;
    Ok(verified)
}

/// Verify a complete, already downloaded artifact directory without trusting filenames or bytes.
pub fn verify_local_bundle_artifacts(
    manifest: &BundleManifest,
    root: &Path,
) -> Result<BTreeMap<String, Vec<u8>>, &'static str> {
    let metadata = fs::symlink_metadata(root).map_err(|_| "artifact_directory_unavailable")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("artifact_directory_unavailable");
    }
    let mut verified = BTreeMap::new();
    for artifact in &manifest.artifacts {
        let path = root.join(&artifact.name);
        let before = fs::symlink_metadata(&path).map_err(|_| "artifact_unavailable")?;
        if !before.is_file()
            || before.file_type().is_symlink()
            || before.len() != artifact.size
            || before.len() > MAX_ARTIFACT_BYTES
        {
            return Err("artifact_invalid");
        }
        let mut file = File::open(&path).map_err(|_| "artifact_unavailable")?;
        let opened = file.metadata().map_err(|_| "artifact_unavailable")?;
        if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
            return Err("artifact_invalid");
        }
        let mut bytes =
            Vec::with_capacity(usize::try_from(opened.len()).map_err(|_| "artifact_invalid")?);
        Read::by_ref(&mut file)
            .take(artifact.size + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "artifact_unavailable")?;
        let after = file.metadata().map_err(|_| "artifact_unavailable")?;
        if (after.dev(), after.ino(), after.len())
            != (opened.dev(), opened.ino(), bytes.len() as u64)
            || bytes.len() as u64 != artifact.size
            || sha256(&bytes)? != artifact.sha256
        {
            return Err("artifact_digest_mismatch");
        }
        verified.insert(artifact.name.clone(), bytes);
    }
    validate_bundle_documents(manifest, &verified)?;
    Ok(verified)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LicenseReport {
    schema_version: u64,
    release: String,
    packages: Vec<LicensePackage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LicensePackage {
    name: String,
    version: String,
    license: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    schema_version: u64,
    release: String,
    source_commit: String,
    source_tree: String,
    builder: String,
    reproducible: bool,
}

/// Enforce semantic policy on already digest-verified metadata artifacts.
pub fn validate_bundle_documents(
    manifest: &BundleManifest,
    artifacts: &BTreeMap<String, Vec<u8>>,
) -> Result<(), &'static str> {
    let licenses: LicenseReport =
        serde_json::from_slice(artifacts.get("licenses").ok_or("license_report_missing")?)
            .map_err(|_| "license_report_invalid")?;
    if licenses.schema_version != 1 || licenses.release != manifest.release {
        return Err("license_report_invalid");
    }
    let mut names = BTreeSet::new();
    for package in licenses.packages {
        if package.name.is_empty()
            || package.version.is_empty()
            || !matches!(
                package.license.as_str(),
                "MIT"
                    | "Apache-2.0"
                    | "MIT OR Apache-2.0"
                    | "Apache-2.0 OR BSL-1.0"
                    | "Unlicense OR MIT"
                    | "(MIT OR Apache-2.0) AND Unicode-3.0"
            )
            || !names.insert(package.name)
        {
            return Err("license_policy_rejected");
        }
    }
    for required in [
        "asb-tui",
        "agent-workflow-coordinator",
        "agent-workflow-quality",
    ] {
        if !names.contains(required) {
            return Err("license_report_incomplete");
        }
    }

    let sbom: serde_json::Value =
        serde_json::from_slice(artifacts.get("sbom").ok_or("sbom_missing")?)
            .map_err(|_| "sbom_invalid")?;
    if sbom.get("spdxVersion").and_then(|value| value.as_str()) != Some("SPDX-2.3")
        || sbom.get("name").and_then(|value| value.as_str())
            != Some(format!("asb-tui-{}", manifest.release).as_str())
    {
        return Err("sbom_invalid");
    }
    let sbom_names: BTreeSet<_> = sbom
        .get("packages")
        .and_then(|value| value.as_array())
        .ok_or("sbom_invalid")?
        .iter()
        .filter_map(|package| package.get("name").and_then(|value| value.as_str()))
        .collect();
    if !names.iter().all(|name| sbom_names.contains(name.as_str())) {
        return Err("sbom_incomplete");
    }

    let provenance: Provenance =
        serde_json::from_slice(artifacts.get("provenance").ok_or("provenance_missing")?)
            .map_err(|_| "provenance_invalid")?;
    if provenance.schema_version != 1
        || provenance.release != manifest.release
        || provenance.source_commit != manifest.source_commit
        || provenance.source_tree != manifest.source_tree
        || provenance.builder != "github-actions"
        || !provenance.reproducible
    {
        return Err("provenance_invalid");
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> Result<String, &'static str> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "digest_verifier_unavailable")?;
    child
        .stdin
        .take()
        .ok_or("digest_verifier_unavailable")?
        .write_all(bytes)
        .map_err(|_| "digest_verification_failed")?;
    let output = child
        .wait_with_output()
        .map_err(|_| "digest_verification_failed")?;
    let digest = String::from_utf8(output.stdout).map_err(|_| "digest_verification_failed")?;
    let value = digest.split_whitespace().next().unwrap_or_default();
    if output.status.success() && is_hex(value, 64) {
        Ok(value.to_owned())
    } else {
        Err("digest_verification_failed")
    }
}

fn immutable_release_url_shape(url: &str) -> bool {
    const PREFIX: &str = "https://github.com/martin-beck/asb-tui/releases/download/v";
    let Some((release, file)) = url
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    !release.is_empty()
        && release
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        && !file.is_empty()
        && !file.contains('/')
        && !url.contains(['?', '#', '%'])
}

fn is_version(value: &str) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    if value.len() > 32 {
        return false;
    }
    let mut components = version.split('.');
    let number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    number(components.next().unwrap_or_default())
        && number(components.next().unwrap_or_default())
        && number(components.next().unwrap_or_default())
        && components.next().is_none()
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
