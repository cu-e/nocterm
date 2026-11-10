//! The hub agents a window offers: the shared service and the hub of the window. Only a hub
//! window offers hub agents; the window of this computer shows none of a hub's data.
use gpui_kit::Entity;
use nocterm_hub_agents::HubAgents;
use nocterm_hubs_model::HubId;

/// What the panel of a window needs of the hubs: the agents service, to follow the hub's chats
/// and agents, and the window's hub. Cheap to clone; the entity is the application's own.
#[derive(Clone)]
pub struct HubAgentsAccess {
    agents: Entity<HubAgents>,
    hub: Option<HubId>,
}

impl HubAgentsAccess {
    /// The installation-wide access; it offers nothing until bound to the window of a hub.
    pub fn new(agents: Entity<HubAgents>) -> Self {
        Self { agents, hub: None }
    }

    /// The access of a window of `hub`: it offers that hub's agents.
    pub fn for_hub_window(&self, hub: &HubId) -> Self {
        Self {
            agents: self.agents.clone(),
            hub: Some(hub.clone()),
        }
    }

    /// The shared service, for the other surfaces that manage the same agents.
    pub fn service(&self) -> &Entity<HubAgents> {
        &self.agents
    }

    pub(crate) fn agents(&self) -> &Entity<HubAgents> {
        &self.agents
    }

    /// The hub of the window, if it is a hub window.
    pub(crate) fn hub(&self) -> Option<&HubId> {
        self.hub.as_ref()
    }
}
