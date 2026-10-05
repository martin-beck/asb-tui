<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Development host preflight

`development_preflight::run()` is the renderer-neutral host gate for the
development install/launch path. It checks only bounded, non-secret facts:
`git`, `cargo`, `setsid`, `cc`, and the private install-root path. Each check
has a stable identifier and an actionable retry/remediation string. The
`Report` serializes directly to JSON and `human_lines()` provides the same
typed result for a terminal screen.

`development_host_ready` permits the materializer to continue. A failed check
returns `development_host_preflight_failed`; the caller must preserve the
previous installation and offer retry after the remediation. Authentication,
signatures, and key-management warnings are deliberately not checks here and
remain development-only non-blocking warnings.

The standalone binary exposes the same report as `asb-tui preflight --json`
or human-readable `asb-tui preflight`; install and upgrade run the same gate
before materialization.

This preflight does not convert an unsuccessful broker launch into success:
`development_launched` remains the only successful launch outcome.
