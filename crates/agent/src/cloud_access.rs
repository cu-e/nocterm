//! The Cloud agents a window offers: the shared service, the hubs that name them, and which
//! hubs this window shows.
use std::sync::Arc;

use gpui_kit::{App, Entity};
use nocterm_cloud_agents::CloudAgents;
use nocterm_cloud_agents_model::{Audience, CloudChatLauncher, HubAgents, Section, arrange};
use nocterm_hubs::{HubStatus, Hubs};
use nocterm_hubs_model::{HubColor, HubId};

/// What the new-chat menu needs to list, start and manage Cloud agents. Cheap to clone; the
/// entities are the application's shared ones, created in the composition root.
#[derive(Clone)]
pub struct CloudAgentsAccess {
    agents: Entity<CloudAgents>,
    hubs: Entity<Hubs>,
    audience: Audience,
    chats: Option<Arc<dyn CloudChatLauncher>>,
}

impl CloudAgentsAccess {
    /// The access of the window of this computer: every signed-in hub.
    pub fn new(agents: Entity<CloudAgents>, hubs: Entity<Hubs>) -> Self {
        Self {
            agents,
            hubs,
            audience: Audience::AllHubs,
            chats: None,
        }
    }

    /// The same access with the port that creates, lists and opens Cloud chats.
    #[must_use]
    pub fn with_chats(self, chats: Arc<dyn CloudChatLauncher>) -> Self {
        Self {
            chats: Some(chats),
            ..self
        }
    }

    pub(crate) fn chats(&self) -> Option<&Arc<dyn CloudChatLauncher>> {
        self.chats.as_ref()
    }

    /// The access of a window of one hub.
    pub fn for_hub_window(&self, hub: &HubId) -> Self {
        self.for_hub(hub)
    }

    pub(crate) fn for_hub(&self, hub: &HubId) -> Self {
        Self {
            audience: Audience::Hub(hub.clone()),
            ..self.clone()
        }
    }

    /// The shared service, for the other surfaces that manage the same agents.
    pub fn service(&self) -> &Entity<CloudAgents> {
        &self.agents
    }

    pub(crate) fn hubs(&self) -> &Entity<Hubs> {
        &self.hubs
    }

    /// The hubs this window shows that are signed in and answer the catalog.
    pub(crate) fn visible_hubs(&self, cx: &App) -> Vec<(HubId, String, HubColor)> {
        let service = self.agents.read(cx);
        self.hubs
            .read(cx)
            .list()
            .into_iter()
            .filter(|hub| match &self.audience {
                Audience::Hub(only) => *only == hub.id,
                Audience::AllHubs => true,
            })
            .filter(|hub| !matches!(hub.status, HubStatus::SignedOut | HubStatus::Revoked))
            .filter(|hub| service.is_offered(&hub.id))
            .map(|hub| (hub.id, hub.label, hub.color.unwrap_or(HubColor::Blue)))
            .collect()
    }

    /// Whether the Cloud tab is shown at all: a hub answered UNIMPLEMENTED or none is signed in
    /// otherwise.
    pub(crate) fn offered(&self, cx: &App) -> bool {
        !self.visible_hubs(cx).is_empty()
    }

    /// The sections for `query`, grouped by hub.
    pub(crate) fn sections(&self, query: &str, cx: &App) -> Vec<Section> {
        let service = self.agents.read(cx);
        let hubs: Vec<HubAgents> = self
            .visible_hubs(cx)
            .into_iter()
            .map(|(hub, name, color)| HubAgents {
                agents: service.agents(&hub).to_vec(),
                hub,
                name,
                color,
            })
            .collect();
        arrange(&hubs, &self.audience, query)
    }
}
