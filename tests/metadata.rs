// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::{collections::BTreeMap, fs, process::Command};

#[test]
fn sbom_is_closed_over_the_exact_cargo_lock() {
    let root = env!("CARGO_MANIFEST_DIR");
    let lock = fs::read_to_string(format!("{root}/Cargo.lock")).unwrap();
    let sbom: Value = serde_json::from_str(
        &fs::read_to_string(format!("{root}/provenance/sbom.spdx.json")).unwrap(),
    )
    .unwrap();
    let packages = sbom["packages"].as_array().unwrap();
    assert_eq!(lock.matches("[[package]]").count(), packages.len());
    let by_name: BTreeMap<_, _> = packages
        .iter()
        .map(|package| {
            (
                (
                    package["name"].as_str().unwrap(),
                    package["versionInfo"].as_str().unwrap(),
                ),
                package,
            )
        })
        .collect();
    for package in packages
        .iter()
        .filter(|package| package["name"] != "asb-tui")
    {
        let name = package["name"].as_str().unwrap();
        let version = package["versionInfo"].as_str().unwrap();
        let digest = package["checksums"][0]["checksumValue"].as_str().unwrap();
        assert!(lock.contains(&format!("name = \"{name}\"")));
        assert!(lock.contains(&format!("version = \"{version}\"")));
        assert!(lock.contains(&format!("checksum = \"{digest}\"")));
    }
    assert_eq!(
        by_name.len(),
        packages.len(),
        "duplicate SBOM package identity"
    );
    let serialized = serde_json::to_string(&sbom).unwrap();
    for private_marker in ["/srv/", "/home/", "PRIVATE", "token"] {
        assert!(!serialized.contains(private_marker));
    }

    let status = Command::new("python3")
        .args(["tools/generate-rust-sbom.py", "--check"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "generated SPDX inventory is stale");
}
