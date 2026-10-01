// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Focused coverage for authenticated adapter projection and wizard fences.

use asb_tui::{
    adapter_catalog::{AdapterCatalog, AuthMethod},
    provider_catalog::development_openrouter_catalog,
    wizard::{Wizard, WizardError},
};

#[test]
fn authenticated_provider_catalog_projects_models_and_auth_methods() {
    let provider_catalog = development_openrouter_catalog();
    let adapters = AdapterCatalog::from_provider_catalog(&provider_catalog).unwrap();
    let adapter = adapters.get("opencode").unwrap();
    assert!(adapter.providers["openrouter"].contains("openai/gpt-4o"));
    assert!(adapter.auth_methods.contains(&AuthMethod::None));
    assert!(
        adapter
            .auth_methods
            .contains(&AuthMethod::CredentialReference)
    );
}

#[test]
fn openrouter_adapter_selection_rejects_unknown_auth_and_missing_catalog() {
    let mut wizard = Wizard::default();
    for value in ["agent", "openrouter", "openai/gpt-4o", "defaults"] {
        wizard.set_value(value).unwrap();
        wizard.advance().unwrap();
    }
    wizard.set_value("unsupported").unwrap();
    assert_eq!(
        wizard.select_openrouter_adapter(),
        Err(WizardError::InvalidValue)
    );

    let mut stable = Wizard::stable();
    for value in ["agent", "openrouter", "openai/gpt-4o", "defaults"] {
        stable.set_value(value).unwrap();
        stable.advance().unwrap();
    }
    stable
        .set_value(
            "credential_reference:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
    assert!(matches!(
        stable.select_openrouter_adapter(),
        Err(WizardError::Catalog(_))
    ));
}
