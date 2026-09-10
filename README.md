<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# asb-tui

`asb-tui` is an optional, independently installed terminal frontend for Agent Systems Benchmark.
It does not own benchmark execution, providers, credentials, run state, or artifacts. Removing this
repository or binary must not affect the installed `asb` program.

This initial boundary is deliberately classified as `unverified_extension`. It defines the closed
capability-negotiation response expected from an installed ASB program, but current ASB releases do
not yet publish that external CLI contract. Run:

```console
$ asb-tui doctor --format json
{"classification":"unverified_extension","protocol":"asb-cli-capabilities","protocol_version":1,"reason":"installed_asb_compatibility_not_verified"}
```

The command exits 3. It does not probe ambient configuration, contact a network service, or claim
that Ratatui/Crossterm rendering is available. Those dependencies remain unavailable until their
separately reviewed immutable closure is accepted.

See `provenance/dependencies.lock.json` for exact tooling provenance and
`protocol/v1/capabilities.schema.json` for the proposed external JSON boundary.

## Verification

`cargo test --locked` exercises the closed capability contract, including unknown, missing,
duplicate, malformed, wrong-version, and wrong-type inputs. It also proves the standalone binary
runs with an empty environment and contains no ASB workspace/path dependency.

The dependency lock has a detached SSH signature under the `asb-tui-release-lock` namespace.
`verify-release-lock` verifies that signature, requires signed annotated upstream tags, and binds
their tag objects, commits, and trees. Mutable tags, unsigned tags, altered lock bytes, incomplete
artifact provenance, and mismatched commit/tree identities fail closed.

For an explicit product-side isolation check, run
`tools/prove-asb-isolation.sh /path/to/agent-systems-benchmark`; it confirms `asb-core` has no
standalone TUI dependency and runs its locked tests. This does not claim that current ASB releases
implement the proposed external capability protocol.

See [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md) before reporting or changing
the boundary.

The current public channel is source-only and explicitly unverified. It has no installable GitHub
release, package publication, or supported top-level ASB command. See
[release channels](docs/RELEASE_CHANNELS.md) for the audit result, public lifecycle expectations,
promotion and rollback gates, and [the capability matrix](docs/CAPABILITY_MATRIX.md) for exact
evidence limits.

## Compatibility detection

`asb-tui compatibility --format json` runs a bounded local probe and emits a deterministic report.
It normalizes the compiled operating system and architecture, a size-limited `/etc/os-release`,
time- and output-limited `asb --version` and external capability discovery, exact embedded
coordinator/quality identities, terminal dimensions and features, TTY/SSH/tmux/screen context, and
executable filesystem/runtime prerequisites. Environment values are mapped only to closed enums or
booleans and are never emitted. Hostnames, addresses, credentials, and paths are not report fields.
Commands run with a fixed minimal environment, two-second deadline, 16 KiB output bound, and an
owned process group that is reaped on failure.

Only Linux Ubuntu 24.04 or Debian 12 probes on x86_64 or AArch64, ASB 0.1.0 with positively observed
external protocol v1, the exact
dependency identities in `provenance/dependencies.lock.json`, an interactive Unicode/color terminal
of at least 80 by 24 with resize events, and all storage/verifier requirements select a bundle.
Unsupported probes return fixed reason codes and exit 3; malformed or unknown input returns a generic
diagnostic and exits 2. Selection is compatibility evaluation, not native platform qualification or
permission to install an unverified bundle.

The production probe never treats a one-time terminal-size snapshot as resize evidence. On an
interactive channel it creates an isolated synthetic PTY, changes only that PTY's size, and requires
both the exact new size and the resulting bounded `WINCH` observation from identity-checked
helpers. Any setup, event, timeout, output, or descendant-cleanup failure reports
`resize_events_unavailable`. Deterministic fixture injection exercises the same selection path
without presenting caller-asserted facts as local detection.

The closed offline/test input and public output contracts are
`protocol/v1/compatibility-probe.schema.json` and `protocol/v1/compatibility-report.schema.json`.

## Bundle verification

The closed `protocol/v1/bundle-manifest.schema.json` contract binds a signed release to its source
commit and tree, exact ASB/protocol/coordinator/quality compatibility, immutable commit/tree/build
digests for all three components, and five required artifacts:
the executable, source archive, license report, SPDX SBOM, and provenance statement. Only immutable
versioned GitHub release URLs are accepted. Expired, future-issued, mutable, oversized, incomplete,
or mismatched manifests fail before retrieval.

`verify_bundle_manifest` authenticates the complete manifest under the dedicated
`asb-tui-bundle-v1` SSH namespace before parsing it. `obtain_verified_bundle` uses bounded HTTPS
range requests with three retries per offset, resumes only at verified byte boundaries, enforces
per-artifact and aggregate quotas, checks every SHA-256 digest, and applies license, SPDX, and
reproducible-provenance policy before returning bytes for extraction or execution. The production
cache requires an existing owner-private directory, uses content-addressed owner-only files, and
never overwrites an entry. Cache bytes are size- and digest-checked on every reuse.

The committed signed fixture permits offline verification tests. It is synthetic release metadata,
not a published or installable bundle. Installation accepts only a complete separately staged
artifact directory whose bytes match an authenticated manifest.

## Delegated lifecycle

`asb-tui lifecycle --format json` accepts one bounded JSON request on standard input for the
`install`, `upgrade`, `status`, `launch`, or `remove` operation. The closed request and response
contracts are `protocol/v1/lifecycle-request.schema.json` and
`protocol/v1/lifecycle-response.schema.json`. Responses contain fixed reason codes and verified
release identities, never caller paths, environment values, or artifact contents.

Install and upgrade authenticate the manifest first, verify all five locally staged bundle
artifacts against their signed sizes and SHA-256 digests, enforce license/SBOM/provenance policy,
write only to the requested owner-private extension root, and execute the exact candidate bytes in
an anonymous file for the protocol/terminal self-test before atomic activation. Launch rechecks the
active digest and self-test and executes those exact bytes. Removal deletes only extension state;
the lifecycle has no benchmark-process handle and cannot signal or remove an ASB run.

This is the standalone half of the command contract. Current ASB releases do not yet route
`asb tui install`, `asb tui`, `asb tui status`, `asb tui upgrade`, or `asb tui remove`; that narrow
product-side router must land independently before those top-level commands are claimed available.
