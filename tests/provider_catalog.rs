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
    provider_catalog::{AgentScope, ProviderDefaultDraft, accept_generation, wizard_options},
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
            package: AgentPackage {
                package_id: "pkg".into(),
                version: "1.0.0".into(),
                sha256: "b".repeat(64),
                signature_sha256: "c".repeat(64),
                signer: asb_tui::agent_catalog::AgentSigner {
                    key_id: "key-1".into(),
                    principal: "asb-release".into(),
                },
            },
            provenance: AgentProvenance {
                source_revision: "d".repeat(40),
                manifest_sha256: "e".repeat(64),
                sbom_sha256: "f".repeat(64),
                license_ref: "MIT".into(),
            },
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
