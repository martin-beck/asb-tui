// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral provider catalog and scoped-default helpers.
//!
//! The runner remains authoritative for provider availability and generations.
//! This module only turns an authenticated catalog into bounded wizard choices
//! and validates the scope of a configuration draft.  It never accepts or
//! transports a raw credential.

use crate::{
    agent_catalog::AgentCatalog,
    control_codec::{
        ConfigurationSelection, ProviderAuthMethod, ProviderAvailability, ProviderCatalog,
        ProviderModel, Revision,
    },
    wizard_catalog::WizardOption,
};

pub const MAX_SCOPED_AGENTS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentScope {
    All,
    Selected(Vec<String>),
}

impl AgentScope {
    pub fn selected<I, S>(agents: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut agents: Vec<String> = agents.into_iter().map(Into::into).collect();
        agents.sort();
        agents.dedup();
        if agents.is_empty() || agents.len() > MAX_SCOPED_AGENTS {
            return Err("agent scope must contain between one and 64 agents".into());
        }
        if agents.iter().any(|id| !is_safe_id(id)) {
            return Err("agent scope contains an invalid agent id".into());
        }
        Ok(Self::Selected(agents))
    }

    pub fn resolve(&self, catalog: &AgentCatalog) -> Result<Vec<String>, String> {
        let available: Vec<&str> = catalog
            .agents
            .iter()
            .filter(|entry| {
                matches!(
                    entry.availability,
                    crate::agent_catalog::AgentAvailability::Available
                )
            })
            .map(|entry| entry.agent_id.as_str())
            .collect();
        let ids = match self {
            Self::All => available.into_iter().map(str::to_owned).collect(),
            Self::Selected(ids) => {
                if ids.iter().any(|id| !available.contains(&id.as_str())) {
                    return Err("agent scope contains an unavailable agent".into());
                }
                ids.clone()
            }
        };
        if ids.is_empty() {
            return Err("agent scope has no available agents".into());
        }
        Ok(ids)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDefaultDraft {
    pub scope: AgentScope,
    pub provider_id: String,
    pub model_id: String,
    pub auth_method: ProviderAuthMethod,
    /// Digest of a resolver-owned credential locator; the secret itself is
    /// intentionally not representable here.
    pub credential_reference_sha256: Option<String>,
}

impl ProviderDefaultDraft {
    pub fn into_selection(
        self,
        agents: &AgentCatalog,
        providers: &ProviderCatalog,
    ) -> Result<ConfigurationSelection, String> {
        if self.provider_id.is_empty() || self.model_id.is_empty() {
            return Err("provider and model are required".into());
        }
        let provider = providers
            .providers
            .iter()
            .find(|provider| provider.provider_id == self.provider_id)
            .ok_or_else(|| "provider is not present in the current catalog".to_owned())?;
        if !matches!(provider.availability, ProviderAvailability::Available) {
            return Err("provider is unavailable".into());
        }
        let model = provider
            .models
            .iter()
            .find(|model| model.model_id == self.model_id)
            .ok_or_else(|| "model is not present for the selected provider".to_owned())?;
        if !matches!(model.availability, ProviderAvailability::Available) {
            return Err("model is unavailable".into());
        }
        if !provider.auth_methods.contains(&self.auth_method) {
            return Err("authentication method is not supported by the provider".into());
        }
        if matches!(self.auth_method, ProviderAuthMethod::CredentialReference)
            != self.credential_reference_sha256.is_some()
        {
            return Err(
                "credential reference is required only for credential authentication".into(),
            );
        }
        let agent_ids = self.scope.resolve(agents)?;
        Ok(ConfigurationSelection {
            agent_ids,
            provider_id: self.provider_id,
            model_id: model.model_id.clone(),
            auth_method: self.auth_method,
            credential_reference_sha256: self.credential_reference_sha256,
        })
    }
}

/// Return the newest generation observed by the frontend. Equal generations
/// are allowed (a refresh may simply report unchanged content); older ones
/// must be rejected before they can replace wizard choices.
pub fn accept_generation(current: Option<Revision>, next: Revision) -> Result<(), String> {
    if next.0 == 0 || current.is_some_and(|value| next.0 < value.0) {
        return Err("provider catalog generation is stale".into());
    }
    Ok(())
}

/// Build the wizard's provider/model sections while preserving unavailable
/// entries as disabled choices with an explanatory label.
pub fn wizard_options(
    catalog: &ProviderCatalog,
) -> Result<(Vec<WizardOption>, Vec<WizardOption>), String> {
    let providers = catalog
        .providers
        .iter()
        .map(|provider| {
            let available = matches!(provider.availability, ProviderAvailability::Available);
            WizardOption::new(
                provider.provider_id.clone(),
                if available {
                    provider.display_name.clone()
                } else {
                    format!("{} (unavailable)", provider.display_name)
                },
                available,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let models = catalog
        .providers
        .iter()
        .flat_map(|provider| {
            provider.models.iter().map(move |model| {
                model_option(
                    provider.provider_id.as_str(),
                    model,
                    matches!(provider.availability, ProviderAvailability::Available),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((providers, models))
}

fn model_option(
    provider_id: &str,
    model: &ProviderModel,
    provider_available: bool,
) -> Result<WizardOption, String> {
    let available =
        provider_available && matches!(model.availability, ProviderAvailability::Available);
    WizardOption::new(
        model.model_id.clone(),
        if available {
            model.model_id.clone()
        } else {
            format!("{} (unavailable)", model.model_id)
        },
        available,
    )?
    .compatible_with(vec![provider_id.to_owned()])
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
