<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Operator quickstart qualification

AR-1654 begins at the public ASB command boundary. The executable qualification
must run `asb tui install`, followed by bare `asb tui`, before exercising the
selection-driven development fixture. The runner is
`tools/run-operator-quickstart.py` and requires isolated ASB and TUI binaries.

The fixture is credential-free and denies benchmark/replay network access. It
checks human-readable and JSON projections, then delegates to the current
paired journey for wizard setup, benchmark selection and launch, recording,
strict offline replay, comparison, and final report/analysis evidence.

The result is development/mock evidence only; it does not prove provider
authentication, production signing, or public release readiness.

Launch is strict: a headless environment that returns
`development_launch_failed` produces a failed qualification rather than a
passing receipt. Run this gate in a terminal-capable worker when collecting
the acceptance receipt. The runner's isolated PTY declares the fixed
`xterm-256color` capability instead of inheriting `TERM` from its automation
parent. On a first run, the bounded quit sequence cancels the automatically
opened wizard and then quits the landing screen; configured runs quit on the
first key. The JSON projection is accepted only as the complete output suffix,
including when terminal teardown and the JSON object share a line.

The bundle directory must be private (`0700` or stricter) and contain the
validated `manifest.json` and `asb-tui` executable. Invoke the runner with
`--asb-binary`, `--asb-checkout`, `--tui-binary`, `--tui-checkout`, `--bundle`,
and `--receipt`.
