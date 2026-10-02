<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Operator quickstart

`asb-tui` is the selection-driven frontend for the ASB operator journey. It
does not own credentials, benchmark execution, or durable run state; the ASB
runner remains authoritative. The development route below is intentionally
credential-free and uses local fixtures.

## 1. Install and inspect

Install the exact TUI bundle alongside the matching ASB release, then inspect
the negotiated boundary:

```sh
asb-tui doctor
asb-tui tui status --development
```

Diagnostics are human-readable by default. Add the explicit `--json` flag for
automation (`asb-tui doctor --json`, `asb-tui tui status --development --json`);
`--format json` remains a compatibility alias. A development bundle may show
`development-only` warnings for missing authentication, signatures, or key
management. These warnings are visible but nonblocking for local fixtures;
they do not qualify a production release or provider authorization.

## 2. Walk the wizard by selection

Open the wizard from the landing route. Each step is a catalog selection, not
free-form provider configuration:

1. Search and select the benchmark agent or use **select all**.
2. Select one compatible provider from the advertised list.
3. Select a model exposed by that provider.
4. Review configuration and benchmark pool/measures.
5. Select a credential reference (the UI shows digest metadata only).
6. Choose `live recording` or an exact `offline replay` cassette.
7. Review every choice, then confirm the plan.

The wizard must keep unavailable agents, providers, models, and near-match
cassettes visible with an explanation. It must not silently substitute a
provider, model, credential, or live run. Selecting several agents is atomic:
if one is incompatible, no partial configuration is applied.

## 3. Launch and follow the benchmark

After the runner acknowledges the confirmed plan, launch once and follow the
identity-matched status/events projection through `Planned`, `Prepared`,
`Running`, `Collecting`, and a terminal state. Use the runner's report and
compare operations after completion. A local-mock result is labeled
development-only; it is not provider, model-quality, or native-performance
evidence.

## 4. Record or replay

For live recording, the review screen must show network, cost, and persistence
consequences and require explicit acknowledgement. For replay, choose an
exact cassette digest from the runner catalog. Strict replay denies provider
network access and has no live fallback. The UI never displays prompts,
responses, credential values, or raw captures as ordinary fields.

## 5. Compare

Report each terminal run before comparison. Compare only when the runner says
`comparable: true`; retain the typed unavailable reason for mismatched or
incomplete evidence. A replay comparison is useful for workflow determinism,
not a claim of fresh provider quality.

The matching ASB command route is documented in the [ASB operator
quickstart](https://github.com/martin-beck/agent-systems-benchmark/blob/main/docs/OPERATOR_QUICKSTART.md).
