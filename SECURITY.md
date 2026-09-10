<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Security policy

Report vulnerabilities privately through GitHub's security advisory interface. Do not open a public
issue containing credentials, private infrastructure identifiers, unpublished artifacts, or exploit
details. No credential is required by `asb-tui`; benchmark execution and provider credentials remain
owned by the separately installed ASB program.

Only immutable, authenticated dependency identities are accepted. The Ratatui closure and its
crate-specific license/duplicate exceptions are documented in
`docs/TERMINAL_DEPENDENCY_POLICY.md`; widening them requires a new review. The current frontend
remains an `unverified_extension`, and accepting dependencies does not claim rendering exists.
