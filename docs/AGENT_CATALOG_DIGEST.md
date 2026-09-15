<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Agent-catalog content digest (v1.4)

`AgentCatalog.catalog_sha256` is the content identity consumed by this
frontend. It is an integrity check, not a signature or an authorization
decision; package signatures and the authenticated control channel remain
separate requirements.

The adapter first validates the complete typed catalog and then serializes all
catalog members except `catalog_sha256`. Every JSON object is recursively
ordered by UTF-8 key bytes. Arrays retain protocol order (agent IDs and
capabilities must already be strictly increasing). Compact UTF-8 JSON with the
pinned `serde_json` escaping and typed integer representation is hashed with
SHA-256 and encoded as lowercase hexadecimal. A malformed value is never
canonicalized as a repair; the advertised digest must equal the recomputed
digest before projection or selection.

For the checked-in one-entry vector (generation `3`, runner `runner-1`, Linux
`x86_64`/glibc `2.35`, package `agent-package`, capabilities `chat` and
`tools`), the canonical digest is:

```text
e24007b09d9f6b112269689a535ba00968300c9653d71e349f37835d0d8c5e92
```

The parser rejects digest mismatches, unknown fields, malformed identities,
target drift, duplicate or unsorted agent IDs/capabilities, and malformed
digests. The implementation and negative/ordering tests are in
`src/agent_catalog.rs` and `tests/agent_catalog.rs`.
