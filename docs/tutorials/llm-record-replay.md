<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Record and replay an LLM response

From **Runs**, choose a completed run and select **Record**. Review the cassette summary before
saving it: the cassette contains redacted response data, an exact benchmark identity, a run
generation, and a content digest. Credentials, provider tokens, host details, and prompt secrets
are never copied into the cassette. Recording is an explicit action; it does not contact a provider
when this tutorial is followed with the synthetic fixture.

Choose **Replay**, select the cassette, and confirm the same benchmark identity and agent. Strict
replay reads only the cassette and refuses a different benchmark, generation, or digest. It keeps a
failed result and its reason for review rather than silently retrying or deleting it. A failed replay
is not evidence that a live provider was contacted.

The paired JSON contract is a syntax-only example. CI validates routes, option ordering, privacy
fields, provenance, and synthetic outcomes without starting an LLM, provider, benchmark, or network.
