// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{ReleaseProbe, verify_lock_signature, verify_release_lock_contents};
use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct Probe {
    mutable: bool,
    authenticated: bool,
    tampered: bool,
}

impl ReleaseProbe for Probe {
    fn tag_object(&self, dependency: &str, _: &str) -> Result<String, String> {
        if self.mutable {
            return Ok("0".repeat(40));
        }
        Ok(match dependency {
            "agent-workflow-coordinator" => "9e862e9e7af328e489b6e2fe958e5df1ddd702c1",
            _ => "79c699258111cc3ae4585d46c6ce4999784001b3",
        }
        .into())
    }
    fn commit(&self, dependency: &str, _: &str) -> Result<String, String> {
        if self.tampered {
            return Ok("1".repeat(40));
        }
        Ok(match dependency {
            "agent-workflow-coordinator" => "510817b93feb80dde13e5a6c61d657954fae2346",
            _ => "8a9f056b7fc7926b9465a0f7a09225d4da1c572a",
        }
        .into())
    }
    fn tree(&self, dependency: &str, _: &str) -> Result<String, String> {
        Ok(match dependency {
            "agent-workflow-coordinator" => "41d08ed42333cb47b07c2c401a9167b56c7cfb81",
            _ => "ca77db478f0737142690d37683d826710cd953b0",
        }
        .into())
    }
    fn tag_is_authenticated(&self, _: &str, _: &str) -> Result<bool, String> {
        Ok(self.authenticated)
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("provenance")
        .join(name)
}

fn unique_directory(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("asb-tui-{name}-{}-{nonce}", std::process::id()))
}

#[test]
fn signed_lock_and_exact_authenticated_releases_pass() {
    verify_lock_signature(
        &fixture("dependencies.lock.json"),
        &fixture("dependencies.lock.json.sig"),
        &fixture("allowed_signers"),
    )
    .unwrap();
    let contents = fs::read_to_string(fixture("dependencies.lock.json")).unwrap();
    verify_release_lock_contents(
        &contents,
        &Probe {
            mutable: false,
            authenticated: true,
            tampered: false,
        },
    )
    .unwrap();
}

#[test]
fn unsigned_tampered_and_mutable_inputs_fail_closed() {
    assert!(
        verify_lock_signature(
            &fixture("dependencies.lock.json"),
            &fixture("missing.sig"),
            &fixture("allowed_signers")
        )
        .is_err()
    );
    let contents = fs::read_to_string(fixture("dependencies.lock.json")).unwrap();
    let tampered_directory = unique_directory("tampered-lock");
    fs::create_dir(&tampered_directory).expect("create unique tampered fixture directory");
    let tampered_path = tampered_directory.join("dependencies.lock.json");
    let mut tampered = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tampered_path)
        .expect("create unique tampered fixture");
    tampered
        .write_all(contents.replace("v0.3.5", "v0.3.6").as_bytes())
        .unwrap();
    drop(tampered);
    assert!(
        verify_lock_signature(
            &tampered_path,
            &fixture("dependencies.lock.json.sig"),
            &fixture("allowed_signers")
        )
        .is_err()
    );
    fs::remove_dir_all(tampered_directory).unwrap();
    for probe in [
        Probe {
            mutable: true,
            authenticated: true,
            tampered: false,
        },
        Probe {
            mutable: false,
            authenticated: false,
            tampered: false,
        },
        Probe {
            mutable: false,
            authenticated: true,
            tampered: true,
        },
    ] {
        assert!(verify_release_lock_contents(&contents, &probe).is_err());
    }
}

#[test]
fn substituted_signer_is_rejected_before_signature_acceptance() {
    let directory = unique_directory("substitute-signer");
    fs::create_dir(&directory).expect("create unique signer fixture directory");
    let key = directory.join("key");
    assert!(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .status()
            .expect("generate substitute key")
            .success()
    );
    let public = fs::read_to_string(key.with_extension("pub")).unwrap();
    let signers = directory.join("allowed_signers");
    fs::write(&signers, format!("martin.beck2@gmx.de {public}")).unwrap();
    assert!(
        verify_lock_signature(
            &fixture("dependencies.lock.json"),
            &fixture("dependencies.lock.json.sig"),
            &signers,
        )
        .is_err()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn malformed_and_incomplete_release_locks_fail_closed() {
    let contents = fs::read_to_string(fixture("dependencies.lock.json")).unwrap();
    let mutations = [
        contents.replace("\"schema_version\": 1", "\"schema_version\": 2"),
        contents.replace(
            "SHA256:a36V6yPvRZyxnQ2113tiA/MlHt7mPfJEXAGByBXVkuE",
            "SHA256:wrong",
        ),
        contents.replace(
            "\"classification\": \"unavailable\"",
            "\"classification\": \"verified\"",
        ),
        contents.replace("\"license\": \"MIT\"", "\"license\": \"unknown\""),
        contents.replace(
            "https://github.com/martin-beck/agent-workflow-coordinator",
            "https://example.invalid/mutable",
        ),
        contents.replace(
            "a51e58ed71dd93979acc55560fc8208db7131d8e6d0a6b7b805fa383fef25b34",
            "short",
        ),
        contents.replace("\"archive_size\": 124386", "\"archive_size\": 0"),
        contents.replace(
            "\"manifest_sha256\": \"05bbb0d4",
            "\"archive_sha256\": \"05bbb0d4",
        ),
        contents.replace("agent-workflow-quality", "unexpected-quality-tool"),
        contents.replacen("agent-workflow-quality", "agent-workflow-coordinator", 1),
        format!("{contents} trailing"),
    ];
    let probe = Probe {
        mutable: false,
        authenticated: true,
        tampered: false,
    };
    for mutation in mutations {
        assert!(verify_release_lock_contents(&mutation, &probe).is_err());
    }
}

#[test]
fn command_verifies_the_signed_lock_through_an_exact_git_probe() {
    let directory = unique_directory("git-probe");
    fs::create_dir(&directory).expect("create unique Git probe fixture directory");
    let coordinator = directory.join("agent-workflow-coordinator");
    let quality = directory.join("agent-workflow-quality");
    fs::create_dir(&coordinator).unwrap();
    fs::create_dir(&quality).unwrap();
    let git = directory.join("git");
    fs::write(&git, r#"#!/bin/sh
set -eu
repo=$2
operation=$3
value=${4-}
case "$*" in *" verify-tag "*) exit 0;; esac
case "$operation:$value:$repo" in
  rev-parse:refs/tags/v0.3.5\^\{tag\}:*agent-workflow-coordinator) echo 9e862e9e7af328e489b6e2fe958e5df1ddd702c1 ;;
  rev-parse:9e862e9e7af328e489b6e2fe958e5df1ddd702c1\^\{commit\}:*) echo 510817b93feb80dde13e5a6c61d657954fae2346 ;;
  rev-parse:510817b93feb80dde13e5a6c61d657954fae2346\^\{tree\}:*) echo 41d08ed42333cb47b07c2c401a9167b56c7cfb81 ;;
  rev-parse:refs/tags/v0.23.0\^\{tag\}:*agent-workflow-quality) echo 79c699258111cc3ae4585d46c6ce4999784001b3 ;;
  rev-parse:79c699258111cc3ae4585d46c6ce4999784001b3\^\{commit\}:*) echo 8a9f056b7fc7926b9465a0f7a09225d4da1c572a ;;
  rev-parse:8a9f056b7fc7926b9465a0f7a09225d4da1c572a\^\{tree\}:*) echo ca77db478f0737142690d37683d826710cd953b0 ;;
  *) exit 1 ;;
esac
"#).unwrap();
    let mut permissions = fs::metadata(&git).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&git, permissions).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verify-release-lock"))
        .args([
            fixture("dependencies.lock.json"),
            fixture("dependencies.lock.json.sig"),
            fixture("allowed_signers"),
            coordinator,
            quality,
        ])
        .env("PATH", format!("{}:/usr/bin:/bin", directory.display()))
        .current_dir(&directory)
        .output()
        .expect("run release verifier command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "authenticated immutable release lock verified\n"
    );
    fs::remove_dir_all(directory).unwrap();
}
