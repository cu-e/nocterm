//! The Cloud agents a window offers: the shared service, the hubs that name them, and the hub
//! of the window. Only a hub window offers Cloud agents; the window of this computer shows none
//! of a hub's data (ADR 0130).
use std::sync::Arc;

use gpui_kit::{App, Entity};
use nocterm_cloud_agents::CloudAgents;
use nocterm_cloud_agents_model::{CloudChatLauncher, HubAgents, Section, arrange};
use nocterm_hubs::{HubStatus, Hubs};
use nocterm_hubs_model::{HubColor, HubId};

/// What the new-chat menu needs to list, start and manage Cloud agents. Cheap to clone; the
/// entities are the application's shared ones, created in the composition root.
#[derive(Clone)]
pub struct CloudAgentsAccess {
    agents: Entity<CloudAgents>,
    hubs: Entity<Hubs>,
    /// The hub of the window; without one the window offers no Cloud agents.
    hub: Option<HubId>,
    chats: Option<Arc<dyn CloudChatLauncher>>,
}

impl CloudAgentsAccess {
    /// The installation-wide access. It offers nothing until it is bound to the window of a
    /// hub with [`Self::for_hub_window`].
    pub fn new(agents: Entity<CloudAgents>, hubs: Entity<Hubs>) -> Self {
        Self {
            agents,
            hubs,
            hub: None,
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

    /// The access of a window of one hub: it offers that hub's Cloud agents.
    pub fn for_hub_window(&self, hub: &HubId) -> Self {
        Self {
            hub: Some(hub.clone()),
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

    /// The hub of the window while it is signed in and answers the catalog.
    pub(crate) fn shown_hub(&self, cx: &App) -> Option<HubId> {
        self.shown(cx).map(|hub| hub.hub)
    }

    /// The window's hub with its name and colour and no agents yet, while it is shown.
    fn shown(&self, cx: &App) -> Option<HubAgents> {
        let id = self.hub.as_ref()?;
        if !self.agents.read(cx).is_offered(id) {
            return None;
        }
        self.hubs
            .read(cx)
            .list()
            .into_iter()
            .find(|hub| hub.id == *id)
            .filter(|hub| !matches!(hub.status, HubStatus::SignedOut | HubStatus::Revoked))
            .map(|hub| HubAgents {
                hub: hub.id,
                name: hub.label,
                color: hub.color.unwrap_or(HubColor::Blue),
                agents: Vec::new(),
            })
    }

    /// Whether the Cloud tab is shown at all: only in the window of a signed-in hub that
    /// answers the catalog (a hub that does not know agents answers UNIMPLEMENTED).
    pub(crate) fn offered(&self, cx: &App) -> bool {
        self.shown_hub(cx).is_some()
    }

    /// The sections for `query`.
    pub(crate) fn sections(&self, query: &str, cx: &App) -> Vec<Section> {
        let Some(mut hub) = self.shown(cx) else {
            return Vec::new();
        };
        hub.agents = self.agents.read(cx).agents(&hub.hub).to_vec();
        arrange(&hub, query)
    }
}
