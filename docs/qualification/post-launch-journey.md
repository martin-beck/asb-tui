<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1668 post-launch journey

Run this from an exact ASB checkout and an exact asb-tui checkout after
building both binaries:

```sh
python3 tools/run-post-launch-journey.py \
  ./target/debug/asb-tui \
  --asb-binary /path/to/agent-systems-benchmark/target/debug/asb \
  --asb-checkout /path/to/agent-systems-benchmark \
  --receipt /tmp/ar-1668-receipt.json \
  --json
```

The runner composes the exact-head quickstart and channel matrix. It records
the selection-driven development fixture (`opencode`, `opendesk`, compatible
models, shared defaults), capture/replay/comparison/analysis, restart,
rollback-preserving upgrade, and removal in one receipt. Installation may
materialize the development build; benchmark and replay run with network
denied. Credentials are removed from the child environment. Missing
development authentication, signatures, and key management are therefore
reported as development/mock conditions and never become prerequisites.
