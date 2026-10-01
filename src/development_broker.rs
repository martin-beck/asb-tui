// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed development-only descriptor for the ASB broker entrypoint.
//!
//! The descriptor carries public build/lifecycle identity only. It is a
//! preflight fence, not an authentication mechanism; the inherited broker
//! descriptor and typed control negotiation remain authoritative.

use crate::protocol_compatibility;
use serde::Deserialize;

const MAX_DESCRIPTOR_BYTES: usize = 16 * 1024;
const EXPECTED_ASB_COMMIT: &str = "ASB_TUI_EXPECTED_ASB_SOURCE_COMMIT";
const EXPECTED_ASB_TREE: &str = "ASB_TUI_EXPECTED_ASB_SOURCE_TREE";
const EXPECTED_TUI_COMMIT: &str = "ASB_TUI_EXPECTED_TUI_SOURCE_COMMIT";
const EXPECTED_TUI_TREE: &str = "ASB_TUI_EXPECTED_TUI_SOURCE_TREE";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentBrokerDescriptor {
    pub schema_version: u16,
    pub profile: String,
    pub development_only: bool,
    pub operation: String,
    pub protocol_minor: u16,
    pub asb_source_commit: String,
    pub asb_source_tree: String,
    pub tui_source_commit: String,
    pub tui_source_tree: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    Missing,
    Oversized,
    Malformed,
    Unsupported,
}

impl DevelopmentBrokerDescriptor {
    pub fn from_env() -> Result<Self, DescriptorError> {
        let value =
            std::env::var_os("ASB_TUI_DEVELOPMENT_DESCRIPTOR").ok_or(DescriptorError::Missing)?;
        let value = value.to_str().ok_or(DescriptorError::Malformed)?;
        let descriptor = Self::parse(value.as_bytes())?;
        let asb_commit = expected_identity(EXPECTED_ASB_COMMIT)?;
        let asb_tree = expected_identity(EXPECTED_ASB_TREE)?;
        let tui_commit = expected_identity(EXPECTED_TUI_COMMIT)?;
        let tui_tree = expected_identity(EXPECTED_TUI_TREE)?;
        descriptor.validate_identity(&asb_commit, &asb_tree, &tui_commit, &tui_tree)?;
        Ok(descriptor)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, DescriptorError> {
        if bytes.is_empty() {
            return Err(DescriptorError::Malformed);
        }
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err(DescriptorError::Oversized);
        }
        let descriptor: Self =
            serde_json::from_slice(bytes).map_err(|_| DescriptorError::Malformed)?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<(), DescriptorError> {
        if self.schema_version != 1
            || self.profile != "development"
            || !self.development_only
            || !matches!(
                self.operation.as_str(),
                "install" | "upgrade" | "status" | "launch" | "remove"
            )
            || protocol_compatibility::validate_version(crate::control_codec::ControlVersion {
                major: 1,
                minor: self.protocol_minor,
            })
            .is_err()
            || !valid_identity(&self.asb_source_commit)
            || !valid_identity(&self.asb_source_tree)
            || !valid_identity(&self.tui_source_commit)
            || !valid_identity(&self.tui_source_tree)
        {
            return Err(DescriptorError::Unsupported);
        }
        Ok(())
    }

    fn validate_identity(
        &self,
        asb_commit: &str,
        asb_tree: &str,
        tui_commit: &str,
        tui_tree: &str,
    ) -> Result<(), DescriptorError> {
        if self.asb_source_commit != asb_commit
            || self.asb_source_tree != asb_tree
            || self.tui_source_commit != tui_commit
            || self.tui_source_tree != tui_tree
        {
            return Err(DescriptorError::Unsupported);
        }
        Ok(())
    }
}

fn expected_identity(name: &str) -> Result<String, DescriptorError> {
    let value = std::env::var(name).map_err(|_| DescriptorError::Unsupported)?;
    if !valid_identity(&value) {
        return Err(DescriptorError::Unsupported);
    }
    Ok(value)
}

fn valid_identity(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASB_COMMIT: &str = "a";
    const ASB_TREE: &str = "b";
    const TUI_COMMIT: &str = "c";
    const TUI_TREE: &str = "d";

    fn identity(value: &str) -> String {
        value.repeat(40)
    }

    fn descriptor() -> String {
        serde_json::json!({
            "schema_version": 1,
            "profile": "development",
            "development_only": true,
            "operation": "launch",
            "protocol_minor": 10,
            "asb_source_commit": identity(ASB_COMMIT),
            "asb_source_tree": identity(ASB_TREE),
            "tui_source_commit": identity(TUI_COMMIT),
            "tui_source_tree": identity(TUI_TREE)
        })
        .to_string()
    }

    #[test]
    fn accepts_closed_development_descriptor() {
        let parsed = DevelopmentBrokerDescriptor::parse(descriptor().as_bytes()).unwrap();
        assert_eq!(parsed.operation, "launch");
        assert!(parsed.development_only);
        parsed
            .validate_identity(
                &identity(ASB_COMMIT),
                &identity(ASB_TREE),
                &identity(TUI_COMMIT),
                &identity(TUI_TREE),
            )
            .unwrap();
    }

    #[test]
    fn rejects_production_unknown_and_stale_shapes() {
        let mut value: serde_json::Value = serde_json::from_str(&descriptor()).unwrap();
        value["development_only"] = serde_json::Value::Bool(false);
        assert_eq!(
            DevelopmentBrokerDescriptor::parse(value.to_string().as_bytes()),
            Err(DescriptorError::Unsupported)
        );

        let mut value: serde_json::Value = serde_json::from_str(&descriptor()).unwrap();
        value["tui_source_commit"] = serde_json::Value::String("a".repeat(40));
        let parsed = DevelopmentBrokerDescriptor::parse(value.to_string().as_bytes()).unwrap();
        assert_eq!(
            parsed.validate_identity(
                &identity(ASB_COMMIT),
                &identity(ASB_TREE),
                &identity(TUI_COMMIT),
                &identity(TUI_TREE),
            ),
            Err(DescriptorError::Unsupported)
        );

        let mut value: serde_json::Value = serde_json::from_str(&descriptor()).unwrap();
        value["secret"] = serde_json::Value::String("never".into());
        assert_eq!(
            DevelopmentBrokerDescriptor::parse(value.to_string().as_bytes()),
            Err(DescriptorError::Malformed)
        );

        let mut value: serde_json::Value = serde_json::from_str(&descriptor()).unwrap();
        value["protocol_minor"] = serde_json::Value::Number(9.into());
        assert_eq!(
            DevelopmentBrokerDescriptor::parse(value.to_string().as_bytes()),
            Err(DescriptorError::Unsupported)
        );
    }

    #[test]
    fn rejects_stale_identity_inputs() {
        let parsed = DevelopmentBrokerDescriptor::parse(descriptor().as_bytes()).unwrap();
        assert_eq!(
            parsed.validate_identity(
                &identity("e"),
                &identity(ASB_TREE),
                &identity(TUI_COMMIT),
                &identity(TUI_TREE),
            ),
            Err(DescriptorError::Unsupported)
        );
    }
}
