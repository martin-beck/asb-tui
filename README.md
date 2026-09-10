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
