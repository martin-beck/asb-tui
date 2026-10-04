<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1657 agent/provider/model matrix

Run the deterministic development matrix from an exact TUI checkout:

```sh
python3 tools/run-agent-provider-matrix.py \
  --receipt /tmp/ar-1657-matrix.json --json
```

The runner exercises both supported coding-agent adapters (`opencode` and
`opendesk`) across every development provider/model tuple. It verifies shared
defaults, per-agent overrides, restart restoration, typed unavailable reasons,
offline validation, and both human/JSON guided-command contracts. The matrix
uses generated fixtures only: missing authentication, signatures, and key
management are visible development warnings and never block qualification.

Network access and credentials are denied. The receipt binds the result to the
exact TUI commit and source tree.
