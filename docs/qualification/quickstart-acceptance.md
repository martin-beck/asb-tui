<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1693 paired default-dev quickstart qualification

Run the bounded acceptance against a built TUI executable:

```sh
cargo build --locked
python3 tools/run-quickstart-acceptance.py target/debug/asb-tui \
  --asb-checkout /srv/data/projects/agent-systems-benchmark \
  --receipt /tmp/ar-1693-quickstart-receipt.json --json
```

The runner uses a disposable PTY and private state roots. It resolves the
omitted channel to `dev`, materializes the exact TUI `main` source, consumes
the paired ASB handoff, and checks status, launch, restart persistence, and
typed manifest tamper diagnostics. It then composes provider/model selection,
selected/all fan-out capture, strict offline replay, comparison, and analysis.
The receipt records exact paired source identities, handoff and installed
executable digests, plus a separate digest for the runner binary itself.
Clone/build materialization may use the network; benchmark and replay run with
network denied and no credentials. This is development/mock evidence, not
live-provider or production-release qualification.
