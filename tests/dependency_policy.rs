// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Anti-widening sentinels for the reviewed Ratatui dependency decision.

use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    process::Command,
};

#[derive(Debug)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

fn field(block: &str, name: &str) -> Option<String> {
    let prefix = format!("{name} = \"");
    block.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix('"'))
            .map(str::to_owned)
    })
}

fn lock_packages() -> Vec<LockPackage> {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))
        .unwrap()
        .split("[[package]]\n")
        .skip(1)
        .map(|block| LockPackage {
            name: field(block, "name").unwrap(),
            version: field(block, "version").unwrap(),
            source: field(block, "source"),
            checksum: field(block, "checksum"),
        })
        .collect()
}

#[test]
fn immutable_terminal_graph_has_only_the_two_reviewed_duplicates() {
    let packages = lock_packages();
    let mut versions: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for package in &packages {
        versions
            .entry(&package.name)
            .or_default()
            .insert(&package.version);
        if package.name != "asb-tui" {
            assert_eq!(
                package.source.as_deref(),
                Some("registry+https://github.com/rust-lang/crates.io-index")
            );
            let checksum = package.checksum.as_deref().unwrap();
            assert_eq!(checksum.len(), 64);
            assert!(checksum.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
    let duplicates: BTreeMap<_, _> = versions
        .into_iter()
        .filter(|(_, versions)| versions.len() > 1)
        .collect();
    assert_eq!(
        duplicates,
        BTreeMap::from([
            ("hashbrown", BTreeSet::from(["0.16.1", "0.17.1"])),
            ("syn", BTreeSet::from(["2.0.119", "3.0.5"])),
        ])
    );

    let exact = BTreeMap::from([
        (
            ("crossterm", "0.29.0"),
            "d8b9f2e4c67f833b660cdb0a3523065869fb35570177239812ed4c905aeff87b",
        ),
        (
            ("foldhash", "0.2.0"),
            "77ce24cb58228fbb8aa041425bb1050850ac19177686ea6e0f41a70416f56fdb",
        ),
        (
            ("hashbrown", "0.16.1"),
            "841d1cc9bed7f9236f321df977030373f4a4163ae1a7dbfe1a51a2c1a51d9100",
        ),
        (
            ("hashbrown", "0.17.1"),
            "ed5909b6e89a2db4456e54cd5f673791d7eca6732202bbf2a9cc504fe2f9b84a",
        ),
        (
            ("kasuari", "0.4.12"),
            "bde5057d6143cc94e861d90f591b9303d6716c6b9602309150bd068853c10899",
        ),
        (
            ("ratatui", "0.30.2"),
            "3274ba0a2c5e1bcad2a2005d20f4dc59dad26b2eb0940fb094500dba4099d57d",
        ),
        (
            ("ratatui-core", "0.1.2"),
            "cbb175c433c8e28a809d1f5773a2ae96e68c0ce40db865cbab1020bf33ae479c",
        ),
        (
            ("ratatui-crossterm", "0.1.2"),
            "567584a3b0e6a8203c23de40b4861497266725eb5363dbfd18a1edd603cca9f0",
        ),
        (
            ("ratatui-widgets", "0.3.2"),
            "66e3d19bcc9130ca376277d93b60767ff121ace3be06f5f95f81dd68956407d1",
        ),
        (
            ("syn", "2.0.119"),
            "872831b642d1a07999a962a351ed35b955ea2cfc8f3862091e2a240a84f17297",
        ),
        (
            ("syn", "3.0.5"),
            "12df2e0110f65b775f769bb17ef989067a1d931b2eb822bd4346631eeada89f9",
        ),
    ]);
    for ((name, version), checksum) in exact {
        assert!(packages.iter().any(|package| {
            package.name == name
                && package.version == version
                && package.checksum.as_deref() == Some(checksum)
        }));
    }
}

#[test]
fn manifest_and_deny_policy_cannot_silently_widen() {
    let root = env!("CARGO_MANIFEST_DIR");
    let manifest = fs::read_to_string(format!("{root}/Cargo.toml")).unwrap();
    assert!(manifest.contains(
        "crossterm = { version = \"=0.29.0\", default-features = false, features = [\"bracketed-paste\", \"events\"] }"
    ));
    assert!(manifest.contains(
        "ratatui = { version = \"=0.30.2\", default-features = false, features = [\"crossterm_0_29\"] }"
    ));
    assert!(!manifest.contains("path ="));
    assert!(!manifest.contains("git ="));

    let deny = fs::read_to_string(format!("{root}/deny.toml")).unwrap();
    assert!(deny.contains("{ allow = [\"Zlib\"], crate = \"foldhash@0.2.0\" }"));
    assert!(
        !deny
            .lines()
            .any(|line| { line.starts_with("allow = [") && line.contains("Zlib") })
    );
    assert_eq!(deny.matches("allow = [\"Zlib\"]").count(), 1);
    assert!(deny.contains("crate = \"hashbrown@0.16.1\""));
    assert!(deny.contains("crate = \"syn@3.0.5\""));
    assert!(!deny.contains("skip-tree"));
    assert_eq!(deny.matches("reason = ").count(), 2);
    assert_eq!(deny.matches("exact = true").count(), 8);
}

#[test]
fn enabled_terminal_features_are_exact_and_calendar_cache_are_absent() {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let identities: BTreeMap<_, _> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| {
            (
                package["id"].as_str().unwrap(),
                (
                    package["name"].as_str().unwrap(),
                    package["version"].as_str().unwrap(),
                ),
            )
        })
        .collect();
    let observed: BTreeMap<_, _> = metadata["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| {
            let identity = identities[node["id"].as_str().unwrap()];
            matches!(
                identity.0,
                "crossterm"
                    | "foldhash"
                    | "hashbrown"
                    | "ratatui"
                    | "ratatui-core"
                    | "ratatui-crossterm"
                    | "ratatui-widgets"
            )
            .then(|| {
                (
                    identity,
                    node["features"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|feature| feature.as_str().unwrap())
                        .collect::<BTreeSet<_>>(),
                )
            })
        })
        .collect();
    let expected = BTreeMap::from([
        (
            ("crossterm", "0.29.0"),
            BTreeSet::from([
                "bracketed-paste",
                "default",
                "derive-more",
                "events",
                "windows",
            ]),
        ),
        (("foldhash", "0.2.0"), BTreeSet::new()),
        (
            ("hashbrown", "0.16.1"),
            BTreeSet::from([
                "allocator-api2",
                "default",
                "default-hasher",
                "equivalent",
                "inline-more",
                "raw-entry",
            ]),
        ),
        (
            ("hashbrown", "0.17.1"),
            BTreeSet::from([
                "allocator-api2",
                "default",
                "default-hasher",
                "equivalent",
                "inline-more",
                "raw-entry",
            ]),
        ),
        (
            ("ratatui", "0.30.2"),
            BTreeSet::from(["crossterm", "crossterm_0_29", "std"]),
        ),
        (
            ("ratatui-core", "0.1.2"),
            BTreeSet::from(["default", "std", "underline-color"]),
        ),
        (
            ("ratatui-crossterm", "0.1.2"),
            BTreeSet::from(["crossterm_0_29", "default", "underline-color"]),
        ),
        (("ratatui-widgets", "0.3.2"), BTreeSet::from(["std"])),
    ]);
    assert_eq!(observed, expected);
    let all_features = format!("{observed:?}");
    assert!(!all_features.contains("calendar"));
    assert!(!all_features.contains("layout-cache"));
    assert!(!all_features.contains("macros"));
}

#[test]
fn spdx_limits_zlib_to_exact_foldhash_release() {
    let sbom: Value = serde_json::from_str(
        &fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/provenance/sbom.spdx.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let zlib: Vec<_> = sbom["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| package["licenseDeclared"].as_str() == Some("Zlib"))
        .collect();
    assert_eq!(zlib.len(), 1);
    assert_eq!(zlib[0]["name"], "foldhash");
    assert_eq!(zlib[0]["versionInfo"], "0.2.0");
    assert_eq!(
        zlib[0]["checksums"][0]["checksumValue"],
        "77ce24cb58228fbb8aa041425bb1050850ac19177686ea6e0f41a70416f56fdb"
    );
}
