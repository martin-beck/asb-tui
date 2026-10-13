<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Provider-bound comparison qualification

The TUI retains an explicit comparison assessment beside its selected-agent
comparison result. A ready comparison is available only when every selected
side is present, valid, and free of provider/model, provenance, or evidence
confounders.

Human output states the aggregate availability, aggregate unavailable reasons
and confounders, then one baseline/candidate line per selected run. Structured
output contains the same bounded information in `availability`,
`unavailable_reasons`, `confounders`, and `sides`. Each side has a role, opaque
run identifier, availability, unavailable reasons, and confounders.

The projection is deliberately fail-closed: an asymmetric candidate remains
visible as unavailable, and a multi-candidate run with a differently bound
candidate remains visible as available for inspection but not comparable. The
qualification uses local development/mock reports only; it requires no
provider connection, credential, signature service, or key manager.
