<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Run one benchmark

Choose **Run** from the landing screen, select one benchmark and one configured agent, and review
the complete confirmation summary before starting. The progress view identifies the synthetic run,
current phase, completed measures, and an actionable error without exposing credentials.

`Esc` cancels a selection or confirmation. Once started, `c` requests cancellation; the view keeps
the run identifier and explains whether cancellation was accepted. If the terminal closes or the
process is interrupted, reopen **Runs**, select the retained run, and choose **Recover**. Recovery
continues only from a checkpoint and never silently starts a second run.

The paired JSON contract is an offline syntax example. CI checks routes, argument ordering,
synthetic identifiers, and response shapes; it never launches ASB, an agent, a provider, an LLM, or
a benchmark.
