//! Keeps the mirror of one hub account in step with the status of the hub.
//!
//! While the device is signed in to the hub the mirror replicates with the account; when the
//! hub signs the device out, revokes it or disappears, the mirror is emptied and the account's
//! vault is locked. Nothing restarts: the entity follows the registry of hubs.
use gpui_kit::{Context, Entity, Subscription};
use nocterm_connections_store::{ProfileReplication, ReplicationEvent, ReplicationStatus};
use nocterm_hubs::{HubStatus, Hubs};
use nocterm_hubs_model::{HubId, source_identity::LocalAccountSourceFactory as _};
use nocterm_vault::VaultAccess;
use std::sync::Arc;

use super::notices::{notify, refusal_message, space_name};

pub(super) struct FollowInputs {
    replica: super::AccountReplicas,
    vault: Arc<dyn VaultAccess>,
    replication: Entity<ProfileReplication>,
    agent_source: Arc<super::agent::AgentSourceAvailability>,
    runtime: nocterm_agent_runtime::RuntimeHandle,
}
impl FollowInputs {
    pub(super) fn new(
        replica: super::AccountReplicas,
        vault: Arc<dyn VaultAccess>,
        replication: Entity<ProfileReplication>,
        agent_source: Arc<super::agent::AgentSourceAvailability>,
        runtime: nocterm_agent_runtime::RuntimeHandle,
    ) -> Self {
        Self {
            replica,
            vault,
            replication,
            agent_source,
            runtime,
        }
    }
}
pub(super) struct Follow {
    hub: HubId,
    hubs: Entity<Hubs>,
    replica: super::AccountReplicas,
    vault: Arc<dyn VaultAccess>,
    replication: Entity<ProfileReplication>,
    attached: bool,
    retired: bool,
    _hubs: Subscription,
    _refusals: Subscription,
    agent_source: Arc<super::agent::AgentSourceAvailability>,
    runtime: nocterm_agent_runtime::RuntimeHandle,
}

impl Follow {
    pub(super) fn new(
        hub: HubId,
        hubs: Entity<Hubs>,
        inputs: FollowInputs,
        cx: &mut Context<Self>,
    ) -> Self {
        let FollowInputs {
            replica,
            vault,
            replication,
            agent_source,
            runtime,
        } = inputs;
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
            retired: false,
            _hubs: observed,
            _refusals: refusals,
            agent_source,
            runtime,
        };
        follow.follow(cx);
        follow
    }

    pub(super) fn retire(&mut self, cx: &mut Context<Self>) {
        self.retired = true;
        let lock_original = self.agent_source.native_current();
        if self.agent_source.refresh(false) {
            self.runtime.refresh_environment_sources(cx);
        }
        if lock_original {
            self.vault.lock();
        }
        if self.attached {
            self.detach_mirror(cx);
        }
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        if self.retired {
            return;
        }
        let status = self.hubs.read(cx).status(&self.hub);
        let allowed = !matches!(
            status,
            None | Some(HubStatus::SignedOut | HubStatus::Revoked)
        );
        let lock_original = !allowed && self.agent_source.native_current();
        let changed = self.agent_source.refresh(allowed);
        if changed && self.attached {
            self.detach_mirror(cx);
        }
        if changed {
            self.runtime.refresh_environment_sources(cx);
        }
        if lock_original {
            self.vault.lock();
        }
        match status {
            Some(HubStatus::Online | HubStatus::Connecting | HubStatus::Offline { .. }) => {
                if self.agent_source.native_current() && !self.attached {
                    self.attach(cx);
                } else if self.agent_source.native_current()
                    && matches!(
                        self.replication.read(cx).status(),
                        ReplicationStatus::Locked
                    )
                {
                    // The keys of the account may have opened since the last round.
                    self.attach(cx);
                }
            }
            None | Some(HubStatus::SignedOut | HubStatus::Revoked) => {
                if self.attached {
                    self.detach_mirror(cx);
                }
            }
            Some(HubStatus::TrustFailed { .. } | HubStatus::UpgradeRequired { .. }) => {}
        }
    }

    fn attach(&mut self, cx: &mut Context<Self>) {
        let captured = self
            .agent_source
            .capture()
            .and_then(|source| (self.replica)(source));
        match captured {
            Ok(replica) => {
                self.replication
                    .update(cx, |replication, cx| replication.attach(replica, cx));
                self.attached = true;
            }
            Err(error) => {
                if self.attached {
                    self.detach_mirror(cx);
                }
                tracing::debug!(%error, "the original native replica is unavailable");
            }
        }
    }

    /// Nothing of the account stays: not its profiles, not an open vault.
    fn detach_mirror(&mut self, cx: &mut Context<Self>) {
        self.replication
            .update(cx, |replication, cx| replication.detach(cx));
        self.attached = false;
    }
}
