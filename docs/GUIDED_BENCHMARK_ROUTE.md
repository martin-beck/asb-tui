<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Guided benchmark route

`benchmark_route::GuidedCampaign` is the renderer-neutral state seam for the
guided journey. The TUI collects supported workload, agent, and measure
choices, then shows a review step before producing an ASB launch intent.

The route has three explicit execution modes:

- `Live` runs the selected campaign through the existing ASB control seam.
- `Record` requests a campaign with response capture through that seam.
- `OfflineReplay` requires a recording identifier and produces a replay intent
  marked `offline_only`; it cannot be started through the live launch method.
  When ASB supplies a recording catalog, `start_offline_replay_from_catalog`
  additionally requires the exact compatible cassette SHA-256 and selected
  agent/profile identity; unavailable or stale choices fail closed without a
  live fallback.

Before review, the measurement screen can use `selection::BenchmarkSelection`.
It projects a generation-bound nested catalog of pool → group → benchmark →
measure nodes. Search matches any visible node without changing hidden choices;
pool, group, and benchmark toggles are deterministic, and unavailable measures
remain excluded. `campaign_handoff` emits the exact pool, group, benchmark, and
measure identifiers plus catalog digest. `GuidedCampaign::start_with_handoff`
requires the reviewed generation and carries that digest into the launch intent,
so a refreshed catalog is rejected instead of silently changing a run.

When an authoritative runner publishes the legacy measurement catalog, the
frontend replaces the development fallback with that validated snapshot and
clears its nested draft. This is an explicit source transition (the visible
catalog rows and generation change together), never a merge of stale choices;
the user must review and select the new catalog before dispatch.

ASB remains authoritative for execution, recording lifecycle, run history, and
reports. The route performs no network or provider work. A renderer dispatches
the returned intent through the existing control client, projects the terminal
result, and calls `finish`. Comparison is available only after successful
completion and delegates compatibility checks to `reports::compare`, which
retains provenance and measure-set conflicts instead of ranking incomparable
runs.

The Run control screen also exposes bounded selected-agent fan-out. `A` expands
the applied configuration's agent identities across the currently selected
benchmark workloads and submits one versioned admission; `Ctrl-Z` cancels the
exact admitted members returned by ASB. The TUI never reconstructs member
identities from labels, and the projection shows the admitted count and
idempotency identity without credentials. Empty or stale selections are
rejected before transport.

The nested selection and generation-recovery guarantees are covered by
`tests/benchmark_selection.rs` and `tests/benchmark_route.rs`; the existing
reports and development-journey tests cover bounded history, strict replay,
and comparison behavior.
