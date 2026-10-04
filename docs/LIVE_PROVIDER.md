<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Development live-provider runs

The setup wizard keeps authentication warning-only, but the execution choice
is explicit. On the Recording step choose `live` (or `online`) for a real
provider call, or `mock` for the credential-free deterministic path. The
choice is carried in the digest-bound materialized configuration as
`provider.execution_mode` (`live` or `local-mock`).

The wizard never stores or sends an API key. Select a credential reference or
helper receipt for the provider, and make the corresponding secret available
to the ASB runtime through its configured development credential channel (for
OpenRouter this is normally `OPENROUTER_API_KEY`). Missing authentication,
signatures, and key-management metadata do not prevent saving or reviewing a
development setup. Starting a live run is the explicit boundary at which ASB
checks reachability and reports a typed, actionable error if the key is not
available.

Offline replay remains a separate, explicit cassette action. It never falls
back to a live request.
