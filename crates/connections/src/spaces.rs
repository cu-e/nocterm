//! Where each saved connection lives: the user's own space or an organization's, and
//! what the account may do there.
//!
//! The views read this snapshot instead of the replication, so a device without an account
//! (no spaces at all) behaves exactly as before: everything is personal and editable.
use gpui_kit::{App, Entity};
use nocterm_connections_model::{
    profiles::ProfileId,
    replica::{ReplicaAccess, ReplicaScope, ReplicaSpace},
};
use nocterm_connections_store::ProfileReplication;

use crate::Installed;

#[derive(Clone, Default)]
pub(crate) struct Spaces {
    replication: Option<Entity<ProfileReplication>>,
    spaces: Vec<ReplicaSpace>,
}

impl Spaces {
    pub(crate) fn of(installed: &Installed, cx: &App) -> Self {
        let replication = installed.replication().cloned();
        let spaces = replication
            .as_ref()
            .map(|replication| replication.read(cx).spaces())
            .unwrap_or_default();
        Self {
            replication,
            spaces,
        }
    }

    /// The scope the connection lives in, or will once the requested move is carried out.
    pub(crate) fn scope(&self, id: ProfileId, cx: &App) -> ReplicaScope {
        self.replication
            .as_ref()
            .map_or(ReplicaScope::Personal, |replication| {
                replication.read(cx).space_of(id, cx)
            })
    }

    /// What the account may do in `scope`. An organization the account no longer reaches is
    /// read-only: nothing can be sent to it.
    pub(crate) fn access(&self, scope: &ReplicaScope) -> ReplicaAccess {
        match self.spaces.iter().find(|space| &space.scope == scope) {
            Some(space) => space.access,
            None if *scope == ReplicaScope::Personal => ReplicaAccess::Write,
            None => ReplicaAccess::Read,
        }
    }

    /// Whether the connection may be edited, deleted, moved or renamed.
    pub(crate) fn writable(&self, id: ProfileId, cx: &App) -> bool {
        self.access(&self.scope(id, cx)) == ReplicaAccess::Write
    }

    /// The organizations the account sees, in the order the replica lists them.
    pub(crate) fn organizations(&self) -> impl Iterator<Item = &ReplicaSpace> {
        self.spaces
            .iter()
            .filter(|space| space.scope != ReplicaScope::Personal)
    }

    /// Whether any organization shares connections with the account.
    pub(crate) fn shared(&self) -> bool {
        self.organizations().next().is_some()
    }

    /// The spaces a connection can be put in: the personal one and every organization the
    /// account writes to. A single choice means there is nothing to choose.
    pub(crate) fn choices(&self) -> Vec<ReplicaSpace> {
        let personal = ReplicaSpace {
            scope: ReplicaScope::Personal,
            name: "Personal".to_owned(),
            access: ReplicaAccess::Write,
        };
        std::iter::once(personal)
            .chain(
                self.organizations()
                    .filter(|space| space.access == ReplicaAccess::Write)
                    .cloned(),
            )
            .collect()
    }

    /// Asks for the connection to live in `scope`; it moves with the next synchronization.
    pub(crate) fn place(&self, id: ProfileId, scope: ReplicaScope, cx: &mut App) {
        if let Some(replication) = &self.replication {
            replication.update(cx, |replication, cx| replication.place(id, scope, cx));
        }
    }
}
