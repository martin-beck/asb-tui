<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# First run and first agent

This tutorial describes the first-run journey for the standalone `asb-tui` frontend. It is paired
with [`first-run-agent-v1.json`](first-run-agent-v1.json), which is an offline contract document
validated against the published ASB tutorial contract (v1). The JSON steps are examples only:
the tutorial validator never executes `asb`, contacts a provider, reads credentials, or starts a
benchmark.

## Journey

1. Open the TUI doctor view to see whether the installed ASB endpoint and the standalone release
   are compatible. An unavailable or unverified endpoint is shown as a reason to configure or
   repair the installation; it is never treated as ready.
2. Open the provider catalog and choose a provider/agent entry. The catalog entry is a stable,
   authenticated identity; it is not a credential or a prompt. Unsupported entries stay visible
   with an actionable reason.
3. Open configuration and enroll the provider using the provider's external credential flow.
   Store only a reference/digest through the authenticated ASB boundary. Never paste or display
   a secret in the TUI, tutorial, fixture, or logs.
4. Review the bounded install preview. Confirming it delegates the verified lifecycle operation;
   cancel leaves the draft and returns safely to the landing view. A dry run never changes the
   active installation.
5. Return to the status/readiness view. It distinguishes configured, incomplete, unavailable,
   and stale states and exposes Configure/Reconfigure without automatically opening the wizard
   for an already configured user.

The command arrays in the machine-readable contract intentionally use synthetic identities and
fixed digests. They declare `network: "denied"` and `credentials: "none"` on every step. The
contract validates syntax, option order, bounded references, and expected output shapes; it does
not claim that any command or provider actually ran.

## Contract and local validation

The grammar and safety rules are defined by the versioned ASB tutorial contract and command
metadata. Run the standalone deterministic check with:

```console
$ python3 tools/test-first-run-tutorial.py
```

This check is intentionally narrower than end-to-end qualification. It proves the document is a
closed, offline, schema-shaped input and that hostile fields, shell syntax, secret-like values,
unknown commands, and reordered options are rejected. End-to-end ASB capability and lifecycle
execution remains a separate cross-repository qualification.
