// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Development-only credential enrollment used by the setup wizard.
//!
//! This flow deliberately creates metadata fixtures, never a provider secret.
//! It lets a local/mock provider be selected and exercised when authentication,
//! signature validation, or key management is unavailable.  Production helper
//! and keychain behavior remains in the authenticated control boundary.

use crate::sha256::digest_hex;

const MAX_PROVIDER_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevelopmentAuthMethod {
    LocalFixture,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DevelopmentAuthStatus {
    Unconfigured,
    Enrolled,
    Tested,
    Rotated,
    Reset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DevelopmentAuthError {
    InvalidProvider,
    UnsupportedMethod,
    NotEnrolled,
    AlreadyReset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevelopmentAuthSnapshot {
    pub provider: String,
    pub method: DevelopmentAuthMethod,
    pub status: DevelopmentAuthStatus,
    pub generation: u64,
    pub endpoint_identity_sha256: Option<String>,
    pub credential_locator_sha256: Option<String>,
    pub signature_sha256: Option<String>,
    pub warning: String,
    pub development_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevelopmentAuthFlow {
    provider: String,
    method: DevelopmentAuthMethod,
    status: DevelopmentAuthStatus,
    generation: u64,
    endpoint_identity_sha256: Option<String>,
    credential_locator_sha256: Option<String>,
    signature_sha256: Option<String>,
}

impl DevelopmentAuthFlow {
    pub fn new(provider: impl Into<String>) -> Result<Self, DevelopmentAuthError> {
        let provider = provider.into();
        if provider.is_empty()
            || provider.len() > MAX_PROVIDER_BYTES
            || !provider.is_ascii()
            || provider.chars().any(char::is_control)
        {
            return Err(DevelopmentAuthError::InvalidProvider);
        }
        Ok(Self {
            provider,
            method: DevelopmentAuthMethod::LocalFixture,
            status: DevelopmentAuthStatus::Unconfigured,
            generation: 0,
            endpoint_identity_sha256: None,
            credential_locator_sha256: None,
            signature_sha256: None,
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> DevelopmentAuthSnapshot {
        DevelopmentAuthSnapshot {
            provider: self.provider.clone(),
            method: self.method,
            status: self.status,
            generation: self.generation,
            endpoint_identity_sha256: self.endpoint_identity_sha256.clone(),
            credential_locator_sha256: self.credential_locator_sha256.clone(),
            signature_sha256: self.signature_sha256.clone(),
            warning: "Development-only local fixture; authentication, signature validation, and key management are not production guarantees.".into(),
            development_only: true,
        }
    }

    pub fn select_method(
        &mut self,
        method: DevelopmentAuthMethod,
    ) -> Result<(), DevelopmentAuthError> {
        if method == DevelopmentAuthMethod::None {
            self.method = method;
            self.reset();
            return Ok(());
        }
        self.method = method;
        Ok(())
    }

    pub fn enroll(&mut self) -> Result<(), DevelopmentAuthError> {
        if self.method != DevelopmentAuthMethod::LocalFixture {
            return Err(DevelopmentAuthError::UnsupportedMethod);
        }
        self.generation = self.generation.saturating_add(1).max(1);
        self.endpoint_identity_sha256 = Some(self.digest("endpoint"));
        self.credential_locator_sha256 = Some(self.digest("locator"));
        self.signature_sha256 = Some(self.digest("signature"));
        self.status = DevelopmentAuthStatus::Enrolled;
        Ok(())
    }

    pub fn test(&mut self) -> Result<(), DevelopmentAuthError> {
        self.require_enrolled()?;
        self.status = DevelopmentAuthStatus::Tested;
        Ok(())
    }

    pub fn rotate(&mut self) -> Result<(), DevelopmentAuthError> {
        self.require_enrolled()?;
        self.generation = self.generation.saturating_add(1);
        self.endpoint_identity_sha256 = Some(self.digest("endpoint"));
        self.credential_locator_sha256 = Some(self.digest("locator"));
        self.signature_sha256 = Some(self.digest("signature"));
        self.status = DevelopmentAuthStatus::Rotated;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.endpoint_identity_sha256 = None;
        self.credential_locator_sha256 = None;
        self.signature_sha256 = None;
        self.status = DevelopmentAuthStatus::Reset;
    }

    pub fn restart(&mut self) {
        self.endpoint_identity_sha256 = None;
        self.credential_locator_sha256 = None;
        self.signature_sha256 = None;
        self.status = DevelopmentAuthStatus::Unconfigured;
    }

    fn require_enrolled(&self) -> Result<(), DevelopmentAuthError> {
        if self.endpoint_identity_sha256.is_some() {
            Ok(())
        } else {
            Err(DevelopmentAuthError::NotEnrolled)
        }
    }

    fn digest(&self, kind: &str) -> String {
        digest_hex(
            format!(
                "asb-tui-development-fixture:v1:{}:{}:{}",
                self.provider, self.generation, kind
            )
            .as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_enroll_test_rotate_and_reset_are_digest_only() {
        let mut flow = DevelopmentAuthFlow::new("openrouter").unwrap();
        assert_eq!(flow.snapshot().status, DevelopmentAuthStatus::Unconfigured);
        flow.enroll().unwrap();
        let first = flow.snapshot();
        assert_eq!(first.status, DevelopmentAuthStatus::Enrolled);
        assert_eq!(first.generation, 1);
        for digest in [
            first.endpoint_identity_sha256.as_ref(),
            first.credential_locator_sha256.as_ref(),
            first.signature_sha256.as_ref(),
        ] {
            assert_eq!(digest.unwrap().len(), 64);
        }
        assert!(first.warning.contains("Development-only"));
        flow.test().unwrap();
        assert_eq!(flow.snapshot().status, DevelopmentAuthStatus::Tested);
        flow.rotate().unwrap();
        assert_eq!(flow.snapshot().generation, 2);
        assert_ne!(
            flow.snapshot().credential_locator_sha256,
            first.credential_locator_sha256
        );
        flow.reset();
        assert_eq!(flow.snapshot().status, DevelopmentAuthStatus::Reset);
        assert!(flow.snapshot().credential_locator_sha256.is_none());
    }

    #[test]
    fn fixture_is_non_blocking_when_external_auth_is_unavailable() {
        let mut flow = DevelopmentAuthFlow::new("mock-provider").unwrap();
        flow.select_method(DevelopmentAuthMethod::LocalFixture)
            .unwrap();
        flow.enroll().unwrap();
        assert!(flow.snapshot().development_only);
        assert!(DevelopmentAuthFlow::new("bad\nprovider").is_err());
    }

    #[test]
    fn unsupported_and_unenrolled_operations_are_typed() {
        let mut flow = DevelopmentAuthFlow::new("mock").unwrap();
        assert_eq!(flow.test(), Err(DevelopmentAuthError::NotEnrolled));
        flow.select_method(DevelopmentAuthMethod::None).unwrap();
        assert_eq!(flow.enroll(), Err(DevelopmentAuthError::UnsupportedMethod));
    }
}
