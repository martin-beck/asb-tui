// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

//! Closed capability parsing and authenticated immutable release discovery.

pub mod bundle;
pub mod compatibility;
pub mod lifecycle;
pub mod system_probe;

use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub analysis: bool,
    pub artifacts: bool,
    pub cancel: bool,
    pub events: bool,
    pub history: bool,
    pub launch: bool,
    pub planning: bool,
    pub repeat: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityResponse {
    pub protocol: String,
    pub protocol_version: u64,
    pub asb_version: String,
    pub capabilities: Capabilities,
}

impl Capabilities {
    pub(crate) fn supports_tui(&self) -> bool {
        self.analysis
            && self.artifacts
            && self.cancel
            && self.events
            && self.history
            && self.launch
            && self.planning
            && self.repeat
    }
}

pub(crate) fn is_safe_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 || !value.is_ascii() {
        return false;
    }
    let (core, suffix) = value
        .split_once('-')
        .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
    let mut components = core.split('.');
    let valid_number =
        |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    valid_number(components.next().unwrap_or_default())
        && valid_number(components.next().unwrap_or_default())
        && valid_number(components.next().unwrap_or_default())
        && components.next().is_none()
        && suffix.is_none_or(|suffix| !suffix.is_empty())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

/// Parse and semantically validate the closed ASB capability response v1.
pub fn parse_capability_response(input: &str) -> Result<CapabilityResponse, String> {
    let response: CapabilityResponse = serde_json::from_str(input)
        .map_err(|error| format!("invalid capability response: {error}"))?;
    if response.protocol != "asb-cli-capabilities" {
        return Err("unsupported capability protocol".into());
    }
    if response.protocol_version != 1 {
        return Err("unsupported capability protocol version".into());
    }
    if !is_safe_version(&response.asb_version) {
        return Err("invalid ASB version".into());
    }
    Ok(response)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderingLock {
    classification: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyLock {
    name: String,
    repository: String,
    tag: String,
    tag_object: String,
    commit: String,
    tree: String,
    license: String,
    license_sha256: String,
    #[serde(default)]
    archive_sha256: Option<String>,
    #[serde(default)]
    archive_size: Option<u64>,
    #[serde(default)]
    provenance_limit: Option<String>,
    #[serde(default)]
    manifest_sha256: Option<String>,
    #[serde(default)]
    wheel_sha256: Option<String>,
    #[serde(default)]
    provenance_sha256: Option<String>,
    #[serde(default)]
    sbom_sha256: Option<String>,
    #[serde(default)]
    sdist_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseLock {
    schema_version: u64,
    signing_key_fingerprint: String,
    dependencies: Vec<DependencyLock>,
    rendering: RenderingLock,
}

/// Supplies independently authenticated immutable upstream Git identities.
pub trait ReleaseProbe {
    fn tag_object(&self, dependency: &str, tag: &str) -> Result<String, String>;
    fn commit(&self, dependency: &str, tag_object: &str) -> Result<String, String>;
    fn tree(&self, dependency: &str, commit: &str) -> Result<String, String>;
    fn tag_is_authenticated(&self, dependency: &str, tag_object: &str) -> Result<bool, String>;
}

/// Validate the release lock and match every identity against an authenticated probe.
pub fn verify_release_lock_contents(input: &str, probe: &impl ReleaseProbe) -> Result<(), String> {
    let lock: ReleaseLock =
        serde_json::from_str(input).map_err(|error| format!("invalid release lock: {error}"))?;
    if lock.schema_version != 1
        || lock.signing_key_fingerprint != "SHA256:a36V6yPvRZyxnQ2113tiA/MlHt7mPfJEXAGByBXVkuE"
        || lock.rendering.classification != "unavailable"
        || lock.rendering.reason != "awaiting-reviewed-immutable-ratatui-crossterm-closure"
        || lock.dependencies.len() != 2
    {
        return Err("unsupported release lock policy".into());
    }
    let mut names = BTreeSet::new();
    for dependency in &lock.dependencies {
        if !names.insert(dependency.name.as_str())
            || dependency.license != "MIT"
            || dependency.repository
                != format!("https://github.com/martin-beck/{}", dependency.name)
            || !is_hex(&dependency.tag_object, 40)
            || !is_hex(&dependency.commit, 40)
            || !is_hex(&dependency.tree, 40)
            || !is_hex(&dependency.license_sha256, 64)
        {
            return Err(format!(
                "invalid immutable identity for {}",
                dependency.name
            ));
        }
        validate_artifact_fields(dependency)?;
        let observed_tag = probe.tag_object(&dependency.name, &dependency.tag)?;
        if observed_tag != dependency.tag_object {
            return Err(format!("mutable tag rejected for {}", dependency.name));
        }
        if !probe.tag_is_authenticated(&dependency.name, &dependency.tag_object)? {
            return Err(format!("unsigned tag rejected for {}", dependency.name));
        }
        if probe.commit(&dependency.name, &dependency.tag_object)? != dependency.commit
            || probe.tree(&dependency.name, &dependency.commit)? != dependency.tree
        {
            return Err(format!(
                "tampered release identity rejected for {}",
                dependency.name
            ));
        }
    }
    if names != BTreeSet::from(["agent-workflow-coordinator", "agent-workflow-quality"]) {
        return Err("release lock has an unexpected dependency set".into());
    }
    Ok(())
}

fn validate_artifact_fields(dependency: &DependencyLock) -> Result<(), String> {
    let hashes = [
        dependency.archive_sha256.as_deref(),
        dependency.manifest_sha256.as_deref(),
        dependency.wheel_sha256.as_deref(),
        dependency.provenance_sha256.as_deref(),
        dependency.sbom_sha256.as_deref(),
        dependency.sdist_sha256.as_deref(),
    ];
    if hashes.into_iter().flatten().any(|hash| !is_hex(hash, 64)) {
        return Err(format!("invalid artifact digest for {}", dependency.name));
    }
    match dependency.name.as_str() {
        "agent-workflow-coordinator"
            if dependency.archive_sha256.is_some()
                && dependency.archive_size.is_some_and(|size| size > 0)
                && dependency.provenance_limit.as_deref()
                    == Some("signed-source-archive-only-no-publisher-sbom")
                && dependency.manifest_sha256.is_none() =>
        {
            Ok(())
        }
        "agent-workflow-quality"
            if dependency.archive_sha256.is_none()
                && dependency.archive_size.is_none()
                && dependency.provenance_limit.is_none()
                && dependency.manifest_sha256.is_some()
                && dependency.wheel_sha256.is_some()
                && dependency.provenance_sha256.is_some()
                && dependency.sbom_sha256.is_some()
                && dependency.sdist_sha256.is_some() =>
        {
            Ok(())
        }
        _ => Err(format!(
            "incomplete provenance closure for {}",
            dependency.name
        )),
    }
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Verify the detached lock signature against the single compiled-in trust anchor.
pub fn verify_lock_signature(
    lock: &Path,
    signature: &Path,
    allowed_signers: &Path,
) -> Result<(), String> {
    if !signature.is_file() {
        return Err("release lock signature is missing".into());
    }
    let fingerprint = Command::new("ssh-keygen")
        .args(["-lf"])
        .arg(allowed_signers)
        .output()
        .map_err(|error| format!("cannot inspect allowed signer file: {error}"))?;
    if !fingerprint.status.success() {
        return Err("allowed signers file is invalid".into());
    }
    let output = String::from_utf8(fingerprint.stdout)
        .map_err(|_| "signer fingerprint output is not UTF-8")?;
    let fingerprints: Vec<&str> = output
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    if fingerprints != ["SHA256:a36V6yPvRZyxnQ2113tiA/MlHt7mPfJEXAGByBXVkuE"] {
        return Err("allowed signer does not match the pinned trust anchor".into());
    }

    let input = fs::read(lock).map_err(|error| format!("cannot read release lock: {error}"))?;
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(allowed_signers)
        .args([
            "-I",
            "martin.beck2@gmx.de",
            "-n",
            "asb-tui-release-lock",
            "-s",
        ])
        .arg(signature)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start signature verifier: {error}"))?;
    use std::io::Write;
    child
        .stdin
        .take()
        .ok_or("signature verifier stdin unavailable")?
        .write_all(&input)
        .map_err(|error| format!("cannot feed signature verifier: {error}"))?;
    if child
        .wait()
        .map_err(|error| format!("cannot wait for signature verifier: {error}"))?
        .success()
    {
        Ok(())
    } else {
        Err("release lock signature is invalid".into())
    }
}

/// Git-backed release probe for exact local upstream checkouts.
pub struct GitProbe<'a> {
    /// Exact coordinator checkout.
    pub coordinator: &'a Path,
    /// Exact quality-tool checkout.
    pub quality: &'a Path,
    /// Allowed-signers file whose sole key must match the pinned fingerprint.
    pub allowed_signers: &'a Path,
}

impl GitProbe<'_> {
    fn repository(&self, dependency: &str) -> Result<&Path, String> {
        match dependency {
            "agent-workflow-coordinator" => Ok(self.coordinator),
            "agent-workflow-quality" => Ok(self.quality),
            _ => Err("unexpected dependency".into()),
        }
    }

    fn git(&self, dependency: &str, args: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.repository(dependency)?)
            .args(args)
            .output()
            .map_err(|error| format!("cannot start git: {error}"))?;
        if !output.status.success() {
            return Err(format!("git probe failed for {dependency}"));
        }
        String::from_utf8(output.stdout)
            .map(|value| value.trim().to_owned())
            .map_err(|_| "git probe emitted non-UTF-8 output".into())
    }
}

impl ReleaseProbe for GitProbe<'_> {
    fn tag_object(&self, dependency: &str, tag: &str) -> Result<String, String> {
        self.git(
            dependency,
            &["rev-parse", &format!("refs/tags/{tag}^{{tag}}")],
        )
    }
    fn commit(&self, dependency: &str, tag_object: &str) -> Result<String, String> {
        self.git(
            dependency,
            &["rev-parse", &format!("{tag_object}^{{commit}}")],
        )
    }
    fn tree(&self, dependency: &str, commit: &str) -> Result<String, String> {
        self.git(dependency, &["rev-parse", &format!("{commit}^{{tree}}")])
    }
    fn tag_is_authenticated(&self, dependency: &str, tag_object: &str) -> Result<bool, String> {
        let status = Command::new("git")
            .arg("-C")
            .arg(self.repository(dependency)?)
            .args([
                "-c",
                &format!(
                    "gpg.ssh.allowedSignersFile={}",
                    self.allowed_signers.display()
                ),
                "verify-tag",
                tag_object,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("cannot start git signature verifier: {error}"))?;
        Ok(status.success())
    }
}
