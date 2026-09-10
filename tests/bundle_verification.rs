// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use asb_tui::{
    bundle::{
        Artifact, ArtifactIoError, ArtifactKind, ArtifactSource, CurlSource, ExpectedCompatibility,
        FilesystemCache, VerifiedCache, obtain_artifact, obtain_verified_bundle,
        parse_and_validate_manifest, validate_bundle_documents, verify_bundle_manifest,
        verify_local_bundle_artifacts,
    },
    compatibility::Architecture,
};
use std::path::Path;
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};
use support::PrivateDirectory;

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
          "components":[
            {{"name":"asb-tui","version":"v0.1.0","commit":"{commit}","tree":"{tree}","artifact_sha256":"{HELLO_SHA}"}},
            {{"name":"agent-workflow-coordinator","version":"v0.3.5","commit":"510817b93feb80dde13e5a6c61d657954fae2346","tree":"41d08ed42333cb47b07c2c401a9167b56c7cfb81","artifact_sha256":"a51e58ed71dd93979acc55560fc8208db7131d8e6d0a6b7b805fa383fef25b34"}},
            {{"name":"agent-workflow-quality","version":"v0.23.0","commit":"8a9f056b7fc7926b9465a0f7a09225d4da1c572a","tree":"ca77db478f0737142690d37683d826710cd953b0","artifact_sha256":"9d480cabe955a5faf88f6f1c8dfd2bfe63045445173c86f93d200a00c97fff89"}}
          ],
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
    assert_eq!(parsed.artifacts().len(), 5);
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
fn appended_signer_is_rejected_even_when_the_fixture_uses_the_original_key() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let directory = PrivateDirectory::create();
    let key = directory.path().join("substitute");
    assert!(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .status()
            .unwrap()
            .success()
    );
    let mut allowed = std::fs::read(root.join("provenance/allowed_signers")).unwrap();
    allowed.extend_from_slice(b"substitute@example.invalid ");
    allowed.extend_from_slice(&std::fs::read(key.with_extension("pub")).unwrap());
    let substituted = directory.path().join("allowed_signers");
    std::fs::write(&substituted, allowed).unwrap();
    assert_eq!(
        verify_bundle_manifest(
            &std::fs::read(root.join("tests/fixtures/bundle/manifest.json")).unwrap(),
            &root.join("tests/fixtures/bundle/manifest.json.sig"),
            &substituted,
            1_800_000_000,
            expected(),
        ),
        Err("invalid_trust_anchor")
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

    let invalid_release = String::from_utf8(manifest())
        .unwrap()
        .replace("v0.1.0", "v0..1");
    assert_eq!(
        parse_and_validate_manifest(invalid_release.as_bytes(), 1_800_000_000, expected()),
        Err("invalid_manifest_policy")
    );

    let over_quota = String::from_utf8(manifest())
        .unwrap()
        .replace("\"size\":5", "\"size\":200000000");
    assert_eq!(
        parse_and_validate_manifest(over_quota.as_bytes(), 1_800_000_000, expected()),
        Err("artifact_quota_exceeded")
    );

    let substituted_component = String::from_utf8(manifest()).unwrap().replace(
        "41d08ed42333cb47b07c2c401a9167b56c7cfb81",
        "cccccccccccccccccccccccccccccccccccccccc",
    );
    assert_eq!(
        parse_and_validate_manifest(substituted_component.as_bytes(), 1_800_000_000, expected(),),
        Err("component_identity_mismatch")
    );
}

#[derive(Default)]
struct Cache {
    values: BTreeMap<String, Vec<u8>>,
    partial: BTreeMap<String, Vec<u8>>,
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

    fn get_partial(&self, digest: &str, maximum_bytes: u64) -> Option<Vec<u8>> {
        self.partial
            .get(digest)
            .filter(|value| (value.len() as u64) < maximum_bytes)
            .cloned()
    }

    fn store_partial(&mut self, digest: &str, bytes: &[u8]) -> Result<(), ArtifactIoError> {
        self.partial.insert(digest.into(), bytes.to_vec());
        Ok(())
    }

    fn clear_partial(&mut self, digest: &str) -> Result<(), ArtifactIoError> {
        self.partial.remove(digest);
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
        Ok(self.bytes[start..].iter().copied().take(limit).collect())
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
    let expected_bytes = vec![b'a'; 65_537];
    let expected_digest = digest(&expected_bytes);
    let mut source = Source {
        bytes: expected_bytes.clone(),
        calls: Vec::new(),
        failures: 2,
    };
    let mut cache = Cache::default();
    let mut item = artifact(&expected_digest);
    item.size = expected_bytes.len() as u64;
    assert_eq!(
        obtain_artifact(&item, &mut source, &mut cache),
        Ok(expected_bytes.clone())
    );
    assert_eq!(source.calls, [0, 0, 0, 65_536]);
    assert_eq!(cache.inserts, 1);
    assert!(cache.partial.is_empty());

    let calls = source.calls.len();
    assert_eq!(
        obtain_artifact(&item, &mut source, &mut cache),
        Ok(expected_bytes)
    );
    assert_eq!(
        source.calls.len(),
        calls,
        "verified cache must avoid transfer"
    );
    assert_eq!(cache.inserts, 1);
}

#[test]
fn interrupted_transfer_persists_unverified_prefix_and_resumes_next_invocation() {
    let expected_bytes = vec![b'z'; 65_537];
    let expected_digest = digest(&expected_bytes);
    let mut item = artifact(&expected_digest);
    item.size = expected_bytes.len() as u64;
    let mut cache = Cache::default();
    let interrupted = Source {
        bytes: expected_bytes.clone(),
        calls: Vec::new(),
        failures: 0,
    };
    struct OneChunkThenFail(Source);
    impl ArtifactSource for OneChunkThenFail {
        fn read(
            &mut self,
            url: &str,
            offset: u64,
            limit: usize,
        ) -> Result<Vec<u8>, ArtifactIoError> {
            if offset > 0 {
                Err(ArtifactIoError)
            } else {
                self.0.read(url, offset, limit)
            }
        }
    }
    let mut first = OneChunkThenFail(interrupted);
    assert_eq!(
        obtain_artifact(&item, &mut first, &mut cache),
        Err("artifact_transfer_failed")
    );
    assert_eq!(cache.partial[&expected_digest].len(), 65_536);

    let mut resumed = Source {
        bytes: expected_bytes.clone(),
        calls: Vec::new(),
        failures: 0,
    };
    assert_eq!(
        obtain_artifact(&item, &mut resumed, &mut cache),
        Ok(expected_bytes)
    );
    assert_eq!(resumed.calls, [65_536]);
    assert!(cache.partial.is_empty());
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
    let mut sbom: serde_json::Value =
        serde_json::from_slice(include_bytes!("../provenance/sbom.spdx.json")).unwrap();
    sbom["name"] = "asb-tui-v0.1.0".into();
    let packages = sbom["packages"].as_array_mut().unwrap();
    packages.push(serde_json::json!({
        "name": "agent-workflow-coordinator",
        "versionInfo": "0.3.5",
        "licenseDeclared": "MIT"
    }));
    packages.push(serde_json::json!({
        "name": "agent-workflow-quality",
        "versionInfo": "0.23.0",
        "licenseDeclared": "MIT"
    }));
    let license_packages: Vec<_> = packages
        .iter()
        .map(|package| {
            serde_json::json!({
                "name": package["name"],
                "version": package["versionInfo"],
                "license": package["licenseDeclared"],
            })
        })
        .collect();
    documents.insert(
        "licenses".into(),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "release": "v0.1.0",
            "packages": license_packages,
        }))
        .unwrap(),
    );
    documents.insert("sbom".into(), serde_json::to_vec(&sbom).unwrap());
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

fn digest(bytes: &[u8]) -> String {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    String::from_utf8(child.wait_with_output().unwrap().stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
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

    for (name, field, rejected) in [
        ("foldhash", "version", "0.2.1"),
        ("foldhash", "name", "other-zlib"),
        ("foldhash", "license", "BSD-3-Clause"),
        ("agent-workflow-coordinator", "license", "Zlib"),
    ] {
        let mut widened = policy_documents();
        let mut report: serde_json::Value = serde_json::from_slice(&widened["licenses"]).unwrap();
        let package = report["packages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|package| package["name"] == name)
            .unwrap();
        package[field] = rejected.into();
        widened.insert("licenses".into(), serde_json::to_vec(&report).unwrap());
        assert_eq!(
            validate_bundle_documents(&parsed, &widened),
            Err("license_policy_rejected")
        );
    }

    let license_report: serde_json::Value = serde_json::from_slice(&documents["licenses"]).unwrap();
    assert_eq!(
        license_report["packages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|package| package["name"] == "hashbrown")
            .count(),
        2,
        "distinct reviewed versions must remain representable"
    );

    let mut duplicate = policy_documents();
    let mut report: serde_json::Value = serde_json::from_slice(&duplicate["licenses"]).unwrap();
    let foldhash = report["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "foldhash")
        .unwrap()
        .clone();
    report["packages"].as_array_mut().unwrap().push(foldhash);
    duplicate.insert("licenses".into(), serde_json::to_vec(&report).unwrap());
    assert_eq!(
        validate_bundle_documents(&parsed, &duplicate),
        Err("license_policy_rejected")
    );

    let mut omitted_license = policy_documents();
    let mut report: serde_json::Value =
        serde_json::from_slice(&omitted_license["licenses"]).unwrap();
    report["packages"]
        .as_array_mut()
        .unwrap()
        .retain(|package| package["name"] != "foldhash");
    omitted_license.insert("licenses".into(), serde_json::to_vec(&report).unwrap());
    assert_eq!(
        validate_bundle_documents(&parsed, &omitted_license),
        Err("license_report_incomplete")
    );

    let mut extra_license = policy_documents();
    let mut report: serde_json::Value = serde_json::from_slice(&extra_license["licenses"]).unwrap();
    report["packages"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "unreviewed-extra",
            "version": "1.0.0",
            "license": "MIT"
        }));
    extra_license.insert("licenses".into(), serde_json::to_vec(&report).unwrap());
    assert_eq!(
        validate_bundle_documents(&parsed, &extra_license),
        Err("license_policy_rejected")
    );

    let mut incomplete = documents.clone();
    let mut sbom: serde_json::Value = serde_json::from_slice(&incomplete["sbom"]).unwrap();
    sbom["packages"]
        .as_array_mut()
        .unwrap()
        .retain(|package| package["name"] != "foldhash");
    incomplete.insert("sbom".into(), serde_json::to_vec(&sbom).unwrap());
    assert_eq!(
        validate_bundle_documents(&parsed, &incomplete),
        Err("sbom_incomplete")
    );

    let mut mismatched_sbom = policy_documents();
    let mut sbom: serde_json::Value = serde_json::from_slice(&mismatched_sbom["sbom"]).unwrap();
    let foldhash = sbom["packages"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|package| package["name"] == "foldhash")
        .unwrap();
    foldhash["licenseDeclared"] = "MIT".into();
    mismatched_sbom.insert("sbom".into(), serde_json::to_vec(&sbom).unwrap());
    assert_eq!(
        validate_bundle_documents(&parsed, &mismatched_sbom),
        Err("sbom_invalid")
    );

    let mut extra_sbom = policy_documents();
    let mut sbom: serde_json::Value = serde_json::from_slice(&extra_sbom["sbom"]).unwrap();
    sbom["packages"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "unreviewed-extra",
            "versionInfo": "1.0.0",
            "licenseDeclared": "MIT"
        }));
    extra_sbom.insert("sbom".into(), serde_json::to_vec(&sbom).unwrap());
    assert_eq!(
        validate_bundle_documents(&parsed, &extra_sbom),
        Err("sbom_invalid")
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

#[test]
fn local_artifact_directory_requires_complete_inode_stable_digest_valid_files() {
    let directory = PrivateDirectory::create();
    let mut artifacts = policy_documents();
    artifacts.insert("asb-tui".into(), b"hello".to_vec());
    artifacts.insert("source".into(), b"source archive".to_vec());
    let mut value: serde_json::Value = serde_json::from_slice(&manifest()).unwrap();
    for artifact in value["artifacts"].as_array_mut().unwrap() {
        let name = artifact["name"].as_str().unwrap().to_owned();
        let bytes = &artifacts[&name];
        artifact["size"] = (bytes.len() as u64).into();
        artifact["sha256"] = digest(bytes).into();
        std::fs::write(directory.path().join(&name), bytes).unwrap();
    }
    let parsed = parse_and_validate_manifest(
        &serde_json::to_vec(&value).unwrap(),
        1_800_000_000,
        expected(),
    )
    .unwrap();
    let verified = verify_local_bundle_artifacts(&parsed, directory.path()).unwrap();
    assert_eq!(verified.len(), 5);

    std::fs::write(directory.path().join("source"), b"source archivf").unwrap();
    assert_eq!(
        verify_local_bundle_artifacts(&parsed, directory.path()),
        Err("artifact_digest_mismatch")
    );
    std::fs::remove_file(directory.path().join("source")).unwrap();
    std::os::unix::fs::symlink("asb-tui", directory.path().join("source")).unwrap();
    assert_eq!(
        verify_local_bundle_artifacts(&parsed, directory.path()),
        Err("artifact_invalid")
    );
}

#[test]
fn filesystem_cache_is_owner_private_content_addressed_and_fail_closed() {
    let directory = PrivateDirectory::create();
    let mut cache = FilesystemCache::open(directory.path()).unwrap();
    cache.insert(HELLO_SHA, b"hello").unwrap();
    assert_eq!(cache.get(HELLO_SHA, 5), Some(b"hello".to_vec()));
    assert_eq!(cache.get(HELLO_SHA, 4), None);
    let entry = directory.path().join(HELLO_SHA);
    assert_eq!(
        std::fs::metadata(&entry).unwrap().permissions().mode() & 0o077,
        0
    );
    std::fs::write(&entry, b"jello").unwrap();
    assert_eq!(cache.get(HELLO_SHA, 5), Some(b"jello".to_vec()));
    cache.store_partial(HELLO_SHA, b"hel").unwrap();
    drop(cache);
    let mut reopened = FilesystemCache::open(directory.path()).unwrap();
    assert_eq!(reopened.get_partial(HELLO_SHA, 5), Some(b"hel".to_vec()));
    reopened.clear_partial(HELLO_SHA).unwrap();
    assert_eq!(reopened.get_partial(HELLO_SHA, 5), None);

    let public = PrivateDirectory::create();
    let mut permissions = std::fs::metadata(public.path()).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(public.path(), permissions).unwrap();
    assert!(FilesystemCache::open(public.path()).is_err());
}

#[test]
fn filesystem_cache_remains_bound_to_the_open_directory_after_path_replacement() {
    let directory = PrivateDirectory::create();
    let original = directory.path().to_owned();
    let moved = original.with_extension("retained");
    let mut cache = FilesystemCache::open(&original).unwrap();
    std::fs::rename(&original, &moved).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();

    cache.insert(HELLO_SHA, b"hello").unwrap();
    assert!(moved.join(HELLO_SHA).is_file());
    assert!(!original.join(HELLO_SHA).exists());

    std::fs::remove_dir(&original).unwrap();
    std::fs::rename(&moved, &original).unwrap();
}

#[test]
fn production_source_rejects_mutable_or_oversized_requests_before_network_access() {
    let mut source = CurlSource;
    assert_eq!(
        source.read("https://example.invalid/latest", 0, 1),
        Err(ArtifactIoError)
    );
    assert_eq!(
        source.read(
            "https://github.com/martin-beck/asb-tui/releases/download/v0.1.0/asb-tui",
            0,
            65_537,
        ),
        Err(ArtifactIoError)
    );
}

struct MappedSource(BTreeMap<String, Vec<u8>>);

impl ArtifactSource for MappedSource {
    fn read(&mut self, url: &str, offset: u64, limit: usize) -> Result<Vec<u8>, ArtifactIoError> {
        let bytes = self.0.get(url).ok_or(ArtifactIoError)?;
        let start = usize::try_from(offset).map_err(|_| ArtifactIoError)?;
        Ok(bytes[start..].iter().copied().take(limit).collect())
    }
}

#[test]
fn complete_bundle_is_returned_only_after_every_document_and_digest_passes() {
    let mut payloads = policy_documents();
    payloads.insert("asb-tui".into(), b"executable".to_vec());
    payloads.insert("source".into(), b"source archive".to_vec());
    let mut document: serde_json::Value = serde_json::from_slice(&manifest()).unwrap();
    for artifact in document["artifacts"].as_array_mut().unwrap() {
        let name = artifact["name"].as_str().unwrap();
        let bytes = payloads.get(name).unwrap();
        artifact["size"] = serde_json::Value::from(bytes.len() as u64);
        artifact["sha256"] = serde_json::Value::from(digest(bytes));
    }
    let executable_digest = document["artifacts"][0]["sha256"].clone();
    document["components"][0]["artifact_sha256"] = executable_digest;
    let manifest_bytes = serde_json::to_vec(&document).unwrap();
    let parsed = parse_and_validate_manifest(&manifest_bytes, 1_800_000_000, expected()).unwrap();
    let mut by_url = BTreeMap::new();
    for artifact in parsed.artifacts() {
        let bytes = payloads.get(&artifact.name).unwrap();
        by_url.insert(artifact.url.clone(), bytes.clone());
    }
    let mut source = MappedSource(by_url);
    let mut cache = Cache::default();
    let verified = obtain_verified_bundle(&parsed, &mut source, &mut cache).unwrap();
    assert_eq!(verified.len(), 5);
    assert_eq!(verified["asb-tui"], b"executable");
    assert_eq!(cache.inserts, 5);
}
