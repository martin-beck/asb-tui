<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Development/mock journey qualification

`development-journey-v1.json` is the standalone asb-tui qualification contract
for a disposable, credential-free path from development fixture enrollment to
provider/model/agent selection, mock capture, strict offline replay, and report
comparison. It is syntax and state-contract evidence only; it is not evidence
of production authentication, live-provider reachability, or secure secret
storage.

The three warning cases are deliberately non-gating in development: missing
authentication, signature validation, or key management must be shown to the
user while the mock journey continues. Network access and credentials remain
denied throughout this qualification.

## Executable AR-1327 gate

`tests/development_qualification.rs` is the local fixture harness for the
full journey. It composes the development journey evaluator, onboarding and
router fixtures, the selection-driven wizard, guided benchmark campaign,
strict offline replay, and report comparison in one test. Its successful
output is bounded to these labels:

| Stage | Required evidence |
| --- | --- |
| install/onboarding | `development_journey_ready`, `development_onboarding_ready` |
| router status | `development_only=true`, fixture lifecycle status |
| wizard | supported development catalog selections reach review |
| benchmark/replay | selected agents and measure reach live intent and strict offline intent |
| comparison | compatible reports retain `development/mock/offline-replay` provenance |

The same harness checks version, catalog, bundle, and protocol mismatch
fixtures. Each stops before execution and returns exactly one recovery choice
(`refresh_version`, `refresh_catalog`, `repair_bundle`, or `retry_broker`).
The evaluator accepts only the `development` profile and rejects unknown
fields. No authenticated router, live provider, production authorization,
credential, or network is required or implied.
