// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Focused coverage for authenticated adapter projection and wizard fences.

use asb_tui::{
    adapter_catalog::{AdapterCatalog, AdapterSelection, AuthMethod},
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

#[test]
fn adapter_choices_are_filtered_and_cancel_safe() {
    let mut wizard = Wizard::default();
    let options = wizard.adapter_compatibility("opendesk").unwrap();
    assert!(options.providers.iter().any(|item| item.id == "local"));
    let local = wizard
        .adapter_compatibility_for("opendesk", Some("local"))
        .unwrap();
    assert_eq!(local.provider_id.as_deref(), Some("local"));
    assert!(local.models.iter().all(|item| item.id == "fixture-model"));
    wizard
        .select_adapter(AdapterSelection {
            adapter_id: "opendesk".into(),
            provider_id: "local".into(),
            model_id: "fixture-model".into(),
            auth: AuthMethod::LocalDaemon,
        })
        .unwrap();
    assert_eq!(wizard.selected_adapter_id(), Some("opendesk"));
    assert!(
        wizard
            .adapter_diagnostics(&AdapterSelection {
                adapter_id: "opendesk".into(),
                provider_id: "openrouter".into(),
                model_id: "openai/gpt-4o".into(),
                auth: AuthMethod::CredentialReference,
            })
            .contains("not supported")
    );
}
