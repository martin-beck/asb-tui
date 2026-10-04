<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1622 current-main quickstart

This is the disposable, credential-free operator path for the paired ASB/TUI
development journey. It materializes the exact TUI checkout head, opens the
selection-driven wizard, exercises provider/model and coding-agent fixture
selection, records selected and all workloads, seals cassettes, replays them
with network denied, and checks comparison/analysis output.

Build both exact checkouts, then run the wrapper from the TUI checkout:

```sh
cargo build --locked
python3 tools/run-current-main-quickstart.py target/debug/asb-tui \
  --asb-binary /path/to/agent-systems-benchmark/target/debug/asb \
  --asb-checkout /path/to/agent-systems-benchmark \
  --tui-ref main \
  --receipt /tmp/ar-1622-quickstart.json --json
```

The wrapper requires both receipt provenance values to equal the exact local
checkout heads. Installation materialization may use the development channel,
but benchmark, capture, replay, comparison, and analysis are fixture-backed
and network-denied. No real credential is accepted or persisted. Human output
is the default; `--json` emits the complete privacy-safe receipt. A detached
TUI checkout must provide `--tui-ref` for the branch/ref used by the local
materializer.

For interactive operation, see [`OPERATOR_QUICKSTART.md`](../OPERATOR_QUICKSTART.md).
