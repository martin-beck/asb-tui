// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Strict, credential-free boundary for an approved local credential helper.
//!
//! The helper owns the secret store.  It returns only public provider identity
//! and SHA-256 locator/endpoint digests; raw key material is not representable
//! in this module.

use serde::Deserialize;

const MAX_RECEIPT_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CredentialEnrollmentReceipt {
    pub provider: String,
    pub endpoint_identity_sha256: String,
    pub credential_locator_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CredentialHelperError {
    Oversized,
    Malformed,
    InvalidProvider,
    InvalidDigest,
}

impl CredentialEnrollmentReceipt {
    /// Decode the bounded helper result.  A raw credential field or any
    /// unknown field is rejected by the closed serde boundary.
    pub fn decode(bytes: &[u8]) -> Result<Self, CredentialHelperError> {
        if bytes.is_empty() || bytes.len() > MAX_RECEIPT_BYTES {
            return Err(CredentialHelperError::Oversized);
        }
        let receipt: Self =
            serde_json::from_slice(bytes).map_err(|_| CredentialHelperError::Malformed)?;
        if !valid_provider(&receipt.provider) {
            return Err(CredentialHelperError::InvalidProvider);
        }
        if !valid_digest(&receipt.endpoint_identity_sha256)
            || !valid_digest(&receipt.credential_locator_sha256)
        {
            return Err(CredentialHelperError::InvalidDigest);
        }
        Ok(receipt)
    }
}

fn valid_provider(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Vec<u8> {
        serde_json::json!({
            "provider": "openai",
            "endpoint_identity_sha256": "a".repeat(64),
            "credential_locator_sha256": "b".repeat(64)
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn accepts_digest_only_receipt() {
        let receipt = CredentialEnrollmentReceipt::decode(&valid()).unwrap();
        assert_eq!(receipt.provider, "openai");
    }

    #[test]
    fn rejects_raw_secret_unknown_fields_and_bad_digests() {
        let secret = String::from_utf8(valid())
            .unwrap()
            .replace("}", ",\"api_key\":\"sk-secret\"}");
        assert_eq!(
            CredentialEnrollmentReceipt::decode(secret.as_bytes()),
            Err(CredentialHelperError::Malformed)
        );
        let bad = String::from_utf8(valid())
            .unwrap()
            .replace(&"a".repeat(64), "not-a-digest");
        assert_eq!(
            CredentialEnrollmentReceipt::decode(bad.as_bytes()),
            Err(CredentialHelperError::InvalidDigest)
        );
    }
}
