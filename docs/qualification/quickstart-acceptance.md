<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1660 selection-driven quickstart acceptance

Run the bounded acceptance against a built TUI executable:

```sh
cargo build --locked
python3 tools/run-quickstart-acceptance.py target/debug/asb-tui \
  --asb-checkout /srv/data/projects/agent-systems-benchmark \
  --receipt /tmp/ar-1660-quickstart-receipt.json --json
```

The runner uses a disposable PTY and private state roots. It composes the
default `dev` channel status/install boundary, development onboarding, the
selection-driven journey fixture, launch-unavailable recovery, OpenCode/OpenDesk
provider/model selection, fan-out/capture/replay coverage, and the existing
end-to-end recording, offline-replay, and comparison qualification test. Its receipt records the
selected provider/auth/agent/model/cassette tuple, exact ASB/TUI commit and
tree identities, executable digest, and release-channel manifest digest.
Network and credentials are denied; this is development/mock evidence, not
live-provider or production-release qualification.
