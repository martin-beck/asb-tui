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

impl AgentInstallRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_binding(&self.binding)?;
        if !valid_identity(&self.idempotency_key) {
            return Err("invalid install idempotency key".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStatusRequest {
    pub binding: AgentLifecycleBinding,
    pub operation_id: Option<String>,
}

impl AgentStatusRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_binding(&self.binding)?;
        if self
            .operation_id
            .as_deref()
            .is_some_and(|id| !valid_identity(id))
        {
            return Err("invalid status operation id".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCancelRequest {
    pub binding: AgentLifecycleBinding,
    pub operation_id: String,
    pub idempotency_key: String,
}

impl AgentCancelRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_mutation(&self.binding, &self.operation_id, &self.idempotency_key)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRetryRequest {
    pub binding: AgentLifecycleBinding,
    pub operation_id: String,
    pub idempotency_key: String,
}

impl AgentRetryRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_mutation(&self.binding, &self.operation_id, &self.idempotency_key)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRemoveRequest {
    pub binding: AgentLifecycleBinding,
    pub idempotency_key: String,
}

impl AgentRemoveRequest {
    pub fn validate(&self) -> Result<(), String> {
        validate_binding(&self.binding)?;
        if !valid_identity(&self.idempotency_key) {
            return Err("invalid remove idempotency key".into());
        }
        Ok(())
    }
}

/// Serialize the exact JSON-RPC v1.5 install call without performing an effect.
pub fn encode_install_request(
    request_id: u64,
    timeout_ms: u64,
    request: &AgentInstallRequest,
) -> Result<String, String> {
    validate_binding(&request.binding)?;
    if !valid_identity(&request.idempotency_key) {
        return Err("invalid install idempotency key".into());
    }
    encode_call(request_id, timeout_ms, "agent_install", request)
}

pub fn encode_status_request(
    request_id: u64,
    timeout_ms: u64,
    request: &AgentStatusRequest,
) -> Result<String, String> {
    validate_binding(&request.binding)?;
    if request
        .operation_id
        .as_deref()
        .is_some_and(|id| !valid_identity(id))
    {
        return Err("invalid status operation id".into());
    }
    encode_call(request_id, timeout_ms, "agent_status", request)
}

pub fn encode_cancel_request(
    request_id: u64,
    timeout_ms: u64,
    request: &AgentCancelRequest,
) -> Result<String, String> {
    validate_mutation(
        &request.binding,
        &request.operation_id,
        &request.idempotency_key,
    )?;
    encode_call(request_id, timeout_ms, "agent_cancel", request)
}

pub fn encode_retry_request(
    request_id: u64,
    timeout_ms: u64,
    request: &AgentRetryRequest,
) -> Result<String, String> {
    validate_mutation(
        &request.binding,
        &request.operation_id,
        &request.idempotency_key,
    )?;
    encode_call(request_id, timeout_ms, "agent_retry", request)
}

pub fn encode_remove_request(
    request_id: u64,
    timeout_ms: u64,
    request: &AgentRemoveRequest,
) -> Result<String, String> {
    validate_binding(&request.binding)?;
    if !valid_identity(&request.idempotency_key) {
        return Err("invalid remove idempotency key".into());
    }
    encode_call(request_id, timeout_ms, "agent_remove", request)
}

fn encode_call<T: Serialize>(
    request_id: u64,
    timeout_ms: u64,
    method: &str,
    params: &T,
) -> Result<String, String> {
    if timeout_ms == 0 || timeout_ms > 300_000 {
        return Err("invalid lifecycle timeout".into());
    }
    serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0", "id": request_id, "timeout_ms": timeout_ms,
        "method": method, "params": params
    }))
    .map_err(|error| format!("cannot encode lifecycle request: {error}"))
}

fn validate_mutation(
    binding: &AgentLifecycleBinding,
    operation_id: &str,
    key: &str,
) -> Result<(), String> {
    validate_binding(binding)?;
    if !valid_identity(operation_id) || !valid_identity(key) {
        return Err("invalid lifecycle mutation identity".into());
    }
    Ok(())
}

fn validate_binding(binding: &AgentLifecycleBinding) -> Result<(), String> {
    if validate_agent_id(&binding.agent_id).is_err()
        || !valid_identity(&binding.runner_instance_id)
        || !valid_digest(&binding.catalog_sha256)
    {
        return Err("invalid lifecycle binding".into());
    }
    Ok(())
}

fn validate_agent_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Err("invalid agent id".into())
    } else {
        Ok(())
    }
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
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

impl AgentLifecycleResponse {
    pub fn validate(&self) -> Result<(), String> {
        validate(self)
    }
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
    validate_agent_id(&value.binding.agent_id)?;
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
