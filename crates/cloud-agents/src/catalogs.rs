//! The agents each tracked hub listed, and how fresh they are.

use std::collections::{BTreeMap, BTreeSet};

use gpui::Task;
use nocterm_cloud_agents_model::{AgentId, CloudAgent};
use nocterm_hubs_model::HubId;

/// How a hub's catalog stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogStatus {
    /// Waiting for the first list.
    Loading,
    /// Listed; kept fresh by the hub's change hints.
    Ready,
    /// The hub offers no managed agents; the tab is hidden for it.
    Unsupported,
    /// The last call failed; it is retried.
    Failed,
}

struct Catalog {
    agents: Vec<CloudAgent>,
    status: CatalogStatus,
    /// The web panel's address as the hub states it.
    web_url: Option<String>,
    /// The agents the person has a seat of on the hub.
    seated: BTreeSet<AgentId>,
    /// The watcher; dropping it stops the hub's tracking.
    _watcher: Task<()>,
}

/// The tracked hubs.
#[derive(Default)]
pub(crate) struct Catalogs {
    hubs: BTreeMap<HubId, Catalog>,
}

impl Catalogs {
    pub(crate) fn contains(&self, hub: &HubId) -> bool {
        self.hubs.contains_key(hub)
    }

    pub(crate) fn insert(&mut self, hub: HubId, watcher: Task<()>) {
        self.hubs.insert(
            hub,
            Catalog {
                agents: Vec::new(),
                status: CatalogStatus::Loading,
                web_url: None,
                seated: BTreeSet::new(),
                _watcher: watcher,
            },
        );
    }

    pub(crate) fn remove(&mut self, hub: &HubId) -> bool {
        self.hubs.remove(hub).is_some()
    }

    pub(crate) fn agents(&self, hub: &HubId) -> &[CloudAgent] {
        self.hubs.get(hub).map_or(&[], |catalog| &catalog.agents)
    }

    pub(crate) fn web_url(&self, hub: &HubId) -> Option<&str> {
        self.hubs.get(hub)?.web_url.as_deref()
    }

    /// Records the address the hub states for its web panel. Returns whether it changed.
    pub(crate) fn stated_web_url(&mut self, hub: &HubId, url: String) -> bool {
        let Some(catalog) = self.hubs.get_mut(hub) else {
            return false;
        };
        let url = Some(url);
        let changed = catalog.web_url != url;
        catalog.web_url = url;
        changed
    }

    /// Whether the person has a seat of the agent on the hub.
    pub(crate) fn has_seat(&self, hub: &HubId, agent: &AgentId) -> bool {
        self.hubs
            .get(hub)
            .is_some_and(|catalog| catalog.seated.contains(agent))
    }

    /// Records the agents the hub lists a seat of. Returns whether that changed.
    pub(crate) fn seats_listed(&mut self, hub: &HubId, agents: Vec<AgentId>) -> bool {
        let Some(catalog) = self.hubs.get_mut(hub) else {
            return false;
        };
        let agents: BTreeSet<AgentId> = agents.into_iter().collect();
        let changed = catalog.seated != agents;
        catalog.seated = agents;
        changed
    }

    /// Records a seat the person gained (`true`) or gave up (`false`). Returns whether that
    /// changed.
    pub(crate) fn seat_changed(&mut self, hub: &HubId, agent: &AgentId, held: bool) -> bool {
        let Some(catalog) = self.hubs.get_mut(hub) else {
            return false;
        };
        if held {
            catalog.seated.insert(agent.clone())
        } else {
            catalog.seated.remove(agent)
        }
    }

    pub(crate) fn status(&self, hub: &HubId) -> Option<CatalogStatus> {
        self.hubs.get(hub).map(|catalog| catalog.status)
    }

    /// Records a list. Returns whether anything a view shows changed.
    pub(crate) fn listed(&mut self, hub: &HubId, agents: Vec<CloudAgent>) -> bool {
        let Some(catalog) = self.hubs.get_mut(hub) else {
            return false;
        };
        let changed = catalog.agents != agents || catalog.status != CatalogStatus::Ready;
        catalog.agents = agents;
        catalog.status = CatalogStatus::Ready;
        changed
    }

    /// Records a failure; an unsupported hub keeps no agents.
    pub(crate) fn settled(&mut self, hub: &HubId, status: CatalogStatus) -> bool {
        let Some(catalog) = self.hubs.get_mut(hub) else {
            return false;
        };
        let changed = catalog.status != status;
        catalog.status = status;
        if status == CatalogStatus::Unsupported {
            catalog.agents.clear();
        }
        changed
    }
}
