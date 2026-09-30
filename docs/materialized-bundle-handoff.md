<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Materialized bundle handoff

`MaterializedBundle::launch_binding()` is the canonical handoff from the
configuration/preflight route to launch. The launch route must validate the
materialization digest, both catalog generations and digests, every selected
agent/provider/model identifier, the pool/group/benchmark/measure IDs, and the
`development_only` provenance flag before making a control request. A stale,
tampered, or incomplete binding is rejected without I/O.

The bundle is the single ASB-TUI-side configuration document. It contains the
complete typed provider and benchmark selection; downstream launch code
translates the validated binding into ASB control requests rather than asking
users to copy command arguments or configuration fragments.
