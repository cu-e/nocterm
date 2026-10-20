//! Completion only retires its original live-shell lease; observation never guesses exit status.
use nocterm_workspace_ports::LiveCommandLease;
use std::{collections::HashMap, sync::Arc};
#[derive(Default)]
pub(super) struct LiveObservations {
    leases: HashMap<String, Arc<LiveCommandLease>>,
}
impl LiveObservations {
    pub(super) fn install(&mut self, id: String, lease: Arc<LiveCommandLease>) {
        self.leases.insert(id, lease);
    }
    pub(super) fn finish_if_same(&mut self, id: &str, lease: &Arc<LiveCommandLease>) {
        if self
            .leases
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, lease))
        {
            self.leases.remove(id);
        }
    }
    pub(super) fn empty(&self) -> bool {
        self.leases.is_empty()
    }
    pub(super) fn active(&self) -> bool {
        self.leases.values().any(|lease| lease.is_active())
    }
    pub(super) fn revoke_detached(&mut self, attached: &[String]) {
        self.leases.retain(|id, lease| {
            let keep = attached.contains(id);
            if !keep {
                lease.cancel();
            }
            keep && lease.is_active()
        });
    }
    pub(super) fn cancel_all(&mut self) {
        for lease in self.leases.values() {
            lease.cancel();
        }
        self.leases.clear();
    }
}
