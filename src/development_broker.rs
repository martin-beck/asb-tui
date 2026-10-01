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
pub const ASB_SOURCE_COMMIT: &str = "e8afc588cbe1c1730add1f8d636fa3241276cc9a";
pub const ASB_SOURCE_TREE: &str = "2700a6a8d1de297308b31a0ea56c9ae6e860d9af";
pub const TUI_SOURCE_COMMIT: &str = "3e69d82ddb5fa6988f30bb7a266c0a9f2956ad44";
pub const TUI_SOURCE_TREE: &str = "ddb6a51c1ba6f413715439a02c15fd31c8c77a8c";

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
        Self::parse(value.as_bytes())
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
            || self.asb_source_commit != ASB_SOURCE_COMMIT
            || self.asb_source_tree != ASB_SOURCE_TREE
            || self.tui_source_commit != TUI_SOURCE_COMMIT
            || self.tui_source_tree != TUI_SOURCE_TREE
        {
            return Err(DescriptorError::Unsupported);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> String {
        serde_json::json!({
            "schema_version": 1,
            "profile": "development",
            "development_only": true,
            "operation": "launch",
            "protocol_minor": 10,
            "asb_source_commit": ASB_SOURCE_COMMIT,
            "asb_source_tree": ASB_SOURCE_TREE,
            "tui_source_commit": TUI_SOURCE_COMMIT,
            "tui_source_tree": TUI_SOURCE_TREE
        })
        .to_string()
    }

    #[test]
    fn accepts_closed_development_descriptor() {
        let parsed = DevelopmentBrokerDescriptor::parse(descriptor().as_bytes()).unwrap();
        assert_eq!(parsed.operation, "launch");
        assert!(parsed.development_only);
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
        assert_eq!(
            DevelopmentBrokerDescriptor::parse(value.to_string().as_bytes()),
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
}
