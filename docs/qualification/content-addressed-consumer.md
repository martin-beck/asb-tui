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

The TUI runner delegates plan creation, validation, local/mock runs, reports,
comparison, capture, and strict replay to ASB's checked-in
`tools/run-content-addressed-qualification.py`. It validates the resulting
paired-head, credential-free receipt and then probes the TUI development
journey. It never builds plan or cassette documents itself, reads credentials,
or contacts a provider. A nonzero ASB qualification result is a hard failure;
the TUI wrapper never converts an unavailable stage into a successful receipt.
