// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

//! Renderer-neutral, catalog-bound local-agent lifecycle state.
//!
//! Effects are represented as events for the authenticated ASB control client.
//! This module never installs, removes, launches, or renders anything.

use crate::agent_catalog::{AgentAvailability, AgentCatalog, AgentCatalogEntry};

pub const MAX_REASON_BYTES: usize = 128;
pub const MAX_PROGRESS_UNITS: u64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleState {
    Disconnected,
    Ready {
        catalog: AgentCatalog,
    },
    Refreshing {
        generation: u64,
    },
    Installing {
        agent_id: String,
        generation: u64,
        completed: u64,
        total: u64,
    },
    Installed {
        agent_id: String,
        generation: u64,
    },
    Cancelling {
        agent_id: String,
        generation: u64,
    },
    Removing {
        agent_id: String,
        generation: u64,
    },
    Reconnecting {
        generation: u64,
    },
    Stale {
        generation: u64,
    },
    Failed {
        generation: u64,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    RefreshRequested,
    CatalogAccepted(AgentCatalog),
    InstallRequested {
        agent_id: String,
        total: u64,
    },
    Progress {
        generation: u64,
        completed: u64,
        total: u64,
    },
    InstallSucceeded {
        generation: u64,
    },
    InstallFailed {
        generation: u64,
        reason: String,
    },
    CancelRequested,
    Cancelled {
        generation: u64,
    },
    RemoveRequested,
    RemoveSucceeded {
        generation: u64,
    },
    ReconnectRequested,
    Reconnected(AgentCatalog),
    RetryRequested,
}

impl LifecycleState {
    pub fn disconnected() -> Self {
        Self::Disconnected
    }

    pub fn apply(self, event: LifecycleEvent) -> Result<Self, String> {
        match (self, event) {
            (Self::Disconnected, LifecycleEvent::RefreshRequested) => {
                Ok(Self::Refreshing { generation: 0 })
            }
            (Self::Refreshing { generation }, LifecycleEvent::CatalogAccepted(catalog))
                if generation == 0 || generation == catalog.generation =>
            {
                Ok(Self::Ready { catalog })
            }
            (Self::Refreshing { .. }, LifecycleEvent::CatalogAccepted(catalog)) => {
                Ok(Self::Stale {
                    generation: catalog.generation,
                })
            }
            (Self::Ready { catalog }, LifecycleEvent::InstallRequested { agent_id, total }) => {
                let agent = find_available(&catalog, &agent_id)?;
                validate_total(total)?;
                Ok(Self::Installing {
                    agent_id: agent.agent_id.clone(),
                    generation: catalog.generation,
                    completed: 0,
                    total,
                })
            }
            (
                Self::Installing {
                    agent_id,
                    generation,
                    completed,
                    total,
                },
                LifecycleEvent::Progress {
                    generation: event_generation,
                    completed: next,
                    total: event_total,
                },
            ) => {
                if generation != event_generation
                    || total != event_total
                    || next < completed
                    || next > total
                {
                    return Err("stale or invalid lifecycle progress".into());
                }
                Ok(Self::Installing {
                    agent_id,
                    generation,
                    completed: next,
                    total,
                })
            }
            (
                Self::Installing {
                    agent_id,
                    generation,
                    ..
                },
                LifecycleEvent::CancelRequested,
            ) => Ok(Self::Cancelling {
                agent_id,
                generation,
            }),
            (
                Self::Cancelling { generation, .. },
                LifecycleEvent::Cancelled {
                    generation: event_generation,
                },
            ) if generation == event_generation => Ok(Self::Disconnected),
            (
                Self::Installing {
                    agent_id,
                    generation,
                    completed,
                    total,
                },
                LifecycleEvent::InstallSucceeded {
                    generation: event_generation,
                },
            ) if generation == event_generation && completed == total => Ok(Self::Installed {
                agent_id,
                generation,
            }),
            (
                Self::Installing { generation, .. },
                LifecycleEvent::InstallFailed {
                    generation: event_generation,
                    reason,
                },
            ) if generation == event_generation => {
                validate_reason(&reason)?;
                Ok(Self::Failed { generation, reason })
            }
            (Self::Failed { generation, .. }, LifecycleEvent::RetryRequested) => {
                Ok(Self::Refreshing { generation })
            }
            (
                Self::Installed {
                    agent_id,
                    generation,
                },
                LifecycleEvent::RemoveRequested,
            ) => Ok(Self::Removing {
                agent_id,
                generation,
            }),
            (
                Self::Removing { generation, .. },
                LifecycleEvent::RemoveSucceeded {
                    generation: event_generation,
                },
            ) if generation == event_generation => Ok(Self::Reconnecting { generation }),
            (Self::Reconnecting { generation }, LifecycleEvent::Reconnected(catalog))
                if catalog.generation == generation =>
            {
                Ok(Self::Ready { catalog })
            }
            (Self::Reconnecting { .. }, LifecycleEvent::Reconnected(catalog)) => Ok(Self::Stale {
                generation: catalog.generation,
            }),
            (Self::Ready { catalog }, LifecycleEvent::ReconnectRequested) => {
                Ok(Self::Reconnecting {
                    generation: catalog.generation,
                })
            }
            (Self::Installed { generation, .. }, LifecycleEvent::ReconnectRequested) => {
                Ok(Self::Reconnecting { generation })
            }
            (Self::Ready { catalog }, LifecycleEvent::RefreshRequested) => Ok(Self::Refreshing {
                generation: catalog.generation,
            }),
            (Self::Installed { generation, .. }, LifecycleEvent::RefreshRequested) => {
                Ok(Self::Refreshing { generation })
            }
            (_, event) => Err(format!(
                "event is invalid for current lifecycle state: {event:?}"
            )),
        }
    }
}

fn find_available<'a>(
    catalog: &'a AgentCatalog,
    id: &str,
) -> Result<&'a AgentCatalogEntry, String> {
    catalog
        .agents
        .iter()
        .find(|agent| agent.agent_id == id)
        .ok_or_else(|| "unknown agent id".into())
        .and_then(|agent| {
            if matches!(agent.availability, AgentAvailability::Available) {
                Ok(agent)
            } else {
                Err("agent is unavailable".into())
            }
        })
}

fn validate_total(total: u64) -> Result<(), String> {
    if (1..=MAX_PROGRESS_UNITS).contains(&total) {
        Ok(())
    } else {
        Err("invalid lifecycle progress total".into())
    }
}

fn validate_reason(reason: &str) -> Result<(), String> {
    if !reason.is_empty()
        && reason.len() <= MAX_REASON_BYTES
        && reason.is_ascii()
        && !reason.bytes().any(|byte| byte.is_ascii_control())
    {
        Ok(())
    } else {
        Err("invalid lifecycle failure reason".into())
    }
}
