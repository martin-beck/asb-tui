// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use asb_tui::{
    bundle::{ExpectedCompatibility, parse_and_validate_manifest},
    compatibility::Architecture,
    lifecycle::{
        ExecutableSelfTest, FilesystemLifecycle, FrontendLauncher, Installation, LifecycleIoError,
        LifecycleStore, ProcessLauncher, SelfTest, install, launch, local_self_test_response,
        remove, status,
    },
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, process::Command};
use support::PrivateDirectory;

fn manifest() -> asb_tui::bundle::BundleManifest {
    parse_and_validate_manifest(
        include_bytes!("fixtures/bundle/manifest.json"),
        1_800_000_000,
        ExpectedCompatibility {
            bundle: "asb-tui-v1-linux-x86_64",
            architecture: Architecture::X86_64,
            asb_version: "0.1.0",
            protocol_version: 1,
        },
    )
    .unwrap()
}

fn artifacts() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([("asb-tui".into(), b"hello".to_vec())])
}

#[derive(Default)]
struct Store {
    active: Option<(Installation, Vec<u8>)>,
    staged: Option<(Installation, Vec<u8>)>,
    corrupt_stage: bool,
    fail_activation: bool,
    events: Vec<&'static str>,
    benchmark_processes: usize,
}

impl LifecycleStore for Store {
    fn active(&self) -> Result<Option<Installation>, LifecycleIoError> {
        Ok(self.active.as_ref().map(|(state, _)| state.clone()))
    }

    fn active_executable(&self, installation: &Installation) -> Result<Vec<u8>, LifecycleIoError> {
        self.active
            .as_ref()
            .filter(|(state, _)| state == installation)
            .map(|(_, bytes)| bytes.clone())
            .ok_or(LifecycleIoError)
    }

    fn stage(
        &mut self,
        installation: &Installation,
        executable: &[u8],
    ) -> Result<(), LifecycleIoError> {
        self.events.push("stage");
        let bytes = if self.corrupt_stage {
            b"jello".to_vec()
        } else {
            executable.to_vec()
        };
        self.staged = Some((installation.clone(), bytes));
        Ok(())
    }

    fn staged_executable(&self, installation: &Installation) -> Result<Vec<u8>, LifecycleIoError> {
        self.staged
            .as_ref()
            .filter(|(state, _)| state == installation)
            .map(|(_, bytes)| bytes.clone())
            .ok_or(LifecycleIoError)
    }

    fn activate(&mut self, _: &Installation) -> Result<(), LifecycleIoError> {
        self.events.push("activate");
        if self.fail_activation {
            return Err(LifecycleIoError);
        }
        self.active = self.staged.take();
        Ok(())
    }

    fn discard_stage(&mut self, _: &Installation) -> Result<(), LifecycleIoError> {
        self.events.push("discard");
        self.staged = None;
        Ok(())
    }

    fn remove(&mut self) -> Result<(), LifecycleIoError> {
        self.events.push("remove");
        self.active = None;
        self.staged = None;
        Ok(())
    }
}

struct Probe {
    pass: bool,
    calls: usize,
}

impl SelfTest for Probe {
    fn verify_protocol_and_terminal(&mut self, installation: &Installation, bytes: &[u8]) -> bool {
        self.calls += 1;
        installation.classification == "verified_extension" && bytes == b"hello" && self.pass
    }
}

#[derive(Default)]
struct Launcher(usize);

impl FrontendLauncher for Launcher {
    fn launch_frontend(&mut self, _: &Installation, _: &[u8]) -> Result<(), LifecycleIoError> {
        self.0 += 1;
        Ok(())
    }
}

#[test]
fn install_stages_rereads_self_tests_then_activates() {
    let mut store = Store {
        benchmark_processes: 3,
        ..Store::default()
    };
    let mut probe = Probe {
        pass: true,
        calls: 0,
    };
    let installed = install(&manifest(), &artifacts(), &mut store, &mut probe).unwrap();
    assert_eq!(store.events, ["stage", "activate"]);
    assert_eq!(probe.calls, 1);
    assert_eq!(store.benchmark_processes, 3);
    assert_eq!(status(&store).release, Some(installed.release));
    assert!(!status(&store).verified);
    assert_eq!(status(&store).reason, "installation_verification_failed");
}

#[test]
fn corrupt_stage_or_failed_self_test_rolls_back_without_replacing_active_version() {
    let mut store = Store::default();
    let mut passing = Probe {
        pass: true,
        calls: 0,
    };
    let original = install(&manifest(), &artifacts(), &mut store, &mut passing).unwrap();
    store.corrupt_stage = true;
    assert_eq!(
        install(&manifest(), &artifacts(), &mut store, &mut passing),
        Err("install_self_test_failed")
    );
    assert_eq!(store.active.as_ref().unwrap().0, original);
    assert!(store.staged.is_none());

    store.corrupt_stage = false;
    let mut failing = Probe {
        pass: false,
        calls: 0,
    };
    assert_eq!(
        install(&manifest(), &artifacts(), &mut store, &mut failing),
        Err("install_self_test_failed")
    );
    assert_eq!(store.active.as_ref().unwrap().0, original);
}

#[test]
fn status_is_read_only_and_detects_tampering() {
    let mut store = Store::default();
    assert_eq!(status(&store).reason, "extension_not_installed");
    let mut probe = Probe {
        pass: true,
        calls: 0,
    };
    install(&manifest(), &artifacts(), &mut store, &mut probe).unwrap();
    let events = store.events.clone();
    store.active.as_mut().unwrap().1 = b"jello".to_vec();
    let observed = status(&store);
    assert!(observed.installed);
    assert!(!observed.verified);
    assert_eq!(observed.reason, "installation_verification_failed");
    assert_eq!(store.events, events);
}

#[test]
fn launch_rechecks_self_test_and_remove_never_touches_benchmark_processes() {
    let mut store = Store {
        benchmark_processes: 2,
        ..Store::default()
    };
    let mut probe = Probe {
        pass: true,
        calls: 0,
    };
    install(&manifest(), &artifacts(), &mut store, &mut probe).unwrap();
    let mut launcher = Launcher::default();
    assert_eq!(
        launch(&store, &mut probe, &mut launcher),
        Err("installation_verification_failed")
    );
    assert_eq!(launcher.0, 0);
    assert_eq!(store.benchmark_processes, 2);

    let mut rejected = Probe {
        pass: false,
        calls: 0,
    };
    assert_eq!(
        launch(&store, &mut rejected, &mut launcher),
        Err("installation_verification_failed")
    );
    assert_eq!(launcher.0, 0);
    remove(&mut store).unwrap();
    assert_eq!(store.benchmark_processes, 2);
    assert_eq!(status(&store).reason, "extension_not_installed");
}

#[test]
fn filesystem_install_is_private_atomic_idempotent_and_removable() {
    let directory = PrivateDirectory::create();
    let mut store = FilesystemLifecycle::open(directory.path()).unwrap();
    let mut probe = Probe {
        pass: true,
        calls: 0,
    };
    let installed = install(&manifest(), &artifacts(), &mut store, &mut probe).unwrap();
    assert!(!status(&store).verified);
    assert_eq!(
        fs::metadata(directory.path().join("active.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let binary = directory
        .path()
        .join("versions")
        .join(&installed.executable_sha256)
        .join("asb-tui");
    assert_eq!(fs::read(&binary).unwrap(), b"hello");
    assert_eq!(
        fs::metadata(binary).unwrap().permissions().mode() & 0o777,
        0o700
    );
    install(&manifest(), &artifacts(), &mut store, &mut probe).unwrap();
    assert!(!status(&store).verified);
    remove(&mut store).unwrap();
    assert_eq!(status(&store).reason, "extension_not_installed");
    assert_eq!(
        fs::read_dir(directory.path().join("versions"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn filesystem_upgrade_reconnect_and_existing_version_reuse_are_verified() {
    let directory = PrivateDirectory::create();
    let mut store = FilesystemLifecycle::open(directory.path()).unwrap();
    let mut first = Installation {
        schema_version: 1,
        release: "v1.0.0".into(),
        executable_sha256: "486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7"
            .into(),
        source_commit: "a".repeat(40),
        source_tree: "b".repeat(40),
        target: "x86_64-unknown-linux-gnu".into(),
        bundle: "asb-tui-v1-linux-x86_64".into(),
        asb_version: "0.1.0".into(),
        protocol_version: 1,
        coordinator_version: "v0.3.5".into(),
        coordinator_commit: "510817b93feb80dde13e5a6c61d657954fae2346".into(),
        quality_version: "v0.23.0".into(),
        quality_commit: "8a9f056b7fc7926b9465a0f7a09225d4da1c572a".into(),
        classification: "verified_extension".into(),
    };
    store.stage(&first, b"world").unwrap();
    store.activate(&first).unwrap();
    assert!(!status(&store).verified);

    first.release = "v1.1.0".into();
    first.executable_sha256 =
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into();
    store.stage(&first, b"hello").unwrap();
    store.activate(&first).unwrap();
    drop(store);
    let mut reconnected = FilesystemLifecycle::open(directory.path()).unwrap();
    assert_eq!(status(&reconnected).release.as_deref(), Some("v1.1.0"));

    let binary = directory
        .path()
        .join("versions")
        .join(&first.executable_sha256)
        .join("asb-tui");
    fs::write(&binary, b"jello").unwrap();
    reconnected.stage(&first, b"hello").unwrap();
    assert!(reconnected.activate(&first).is_err());
    assert_eq!(
        status(&reconnected).reason,
        "installation_verification_failed"
    );
}

#[test]
fn filesystem_open_recovers_interrupted_stage_and_retains_original_directory() {
    let directory = PrivateDirectory::create();
    let versions = directory.path().join("versions");
    let stale = versions.join(".stage-stale");
    fs::create_dir(&versions).unwrap();
    fs::set_permissions(&versions, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(&stale).unwrap();
    let store = FilesystemLifecycle::open(directory.path()).unwrap();
    assert!(!stale.exists());

    let replacement = directory.path().with_extension("replacement");
    fs::rename(directory.path(), &replacement).unwrap();
    fs::create_dir(directory.path()).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(status(&store).reason, "extension_not_installed");
    fs::remove_dir(directory.path()).unwrap();
    fs::rename(&replacement, directory.path()).unwrap();
}

#[test]
fn filesystem_rejects_public_or_symlink_roots() {
    let directory = PrivateDirectory::create();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(FilesystemLifecycle::open(directory.path()).is_err());
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let link = directory.path().with_extension("link");
    std::os::unix::fs::symlink(directory.path(), &link).unwrap();
    assert!(FilesystemLifecycle::open(&link).is_err());
    fs::remove_file(link).unwrap();
}

#[test]
fn filesystem_rejects_symlink_or_public_lifecycle_lock() {
    let directory = PrivateDirectory::create();
    let target = directory.path().join("target-lock");
    fs::write(&target, b"").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    let lock = directory.path().join(".lifecycle.lock");
    std::os::unix::fs::symlink(&target, &lock).unwrap();
    assert!(FilesystemLifecycle::open(directory.path()).is_err());
    fs::remove_file(&lock).unwrap();
    fs::write(&lock, b"").unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(FilesystemLifecycle::open(directory.path()).is_err());
}

#[test]
fn filesystem_rejects_concurrent_lifecycle_owner_without_waiting() {
    let directory = PrivateDirectory::create();
    let _store = FilesystemLifecycle::open(directory.path()).unwrap();
    assert!(FilesystemLifecycle::open(directory.path()).is_err());
}

#[test]
fn executable_self_test_runs_exact_candidate_and_requires_closed_ready_response() {
    const CHILD: &str = "ASB_TUI_EXECUTABLE_SELF_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let command = format!(
            "{} --exact executable_self_test_runs_exact_candidate_and_requires_closed_ready_response --nocapture",
            std::env::current_exe().unwrap().display()
        );
        let status = Command::new("/usr/bin/script")
            .args(["-q", "-e", "-c", &command, "/dev/null"])
            .env(CHILD, "1")
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .status()
            .unwrap();
        assert!(status.success(), "PTY self-test probe failed");
        return;
    }

    let mut store = Store::default();
    let mut probe = Probe {
        pass: true,
        calls: 0,
    };
    let installed = install(&manifest(), &artifacts(), &mut store, &mut probe).unwrap();
    let response = format!(
        concat!(
            "{{\"schema_version\":1,\"classification\":\"verified_extension\",",
            "\"release\":\"{}\",\"target\":\"{}\",",
            "\"source_commit\":\"{}\",\"source_tree\":\"{}\",",
            "\"asb_version\":\"{}\",",
            "\"protocol_version\":1,",
            "\"coordinator_version\":\"{}\",\"coordinator_commit\":\"{}\",",
            "\"quality_version\":\"{}\",\"quality_commit\":\"{}\",\"ready\":true}}"
        ),
        installed.release,
        installed.target,
        installed.source_commit,
        installed.source_tree,
        installed.asb_version,
        installed.coordinator_version,
        installed.coordinator_commit,
        installed.quality_version,
        installed.quality_commit
    );
    let candidate = format!(
        concat!(
            "#!/bin/sh\nset -eu\n",
            "test -t 0\n",
            "case $XDG_CONFIG_HOME in /proc/self/fd/*) ;; *) exit 11;; esac\n",
            "case $XDG_CACHE_HOME in /proc/self/fd/*) ;; *) exit 12;; esac\n",
            "test -d \"$XDG_CONFIG_HOME\" && test -d \"$XDG_CACHE_HOME\"\n",
            "printf probe >\"$XDG_CONFIG_HOME/write-test\"\n",
            "printf probe >\"$XDG_CACHE_HOME/write-test\"\n",
            "test \"$1\" = lifecycle-self-test && test \"$2\" = --release\n",
            "test \"$4\" = --asb-version && test \"$5\" = 0.1.0\n",
            "test \"$6\" = --protocol-version && test \"$7\" = 1\n",
            "test \"$8\" = --format && test \"$9\" = json\n",
            "printf '%s\\n' '{}'\n"
        ),
        response
    );
    let directory = PrivateDirectory::create();
    let original = directory.path().join("original");
    let retained = directory.path().join("retained");
    fs::create_dir(&original).unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_store = FilesystemLifecycle::open(&original).unwrap();
    let mut self_test = ExecutableSelfTest::for_store_with_supervisor(
        &runtime_store,
        std::path::Path::new(env!("CARGO_BIN_EXE_asb-tui")),
    )
    .unwrap();
    fs::rename(&original, &retained).unwrap();
    fs::create_dir(&original).unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(self_test.verify_protocol_and_terminal(&installed, candidate.as_bytes()));
    let runtime_artifacts = || {
        fs::read_dir(&retained)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".self-test-runtime-")
            })
            .count()
    };
    assert_eq!(runtime_artifacts(), 0, "successful probe residue remained");
    assert!(
        self_test.verify_protocol_and_terminal(&installed, candidate.as_bytes()),
        "a second probe should receive a fresh private runtime"
    );
    assert_eq!(runtime_artifacts(), 0, "repeated probe residue remained");
    assert!(
        fs::read_dir(&original).unwrap().next().is_none(),
        "the replacement root was modified"
    );

    let hostile = candidate.replace("\"ready\":true", "\"ready\":true,\"host\":\"secret\"");
    assert!(!self_test.verify_protocol_and_terminal(&installed, hostile.as_bytes()));
    assert_eq!(runtime_artifacts(), 0, "rejected probe residue remained");
    let crashing = "#!/bin/sh\nset -eu\nmkdir -p \"$XDG_CONFIG_HOME/nested\"\nprintf residue >\"$XDG_CONFIG_HOME/nested/file\"\nkill -KILL $$\n";
    assert!(!self_test.verify_protocol_and_terminal(&installed, crashing.as_bytes()));
    assert_eq!(runtime_artifacts(), 0, "crashed probe residue remained");
    assert!(!self_test.verify_protocol_and_terminal(&installed, b"not executable"));
    assert_eq!(runtime_artifacts(), 0, "spawn failure residue remained");

    let assert_reaped = |marker: &std::path::Path| {
        for _ in 0..100 {
            if let Ok(pid) = fs::read_to_string(marker).map(|value| value.trim().to_owned())
                && !std::path::Path::new(&format!("/proc/{pid}")).exists()
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("candidate descendant remained live: {}", marker.display());
    };

    let rejected_marker = directory.path().join("rejected-child.pid");
    let rejected = format!(
        "#!/bin/sh\nsleep 30 & echo $! >'{}'\nexit 7\n",
        rejected_marker.display()
    );
    let started = std::time::Instant::now();
    assert!(!self_test.verify_protocol_and_terminal(&installed, rejected.as_bytes()));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_reaped(&rejected_marker);

    let escaped_marker = directory.path().join("escaped-child.pid");
    let escape_denied = directory.path().join("setsid-denied");
    let regroup_denied = directory.path().join("setpgid-denied");
    let escaped = format!(
        concat!(
            "#!/bin/sh\n",
            "setsid sh -c 'sleep 30' || printf denied >'{}'\n",
            "/usr/bin/python3 -c 'import os; os.setpgid(0, 0)' || printf denied >'{}'\n",
            "(sh -c 'sleep 30 & echo $! >\"{}\"' &)\n",
            "while [ ! -s '{}' ]; do :; done\n",
            "exit 7\n"
        ),
        escape_denied.display(),
        regroup_denied.display(),
        escaped_marker.display(),
        escaped_marker.display()
    );
    assert!(!self_test.verify_protocol_and_terminal(&installed, escaped.as_bytes()));
    assert_eq!(fs::read_to_string(&escape_denied).unwrap(), "denied");
    assert_eq!(fs::read_to_string(&regroup_denied).unwrap(), "denied");
    assert_reaped(&escaped_marker);

    let success_marker = directory.path().join("success-child.pid");
    let success_with_child = candidate.replace(
        "printf '%s\\n'",
        &format!(
            "sleep 30 & echo $! >'{}'\nprintf '%s\\n'",
            success_marker.display()
        ),
    );
    assert!(self_test.verify_protocol_and_terminal(&installed, success_with_child.as_bytes()));
    assert_reaped(&success_marker);

    let timeout_marker = directory.path().join("timeout-child.pid");
    let timeout = format!(
        "#!/bin/sh\nsleep 30 & echo $! >'{}'\nsleep 30\n",
        timeout_marker.display()
    );
    assert!(!self_test.verify_protocol_and_terminal(&installed, timeout.as_bytes()));
    assert_reaped(&timeout_marker);

    for (name, body) in [
        ("malformed", "printf 'not-json\\n'"),
        ("oversized", "head -c 70000 /dev/zero"),
        ("signalled", "kill -TERM $$"),
    ] {
        let marker = directory.path().join(format!("{name}-child.pid"));
        let probe = format!(
            "#!/bin/sh\nsleep 30 & echo $! >'{}'\n{body}\n",
            marker.display()
        );
        assert!(!self_test.verify_protocol_and_terminal(&installed, probe.as_bytes()));
        assert_reaped(&marker);
    }

    for repetition in 0..10 {
        let marker = directory
            .path()
            .join(format!("race-child-{repetition}.pid"));
        let probe = format!(
            "#!/bin/sh\nsleep 30 & echo $! >'{}'\nexit 7\n",
            marker.display()
        );
        assert!(!self_test.verify_protocol_and_terminal(&installed, probe.as_bytes()));
        assert_reaped(&marker);
    }

    let ready = directory.path().join("concurrent-ready");
    let concurrent_candidate = candidate.replace(
        "printf '%s\\n'",
        &format!(
            "printf ready >'{}'\nsleep 0.2\nprintf '%s\\n'",
            ready.display()
        ),
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    let unrelated_thread = std::thread::spawn(move || {
        for _ in 0..100 {
            if ready.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        sender
            .send(Command::new("/usr/bin/sleep").arg("30").spawn().unwrap())
            .unwrap();
    });
    assert!(self_test.verify_protocol_and_terminal(&installed, concurrent_candidate.as_bytes()));
    unrelated_thread.join().unwrap();
    let mut unrelated = receiver.recv().unwrap();
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "concurrently spawned unrelated child was signalled or reaped"
    );
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();
}

#[test]
fn process_launcher_requires_a_controlling_terminal() {
    const CHILD: &str = "ASB_TUI_LAUNCHER_NO_TTY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new("/usr/bin/setsid")
            .args([
                "--fork",
                "--wait",
                std::env::current_exe().unwrap().to_str().unwrap(),
                "--exact",
                "process_launcher_requires_a_controlling_terminal",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success(), "detached launcher probe failed");
        return;
    }

    let mut store = Store::default();
    let mut probe = Probe {
        pass: true,
        calls: 0,
    };
    let installed = install(&manifest(), &artifacts(), &mut store, &mut probe).unwrap();
    let mut launcher = ProcessLauncher;
    assert!(
        launcher
            .launch_frontend(&installed, b"#!/bin/sh\nexit 0\n")
            .is_err()
    );
}

#[test]
fn local_self_test_response_validates_release_and_reports_observed_readiness() {
    assert!(local_self_test_response("1.2.3", "0.1.0", 1).is_none());
    assert!(local_self_test_response("v1.2", "0.1.0", 1).is_none());
    assert!(local_self_test_response("v1.two.3", "0.1.0", 1).is_none());
    assert!(local_self_test_response("v1.2.3", "9.9.9", 1).is_none());
    assert!(local_self_test_response("v1.2.3", "0.1.0", 2).is_none());
    let response = local_self_test_response("v1.2.3", "0.1.0", 1).unwrap();
    assert_eq!(response.classification, "source_only_unverified");
    assert!(!response.ready);
}
