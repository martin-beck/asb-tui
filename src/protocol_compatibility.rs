// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Cross-project ASB control compatibility policy.
//!
//! This is a development qualification contract, not a claim that every
//! operation is available from every ASB deployment.  The matrix names only
//! versions and operations this TUI can decode and validate; the runner still
//! remains authoritative for capabilities and authorization.

use crate::control_codec::{
    ControlCall, ControlVersion, V1_0, V1_2, V1_3, V1_4, V1_5, V1_6, V1_7, V1_8, V1_10,
};
use std::collections::BTreeSet;

pub const MATRIX_SCHEMA_VERSION: u16 = 1;

/// Exact ASB control minors represented by this TUI codec.  Gaps are
/// intentional: only published, independently validated minors are offered.
pub const SUPPORTED_CONTROL_VERSIONS: &[ControlVersion] =
    &[V1_0, V1_2, V1_3, V1_4, V1_5, V1_6, V1_7, V1_8, V1_10];

/// Public matrix row for tooling and development qualification reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationRow {
    pub name: &'static str,
    pub minimum: ControlVersion,
}

/// Return the closed operation matrix without request data or private paths.
#[must_use]
pub fn operation_matrix() -> Vec<OperationRow> {
    // Keep this list synchronized with `ControlCall::operation_name` and
    // `ControlCall::minimum_version`; the test below makes drift visible.
    vec![
        OperationRow {
            name: "negotiate",
            minimum: V1_0,
        },
        OperationRow {
            name: "capabilities",
            minimum: V1_0,
        },
        OperationRow {
            name: "measurement_catalog",
            minimum: V1_2,
        },
        OperationRow {
            name: "benchmark_catalog",
            minimum: V1_10,
        },
        OperationRow {
            name: "agent_catalog",
            minimum: V1_4,
        },
        OperationRow {
            name: "agent_install",
            minimum: V1_5,
        },
        OperationRow {
            name: "agent_status",
            minimum: V1_5,
        },
        OperationRow {
            name: "agent_cancel",
            minimum: V1_5,
        },
        OperationRow {
            name: "agent_retry",
            minimum: V1_5,
        },
        OperationRow {
            name: "agent_remove",
            minimum: V1_5,
        },
        OperationRow {
            name: "auth_enroll",
            minimum: V1_6,
        },
        OperationRow {
            name: "auth_status",
            minimum: V1_6,
        },
        OperationRow {
            name: "auth_rotate",
            minimum: V1_6,
        },
        OperationRow {
            name: "auth_revoke",
            minimum: V1_6,
        },
        OperationRow {
            name: "auth_helper_invoke",
            minimum: V1_10,
        },
        OperationRow {
            name: "validate_settings",
            minimum: V1_0,
        },
        OperationRow {
            name: "create_plan",
            minimum: V1_0,
        },
        OperationRow {
            name: "launch",
            minimum: V1_0,
        },
        OperationRow {
            name: "status",
            minimum: V1_0,
        },
        OperationRow {
            name: "cancel",
            minimum: V1_0,
        },
        OperationRow {
            name: "history",
            minimum: V1_0,
        },
        OperationRow {
            name: "repeat",
            minimum: V1_0,
        },
        OperationRow {
            name: "analyze",
            minimum: V1_0,
        },
        OperationRow {
            name: "events",
            minimum: V1_0,
        },
        OperationRow {
            name: "artifact_metadata",
            minimum: V1_0,
        },
        OperationRow {
            name: "provider_catalog",
            minimum: V1_7,
        },
        OperationRow {
            name: "configuration_status",
            minimum: V1_7,
        },
        OperationRow {
            name: "configuration_apply",
            minimum: V1_7,
        },
        OperationRow {
            name: "recording_campaign_plan",
            minimum: V1_7,
        },
        OperationRow {
            name: "recording_campaign_status",
            minimum: V1_7,
        },
        OperationRow {
            name: "recording_campaign_estimate",
            minimum: V1_7,
        },
        OperationRow {
            name: "recording_campaign_execute",
            minimum: V1_8,
        },
        OperationRow {
            name: "recording_campaign_progress",
            minimum: V1_8,
        },
        OperationRow {
            name: "recording_campaign_cancel",
            minimum: V1_8,
        },
        OperationRow {
            name: "recording_campaign_reconcile",
            minimum: V1_8,
        },
        OperationRow {
            name: "recording_campaign_offline_default",
            minimum: V1_8,
        },
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityError {
    UnsupportedSchema,
    UnsupportedVersion,
    OperationUnavailable,
}

#[must_use]
pub fn supported_versions() -> BTreeSet<ControlVersion> {
    SUPPORTED_CONTROL_VERSIONS.iter().copied().collect()
}

pub fn validate_schema(schema_version: u16) -> Result<(), CompatibilityError> {
    (schema_version == MATRIX_SCHEMA_VERSION)
        .then_some(())
        .ok_or(CompatibilityError::UnsupportedSchema)
}

pub fn validate_version(version: ControlVersion) -> Result<(), CompatibilityError> {
    supported_versions()
        .contains(&version)
        .then_some(())
        .ok_or(CompatibilityError::UnsupportedVersion)
}

pub fn validate_operation(
    version: ControlVersion,
    operation: &ControlCall,
) -> Result<(), CompatibilityError> {
    validate_version(version)?;
    match operation.minimum_version() {
        Some(minimum) if version < minimum => Err(CompatibilityError::OperationUnavailable),
        _ => Ok(()),
    }
}

#[must_use]
pub fn operation_minimum(operation: &ControlCall) -> ControlVersion {
    operation.minimum_version().unwrap_or(V1_0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_codec::{
        ControlCall, ControlLimits, ControlRequest, NegotiateParams, RequestId,
    };

    #[test]
    fn matrix_is_explicit_and_has_no_unsupported_gaps() {
        assert_eq!(SUPPORTED_CONTROL_VERSIONS.len(), 9);
        assert!(validate_version(V1_10).is_ok());
        assert_eq!(
            validate_version(ControlVersion { major: 1, minor: 9 }),
            Err(CompatibilityError::UnsupportedVersion)
        );
        assert_eq!(
            validate_version(ControlVersion { major: 2, minor: 0 }),
            Err(CompatibilityError::UnsupportedVersion)
        );
    }

    #[test]
    fn operation_minors_are_checked_before_transport() {
        let operation = ControlCall::AgentInstall(
            serde_json::from_value(serde_json::json!({
                "binding": {
                    "agent_id": "fixture-agent",
                    "runner_instance_id": "fixture-runner",
                    "catalog_sha256": "a".repeat(64)
                },
                "catalog_generation": 1,
                "idempotency_key": "fixture-operation"
            }))
            .unwrap(),
        );
        assert_eq!(operation.operation_name(), "agent_install");
        assert_eq!(operation_minimum(&operation), V1_5);
        assert_eq!(
            validate_operation(V1_4, &operation),
            Err(CompatibilityError::OperationUnavailable)
        );
        assert!(validate_operation(V1_5, &operation).is_ok());
    }

    #[test]
    fn benchmark_catalog_requires_the_asb_v110_common_version() {
        let operation = ControlCall::BenchmarkCatalog;
        assert_eq!(operation_minimum(&operation), V1_10);
        assert_eq!(
            validate_operation(V1_8, &operation),
            Err(CompatibilityError::OperationUnavailable)
        );
        assert!(validate_operation(V1_10, &operation).is_ok());
    }

    #[test]
    fn matrix_schema_is_closed() {
        assert!(validate_schema(MATRIX_SCHEMA_VERSION).is_ok());
        assert_eq!(
            validate_schema(2),
            Err(CompatibilityError::UnsupportedSchema)
        );
        let request = ControlRequest {
            jsonrpc: "2.0".into(),
            id: RequestId(1),
            timeout_ms: 1_000,
            call: ControlCall::Negotiate(NegotiateParams {
                versions: supported_versions(),
                limits: ControlLimits::default(),
            }),
        };
        assert_eq!(request.call.operation_name(), "negotiate");
    }

    #[test]
    fn operation_matrix_has_unique_closed_names() {
        let rows = operation_matrix();
        let names: BTreeSet<_> = rows.iter().map(|row| row.name).collect();
        assert_eq!(names.len(), rows.len());
        assert!(rows.iter().all(|row| {
            supported_versions().contains(&row.minimum)
                && row.minimum.major == 1
                && !row.name.is_empty()
        }));
    }
}
