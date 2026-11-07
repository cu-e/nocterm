//! The ports the service needs and the handle to the installed entity.

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity};
use nocterm_hub_agents_model::{HubAgentDirectory, HubChatChanges, HubServers};
use nocterm_ssh_keys::authorized_keys::AgentHosts;

use crate::service::HubAgents;

/// The agents ports, supplied by the composition root.
#[derive(Clone)]
pub struct Deps {
    pub directory: Arc<dyn HubAgentDirectory>,
    pub servers: Arc<dyn HubServers>,
    pub chats: Arc<dyn HubChatChanges>,
    /// The sessions of saved connections, through which Agent access edits `authorized_keys`.
    pub hosts: Arc<dyn AgentHosts>,
}

/// The entity shared by every window, kept alive by whoever composes the application.
#[derive(Clone)]
pub struct Installed {
    agents: Entity<HubAgents>,
}

impl Installed {
    pub fn agents(&self) -> &Entity<HubAgents> {
        &self.agents
    }
}

/// Creates the service. It tracks no hub until [`HubAgents::track`] is called for it.
pub fn install(deps: Deps, cx: &mut App) -> Installed {
    Installed {
        agents: cx.new(|_| HubAgents::new(deps)),
    }
}
