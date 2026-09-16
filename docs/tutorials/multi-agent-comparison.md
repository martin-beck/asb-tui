<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# Compare agents from the same benchmark

Open **Reports → Compare**, select two or more completed runs, and verify that the benchmark
identity, measure set, revision, and evaluation generation match. The report keeps each agent and
run identity visible, retains failed measures, and compares only the common measures. Results from
different benchmark revisions or measure sets are marked incomparable rather than ranked.

Use **Report** to inspect the deterministic comparison. An incomparability warning is an intentional
safe refusal: change the selected runs or benchmark before trying again. This tutorial never runs a
benchmark and never contacts an agent or provider.

The paired JSON contract and synthetic reports are checked offline in CI. They validate exact run
selection, conservative comparison, failure retention, and privacy-safe explanations.
