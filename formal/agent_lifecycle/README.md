<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Agent lifecycle model

`AgentLifecycle.tla` is the state/event contract for the renderer-neutral
`agent_lifecycle` Rust module. It describes the allowed lifecycle states and
the safety invariant that progress is bounded and monotonic. The Rust model
adds the catalog and runner-generation guards required by the wire contract;
events that fail those guards return an error without changing state.

Rendering, filesystem effects, process execution, and credential handling are
outside this model. The authenticated ASB control client is the sole effect
executor and must emit only events for the accepted runner generation.
