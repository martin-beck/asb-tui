<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Contributing

Changes require a focused branch, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --locked`, clean privacy and secret scans, an SSH-signed commit, and a matching
`Signed-off-by` trailer. Dependency updates must bind immutable upstream identities, licenses, and
digests; mutable tags, unsigned inputs, vendored private history, and credentials are rejected.

Do not run code from public pull requests on persistent self-hosted runners. The self-hosted canary
is manual, main-only, checkout-free, and contains no repository-controlled execution.
