<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Cross-repository qualification

The wire adapter is qualified against immutable ASB revisions, not against an
unrelated checkout or a hand-written approximation of a response. The current
qualification pins are:

| Component | Revision | Evidence |
| --- | --- | --- |
| ASB catalog on `main` | `4f855514` | authenticated v1.4 catalog contract |
| ASB lifecycle change | `6d210836` | v1.5 lifecycle contract and fixtures |
| asb-tui adapter | `4106c6de` | this adapter revision |

The canonical ASB fixtures are the authority for the JSON-RPC envelope,
operation wrapper, request digest, field names, numeric generations, target
metadata, and closed lifecycle states. The checked-in adapter fixtures must be
updated whenever either ASB pin changes. A fixture-only change is insufficient
for release qualification: the adapter must parse the canonical ASB response
and encode requests accepted by the same ASB schema.

## Current boundary

This work proves protocol compatibility only. ASB currently returns
`CapabilityUnavailable` for the catalog and lifecycle control calls, and does
not expose the top-level `asb tui` router. Therefore `asb tui install` is not
yet an installable end-to-end workflow. Release qualification remains blocked
until ASB provides the authenticated control endpoint, the standalone client
adopts that endpoint, and a clean-machine test exercises catalog, install,
progress, retry/cancel, status, and removal against the exact revisions above.

The end-to-end test must assert all of the following:

1. negotiation selects v1.4 or v1.5 and rejects unsupported versions;
2. catalog identity and runner generation are retained into the install
   binding;
3. an install request reaches ASB with the catalog digest, numeric generation,
   and idempotency key unchanged;
4. every lifecycle response is decoded into the renderer-neutral state model,
   including progress, terminal failure, retry, cancellation, and stale
   generation;
5. repeated idempotent requests do not create a second operation; and
6. the command exits closed with a useful reason when ASB is unavailable or
   the catalog binding is stale.

No renderer, Ratatui, or terminal implementation belongs in ASB for this
qualification. The executable and its UI remain solely in this repository.
