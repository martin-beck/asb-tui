# Guided benchmark route

`benchmark_route::GuidedCampaign` is the renderer-neutral state seam for the
guided journey. The TUI collects supported workload, agent, and measure
choices, then shows a review step before producing an ASB launch intent.

The route has three explicit execution modes:

- `Live` runs the selected campaign through the existing ASB control seam.
- `Record` requests a campaign with response capture through that seam.
- `OfflineReplay` requires a recording identifier and produces a replay intent
  marked `offline_only`; it cannot be started through the live launch method.

ASB remains authoritative for execution, recording lifecycle, run history, and
reports. The route performs no network or provider work. A renderer dispatches
the returned intent through the existing control client, projects the terminal
result, and calls `finish`. Comparison is available only after successful
completion and delegates compatibility checks to `reports::compare`, which
retains provenance and measure-set conflicts instead of ranking incomparable
runs.

The selection and replay guarantees are covered by
`tests/benchmark_route.rs`; the existing reports and development-journey tests
cover bounded history, strict replay, and comparison behavior.
