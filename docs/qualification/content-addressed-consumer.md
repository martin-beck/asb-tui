<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1686 content-addressed consumer qualification

Run this check from a clean, already-built ASB/TUI pair:

```sh
python3 tools/test-content-addressed-qualification.py
python3 tools/run-content-addressed-qualification.py target/release/asb-tui \
  --asb-binary /path/to/asb/target/release/asb \
  --asb-checkout /path/to/agent-systems-benchmark \
  --receipt /tmp/ar-1686-tui-receipt.json
```

The disposable runner creates plans only through `asb plan create`, validates
their content address, runs two local/mock points, reports them, checks stale
executable rejection, and probes the TUI development journey. It never reads
credentials or contacts a provider. The receipt is deliberately incomplete
until the ASB runner supplies provider-bound capture/replay authority and the
generated plans compare as compatible; an unavailable reason is evidence, not
a zero-valued comparison.

Exit status `2` means the receipt was written but a required downstream stage
is unavailable. This is expected for the current development prototype and
must not be converted into an AR completion claim.
