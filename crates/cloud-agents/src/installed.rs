//! The ports the service needs and the handle to the installed entity.

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity};
use nocterm_cloud_agents_model::{
    AgentSeats, AgentUsage, ChatSshCertifier, CloudAgentCatalog, CloudAgentLauncher,
    OwnRunnerLauncher, PersonalAgents, PoolPinStore,
};

use crate::service::CloudAgents;

/// The Cloud agents ports, supplied by the composition root.
#[derive(Clone)]
pub struct Deps {
    pub catalog: Arc<dyn CloudAgentCatalog>,
    pub launcher: Arc<dyn CloudAgentLauncher>,
    pub own_runner: Arc<dyn OwnRunnerLauncher>,
    pub personal: Arc<dyn PersonalAgents>,
    pub seats: Arc<dyn AgentSeats>,
    pub usage: Arc<dyn AgentUsage>,
    pub pins: Arc<dyn PoolPinStore>,
    /// Answers the certificate requests of bound Cloud chats (ADR 0139).
    pub certifier: Arc<dyn ChatSshCertifier>,
}

/// The entity shared by every window, kept alive by whoever composes the application.
#[derive(Clone)]
pub struct Installed {
    agents: Entity<CloudAgents>,
}

impl Installed {
    /// The shared service.
    pub fn agents(&self) -> &Entity<CloudAgents> {
        &self.agents
    }
}

/// Creates the service. It tracks no hub until [`CloudAgents::track`] is called for it.
pub fn install(deps: Deps, cx: &mut App) -> Installed {
    Installed {
        agents: cx.new(|_| CloudAgents::new(deps)),
    }
}
