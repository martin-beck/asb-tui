<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Check benchmark readiness

This tutorial is an offline, synthetic walkthrough for the TUI readiness screen.
It checks the selected agent, provider capability and local configuration before
benchmark work is requested. A readiness check never starts ASB, an agent, a
provider, an LLM, a benchmark, or a network connection.

## Navigation and actions

Open **Readiness** from the landing screen (`r`), select an agent with the
arrow keys, and press `Enter` to inspect it. `c` refreshes capabilities, `d`
opens the diagnostic details, and `Esc` returns to the previous screen. The
contextual help panel always describes the currently selected action.

The tutorial checks these deterministic responses: `ready` means every local
requirement is present; `incomplete` identifies configuration still required;
`unavailable` means the provider or agent is not installed; and
`platform-incompatible` means the declared platform cannot support it.

Readiness is not execution. The action is refused when configuration is
incomplete, the platform is incompatible, or a credential value is supplied.
Credential checks accept only a redacted reference or digest (never a token or
secret), and the refusal explains how to configure the reference and retry.

The JSON contract contains argument arrays and stable expected response shapes.
The validator checks command ordering, navigation/action coverage, safe
references, refusal explanations, and fixture transitions without invoking any
command.
