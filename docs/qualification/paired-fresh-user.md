<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Paired fresh-user qualification

AR-1689 composes two versioned qualifications at the ASB/TUI boundary. The
TUI-owned route materializes and verifies the development-channel executable,
walks the bounded wizard selection route, and verifies selection, recording,
offline replay, comparison, and analysis projections. The ASB-owned route
creates a fresh content-addressed plan, performs local/mock execution,
records selected and all workloads, performs strict offline replay, and
retains typed stale-identity and incomplete-cassette negatives.

The runner records the supplied exact ASB and asb-tui commits, source trees,
binary digests, and subordinate receipt digests. It accepts no credential and
permits network only while the disposable development executable is
materialized; plan, capture, replay, comparison, and analysis remain denied
network access.

```sh
python3 tools/run-paired-fresh-user-qualification.py target/debug/asb-tui \
  --asb-binary /path/to/asb/target/debug/asb \
  --asb-checkout /path/to/agent-systems-benchmark \
  --tui-ref main --receipt /tmp/ar-1689-receipt.json --json
```

The command deliberately consumes the existing ASB and TUI qualification
entrypoints. It does not recreate plan or cassette formats in asb-tui.
