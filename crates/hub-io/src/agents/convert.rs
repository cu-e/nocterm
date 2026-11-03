//! Hub listings as the model's agents.

use std::collections::HashMap;

use nocterm_cloud_agents_model::{
    AgentId, AgentLimits, AgentScope, AgentStatus, CloudAgent, PoolId, PoolLabel,
};
use nocterm_hub_model::agents::{AgentListing, AgentScope as WireScope, Availability};

/// What the heading of an organization the hub did not name says.
const UNNAMED_ORG: &str = "Organization";

pub(super) fn agent(
    listing: AgentListing,
    org_name: Option<String>,
    pool_names: &HashMap<String, String>,
) -> Option<CloudAgent> {
    let scope = match listing.scope {
        WireScope::Account => AgentScope::Personal,
        // The hub names the organization; without a name the person sees a plain label, never
        // the identifier.
        WireScope::Org(org) => AgentScope::Org {
            id: org.to_string(),
            name: org_name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| UNNAMED_ORG.to_owned()),
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
        provider: super::provider::canonical(&listing.provider_kind),
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

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_hub_model::OrgId;
    use nocterm_hub_model::agents::{AgentAccess, ResourceRequest};

    fn listing(scope: WireScope) -> AgentListing {
        AgentListing {
            id: nocterm_hub_model::AgentId::new(),
            scope,
            version: 1,
            name: "Scribe".into(),
            description: String::new(),
            icon: String::new(),
            color: String::new(),
            provider_kind: "openai_chat".into(),
            models: vec!["gpt".into()],
            resources: ResourceRequest {
                cpu_milli: 500,
                mem_mib: 256,
                disk_mib: 512,
                max_parallel_runs: 1,
            },
            idle_ttl_s: 600,
            access: AgentAccess::All,
            allow_personal_on_pool: false,
            enabled: true,
            pools: Vec::new(),
            availability: Availability::Ready,
        }
    }

    fn scope_name(listed: &CloudAgent) -> String {
        match &listed.scope {
            AgentScope::Org { name, .. } => name.clone(),
            other => panic!("not an organization: {other:?}"),
        }
    }

    #[test]
    fn an_organization_agent_shows_the_name_the_hub_sent() {
        let org = OrgId::new();
        let named = agent(
            listing(WireScope::Org(org)),
            Some("Acme".into()),
            &HashMap::new(),
        )
        .expect("agent");
        assert_eq!(scope_name(&named), "Acme");
    }

    #[test]
    fn an_organization_without_a_name_shows_a_plain_label_not_its_identifier() {
        let org = OrgId::new();
        for missing in [None, Some("  ".to_owned())] {
            let unnamed =
                agent(listing(WireScope::Org(org)), missing, &HashMap::new()).expect("agent");
            assert_eq!(scope_name(&unnamed), UNNAMED_ORG);
            assert!(matches!(&unnamed.scope, AgentScope::Org { id, .. } if *id == org.to_string()));
        }
    }
}
