//! The hubs installation windows are composed from.

use gpui_kit::{App, Entity};
use nocterm_hubs::Hubs;
use nocterm_hubs_model::HubId;

/// The registry of known hubs, kept by the application and given to each window.
#[derive(Clone)]
pub struct Installed {
    hubs: nocterm_hubs::Installed,
    cloud_agents: Option<Entity<nocterm_cloud_agents::CloudAgents>>,
}

impl Installed {
    /// The name the user gave the server, or `None` when it is not in the list.
    pub fn label(&self, hub: &HubId, cx: &App) -> Option<String> {
        self.hubs()
            .read(cx)
            .summary(hub)
            .map(|summary| summary.label)
    }

    /// The Servers page also manages the person's Cloud agents through `agents`.
    pub fn with_cloud_agents(mut self, agents: Entity<nocterm_cloud_agents::CloudAgents>) -> Self {
        self.cloud_agents = Some(agents);
        self
    }

    pub(crate) fn cloud_agents(&self) -> Option<&Entity<nocterm_cloud_agents::CloudAgents>> {
        self.cloud_agents.as_ref()
    }

    pub(crate) fn hubs(&self) -> &Entity<Hubs> {
        self.hubs.hubs()
    }
}

/// Keeps the service installation for the views of every window.
pub fn install(hubs: nocterm_hubs::Installed) -> Installed {
    Installed {
        hubs,
        cloud_agents: None,
    }
}
