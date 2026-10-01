<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Installed frontend handoff

An installed frontend is launched only after lifecycle self-test and digest
verification. The lifecycle process supplies the versioned
`asb-tui-frontend/v1` handoff as environment variables. It binds the ASB
control endpoint (`asb://control/v1`), the `verified` channel, the manifest
identity and source commit/tree, and private state/config/cache roots.

The values are copied from the authenticated installation record, written
atomically alongside the executable. Reopening the installation reads the
same record; an incompatible or tampered record is unverified and cannot
launch. A failed upgrade leaves the previous activated version available for
rollback because activation occurs only after staging, digest, and self-test.
