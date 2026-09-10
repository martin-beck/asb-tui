<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Ratatui terminal dependency decision

## Decision and boundary

The exact Ratatui 0.30.2 and Crossterm 0.29.0 crates.io closure is accepted for source use by the
standalone `asb-tui` application, subject to the fail-closed constraints below. This is a reviewed,
crate-specific policy decision; it is not a global Zlib allowance, a general permission for
duplicate versions, or evidence that a renderer, platform qualification, or installable release
exists.

Ratatui uses defaults disabled and only its released Crossterm 0.29 backend. The application pins
Crossterm separately with only `bracketed-paste` and `events` requested. The released backend
currently activates Crossterm's default feature as well; the exact resulting feature set is locked
and tested rather than hidden.

## Immutable identities

All packages come from the crates.io registry configured in `deny.toml`; Git, path, vendor, and
unknown registry sources remain denied. Cargo.lock binds every package. The decision-critical
identities are:

| Package | Version | crates.io SHA-256 |
| --- | --- | --- |
| ratatui | 0.30.2 | `3274ba0a2c5e1bcad2a2005d20f4dc59dad26b2eb0940fb094500dba4099d57d` |
| ratatui-core | 0.1.2 | `cbb175c433c8e28a809d1f5773a2ae96e68c0ce40db865cbab1020bf33ae479c` |
| ratatui-crossterm | 0.1.2 | `567584a3b0e6a8203c23de40b4861497266725eb5363dbfd18a1edd603cca9f0` |
| ratatui-widgets | 0.3.2 | `66e3d19bcc9130ca376277d93b60767ff121ace3be06f5f95f81dd68956407d1` |
| crossterm | 0.29.0 | `d8b9f2e4c67f833b660cdb0a3523065869fb35570177239812ed4c905aeff87b` |
| kasuari | 0.4.12 | `bde5057d6143cc94e861d90f591b9303d6716c6b9602309150bd068853c10899` |
| hashbrown | 0.16.1 | `841d1cc9bed7f9236f321df977030373f4a4163ae1a7dbfe1a51a2c1a51d9100` |
| hashbrown | 0.17.1 | `ed5909b6e89a2db4456e54cd5f673791d7eca6732202bbf2a9cc504fe2f9b84a` |
| foldhash | 0.2.0 | `77ce24cb58228fbb8aa041425bb1050850ac19177686ea6e0f41a70416f56fdb` |
| syn | 2.0.119 | `872831b642d1a07999a962a351ed35b955ea2cfc8f3862091e2a240a84f17297` |
| syn | 3.0.5 | `12df2e0110f65b775f769bb17ef989067a1d931b2eb822bd4346631eeada89f9` |

The complete 93-package lock inventory, including target-specific and disabled optional packages,
is reproduced in `provenance/sbom.spdx.json`. `tools/generate-rust-sbom.py --check` fails when that
inventory differs from Cargo.lock or offline Cargo metadata. The generator has 4 MiB lockfile,
16 MiB metadata, 512-package, and 60-second bounds and rejects every non-registry package.

## Exact feature boundary

The enabled terminal feature sets are asserted both by `deny.toml` and an executable metadata test:

```text
ratatui 0.30.2: crossterm,crossterm_0_29,std
ratatui-core 0.1.2: default,std,underline-color
ratatui-crossterm 0.1.2: crossterm_0_29,default,underline-color
ratatui-widgets 0.3.2: std
crossterm 0.29.0: bracketed-paste,default,derive-more,events,windows
hashbrown 0.16.1 and 0.17.1: allocator-api2,default,default-hasher,equivalent,inline-more,raw-entry
foldhash 0.2.0: no named feature
```

Ratatui calendar, layout-cache, macro, palette, serde, and extra widget features are not enabled.
Any feature expansion fails `cargo deny` and `tests/dependency_policy.rs`.

## Zlib compatibility and foldhash threat review

`foldhash 0.2.0` is licensed under Zlib. Its exact license file has SHA-256
`b1181a40b2a7b25cf66fd01481713bc1005df082c53ef73e851e55071b102744`. The license permits use,
modification, and redistribution, including commercial use; requires altered sources to be marked;
prohibits misrepresenting origin; and requires its notice to remain in source distributions. Those
conditions are compatible with this MIT application. The SPDX inventory preserves the exact Zlib
identity and release packaging must retain the upstream notice.

The exception is expressed as `foldhash@0.2.0` under `licenses.exceptions`. Zlib is deliberately not
added to the global allow list, so another Zlib package or another foldhash version fails closed.

Foldhash is explicitly non-cryptographic and only minimally resistant to an adaptive collision
attack. In this closure it is selected by both hashbrown versions. Kasuari uses those maps for
solver-created symbols, variables, rows, edits, and renderer-authored constraints; Ratatui Core uses
them for internal layout variables. The accepted application boundary does not place credentials,
artifact bytes, arbitrary benchmark strings, persistent identities, or protocol keys in these maps.
Terminal input is reduced to a closed action set, and the renderer must keep layout definitions and
visible collections bounded before invoking the solver.

Residual risk is denial-of-service in a long-running process if a future feature gives an adaptive
attacker control over hash keys or unbounded layout construction. The exception becomes invalid if
the application stores untrusted strings in these maps, enables caller-authored arbitrary layout
constraints, removes collection bounds, relies on stable hash output, or uses foldhash for any
security property. Such a change requires a new threat review before merge.

## Exact duplicate exceptions

Only two duplicate package names are permitted:

- `hashbrown 0.16.1` is the exact skipped entry because Kasuari 0.4.12 requires the 0.16 line while
  Ratatui Core, Ratatui Widgets, and LRU require 0.17.1. It is runtime code, but both versions are
  advisory-clean and select the same exact foldhash implementation. Remove the skip when immutable
  Kasuari and Ratatui releases converge on one hashbrown line or expose a standard-collection
  backend.
- `syn 3.0.5` is the exact skipped entry because Ratatui's maintained instability/thiserror macro
  closure requires syn 3 while Crossterm derive-more, Strum, and current application derives require
  syn 2.0.119. Syn is build-time procedural-macro code and is absent from the runtime binary. Remove
  the skip when maintained immutable releases converge on one syn major.

No subtree skip is used. A patch release, third version, or additional duplicated package does not
match either exception and fails the build.

## Rejected alternatives and upstream removal path

Ratatui 0.24 through 0.29 retain the previously identified vulnerable or unmaintained `lru`/`paste`
closures. Ratatui 0.30.2 with its 0.28 backend activates both Crossterm 0.28 and 0.29 and adds
rustix, linux-raw-sys, and windows-sys duplicates. Git or path patches would violate source policy;
vendoring Kasuari or Ratatui would create a private maintenance fork.

The preferred removal path remains maintained upstream releases: Kasuari should expose a standard
collection backend or converge on Ratatui's hashbrown line, and Ratatui should make its backend
dependency defaults selectable. As of 2026-09-10, the latest releases are
[Ratatui 0.30.2](https://github.com/ratatui/ratatui/releases/tag/ratatui-v0.30.2) and
[Kasuari 0.4.12](https://github.com/ratatui/kasuari/releases/tag/v0.4.12); neither exposes that
boundary.

## Verification and claims

The locked graph must pass Cargo build/test, `cargo deny --locked check`, and
`cargo audit --deny warnings` on Rust 1.93.0. Dependency/build checks run for
`x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`; cross-compilation is not native AArch64
execution evidence. Repository classification remains `source_only_unverified` until the separate
renderer, integration, platform, release, and install gates pass.

The separately signed release lock therefore continues to report rendering as unavailable. This
dependency decision does not re-sign that lock or promote an install channel; those changes belong
to the later renderer and release qualification work.
