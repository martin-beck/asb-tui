<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# AR-1676 channel compatibility matrix

`tools/run-channel-compatibility-matrix.py` is the disposable paired-channel
runner. It invokes the supplied release executable through a fresh PTY for
every case and writes a JSON receipt containing the exact checkout `HEAD`:

```sh
cargo build --locked
python3 tools/run-channel-compatibility-matrix.py target/debug/asb-tui \
  --receipt /tmp/ar-1676-channel-receipt.json
```

The matrix covers omitted and explicit `dev`, future/unavailable channels,
cross-process persistence and restoration, failed upgrade rollback preserving
the active pair, removal, and a malformed provenance manifest. It uses a
private install/config root, a missing local repository for the failed upgrade,
and `ASB_TUI_NETWORK_POLICY=deny` with loopback-only proxy settings. The
receipt is therefore development/mock evidence: no provider, credentials, or
network endpoint is used, and it does not qualify a production release.
