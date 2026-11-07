//! Where each saved connection lives: the user's own space or an organization's, and
//! what the account may do there.
//!
//! The views read this snapshot instead of the replication, so a device without an account
//! (no spaces at all) behaves exactly as before: everything is personal and editable.
use gpui_kit::{App, Entity};
use nocterm_connections_model::{
    profiles::{ProfileId, Profiles},
    replica::{ReplicaAccess, ReplicaScope, ReplicaSpace},
};
use nocterm_connections_store::{ProfileReplication, ReplicationStatus};

use crate::{
    Installed,
    panel::account_state::{self, AccountNotice},
};

#[derive(Clone, Default)]
pub(crate) struct Spaces {
    replication: Option<Entity<ProfileReplication>>,
    spaces: Vec<ReplicaSpace>,
    /// Where synchronization stands; none without an account.
    status: Option<ReplicationStatus>,
}

impl Spaces {
    pub(crate) fn of(installed: &Installed, cx: &App) -> Self {
        let replication = installed.replication().cloned();
        let spaces = replication
            .as_ref()
            .map(|replication| replication.read(cx).spaces())
            .unwrap_or_default();
        let status = replication
            .as_ref()
            .map(|replication| replication.read(cx).status().clone());
        Self {
            replication,
            spaces,
            status,
        }
    }

    /// Whether the list may be changed at all: always without an account, and with one only
    /// while the copy is in step with it, because a change that stays here is lost.
    pub(crate) fn editable(&self) -> bool {
        self.status.as_ref().is_none_or(account_state::editable)
    }

    /// What to tell the user about the account above the list.
    pub(crate) fn notice(&self) -> Option<AccountNotice> {
        self.status.as_ref().and_then(account_state::notice)
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
        self.editable() && self.access(&self.scope(id, cx)) == ReplicaAccess::Write
    }

    /// Whether the connection is the user's own, filed in folders, rather than an
    /// organization's. A connection of an organization this account does not reach now is not
    /// personal: it is read-only until the organization is back.
    pub(crate) fn filed_personally(&self, id: ProfileId, cx: &App) -> bool {
        self.scope(id, cx) == ReplicaScope::Personal
    }

    /// The user's own connections in folder `group` of `profiles`. A folder is listed only
    /// among them, so what it does reaches no connection of an organization that uses the
    /// same name.
    pub(crate) fn folder_members(
        &self,
        profiles: &Profiles,
        group: &str,
        cx: &App,
    ) -> Vec<ProfileId> {
        profiles
            .in_group(Some(group), "")
            .iter()
            .map(|profile| profile.id)
            .filter(|id| self.filed_personally(*id, cx))
            .collect()
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
}
