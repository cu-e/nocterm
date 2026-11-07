//! Which agents the user can start now, and from which source.
//!
//! Owns: `AgentsHandle`, the one resolution of the injected `AgentCatalog`, the `ai` settings
//! and the agents of the signed-in hubs ([`HubAgentLaunches`]) into `AvailableAgents` that
//! every consumer reads.
//! Does not own: agent processes (`nocterm-agent-runtime`), profile and
//! resolution rules (`nocterm-ai`), the built-in profiles (`nocterm-agent-providers`) or views.
//! Ports used: SettingsHandle, HubAgentLaunches. Ports provided: AgentsHandle.
//! Settings section: reads `ai`. Globals: none.
//! Invariants: consumers see the agents of the current settings revision;
//! nothing outside this crate resolves settings into agents.
use gpui::App;
use nocterm_ai::{
    AgentCatalog, AiSettings, AvailableAgents,
    agents::{AgentLaunch, AgentProfile, AvailableAgent},
};
use nocterm_store::SettingsHandle;
use std::rc::Rc;

/// The agents of the hubs this device is signed in to, as they are now.
pub type HubAgentLaunches = Rc<dyn Fn(&App) -> Vec<AgentLaunch>>;

/// Explicit access to the available agents, retained by the application.
#[derive(Clone)]
pub struct AgentsHandle {
    catalog: AgentCatalog,
    settings: SettingsHandle,
    hubs: Option<HubAgentLaunches>,
}

impl AgentsHandle {
    pub fn new(catalog: AgentCatalog, settings: SettingsHandle, cx: &mut App) -> Self {
        settings.register::<AiSettings>(cx);
        Self {
            catalog,
            settings,
            hubs: None,
        }
    }

    /// These agents and the agents of the signed-in hubs.
    #[must_use]
    pub fn with_hub_agents(mut self, hubs: HubAgentLaunches) -> Self {
        self.hubs = Some(hubs);
        self
    }

    /// The profiles nocterm ships, whether or not settings enable them.
    pub fn catalog(&self) -> &AgentCatalog {
        &self.catalog
    }

    /// The agents the current settings allow, with skipped configurations.
    pub fn available(&self, cx: &App) -> AvailableAgents {
        let settings = self.settings.setting::<AiSettings>(cx);
        let agents = AvailableAgents::resolve(&self.catalog, settings);
        match &self.hubs {
            Some(hubs) if settings.enabled => agents.with_hub_agents(hubs(cx)),
            _ => agents,
        }
    }

    pub fn get(&self, id: &str, cx: &App) -> Option<AvailableAgent> {
        self.available(cx).get(id).cloned()
    }

    /// The profile of agent `id`: its own when available, else the catalog's.
    pub fn profile(&self, id: &str, cx: &App) -> Option<AgentProfile> {
        self.profile_in(&self.available(cx), id)
    }

    /// The profile of agent `id` among agents already resolved by [`Self::available`],
    /// for callers that look up many agents at once.
    pub fn profile_in(&self, available: &AvailableAgents, id: &str) -> Option<AgentProfile> {
        match available.get(id) {
            Some(agent) => agent.profile().copied(),
            None => self.catalog.get(id).copied(),
        }
    }
}

#[cfg(test)]
mod tests;
