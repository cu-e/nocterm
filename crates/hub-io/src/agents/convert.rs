//! Hub listings as the model's agents.

use std::collections::HashMap;

use nocterm_cloud_agents_model::{
    AgentId, AgentLimits, AgentScope, AgentStatus, CloudAgent, PoolId, PoolLabel,
};
use nocterm_hub_model::agents::{AgentListing, AgentScope as WireScope, Availability};

pub(super) fn agent(
    listing: AgentListing,
    pool_names: &HashMap<String, String>,
) -> Option<CloudAgent> {
    let scope = match listing.scope {
        WireScope::Account => AgentScope::Personal,
        WireScope::Org(org) => AgentScope::Org {
            id: org.to_string(),
            name: org.to_string(),
        },
        WireScope::Instance => AgentScope::Server,
    };
    let pools = listing
        .pools
        .iter()
        .filter_map(|pool| {
            let id = pool.to_string();
            let name = pool_names.get(&id).cloned().unwrap_or_else(|| id.clone());
            Some(PoolLabel {
                id: PoolId::new(id).ok()?,
                name,
            })
        })
        .collect();
    Some(CloudAgent {
        id: AgentId::new(listing.id.to_string()).ok()?,
        scope,
        version: listing.version,
        name: listing.name,
        description: listing.description,
        icon: listing.icon,
        provider: listing.provider_kind,
        models: listing.models,
        limits: AgentLimits {
            cpu_milli: listing.resources.cpu_milli,
            mem_mib: listing.resources.mem_mib,
            disk_mib: listing.resources.disk_mib,
            max_parallel_runs: listing.resources.max_parallel_runs,
            idle_ttl_s: listing.idle_ttl_s,
        },
        status: if listing.enabled {
            status(listing.availability)
        } else {
            AgentStatus::Disabled
        },
        pools,
    })
}

fn status(availability: Availability) -> AgentStatus {
    match availability {
        Availability::Ready => AgentStatus::Ready,
        Availability::Sleeping => AgentStatus::Sleeping,
        Availability::Starting => AgentStatus::Starting,
        Availability::NoCapacity => AgentStatus::NoCapacity,
        Availability::NeedsKey => AgentStatus::NeedsKey,
        Availability::NeedsPoolTrust => AgentStatus::NeedsPoolTrust,
        Availability::Disabled => AgentStatus::Disabled,
    }
}
