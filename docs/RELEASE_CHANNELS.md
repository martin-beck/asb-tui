<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Release channels

## Current channel: source-only, optional, unverified

The public repository is available for inspection and reproducible verification. It is **not** an
installable release channel: there is no supported binary release, package publication, or
top-level `asb tui` command. Do not install a binary copied from CI, a pull request, an unversioned
URL, or a synthetic test fixture.

Developers may clone an immutable revision and run `cargo test --locked`. This verifies the source
contract and renderer tests; it does not promote the revision or qualify a user workflow. The
lifecycle JSON API
authenticates separately staged bundles, but a source-only build embeds no promoted bundle identity.
Even an authentic test fixture therefore fails with `release_channel_unverified` before creating an
installation root. Until promotion, public install, upgrade, launch, status, and remove workflows
are unavailable and must fail closed.

## Verified-channel promotion

A release manager may create a signed annotated version tag and GitHub release only after one exact
commit and tree satisfy every item below:

- ASB publishes and tests the matching external protocol and top-level lifecycle router.
- Ratatui/Crossterm rendering and the supported terminal UX are independently qualified.
- x86_64 and AArch64 entries in the capability matrix have exact platform evidence.
- all source, license, SBOM, security, protocol, UX, and privacy audits pass;
- hosted CI and the protected trusted-main workflow pass for the exact public main revision;
- the five bundle artifacts required by `protocol/v1/bundle-manifest.schema.json` are reproducibly
  built, digest-bound, and complete;
- the bundle manifest and provenance statement bind the tag object, commit, tree, workflow identity,
  coordinator identity, quality-workflow identity, artifact sizes, and SHA-256 digests;
- the manifest is signed under the `asb-tui-bundle-v1` namespace by an allowed release signer;
- an independent verifier validates the downloaded artifacts before publication; and
- installation, upgrade, rollback, removal, interrupted recovery, and active-run isolation pass.

The release build **must** set both `ASB_TUI_SOURCE_COMMIT` and `ASB_TUI_SOURCE_TREE` to the
lowercase 40-hex commit and tree selected for the signed tag. A candidate compiled without either
value, or with values that do not match the signed external manifest, is not releasable. Final
qualification must exercise the real release-compiled executable through the delegated lifecycle:
the parent digest-checks those exact bytes against the signed manifest, executes them from the
anonymous file, and requires the candidate self-test to return the injected source identity,
release, target, and protocol. A unit-test helper or source-only binary is not release evidence.

The release page must keep the source archive, executable, license report, SPDX SBOM, provenance
statement, and signed manifest together at immutable versioned URLs. CI artifacts are temporary
evidence and are never the release channel.

## Rollback and cleanup

Promotion never overwrites a prior version. Installation is content-addressed and activation is an
atomic pointer change. On failed verification or self-test, the previous active version remains in
place and untrusted staging bytes are removed. On a post-promotion defect, stop new downloads,
publish a security advisory when appropriate, mark the affected version unsupported, and atomically
reactivate the last independently verified version. Never move or replace an existing version tag.

Removal deletes only the extension-owned root after taking its lifecycle lock; it must not remove
ASB data or signal an ASB run. Retained inactive versions and bounded download fragments may be
removed only while holding that lock. Release evidence remains on the public release page even when
local artifacts are cleaned up.

`release/channel-status.json` is the machine-readable audit state. The repository quality gate
validates it and rejects a verified classification while any required promotion condition is false.
It deliberately contains no source commit/tree, manifest digest or executable digest: embedding
those values in the tracked source or final executable would create an impossible cryptographic
self-reference. Dynamic release identities live in the separately signed bundle manifest. The
release builder injects the source commit/tree as mandatory immutable build metadata through
`ASB_TUI_SOURCE_COMMIT` and `ASB_TUI_SOURCE_TREE`, and the parent verifies the final executable
digest before running the candidate from an anonymous file. This separates the bootstrap trust
anchor (embedded static signer and promotion policy) from the candidate identity (external signed
manifest plus injected source identity) without requiring a candidate to assert its own digest.

## Stable delegated JSON contract

The unreleased lifecycle v1 request was corrected before an ASB router consumed it. Install and
upgrade requests provide staged manifest, signature and artifact paths plus the expected target,
ASB version, protocol version, release, source commit/tree and executable digest. They cannot
provide a trust anchor or verification timestamp. The superseded shape containing
`allowed_signers` or `now_unix`, unknown fields, unsupported schema versions, and malformed
identities returns one closed JSON response and exits 3 without filesystem mutation. Invalid usage
exits 2. Successful lifecycle operations exit 0; policy and compatibility failures exit 3.
Signer rotation requires a reviewed `trust_policy_version` change and a new release binary; it
cannot be delegated through lifecycle JSON.
