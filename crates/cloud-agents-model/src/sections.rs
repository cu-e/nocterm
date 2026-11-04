//! How the Cloud tab groups, filters and orders agents.

use nocterm_hubs_model::{HubColor, HubId};

use crate::agent::CloudAgent;
use crate::scope::AgentScope;

/// A hub and the agents it listed for the signed-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubAgents {
    /// The hub.
    pub hub: HubId,
    /// The name the person gave the hub.
    pub name: String,
    /// The colour that marks the hub.
    pub color: HubColor,
    /// Its agents, in any order.
    pub agents: Vec<CloudAgent>,
}

/// The heading of a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionTitle {
    /// The person's own agents.
    Personal,
    /// An organization, by name.
    Org(String),
    /// The server's agents, by the hub's name.
    Server(String),
}

impl SectionTitle {
    /// The heading text.
    pub fn text(&self) -> String {
        match self {
            Self::Personal => "Personal".to_owned(),
            Self::Org(name) => name.clone(),
            Self::Server(hub) => format!("Server {hub}"),
        }
    }
}

/// Agents of one scope of one hub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// The hub they belong to.
    pub hub: HubId,
    /// The name the person gave the hub.
    pub hub_name: String,
    /// The hub's colour.
    pub color: HubColor,
    /// The heading.
    pub title: SectionTitle,
    /// The scope, for actions on the section.
    pub scope: AgentScope,
    /// The agents, usable ones first, then by name.
    pub agents: Vec<CloudAgent>,
}

/// Sections for the tab of a hub window: Personal, organizations by name, then the server;
/// empty sections are left out. `query` filters by [`CloudAgent::matches`]. The window of this
/// computer shows no hub's agents (ADR 0130).
pub fn arrange(hub: &HubAgents, query: &str) -> Vec<Section> {
    let mut scopes: Vec<&AgentScope> = Vec::new();
    for agent in &hub.agents {
        if !scopes.contains(&&agent.scope) {
            scopes.push(&agent.scope);
        }
    }
    scopes.sort_by(|a, b| a.display_order(b));
    scopes
        .into_iter()
        .filter_map(|scope| {
            let mut agents: Vec<CloudAgent> = hub
                .agents
                .iter()
                .filter(|agent| agent.scope == *scope && agent.matches(query))
                .cloned()
                .collect();
            if agents.is_empty() {
                return None;
            }
            agents.sort_by(|a, b| {
                a.status
                    .rank()
                    .cmp(&b.status.rank())
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                    .then_with(|| a.id.cmp(&b.id))
            });
            Some(Section {
                hub: hub.hub.clone(),
                hub_name: hub.name.clone(),
                color: hub.color,
                title: match scope {
                    AgentScope::Personal => SectionTitle::Personal,
                    AgentScope::Org { name, .. } => SectionTitle::Org(name.clone()),
                    AgentScope::Server => SectionTitle::Server(hub.name.clone()),
                },
                scope: scope.clone(),
                agents,
            })
        })
        .collect()
}
