// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Renderer-neutral adapter for ASB control protocol v1.4 agent catalogs.

use serde::{Deserialize, Serialize};

const MAX_CATALOG_AGENTS: usize = 32;
const MAX_CAPABILITIES: usize = 32;
const MAX_STRING_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCatalogAction {
    Status,
    Refresh,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalogRequest {
    pub action: AgentCatalogAction,
    pub runner_instance_id: String,
    pub known_generation: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentUnavailableReason {
    IncompatibleTarget,
    IncompleteProvenance,
    UnverifiedArtifact,
    MissingCapability,
    StaleCatalog,
    PolicyDenied,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "status",
    content = "reason",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AgentAvailability {
    Available,
    Unavailable(AgentUnavailableReason),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTarget {
    pub operating_system: String,
    pub architecture: String,
    pub libc: String,
    pub libc_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPackage {
    pub package_id: String,
    pub version: String,
    pub sha256: String,
    pub signature_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenance {
    pub source_revision: String,
    pub manifest_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalogEntry {
    pub agent_id: String,
    pub target: AgentTarget,
    pub package: AgentPackage,
    pub provenance: AgentProvenance,
    pub capabilities: Vec<String>,
    pub availability: AgentAvailability,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalog {
    pub runner_instance_id: String,
    pub generation: u64,
    pub catalog_sha256: String,
    pub target: AgentTarget,
    pub agents: Vec<AgentCatalogEntry>,
    pub refreshed: bool,
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
    value: AgentCatalog,
}

/// Decode and validate ASB's complete v1.4 JSON-RPC response.
pub fn parse_agent_catalog_response(input: &str) -> Result<AgentCatalog, String> {
    let response: RpcResponse = serde_json::from_str(input)
        .map_err(|error| format!("invalid agent catalog response: {error}"))?;
    let _ = response.id;
    if response.jsonrpc != "2.0"
        || response.result.kind != "operation"
        || response.result.value.result.kind != "agent_catalog"
    {
        return Err("unsupported agent catalog response envelope".into());
    }
    validate_digest(&response.result.value.request_sha256)?;
    validate_catalog(response.result.value.result.value)
}

fn validate_catalog(catalog: AgentCatalog) -> Result<AgentCatalog, String> {
    validate_token(&catalog.runner_instance_id, "runner instance")?;
    validate_digest(&catalog.catalog_sha256)?;
    validate_target(&catalog.target)?;
    if catalog.agents.is_empty() || catalog.agents.len() > MAX_CATALOG_AGENTS {
        return Err("invalid agent catalog size".into());
    }
    let mut prior = None;
    for entry in &catalog.agents {
        validate_token(&entry.agent_id, "agent id")?;
        if prior.is_some_and(|id: &str| id >= entry.agent_id.as_str()) {
            return Err("agent catalog is not canonically sorted".into());
        }
        prior = Some(entry.agent_id.as_str());
        if entry.target != catalog.target {
            return Err("agent target does not match catalog target".into());
        }
        validate_token(&entry.package.package_id, "package id")?;
        validate_token(&entry.package.version, "package version")?;
        validate_digest(&entry.package.sha256)?;
        validate_digest(&entry.package.signature_sha256)?;
        if entry.provenance.source_revision.len() != 40
            || !entry
                .provenance
                .source_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("invalid source revision".into());
        }
        validate_digest(&entry.provenance.manifest_sha256)?;
        if entry.capabilities.is_empty() || entry.capabilities.len() > MAX_CAPABILITIES {
            return Err("invalid agent capability count".into());
        }
        let mut capabilities = std::collections::BTreeSet::new();
        for capability in &entry.capabilities {
            validate_token(capability, "agent capability")?;
            if !capabilities.insert(capability) {
                return Err("duplicate agent capability".into());
            }
        }
    }
    Ok(catalog)
}

fn validate_target(target: &AgentTarget) -> Result<(), String> {
    validate_token(&target.operating_system, "target operating system")?;
    validate_token(&target.architecture, "target architecture")?;
    validate_token(&target.libc, "target libc")?;
    validate_token(&target.libc_version, "target libc version")
}

fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Err("invalid SHA-256 digest".into())
    } else {
        Ok(())
    }
}

fn validate_token(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_STRING_BYTES
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
    {
        Err(format!("invalid {field}"))
    } else {
        Ok(())
    }
}
