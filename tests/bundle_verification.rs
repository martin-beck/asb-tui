// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    bundle::{
        Artifact, ArtifactIoError, ArtifactKind, ArtifactSource, ExpectedCompatibility,
        VerifiedCache, obtain_artifact, parse_and_validate_manifest, validate_bundle_documents,
        verify_bundle_manifest,
    },
    compatibility::Architecture,
};
use std::collections::BTreeMap;
use std::path::Path;

const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn expected() -> ExpectedCompatibility<'static> {
    ExpectedCompatibility {
        bundle: "asb-tui-v1-linux-x86_64",
        architecture: Architecture::X86_64,
        asb_version: "0.1.0",
        protocol_version: 1,
    }
}

fn manifest() -> Vec<u8> {
    format!(
        r#"{{
          "schema_version":1,
          "release":"v0.1.0",
          "source_commit":"{commit}",
          "source_tree":"{tree}",
          "issued_unix":1789000000,
          "expires_unix":1800000001,
          "compatibility":{{
            "bundle":"asb-tui-v1-linux-x86_64",
            "architecture":"x86_64",
            "asb_version":"0.1.0",
            "protocol_version":1,
            "coordinator_version":"v0.3.5",
            "coordinator_commit":"510817b93feb80dde13e5a6c61d657954fae2346",
            "quality_version":"v0.23.0",
            "quality_commit":"8a9f056b7fc7926b9465a0f7a09225d4da1c572a"
          }},
          "artifacts":[
            {artifact},
            {source},
            {licenses},
            {sbom},
            {provenance}
          ]
        }}"#,
        commit = "a".repeat(40),
        tree = "b".repeat(40),
        artifact = entry("asb-tui", "executable", "asb-tui"),
        source = entry("source", "source", "source.tar.gz"),
        licenses = entry("licenses", "license_report", "licenses.json"),
        sbom = entry("sbom", "sbom", "sbom.spdx.json"),
        provenance = entry("provenance", "provenance", "provenance.json"),
    )
    .into_bytes()
}

fn entry(name: &str, kind: &str, file: &str) -> String {
    format!(
        r#"{{"name":"{name}","kind":"{kind}","url":"https://github.com/martin-beck/asb-tui/releases/download/v0.1.0/{file}","size":5,"sha256":"{HELLO_SHA}"}}"#
    )
}

#[test]
fn accepts_complete_immutable_compatible_manifest() {
    let parsed = parse_and_validate_manifest(&manifest(), 1_800_000_000, expected()).unwrap();
    assert_eq!(parsed.artifacts.len(), 5);
}

#[test]
fn authentic_static_fixture_verifies_offline_before_parsing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read(root.join("tests/fixtures/bundle/manifest.json")).unwrap();
    assert!(
        verify_bundle_manifest(
            &manifest,
            &root.join("tests/fixtures/bundle/manifest.json.sig"),
            &root.join("provenance/allowed_signers"),
            1_800_000_000,
            expected(),
        )
        .is_ok()
    );

    let mut tampered = manifest;
    tampered[20] ^= 1;
    assert_eq!(
        verify_bundle_manifest(
            &tampered,
            &root.join("tests/fixtures/bundle/manifest.json.sig"),
            &root.join("provenance/allowed_signers"),
            1_800_000_000,
            expected(),
        ),
        Err("signature_verification_failed")
    );
}

#[test]
fn rejects_expired_incompatible_mutable_incomplete_and_unknown_metadata() {
    assert_eq!(
        parse_and_validate_manifest(&manifest(), 1_800_000_001, expected()),
        Err("invalid_manifest_policy")
    );

    let mut mismatch = expected();
    mismatch.protocol_version = 2;
    assert_eq!(
        parse_and_validate_manifest(&manifest(), 1_800_000_000, mismatch),
        Err("incompatible_bundle")
    );

    let mutable = String::from_utf8(manifest())
        .unwrap()
        .replace("releases/download/v0.1.0", "releases/latest/download");
    assert_eq!(
        parse_and_validate_manifest(mutable.as_bytes(), 1_800_000_000, expected()),
        Err("invalid_artifact")
    );

    let incomplete = String::from_utf8(manifest()).unwrap().replace(
        &format!(
            ",\n            {}",
            entry("provenance", "provenance", "provenance.json")
        ),
        "",
    );
    assert_eq!(
        parse_and_validate_manifest(incomplete.as_bytes(), 1_800_000_000, expected()),
        Err("incomplete_artifact_set")
    );

    let unknown = String::from_utf8(manifest()).unwrap().replace(
        "\"schema_version\":1",
        "\"schema_version\":1,\"private_path\":\"redacted\"",
    );
    assert_eq!(
        parse_and_validate_manifest(unknown.as_bytes(), 1_800_000_000, expected()),
        Err("invalid_manifest")
    );
}

#[derive(Default)]
struct Cache {
    values: BTreeMap<String, Vec<u8>>,
    inserts: usize,
}

impl VerifiedCache for Cache {
    fn get(&self, digest: &str, maximum_bytes: u64) -> Option<Vec<u8>> {
        self.values
            .get(digest)
            .filter(|value| value.len() as u64 <= maximum_bytes)
            .cloned()
    }

    fn insert(&mut self, digest: &str, bytes: &[u8]) -> Result<(), ArtifactIoError> {
        self.inserts += 1;
        self.values.insert(digest.to_owned(), bytes.to_vec());
        Ok(())
    }
}

struct Source {
    bytes: Vec<u8>,
    calls: Vec<u64>,
    failures: usize,
}

impl ArtifactSource for Source {
    fn read(&mut self, _: &str, offset: u64, limit: usize) -> Result<Vec<u8>, ArtifactIoError> {
        self.calls.push(offset);
        if self.failures > 0 {
            self.failures -= 1;
            return Err(ArtifactIoError);
        }
        let start = usize::try_from(offset).unwrap();
        Ok(self.bytes[start..]
            .iter()
            .copied()
            .take(limit.min(2))
            .collect())
    }
}

fn artifact(digest: &str) -> Artifact {
    Artifact {
        name: "asb-tui".into(),
        kind: ArtifactKind::Executable,
        url: "https://github.com/martin-beck/asb-tui/releases/download/v0.1.0/asb-tui".into(),
        size: 5,
        sha256: digest.into(),
    }
}

#[test]
fn bounded_retry_resumes_then_populates_and_reuses_verified_cache() {
    let mut source = Source {
        bytes: b"hello".to_vec(),
        calls: Vec::new(),
        failures: 2,
    };
    let mut cache = Cache::default();
    assert_eq!(
        obtain_artifact(&artifact(HELLO_SHA), &mut source, &mut cache),
        Ok(b"hello".to_vec())
    );
    assert_eq!(source.calls, [0, 0, 0, 2, 4]);
    assert_eq!(cache.inserts, 1);

    let calls = source.calls.len();
    assert_eq!(
        obtain_artifact(&artifact(HELLO_SHA), &mut source, &mut cache),
        Ok(b"hello".to_vec())
    );
    assert_eq!(
        source.calls.len(),
        calls,
        "verified cache must avoid transfer"
    );
    assert_eq!(cache.inserts, 1);
}

#[test]
fn corrupted_cache_is_never_trusted_and_tampered_download_is_never_inserted() {
    let mut cache = Cache::default();
    cache.values.insert(HELLO_SHA.into(), b"jello".to_vec());
    let mut source = Source {
        bytes: b"world".to_vec(),
        calls: Vec::new(),
        failures: 0,
    };
    assert_eq!(
        obtain_artifact(&artifact(HELLO_SHA), &mut source, &mut cache),
        Err("artifact_digest_mismatch")
    );
    assert_eq!(cache.inserts, 0);
}

#[test]
fn transfer_stops_after_three_failures_without_cache_side_effects() {
    let mut source = Source {
        bytes: b"hello".to_vec(),
        calls: Vec::new(),
        failures: 3,
    };
    let mut cache = Cache::default();
    assert_eq!(
        obtain_artifact(&artifact(HELLO_SHA), &mut source, &mut cache),
        Err("artifact_transfer_failed")
    );
    assert_eq!(source.calls, [0, 0, 0]);
    assert_eq!(cache.inserts, 0);
}

fn policy_documents() -> BTreeMap<String, Vec<u8>> {
    let mut documents = BTreeMap::new();
    documents.insert(
        "licenses".into(),
        br#"{"schema_version":1,"release":"v0.1.0","packages":[{"name":"asb-tui","version":"0.1.0","license":"MIT"},{"name":"agent-workflow-coordinator","version":"0.3.5","license":"MIT"},{"name":"agent-workflow-quality","version":"0.23.0","license":"MIT"}]}"#.to_vec(),
    );
    documents.insert(
        "sbom".into(),
        br#"{"spdxVersion":"SPDX-2.3","name":"asb-tui-v0.1.0","packages":[{"name":"asb-tui"},{"name":"agent-workflow-coordinator"},{"name":"agent-workflow-quality"}]}"#.to_vec(),
    );
    documents.insert(
        "provenance".into(),
        format!(
            r#"{{"schema_version":1,"release":"v0.1.0","source_commit":"{}","source_tree":"{}","builder":"github-actions","reproducible":true}}"#,
            "a".repeat(40),
            "b".repeat(40)
        )
        .into_bytes(),
    );
    documents
}

#[test]
fn license_sbom_and_provenance_policy_is_bound_to_the_manifest() {
    let parsed = parse_and_validate_manifest(&manifest(), 1_800_000_000, expected()).unwrap();
    let documents = policy_documents();
    assert_eq!(validate_bundle_documents(&parsed, &documents), Ok(()));

    let mut denied = documents.clone();
    denied.insert(
        "licenses".into(),
        String::from_utf8(denied["licenses"].clone())
            .unwrap()
            .replace("\"MIT\"", "\"GPL-3.0-only\"")
            .into_bytes(),
    );
    assert_eq!(
        validate_bundle_documents(&parsed, &denied),
        Err("license_policy_rejected")
    );

    let mut incomplete = documents.clone();
    incomplete.insert(
        "sbom".into(),
        br#"{"spdxVersion":"SPDX-2.3","name":"asb-tui-v0.1.0","packages":[{"name":"asb-tui"}]}"#
            .to_vec(),
    );
    assert_eq!(
        validate_bundle_documents(&parsed, &incomplete),
        Err("sbom_incomplete")
    );

    let mut substituted = documents;
    substituted.insert(
        "provenance".into(),
        String::from_utf8(substituted["provenance"].clone())
            .unwrap()
            .replace(&"a".repeat(40), &"c".repeat(40))
            .into_bytes(),
    );
    assert_eq!(
        validate_bundle_documents(&parsed, &substituted),
        Err("provenance_invalid")
    );
}
