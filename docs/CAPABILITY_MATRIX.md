<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Capability matrix

| Capability | Public source contract | Verified release |
| --- | --- | --- |
| Independent MIT-licensed source | Available | Required |
| Closed capability/protocol schemas | Available and tested | Required at exact released versions |
| Signed dependency closure and SPDX SBOM | Available for source audit | Required in release bundle |
| Bundle authentication and bounded acquisition | Implemented and tested with synthetic fixtures | Required against published artifacts |
| Transactional lifecycle and recovery | Implemented and tested locally | Development router fixture is available; live ASB router remains future qualification |
| Top-level `asb tui` routing | Development router and onboarding commands available | Live ASB routing remains future qualification |
| Ratatui/Crossterm interactive UI | Implemented and source-tested | Required to be platform-qualified |
| Linux x86_64 qualification | Source CI only | Required on exact release |
| Linux AArch64 qualification | Contract/emulation evidence only | Required on exact release |
| Installation, upgrade, rollback, removal | Development lifecycle and onboarding fixtures available | Live end-to-end qualification remains future work |

Green source CI proves the checked-in contracts and tests at its exact revision. It does not prove
that ASB exposes the proposed protocol, that source-tested rendering is platform-qualified, that
synthetic bundle fixtures are releases, or that an installable artifact is qualified. Unknown
platforms and capabilities remain unsupported; absence of evidence never upgrades them to verified.
