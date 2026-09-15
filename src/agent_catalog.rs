// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Renderer-neutral projection of ASB's authenticated local-agent catalog.
//!
//! The control transport is responsible for authenticating the peer and binding
//! the response to the current runner generation.  This module only accepts the
//! resulting, bounded public projection; it never installs, launches, or renders
//! an agent.

use serde::Deserialize;
use std::collections::BTreeSet;

const MAX_CATALOG_BYTES: usize = 256 * 1024;
const MAX_AGENTS: usize = 256;
const MAX_CAPABILITIES: usize = 32;
const MAX_TARGETS: usize = 16;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalog {
    pub protocol: String,
    pub protocol_version: u64,
    pub runner_generation: String,
    pub catalog_digest: String,
    pub authentication: Authentication,
    pub agents: Vec<AgentDescriptor>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Authentication {
    pub status: String,
    pub authorization: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentDescriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub targets: Vec<Target>,
    pub package: Package,
    pub capabilities: Vec<String>,
    pub availability: Availability,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub operating_system: String,
    pub architecture: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub format: String,
    pub digest: String,
    pub signature: String,
    pub provenance: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Availability {
    pub state: String,
    pub reason: Option<String>,
}

/// Parse and validate the bounded v1 agent catalog projection.
pub fn parse_agent_catalog(input: &str) -> Result<AgentCatalog, String> {
    if input.len() > MAX_CATALOG_BYTES {
        return Err("agent catalog exceeds the size limit".into());
    }
    let catalog: AgentCatalog =
        serde_json::from_str(input).map_err(|error| format!("invalid agent catalog: {error}"))?;
    if catalog.protocol != "asb-agent-catalog" || catalog.protocol_version != 1 {
        return Err("unsupported agent catalog protocol or version".into());
    }
    if catalog.authentication.status != "verified"
        || catalog.authentication.authorization != "agent-catalog-read"
    {
        return Err("agent catalog authentication is not verified".into());
    }
    validate_token(&catalog.runner_generation, 128, "runner generation")?;
    validate_digest(&catalog.catalog_digest, "catalog digest")?;
    if catalog.agents.is_empty() || catalog.agents.len() > MAX_AGENTS {
        return Err("agent catalog has an invalid agent count".into());
    }
    let mut ids = BTreeSet::new();
    for agent in &catalog.agents {
        validate_agent(agent)?;
        if !ids.insert(&agent.id) {
            return Err("agent catalog contains a duplicate agent id".into());
        }
    }
    Ok(catalog)
}

fn validate_agent(agent: &AgentDescriptor) -> Result<(), String> {
    if agent.id.is_empty()
        || agent.id.len() > 64
        || !agent.id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
        || agent.id.starts_with(['.', '-', '_'])
    {
        return Err("agent catalog contains an invalid agent id".into());
    }
    validate_public_text(&agent.name, 128, "agent name")?;
    validate_token(&agent.version, 64, "agent version")?;
    if agent.targets.is_empty() || agent.targets.len() > MAX_TARGETS {
        return Err("agent catalog contains an invalid target count".into());
    }
    let mut targets = BTreeSet::new();
    for target in &agent.targets {
        validate_token(&target.operating_system, 32, "target operating system")?;
        validate_token(&target.architecture, 32, "target architecture")?;
        if !targets.insert((&target.operating_system, &target.architecture)) {
            return Err("agent catalog contains a duplicate target".into());
        }
    }
    validate_package(&agent.package)?;
    if agent.capabilities.is_empty() || agent.capabilities.len() > MAX_CAPABILITIES {
        return Err("agent catalog contains an invalid capability count".into());
    }
    let mut capabilities = BTreeSet::new();
    for capability in &agent.capabilities {
        validate_token(capability, 64, "agent capability")?;
        if !capabilities.insert(capability) {
            return Err("agent catalog contains a duplicate capability".into());
        }
    }
    match (
        agent.availability.state.as_str(),
        agent.availability.reason.as_deref(),
    ) {
        ("available", None) => Ok(()),
        ("unavailable", Some(reason)) => validate_token(reason, 64, "availability reason"),
        _ => Err("agent catalog contains an invalid availability state".into()),
    }
}

fn validate_package(package: &Package) -> Result<(), String> {
    validate_token(&package.format, 32, "package format")?;
    validate_digest(&package.digest, "package digest")?;
    validate_token(&package.signature, 256, "package signature")?;
    validate_token(&package.provenance, 256, "package provenance")
}

fn validate_digest(value: &str, field: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{field} is not a lowercase SHA-256 digest"));
    }
    Ok(())
}

fn validate_token(value: &str, limit: usize, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > limit
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'/' || byte == b'\\')
    {
        return Err(format!("{field} is invalid"));
    }
    Ok(())
}

fn validate_public_text(value: &str, limit: usize, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > limit
        || value.chars().any(|character| character.is_control())
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(format!("{field} is invalid"));
    }
    Ok(())
}
