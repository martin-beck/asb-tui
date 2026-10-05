<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1657 agent/provider/model matrix

Run the deterministic development matrix from an exact TUI checkout:

```sh
python3 tools/run-agent-provider-matrix.py \
  --asb-binary /path/to/agent-systems-benchmark/target/debug/asb \
  --tui-binary /path/to/installed/asb-tui \
  --receipt /tmp/ar-1657-matrix.json --json
```

The runner exercises both supported coding-agent adapters (`opencode` and
`opendesk`) across every development provider/model tuple. It verifies shared
defaults, per-agent overrides, restart restoration, typed unavailable reasons,
offline validation, and both human/JSON guided-command contracts. The matrix
uses generated fixtures only: missing authentication, signatures, and key
management are visible development warnings and never block qualification.

With `--asb-binary`, the runner also invokes the parent `asb tui install`
route under both online and offline policy, then checks the installed
`asb tui status` route reports the same install operation without credentials,
then the explicitly supplied installed `asb-tui` executable is invoked
directly. The receipt preserves both structured responses and the direct
installed-executable digest, preventing accidental reuse of the ASB binary for
TUI assertions.

Network access and credentials are denied. The receipt binds the result to the
exact TUI commit and source tree. The runner also queries the paired ASB
`provider-catalog` and fails closed unless every selectable ASB provider/model
choice is represented by the TUI matrix; the local development fixture is
never treated as a substitute for that authoritative catalog.
