//! Which agents the user can start now, and from which source.
//!
//! Owns: `AgentsHandle`, the one resolution of the injected `AgentCatalog`
//! and the `ai` settings into `AvailableAgents` that every consumer reads.
//! Does not own: agent processes (`nocterm-agent-runtime`), profile and
//! resolution rules (`nocterm-ai`), the built-in profiles (`nocterm-agent-providers`) or views.
//! Ports used: SettingsHandle. Ports provided: AgentsHandle.
//! Settings section: reads `ai`. Globals: none.
//! Invariants: consumers see the agents of the current settings revision;
//! nothing outside this crate resolves settings into agents.
use gpui::App;
use nocterm_ai::{
    AgentCatalog, AiSettings, AvailableAgents,
    agents::{AgentProfile, AvailableAgent},
};
use nocterm_store::SettingsHandle;

/// Explicit access to the available agents, retained by the application.
#[derive(Clone)]
pub struct AgentsHandle {
    catalog: AgentCatalog,
    settings: SettingsHandle,
}

impl AgentsHandle {
    pub fn new(catalog: AgentCatalog, settings: SettingsHandle, cx: &mut App) -> Self {
        settings.register::<AiSettings>(cx);
        Self { catalog, settings }
    }

    /// The profiles nocterm ships, whether or not settings enable them.
    pub fn catalog(&self) -> &AgentCatalog {
        &self.catalog
    }

    /// The agents the current settings allow, with skipped configurations.
    pub fn available(&self, cx: &App) -> AvailableAgents {
        AvailableAgents::resolve(&self.catalog, self.settings.setting::<AiSettings>(cx))
    }

    pub fn get(&self, id: &str, cx: &App) -> Option<AvailableAgent> {
        self.available(cx).get(id).cloned()
    }

    /// The profile of agent `id`: its own when available, else the catalog's.
    pub fn profile(&self, id: &str, cx: &App) -> Option<AgentProfile> {
        match self.available(cx).get(id) {
            Some(agent) => agent.profile().copied(),
            None => self.catalog.get(id).copied(),
        }
    }
}

#[cfg(test)]
mod tests;
