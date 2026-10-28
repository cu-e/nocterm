//! Keeps the mirror of one hub account in step with the status of the hub.
//!
//! While the device is signed in to the hub the mirror replicates with the account; when the
//! hub signs the device out, revokes it or disappears, the mirror is emptied and the account's
//! vault is locked. Nothing restarts: the entity follows the registry of hubs.
use gpui_kit::{Context, Entity, Subscription};
use nocterm_connections_model::replica::ProfileReplica;
use nocterm_connections_store::{ProfileReplication, ReplicationEvent, ReplicationStatus};
use nocterm_hubs::{HubStatus, Hubs};
use nocterm_hubs_model::HubId;
use nocterm_vault::VaultAccess;
use std::sync::Arc;

use super::notices::{notify, refusal_message, space_name};

pub(super) struct Follow {
    hub: HubId,
    hubs: Entity<Hubs>,
    replica: Arc<dyn ProfileReplica>,
    vault: Arc<dyn VaultAccess>,
    replication: Entity<ProfileReplication>,
    attached: bool,
    _hubs: Subscription,
    _refusals: Subscription,
}

impl Follow {
    pub(super) fn new(
        hub: HubId,
        hubs: Entity<Hubs>,
        replica: Arc<dyn ProfileReplica>,
        vault: Arc<dyn VaultAccess>,
        replication: Entity<ProfileReplication>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observed = cx.observe(&hubs, |this, _, cx| this.follow(cx));
        let refusals = cx.subscribe(&replication, |_, replication, event, cx| {
            if let ReplicationEvent::Refused {
                profile,
                scope,
                restored,
            } = event
            {
                let space = space_name(&replication.read(cx).spaces(), scope);
                let message = refusal_message(profile, &space, scope, *restored);
                gpui_kit::App::defer(cx, move |cx| notify(message, cx));
            }
        });
        let mut follow = Self {
            hub,
            hubs,
            replica,
            vault,
            replication,
            attached: false,
            _hubs: observed,
            _refusals: refusals,
        };
        follow.follow(cx);
        follow
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let status = self.hubs.read(cx).status(&self.hub);
        match status {
            Some(HubStatus::Online | HubStatus::Connecting | HubStatus::Offline { .. }) => {
                if !self.attached {
                    self.attach(cx);
                } else if matches!(
                    self.replication.read(cx).status(),
                    ReplicationStatus::Locked
                ) {
                    // The keys of the account may have opened since the last round.
                    self.attach(cx);
                }
            }
            None | Some(HubStatus::SignedOut | HubStatus::Revoked) => {
                if self.attached {
                    self.detach(cx);
                }
            }
            Some(HubStatus::TrustFailed { .. } | HubStatus::UpgradeRequired { .. }) => {}
        }
    }

    fn attach(&mut self, cx: &mut Context<Self>) {
        let replica = Arc::clone(&self.replica);
        self.replication
            .update(cx, |replication, cx| replication.attach(replica, cx));
        self.attached = true;
    }

    /// Nothing of the account stays: not its profiles, not an open vault.
    fn detach(&mut self, cx: &mut Context<Self>) {
        self.replication
            .update(cx, |replication, cx| replication.detach(cx));
        self.vault.lock();
        self.attached = false;
    }
}
