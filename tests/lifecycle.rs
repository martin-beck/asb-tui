// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

mod support;

use asb_tui::{
    bundle::{ExpectedCompatibility, parse_and_validate_manifest},
    compatibility::Architecture,
    lifecycle::{
        FilesystemLifecycle, FrontendLauncher, Installation, LifecycleIoError, LifecycleStore,
        SelfTest, install, launch, remove, status,
    },
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
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
        installation.classification == "unverified_extension" && bytes == b"hello" && self.pass
    }
}

#[derive(Default)]
struct Launcher(usize);

impl FrontendLauncher for Launcher {
    fn launch_frontend(&mut self, _: &Installation) -> Result<(), LifecycleIoError> {
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
    assert!(status(&store).verified);
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
    launch(&store, &mut probe, &mut launcher).unwrap();
    assert_eq!(launcher.0, 1);
    assert_eq!(store.benchmark_processes, 2);

    let mut rejected = Probe {
        pass: false,
        calls: 0,
    };
    assert_eq!(
        launch(&store, &mut rejected, &mut launcher),
        Err("launch_self_test_failed")
    );
    assert_eq!(launcher.0, 1);
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
    assert!(status(&store).verified);
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
    assert!(status(&store).verified);
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
