// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Renderer-neutral adapter for ASB control protocol v1.4 agent catalogs.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

impl AgentCatalogRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_token(&self.runner_instance_id) {
            return Err("invalid runner instance".into());
        }
        Ok(())
    }
}

/// Serialize the exact JSON-RPC v1.4 catalog call without performing a refresh locally.
pub fn encode_agent_catalog_request(
    request_id: u64,
    timeout_ms: u64,
    request: &AgentCatalogRequest,
) -> Result<String, String> {
    request.validate()?;
    if timeout_ms == 0 || timeout_ms > 300_000 {
        return Err("invalid catalog timeout".into());
    }
    serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0", "id": request_id, "timeout_ms": timeout_ms,
        "method": "agent_catalog", "params": request
    }))
    .map_err(|error| format!("cannot encode catalog request: {error}"))
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
    pub signer: AgentSigner,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSigner {
    pub key_id: String,
    pub principal: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenance {
    pub source_revision: String,
    pub manifest_sha256: String,
    pub sbom_sha256: String,
    pub license_ref: String,
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

impl AgentCatalog {
    pub fn validate(&self) -> Result<(), String> {
        validate_catalog(self.clone()).map(|_| ())
    }

    /// Return the canonical v1.4 bytes used for `catalog_sha256`.
    ///
    /// The digest field itself is excluded; every other value is retained,
    /// arrays keep their protocol order, and object members are recursively
    /// ordered by their UTF-8 key bytes.  Callers must validate the catalog
    /// before using these bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        let mut value = serde_json::to_value(self).map_err(|_| "cannot serialize catalog")?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| "catalog is not an object".to_string())?;
        object.remove("catalog_sha256");
        // `refreshed` describes this response operation, not catalog identity.
        // Excluding it keeps Status and Refresh bound to the same snapshot.
        object.remove("refreshed");
        let value = canonical_value(value)?;
        serde_json::to_vec(&value).map_err(|_| "cannot serialize canonical catalog".into())
    }

    /// Compute the canonical content identity independently of ASB internals.
    pub fn computed_digest(&self) -> Result<String, String> {
        let bytes = self.canonical_bytes()?;
        Ok(crate::sha256::digest_hex(&bytes))
    }
}

fn canonical_value(value: Value) -> Result<Value, String> {
    match value {
        Value::Object(object) => {
            let mut ordered = Map::new();
            for (key, value) in object {
                ordered.insert(key, canonical_value(value)?);
            }
            // serde_json::Map is sorted when its `preserve_order` feature is
            // disabled, which is the pinned configuration of this crate.
            Ok(Value::Object(ordered))
        }
        Value::Array(values) => values
            .into_iter()
            .map(canonical_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        scalar => Ok(scalar),
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
        validate_agent_id(&entry.agent_id, "agent id")?;
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
        validate_token(&entry.package.signer.key_id, "signer key id")?;
        validate_token(&entry.package.signer.principal, "signer principal")?;
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
        validate_digest(&entry.provenance.sbom_sha256)?;
        validate_token(&entry.provenance.license_ref, "license reference")?;
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
    let computed = catalog.computed_digest()?;
    if computed != catalog.catalog_sha256 {
        return Err("agent catalog digest does not match content".into());
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

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_STRING_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

fn validate_agent_id(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_STRING_BYTES
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Err(format!("invalid {field}"))
    } else {
        Ok(())
    }
}
