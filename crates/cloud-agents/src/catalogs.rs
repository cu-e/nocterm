//! The agents each tracked hub listed, and how fresh they are.

use std::collections::BTreeMap;

use gpui::Task;
use nocterm_cloud_agents_model::CloudAgent;
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
