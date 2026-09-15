<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Contributing

Changes require a focused branch, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --locked`, clean privacy and secret scans, an SSH-signed commit, and a matching
`Signed-off-by` trailer. Dependency updates must bind immutable upstream identities, licenses, and
digests; mutable tags, unsigned inputs, vendored private history, and credentials are rejected.

## Formal UI model

Every route, window, focusable or hoverable element, key binding, help entry, and state transition
is owned by `docs/ui-state-model.json`. After changing the UI surface, update that authored model
and its focused tests, then run:

```sh
python3 tools/generate-ui-state-model.py
python3 tools/test-ui-state-model.py
python3 tools/validate-ui-state-model.py
```

The generated JSON is checked byte-for-byte in pull-request and trusted-main CI. The gate fails
closed for stale artifacts, duplicate or unreachable IDs, missing meaningful help prose, invalid
focus/hover metadata, broken parent links, and routes without an escape path. On a pull request,
the validator also compares the exact base and head revisions and reports UI files changed without
the corresponding model and focused-test update. Model fixtures are deterministic and offline;
never add provider output, credentials, private paths, or captured prompts.

Do not run code from public pull requests on persistent self-hosted runners. The self-hosted canary
is manual, main-only, checkout-free, and contains no repository-controlled execution.
