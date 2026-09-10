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

## Compatibility detection

`asb-tui compatibility --format json` reads one closed v1 probe document from standard input and
emits a deterministic report. The probe normalizes the operating system, Linux distribution,
architecture, installed ASB and protocol versions, exact coordinator/quality commits, terminal
dimensions and features, TTY/SSH/tmux/screen context, and filesystem/runtime prerequisites. It never
accepts a hostname, address, credential, arbitrary environment value, or filesystem path.

Only Linux Ubuntu 24.04 or Debian 12 probes on x86_64 or AArch64, external protocol v1, the exact
dependency identities in `provenance/dependencies.lock.json`, an interactive Unicode/color terminal
of at least 80 by 24 with resize events, and all storage/verifier requirements select a bundle.
Unsupported probes return fixed reason codes and exit 3; malformed or unknown input returns a generic
diagnostic and exits 2. Selection is compatibility evaluation, not native platform qualification or
permission to install an unverified bundle.

The closed input and output contracts are `protocol/v1/compatibility-probe.schema.json` and
`protocol/v1/compatibility-report.schema.json`.
