// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    agent_catalog::{
        AgentAvailability, AgentCatalog, AgentCatalogEntry, AgentPackage, AgentProvenance,
        AgentTarget,
    },
    control_codec::{
        ProviderAuthMethod, ProviderAvailability, ProviderCatalog, ProviderCatalogEntry,
        ProviderModel, Revision,
    },
    provider_catalog::{
        AgentScope, ProviderDefaultDraft, ProviderDefaultRecord, ProviderDefaultsStore,
        SharedProviderDefaults, accept_generation, wizard_options,
    },
};

fn target() -> AgentTarget {
    AgentTarget {
        operating_system: "linux".into(),
        architecture: "x86_64".into(),
        libc: "glibc".into(),
        libc_version: "2.35".into(),
    }
}

fn agents() -> AgentCatalog {
    AgentCatalog {
        runner_instance_id: "runner-1".into(),
        generation: 1,
        catalog_sha256: "a".repeat(64),
        target: target(),
        agents: vec![AgentCatalogEntry {
            agent_id: "agent-a".into(),
            target: target(),
            package: Some(AgentPackage {
                package_id: "pkg".into(),
                version: "1.0.0".into(),
                sha256: "b".repeat(64),
                signature_sha256: "c".repeat(64),
                signer: asb_tui::agent_catalog::AgentSigner {
                    key_id: "key-1".into(),
                    principal: "asb-release".into(),
                },
            }),
            provenance: Some(AgentProvenance {
                source_revision: "d".repeat(40),
                manifest_sha256: "e".repeat(64),
                sbom_sha256: "f".repeat(64),
                license_ref: "MIT".into(),
            }),
            capabilities: vec!["bench".into()],
            availability: AgentAvailability::Available,
        }],
        refreshed: true,
    }
}

fn providers() -> ProviderCatalog {
    ProviderCatalog {
        runner_instance_id: "runner-1".into(),
        generation: Revision(2),
        catalog_sha256: "f".repeat(64),
        refreshed: true,
        providers: vec![ProviderCatalogEntry {
            provider_id: "provider-a".into(),
            display_name: "Provider A".into(),
            auth_methods: vec![ProviderAuthMethod::CredentialReference],
            availability: ProviderAvailability::Available,
            models: vec![ProviderModel {
                model_id: "model-a".into(),
                revision: "r1".into(),
                availability: ProviderAvailability::Available,
            }],
        }],
    }
}

#[test]
fn scoped_defaults_resolve_all_and_selected_agents_without_secrets() {
    let draft = ProviderDefaultDraft {
        scope: AgentScope::All,
        provider_id: "provider-a".into(),
        model_id: "model-a".into(),
        auth_method: ProviderAuthMethod::CredentialReference,
        credential_reference_sha256: Some("1".repeat(64)),
    };
    assert_eq!(
        draft
            .into_selection(&agents(), &providers())
            .unwrap()
            .agent_ids,
        vec!["agent-a"]
    );
    let selected = AgentScope::selected(["agent-a"]).unwrap();
    assert_eq!(selected.resolve(&agents()).unwrap(), vec!["agent-a"]);
}

#[test]
fn stale_generations_and_unavailable_choices_fail_closed() {
    assert!(accept_generation(Some(Revision(3)), Revision(2)).is_err());
    assert!(accept_generation(Some(Revision(3)), Revision(3)).is_ok());
    let mut catalog = providers();
    catalog.providers[0].availability = ProviderAvailability::Unavailable("not enrolled".into());
    let (provider_options, model_options) = wizard_options(&catalog).unwrap();
    assert!(!provider_options[0].available);
    assert!(!model_options[0].available);
}

#[test]
fn scope_rejects_unknown_or_unavailable_agents() {
    let scope = AgentScope::selected(["missing"]).unwrap();
    assert!(scope.resolve(&agents()).is_err());
}

#[test]
fn shared_defaults_apply_is_atomic_for_selected_and_all_agents() {
    let mut defaults = SharedProviderDefaults::default();
    let draft = ProviderDefaultDraft {
        scope: AgentScope::selected(["agent-a"]).unwrap(),
        provider_id: "provider-a".into(),
        model_id: "model-a".into(),
        auth_method: ProviderAuthMethod::CredentialReference,
        credential_reference_sha256: Some("1".repeat(64)),
    };
    defaults.apply(draft, &agents(), &providers()).unwrap();
    let before = defaults.clone();
    let invalid = ProviderDefaultDraft {
        scope: AgentScope::All,
        provider_id: "missing".into(),
        model_id: "model-a".into(),
        auth_method: ProviderAuthMethod::None,
        credential_reference_sha256: None,
    };
    assert!(defaults.apply(invalid, &agents(), &providers()).is_err());
    assert_eq!(defaults, before);
    assert_eq!(
        defaults.for_agent("agent-a").unwrap().provider_id,
        "provider-a"
    );
}

#[test]
fn shared_defaults_restart_persistence_and_rollback_are_safe() {
    let root =
        std::env::temp_dir().join(format!("asb-tui-provider-defaults-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let store = ProviderDefaultsStore::new(root.join("defaults.json"));
    let mut defaults = SharedProviderDefaults::default();
    defaults.entries.push(ProviderDefaultRecord {
        scope: AgentScope::All,
        provider_id: "provider-a".into(),
        model_id: "model-a".into(),
        auth_method: ProviderAuthMethod::CredentialReference,
        credential_reference_sha256: Some("a".repeat(64)),
    });
    store.save(&defaults).unwrap();
    assert_eq!(store.load().unwrap(), defaults);
    let previous = defaults.clone();
    defaults.entries.clear();
    defaults.rollback(previous.clone()).unwrap();
    assert_eq!(defaults, previous);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn diagnostics_are_human_and_json_safe() {
    let mut defaults = SharedProviderDefaults::default();
    defaults.entries.push(ProviderDefaultRecord {
        scope: AgentScope::All,
        provider_id: "provider-a".into(),
        model_id: "model-a".into(),
        auth_method: ProviderAuthMethod::CredentialReference,
        credential_reference_sha256: Some("b".repeat(64)),
    });
    let diagnostic = defaults.diagnostic();
    assert!(diagnostic.to_string().contains("1 provider default"));
    let json = diagnostic.to_json().unwrap();
    assert!(json.contains("sha256:") && json.contains(&"b".repeat(64)));
    assert!(!json.contains("secret") && !json.contains("api_key"));
}

#[test]
fn missing_file_in_existing_private_directory_is_first_run_only() {
    let root = std::env::temp_dir().join(format!(
        "asb-tui-provider-defaults-missing-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let store = ProviderDefaultsStore::new(root.join("defaults.json"));
    assert_eq!(
        store.load_or_default().unwrap(),
        SharedProviderDefaults::default()
    );
    std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    assert!(matches!(
        store.load_or_default(),
        Err(asb_tui::provider_catalog::ProviderDefaultsError::Invalid(message))
            if message.contains("private directory")
    ));
    let missing_parent = ProviderDefaultsStore::new(root.join("missing").join("defaults.json"));
    assert!(missing_parent.load_or_default().is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn save_rejects_symlinked_parent_directory() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "asb-tui-provider-defaults-symlink-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    let real = root.join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = root.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let store = ProviderDefaultsStore::new(link.join("defaults.json"));
    assert!(matches!(
        store.save(&SharedProviderDefaults::default()),
        Err(asb_tui::provider_catalog::ProviderDefaultsError::SymlinkRefused)
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn provider_selection_rejects_invalid_auth_model_and_provider_states() {
    let mut draft = ProviderDefaultDraft {
        scope: AgentScope::All,
        provider_id: "provider-a".into(),
        model_id: "model-a".into(),
        auth_method: ProviderAuthMethod::None,
        credential_reference_sha256: None,
    };
    assert!(
        draft
            .clone()
            .into_selection(&agents(), &providers())
            .is_err()
    );
    draft.auth_method = ProviderAuthMethod::CredentialReference;
    draft.credential_reference_sha256 = None;
    assert!(
        draft
            .clone()
            .into_selection(&agents(), &providers())
            .is_err()
    );
    draft.credential_reference_sha256 = Some("a".repeat(64));
    draft.model_id = "missing".into();
    assert!(
        draft
            .clone()
            .into_selection(&agents(), &providers())
            .is_err()
    );
    draft.model_id = "model-a".into();
    draft.provider_id = "missing".into();
    assert!(draft.into_selection(&agents(), &providers()).is_err());
}

#[test]
fn defaults_validate_auth_references_scopes_and_schema() {
    let mut defaults = SharedProviderDefaults::default();
    defaults.entries.push(ProviderDefaultRecord {
        scope: AgentScope::All,
        provider_id: "provider-a".into(),
        model_id: "model-a".into(),
        auth_method: ProviderAuthMethod::None,
        credential_reference_sha256: Some("a".repeat(64)),
    });
    assert!(defaults.validate().is_err());
    defaults.entries[0].credential_reference_sha256 = None;
    assert!(defaults.validate().is_ok());
    defaults.entries[0].provider_id = "bad/id".into();
    assert!(defaults.validate().is_err());
    defaults.entries[0].provider_id = "provider-a".into();
    defaults.entries[0].auth_method = ProviderAuthMethod::CredentialReference;
    defaults.entries[0].credential_reference_sha256 = Some("not-a-digest".into());
    assert!(defaults.validate().is_err());
    defaults.schema_version = 2;
    assert!(defaults.validate().is_err());
}

#[test]
fn selected_scope_is_sorted_deduplicated_and_bounded() {
    assert_eq!(
        AgentScope::selected(["z", "a", "z"]).unwrap(),
        AgentScope::Selected(vec!["a".into(), "z".into()])
    );
    assert!(AgentScope::selected(std::iter::empty::<&str>()).is_err());
    assert!(AgentScope::selected(["bad/id"]).is_err());
    let too_many = (0..65).map(|index| format!("agent-{index}"));
    assert!(AgentScope::selected(too_many).is_err());
}

#[test]
fn wizard_options_preserve_provider_model_compatibility_and_generation_rules() {
    assert!(accept_generation(None, Revision(0)).is_err());
    assert!(accept_generation(Some(Revision(2)), Revision(1)).is_err());
    assert!(accept_generation(Some(Revision(2)), Revision(2)).is_ok());
    let mut catalog = providers();
    catalog.providers[0].models[0].availability =
        ProviderAvailability::Unavailable("offline".into());
    let (provider_options, model_options) = wizard_options(&catalog).unwrap();
    assert!(provider_options[0].available);
    assert!(!model_options[0].available);
    assert_eq!(model_options[0].compatible_ids(), ["provider-a"]);
}
