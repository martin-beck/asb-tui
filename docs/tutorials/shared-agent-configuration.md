<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Apply shared configuration to agents

Open **Configuration → Agents**, select the agents to change, and review the shared draft. The
preview lists the provider, model, benchmark defaults, and the origin of each value (explicit
selection, saved configuration, or default). An explicit value wins over a saved value; defaults
are shown so they are never mistaken for a user choice.

Confirming applies the draft atomically to every selected agent. If one agent is incompatible, the
whole operation is refused with the affected agent and reason; compatible agents remain unchanged.
Remove an agent from the selection and review again before retrying. `Esc` discards the draft and
`?` opens contextual help for the current field.

The paired JSON contract and synthetic fixtures are checked offline in CI. They verify selection,
configuration origins, confirmation, atomic success, and atomic incompatibility without contacting
ASB or a provider and without reading credentials.
