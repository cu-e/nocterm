//! The hubs installation windows are composed from.

use gpui_kit::{App, Entity};
use nocterm_hubs::Hubs;
use nocterm_hubs_model::HubId;

/// The registry of known hubs, kept by the application and given to each window.
#[derive(Clone)]
pub struct Installed {
    hubs: nocterm_hubs::Installed,
    hub_agents: Option<Entity<nocterm_hub_agents::HubAgents>>,
}

impl Installed {
    /// The name the user gave the server, or `None` when it is not in the list.
    pub fn label(&self, hub: &HubId, cx: &App) -> Option<String> {
        self.hubs()
            .read(cx)
            .summary(hub)
            .map(|summary| summary.label)
    }

    /// The Servers page also manages the account's agents, providers and agent servers
    /// through `agents`.
    pub fn with_hub_agents(mut self, agents: Entity<nocterm_hub_agents::HubAgents>) -> Self {
        self.hub_agents = Some(agents);
        self
    }

    pub(crate) fn hub_agents(&self) -> Option<&Entity<nocterm_hub_agents::HubAgents>> {
        self.hub_agents.as_ref()
    }

    pub(crate) fn hubs(&self) -> &Entity<Hubs> {
        self.hubs.hubs()
    }
}

/// Keeps the service installation for the views of every window.
pub fn install(hubs: nocterm_hubs::Installed) -> Installed {
    Installed {
        hubs,
        hub_agents: None,
    }
}
