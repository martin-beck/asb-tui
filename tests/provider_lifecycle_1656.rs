// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    adapter_catalog::{AdapterCatalog, AdapterSelection, AuthMethod, CompatibilityMatrix},
    control_codec::{
        ProviderAuthMethod, ProviderAvailability, ProviderCatalog, ProviderCatalogEntry,
        ProviderModel, Revision,
    },
    provider_catalog::{ConnectedProviderRegistry, ProviderProfile, ProviderRefreshError},
    wizard::Wizard,
};

fn catalog(provider: &str, model: &str) -> ProviderCatalog {
    ProviderCatalog {
        runner_instance_id: "fixture".into(),
        generation: Revision(2),
        catalog_sha256: "a".repeat(64),
        refreshed: true,
        providers: vec![ProviderCatalogEntry {
            provider_id: provider.into(),
            display_name: provider.into(),
            auth_methods: vec![ProviderAuthMethod::None],
            availability: ProviderAvailability::Available,
            models: vec![ProviderModel {
                model_id: model.into(),
                revision: "1".into(),
                availability: ProviderAvailability::Available,
            }],
        }],
    }
}

fn profile() -> ProviderProfile {
    ProviderProfile {
        provider_id: "openrouter".into(),
        display_name: "OpenRouter".into(),
        auth_method: ProviderAuthMethod::None,
        credential_reference_sha256: None,
    }
}

#[test]
fn add_refresh_filters_models_and_cancel_preserves_last_valid_profile() {
    let mut registry = ConnectedProviderRegistry::new(None, None).unwrap();
    registry.begin_add(profile()).unwrap();
    registry
        .refresh(|_| Ok(catalog("openrouter", "fixture-model")))
        .unwrap();
    assert_eq!(
        registry
            .available_models()
            .iter()
            .map(|m| m.model_id.as_str())
            .collect::<Vec<_>>(),
        ["fixture-model"]
    );
    registry.begin_edit().unwrap();
    registry.cancel();
    assert_eq!(registry.profile().unwrap().provider_id, "openrouter");
}

#[test]
fn failed_refresh_is_typed_redacted_and_keeps_catalog() {
    let mut registry = ConnectedProviderRegistry::new(None, None).unwrap();
    registry.begin_add(profile()).unwrap();
    registry
        .refresh(|_| Ok(catalog("openrouter", "fixture-model")))
        .unwrap();
    registry.begin_edit().unwrap();
    let error = registry
        .refresh(|_| Err("raw api_key=never-returned".into()))
        .unwrap_err();
    assert!(matches!(error, ProviderRefreshError::Unavailable(_)));
    let rendered = error.to_string();
    assert!(rendered.contains("details redacted"));
    assert!(!rendered.contains("api_key"));
    assert!(!rendered.contains("never-returned"));
    assert_eq!(registry.available_models().len(), 1);
    let json = registry.diagnostic().to_json().unwrap();
    assert!(!json.contains("api_key") && !json.contains("never-returned"));
}

#[test]
fn wizard_projects_connected_provider_choices_without_erasing_draft() {
    let mut wizard = Wizard::default();
    wizard.set_provider_catalog(catalog("openrouter", "fixture-model"));
    let (_, models) = wizard.connected_provider_options().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "fixture-model");
}

#[test]
fn compatibility_matrix_supports_defaults_overrides_restart_and_offline_validation() {
    let mut matrix = CompatibilityMatrix::development();
    let default = AdapterSelection {
        adapter_id: "opencode".into(),
        provider_id: "openai".into(),
        model_id: "gpt-4o".into(),
        auth: AuthMethod::None,
    };
    let override_selection = AdapterSelection {
        provider_id: "openrouter".into(),
        model_id: "openai/gpt-4o".into(),
        ..default.clone()
    };
    matrix.set_default("agent-a", default.clone()).unwrap();
    matrix
        .set_override("agent-a", override_selection.clone())
        .unwrap();
    assert_eq!(matrix.resolve("agent-a"), Some(&override_selection));
    matrix.mark_unavailable("openrouter", "development auth unavailable");
    let options = matrix.options("opencode", Some("openrouter")).unwrap();
    assert!(!options.models[0].available);
    assert!(
        options.models[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("development")
    );
    assert!(matrix.validate_offline(override_selection).is_ok());
    matrix.restart();
    assert_eq!(matrix.resolve("agent-a"), Some(&default));
}

#[test]
fn matrix_evaluates_both_adapters_and_keeps_unavailable_reason_typed() {
    let mut matrix = CompatibilityMatrix::development();
    let all = matrix.evaluate();
    assert!(all.iter().any(|case| {
        case.adapter_id == "opencode"
            && case.provider_id == "openrouter"
            && case.model_id == "openai/gpt-4o"
            && case.supported
    }));
    assert!(all.iter().any(|case| {
        case.adapter_id == "opendesk"
            && case.provider_id == "local"
            && case.model_id == "fixture-model"
            && case.supported
    }));

    matrix.mark_unavailable("openrouter", "development authentication unavailable");
    let unavailable = matrix
        .evaluate()
        .into_iter()
        .find(|case| case.adapter_id == "opencode" && case.provider_id == "openrouter")
        .unwrap();
    assert!(!unavailable.supported);
    assert_eq!(
        unavailable.reason.as_deref(),
        Some("development authentication unavailable")
    );
}

#[test]
fn connected_catalog_projects_both_adapter_routes() {
    let catalog = catalog("openrouter", "fixture-model");
    let adapters = AdapterCatalog::from_provider_catalog(&catalog).unwrap();
    for adapter in ["opencode", "opendesk"] {
        let options = asb_tui::adapter_catalog::SelectionSession::new(adapters.clone())
            .compatibility_options(adapter)
            .unwrap();
        assert!(
            options
                .providers
                .iter()
                .any(|option| option.id == "openrouter")
        );
        assert!(
            options
                .models
                .iter()
                .any(|option| option.id == "fixture-model")
        );
    }
}

#[test]
fn connected_catalog_keeps_unavailable_provider_and_model_visible_but_unselectable() {
    let mut catalog = catalog("openrouter", "fixture-model");
    catalog.providers[0].models[0].availability =
        ProviderAvailability::Unavailable("development authentication unavailable".into());
    let adapters = AdapterCatalog::from_provider_catalog(&catalog).unwrap();
    let session = asb_tui::adapter_catalog::SelectionSession::new(adapters);
    let options = session
        .compatibility_options("opendesk")
        .expect("connected adapter route");
    let provider = options
        .providers
        .iter()
        .find(|option| option.id == "openrouter")
        .expect("provider remains visible");
    assert!(provider.available);
    assert!(provider.reason.is_none());
    let model = options
        .models
        .iter()
        .find(|option| option.id == "fixture-model")
        .expect("model remains visible");
    assert!(!model.available);
    assert_eq!(
        model.reason.as_deref(),
        Some("development authentication unavailable")
    );
    assert!(
        session.validate(AdapterSelection {
            adapter_id: "opendesk".into(),
            provider_id: "openrouter".into(),
            model_id: "fixture-model".into(),
            auth: AuthMethod::None,
        }) == Err(asb_tui::adapter_catalog::SelectionError::UnavailableModel(
            "development authentication unavailable".into()
        ))
    );
}

#[test]
fn matrix_covers_every_development_tuple_and_opendesk_defaults_offline() {
    let mut matrix = CompatibilityMatrix::development();
    let tuples = matrix
        .evaluate()
        .into_iter()
        .map(|case| {
            (
                case.adapter_id,
                case.provider_id,
                case.model_id,
                case.supported,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(tuples.len(), 6);
    assert!(tuples.iter().all(|(_, _, _, supported)| *supported));
    for expected in [
        ("opencode", "openai", "gpt-4o"),
        ("opencode", "openai", "fixture-model"),
        ("opencode", "openrouter", "openai/gpt-4o"),
        ("opendesk", "openai", "gpt-4o"),
        ("opendesk", "openai", "fixture-model"),
        ("opendesk", "local", "fixture-model"),
    ] {
        assert!(tuples.iter().any(|(adapter, provider, model, _)| {
            (adapter.as_str(), provider.as_str(), model.as_str()) == expected
        }));
    }

    let default = AdapterSelection {
        adapter_id: "opendesk".into(),
        provider_id: "local".into(),
        model_id: "fixture-model".into(),
        auth: AuthMethod::None,
    };
    matrix.set_default("agent-b", default.clone()).unwrap();
    assert_eq!(matrix.resolve("agent-b"), Some(&default));
    assert!(matrix.validate_offline(default).is_ok());
    matrix.restart();
    assert_eq!(
        matrix.resolve("agent-b"),
        Some(&AdapterSelection {
            adapter_id: "opendesk".into(),
            provider_id: "local".into(),
            model_id: "fixture-model".into(),
            auth: AuthMethod::None,
        })
    );
}

#[test]
fn emit_development_matrix_catalog_for_external_acceptance_runner() {
    let catalog = CompatibilityMatrix::development();
    let tuples = catalog
        .evaluate()
        .into_iter()
        .map(|case| {
            serde_json::json!({
                "agent": case.adapter_id,
                "provider": case.provider_id,
                "model": case.model_id,
                "supported": case.supported,
                "reason": case.reason,
            })
        })
        .collect::<Vec<_>>();
    println!(
        "AR1657_CATALOG_JSON={}",
        serde_json::to_string(&tuples).unwrap()
    );
}
