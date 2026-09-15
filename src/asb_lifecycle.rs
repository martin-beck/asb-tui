// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Renderer-neutral adapter for ASB control protocol v1.5 lifecycle messages.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleState {
    Pending,
    Staging,
    Active,
    Cancelled,
    Failed,
    Removed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleFailure {
    StaleCatalog,
    UnauthenticatedCatalog,
    VerificationFailed,
    IncompatibleTarget,
    SelfTestFailed,
    Busy,
    NeedsReconciliation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLifecycleBinding {
    pub agent_id: String,
    pub runner_instance_id: String,
    pub catalog_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInstallRequest {
    pub binding: AgentLifecycleBinding,
    pub catalog_generation: u64,
    pub idempotency_key: String,
}

/// Serialize the exact JSON-RPC v1.5 install call without performing an effect.
pub fn encode_install_request(
    request_id: u64,
    timeout_ms: u64,
    request: &AgentInstallRequest,
) -> Result<String, String> {
    if timeout_ms == 0 || request.binding.agent_id.is_empty() || request.idempotency_key.is_empty()
    {
        return Err("invalid install request".into());
    }
    serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0", "id": request_id, "timeout_ms": timeout_ms,
        "method": "agent_install", "params": request
    }))
    .map_err(|error| format!("cannot encode install request: {error}"))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLifecycleResponse {
    pub binding: AgentLifecycleBinding,
    pub operation_id: String,
    pub state: AgentLifecycleState,
    pub generation: u64,
    pub progress_percent: u8,
    pub failure: Option<AgentLifecycleFailure>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcResponse {
    jsonrpc: String,
    id: u64,
    result: RpcOperation,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcOperation {
    kind: String,
    value: RpcValue,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcValue {
    request_sha256: String,
    result: RpcResult,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcResult {
    kind: String,
    value: AgentLifecycleResponse,
}

/// Decode and validate ASB's complete v1.5 lifecycle response.
pub fn parse_lifecycle_response(input: &str) -> Result<AgentLifecycleResponse, String> {
    let response: RpcResponse = serde_json::from_str(input)
        .map_err(|error| format!("invalid lifecycle response: {error}"))?;
    let _ = response.id;
    validate_digest(&response.result.value.request_sha256)?;
    if response.jsonrpc != "2.0"
        || response.result.kind != "operation"
        || response.result.value.result.kind != "agent_lifecycle"
    {
        return Err("unsupported lifecycle response envelope".into());
    }
    let value = response.result.value.result.value;
    validate(&value)?;
    Ok(value)
}

fn validate(value: &AgentLifecycleResponse) -> Result<(), String> {
    validate_identity(&value.binding.agent_id)?;
    validate_identity(&value.binding.runner_instance_id)?;
    validate_digest(&value.binding.catalog_sha256)?;
    validate_identity(&value.operation_id)?;
    if value.progress_percent > 100 {
        return Err("invalid lifecycle progress".into());
    }
    if value.state == AgentLifecycleState::Failed {
        if value.failure.is_none() {
            return Err("failed lifecycle response has no reason".into());
        }
    } else if value.failure.is_some() {
        return Err("non-failed lifecycle response has a failure".into());
    }
    if matches!(
        value.state,
        AgentLifecycleState::Active | AgentLifecycleState::Cancelled | AgentLifecycleState::Removed
    ) && value.progress_percent != 100
    {
        return Err("terminal lifecycle response is incomplete".into());
    }
    Ok(())
}

fn validate_identity(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
    {
        Err("invalid lifecycle identity".into())
    } else {
        Ok(())
    }
}
fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Err("invalid lifecycle digest".into())
    } else {
        Ok(())
    }
}
